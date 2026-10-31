//! Paragraph formatting: line breaking, line metrics and horizontal placement.

use hm_model::units::{mul_div, unpack_distance};
use hm_model::{Align, DocInfo, LineSpacingKind, ParaShape, Paragraph, Script};

use crate::heads::{Head, ImageHead};
use crate::measure::{Kind, Measured, advance, measure, natural_advance};
use crate::script::script_of;
use crate::script::{BreakClass, break_class};
use crate::style::Styles;

/// One laid-out line. Horizontal values are relative to the column's left
/// edge; vertical values to the line's top.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Line {
    /// Wire positions `[start, end)`.
    pub start: u32,
    pub end: u32,
    /// Item index range in [`Measured::items`].
    pub first_item: u32,
    pub end_item: u32,
    /// The line's segment in the column: from the left margin to the right
    /// margin, as Hancom stores `horzpos` and `horzsize`.
    pub seg_x: i32,
    pub seg_width: i32,
    /// Where content starts: the segment start plus the indent.
    pub x: i32,
    /// Width available to content.
    pub width: i32,
    /// Natural content width, trailing spaces excluded.
    pub used: i32,
    pub height: i32,
    pub text_height: i32,
    pub baseline: i32,
    /// Space after the line (may be negative).
    pub spacing: i32,
    /// The line ends with a forced line break.
    pub forced: bool,
    pub last: bool,
    /// The line keeps the segment Hancom stored for it: objects it wraps
    /// beside are already accounted for.
    pub stored: bool,
}

impl Line {
    /// Vertical advance to the next line of the same paragraph.
    pub fn advance(&self) -> i32 {
        self.height + self.spacing
    }
}

/// A paragraph head as measured: glyphs drawn before the first line.
#[derive(Clone, Debug, Default)]
pub struct HeadLayout {
    /// `(character, style, x)` with x from the head's start.
    pub glyphs: Vec<(char, u32, i32)>,
    /// Picture head, positioned in a square logical em.
    pub image: Option<ImageHead>,
    /// Width the head takes from the first line, distance to the text included.
    pub width: i32,
    /// Line size of the head's style, which seeds the first line's metrics.
    pub size: i32,
}

/// A formatted paragraph for one column width.
#[derive(Clone, Debug, Default)]
pub struct ParaLayout {
    pub lines: Vec<Line>,
    /// Facts for these line ranges, filled only when body flow needs them.
    /// Clones share initialization; changing line membership replaces this cell.
    pub line_facts: std::sync::Arc<std::sync::OnceLock<crate::table::LineFacts>>,
    pub head: Option<HeadLayout>,
    /// Shared with the copies [`crate::LayoutCache`] keeps for reuse.
    pub measured: std::sync::Arc<Measured>,
    /// Paragraph spacing before the first and after the last line.
    pub before: i32,
    pub after: i32,
    /// The paragraph's right margin.
    pub right: i32,
    pub align: Option<Align>,
    /// Exclude the final forbidden glyph from horizontal alignment.
    pub hang_last_forbidden: bool,
    /// Paragraph vertical alignment: 0 baseline, 1 top, 2 center, 3 bottom.
    pub valign: u32,
    /// The column width and section tab interval this layout was made for.
    pub column_width: i32,
    pub default_tab: i32,
    /// Indent of the lines after the first under an automatically hanging
    /// head.
    pub hang: i32,
    /// Layouts of the tables among the paragraph's controls, by control index.
    /// Shared on cache hits so unchanged tables keep their cell layouts.
    pub tables: Vec<(u32, std::sync::Arc<crate::table::TableLayout>)>,
}

impl ParaLayout {
    pub(crate) fn line_facts(&self, para: &Paragraph) -> &crate::table::LineFacts {
        self.line_facts.get_or_init(|| {
            let mut facts = crate::table::LineFacts::default();
            facts.reset(para, &self.lines);
            facts
        })
    }

    /// The layout of the table at control index `control`.
    pub fn table(&self, control: u32) -> Option<&crate::table::TableLayout> {
        self.tables
            .iter()
            .find(|(k, _)| *k == control)
            .map(|(_, t)| t.as_ref())
    }
}

impl ParaLayout {
    /// Where a run of line size `size` sits on `line`, from the line top:
    /// on the shared baseline, or aligned to the top, middle or bottom of the
    /// line with its own 85 percent baseline.
    pub fn run_baseline(&self, line: &Line, size: i32) -> i32 {
        let own = mul_div(size, 85, 100);
        match self.valign {
            1 => own,
            2 => (line.height - size) / 2 + own,
            3 => line.height - size + own,
            _ => line.baseline,
        }
    }

    /// Where an inline box of height `height` with its baseline `baseline`
    /// below its top sits on `line`: its top from the line top, on the
    /// shared baseline, or aligned to the top, middle or bottom of the line
    /// as runs are.
    pub fn box_top(&self, line: &Line, height: i32, baseline: i32) -> i32 {
        match self.valign {
            1 => 0,
            2 => (line.height - height) / 2,
            3 => line.height - height,
            _ => line.baseline - baseline,
        }
    }

    /// Lines plus their spacing, without paragraph spacing.
    pub fn lines_height(&self) -> i32 {
        self.lines.iter().map(Line::advance).sum()
    }
}

/// Layout parameters that come from outside the paragraph.
#[derive(Clone, Copy, Debug)]
pub struct ParaEnv {
    pub column_width: i32,
    /// Section default tab interval in HU.
    pub default_tab: i32,
}

/// Measures a paragraph head with its own char shape, or the paragraph's
/// last one when the head's reference is invalid.
fn head_layout(
    head: &Head,
    para: &Paragraph,
    info: &DocInfo,
    styles: &mut Styles<'_>,
) -> HeadLayout {
    if head.text.is_empty() && head.image.is_none() {
        return HeadLayout::default();
    }
    let cs = head
        .char_shape(info)
        .or_else(|| para.runs.last().map(|r| r.shape))
        .unwrap_or(0);
    let mut out = HeadLayout::default();
    if let Some(image) = head.image {
        let si = styles.style_for(cs, Script::Hangul, None);
        out.size = styles.get(si).line_size;
        out.width = out.size + head.distance(out.size) + head.info.width_adjust as i32;
        out.image = Some(image);
        return out;
    }
    // The head's text width leaves out the character spacing of its last
    // glyph, as a line's used width does: a one-glyph head such as "-" in
    // a tightly spaced paragraph takes its full advance.
    let mut x = 0;
    for c in head.text.chars() {
        let style = styles.style_for(cs, script_of(c).unwrap_or(Script::Hangul), Some(c));
        out.glyphs.push((c, style, x));
        let face = styles.face(style).map(|f| &**f);
        x += advance(styles.get(style), face, c);
        out.size = out.size.max(styles.get(style).line_size);
    }
    if let Some(&(c, style, at)) = out.glyphs.last() {
        x = at + natural_advance(styles.get(style), styles.face(style).map(|f| &**f), c);
    }
    let latin = head.text.chars().all(|c| (c as u32) < 0x100);
    let bullet = info
        .para_shape(para.header.para_shape as u32)
        .is_some_and(|s| s.head_kind() == 3);
    // A fixed soft-hyphen bullet and a narrow automatic hanging head
    // reserve one half-em cell; the glyphs keep their measured positions
    // within it.
    let fixed_cell = (bullet && head.text == "\u{ad}") || head.info.attr & (1 << 3) != 0;
    let text = if fixed_cell && latin && head.info.attr & (1 << 2) == 0 {
        out.size / 2
    } else if head.info.attr & (1 << 2) != 0 || latin {
        x
    } else {
        x.max(out.size)
    };
    // Head attribute bits 0-1 align the glyphs in that box.
    let shift = match head.info.attr & 3 {
        1 => (text - x) / 2,
        2 => text - x,
        _ => 0,
    };
    out.glyphs.iter_mut().for_each(|g| g.2 += shift);
    out.width = text + head.distance(out.size) + head.info.width_adjust as i32;
    out
}

/// Formats one paragraph. `head` is its bullet or number, if any.
pub fn format(
    para: &Paragraph,
    info: &DocInfo,
    styles: &mut Styles<'_>,
    env: ParaEnv,
    head: Option<&Head>,
) -> ParaLayout {
    crate::incr::memoized(para, styles, env, head, |styles| {
        format_fresh(para, info, styles, env, head, false, false)
    })
}

/// Formats an endnote before its lines are assigned to page/column
/// fragments. Cached vertical coordinates describe those final fragments,
/// independently of horizontal line membership. This path bypasses the
/// paragraph memo, whose key describes generic formatting only.
pub(crate) fn format_endnote(
    para: &Paragraph,
    info: &DocInfo,
    styles: &mut Styles<'_>,
    env: ParaEnv,
    head: Option<&Head>,
) -> ParaLayout {
    format_fresh(para, info, styles, env, head, true, false)
}

/// Formats body text before the page flow wraps its square drawings and
/// fits numbered heads. Nested lists keep their imported line membership.
pub(crate) fn format_body(
    para: &Paragraph,
    info: &DocInfo,
    styles: &mut Styles<'_>,
    env: ParaEnv,
    head: Option<&Head>,
) -> ParaLayout {
    let square = para.controls.iter().any(|c| {
        c.as_object().is_some_and(|o| {
            let o = &o.header.common;
            !o.treat_as_char() && o.text_wrap() == 0
        })
    });
    let refit_head = head.is_some()
        && info
            .para_shape(para.header.para_shape as u32)
            .is_some_and(|s| matches!(s.head_kind(), 1 | 2));
    let has_border = |shape| {
        info.char_shape(shape)
            .and_then(|c| c.border_fill)
            .filter(|&id| id > 0)
            .and_then(|id| info.border_fills.get(id as usize - 1))
            .is_some_and(|fill| fill.sides.iter().any(|side| side.visible()))
    };
    let bordered = if para.runs.is_empty() {
        has_border(0)
    } else {
        para.runs.iter().any(|run| has_border(run.shape))
    };
    if square || refit_head || bordered {
        // The generic memo key does not include the formatting context.
        format_fresh(para, info, styles, env, head, false, true)
    } else {
        format(para, info, styles, env, head)
    }
}

fn format_fresh(
    para: &Paragraph,
    info: &DocInfo,
    styles: &mut Styles<'_>,
    env: ParaEnv,
    head: Option<&Head>,
    endnote: bool,
    body: bool,
) -> ParaLayout {
    let wrap_square = body
        && para.controls.iter().any(|c| {
            c.as_object().is_some_and(|o| {
                let o = &o.header.common;
                !o.treat_as_char() && o.text_wrap() == 0
            })
        });
    let grid = styles.char_grid_of(para);
    let mut b = Breaker::on_grid(para, info, env, grid);
    // A visual object refreshes tabs throughout its paragraph, including
    // floating objects that leave stored line membership intact.
    b.imported_tabs = styles.replay_imported_lines()
        && para.src.is_some()
        && !para.line_segs.is_empty()
        && !para.controls.iter().any(|c| c.object_common().is_some());
    let tables = crate::table::layout_controls(para, info, styles, env.default_tab);
    let sizes: Vec<(u32, i32, i32)> = tables
        .iter()
        .map(|(k, t)| {
            let (w, h) = t.outer_size();
            (*k, w, h)
        })
        .collect();
    let mut m = measure(para, styles, b.auto, &sizes);
    // Character borders take the room of their side lines in every list:
    // body, cells, text boxes, notes and page furniture.
    crate::measure::border_room(para, &mut m, styles);
    crate::objects::span_column(para, &mut m, env.column_width);
    if grid.pitch > 0 {
        crate::measure::to_char_grid(&mut m, grid);
    }
    let stored = styles
        .replay_imported_lines()
        .then(|| {
            stored_lines(
                para,
                b.left,
                env.column_width - b.right,
                endnote,
                &tables,
                &m,
                crate::compat::effective(info)[3] & (1 << 8) != 0,
                styles.guides,
            )
        })
        .flatten();
    // An unusable line cache does not itself invalidate imported tabs.
    let unreplayed_tabs = stored.is_none() && b.imported_tabs;
    let auto_hang = head.is_some_and(|h| h.info.attr & (1 << 3) != 0);
    let refit_cached_head = auto_hang || head.is_some_and(|h| h.text.is_empty());
    let refit_head = (body || auto_hang) && head.is_some() && matches!(b.shape.head_kind(), 1 | 2);
    let head = head.map(|h| head_layout(h, para, info, styles));
    let has_head = head.as_ref().is_some_and(|h| h.width != 0);
    // A head with automatic hanging (head attribute bit 3) indents the
    // following lines by its width.
    let hang = head.as_ref().filter(|_| auto_hang).map_or(0, |h| h.width);
    let lines = b.lines(&mut m, styles, head.as_ref(), hang, 0, 0, &mut |at, k| {
        let spec = match &stored {
            Some(r) => r.line(
                at,
                k,
                wrap_square,
                refit_head && (refit_cached_head || r.segs.len() == 1),
                refit_head,
            ),
            None => LineSpec {
                cached_tabs: unreplayed_tabs,
                ..LineSpec::FIT
            },
        };
        // Cached gaps on the head's line follow Hancom's head, which is
        // not stored; ours places those tabs by their stops.
        LineSpec {
            cached_tabs: spec.cached_tabs && !(k == 0 && has_head),
            ..spec
        }
    });
    ParaLayout {
        lines,
        line_facts: Default::default(),
        head,
        measured: std::sync::Arc::new(m),
        before: b.shape.before(b.basis),
        after: b.shape.after(b.basis),
        right: b.right,
        align: Some(b.shape.align()),
        hang_last_forbidden: crate::compat::effective(info)[1] & (1 << 22) != 0,
        valign: b.shape.vertical_align(),
        column_width: env.column_width,
        default_tab: env.default_tab,
        hang,
        tables,
    }
}

/// Breaks the lines of a formatted paragraph again from line `from` on, each
/// in its own segment: `interval(k)` gives line `k`'s segment as its shift
/// right of the paragraph's left margin and its width (the full segment is
/// `(0, column width - left margin - right margin)`). The indent and head
/// apply inside the segment as usual. Lines before `from` stay.
pub fn reflow(
    layout: &mut ParaLayout,
    para: &Paragraph,
    info: &DocInfo,
    styles: &mut Styles<'_>,
    from: usize,
    interval: &mut dyn FnMut(usize) -> (i32, i32),
) {
    let Some(start) = layout.lines.get(from).map(|l| l.first_item as usize) else {
        return;
    };
    layout.line_facts = Default::default();
    let env = ParaEnv {
        column_width: layout.column_width,
        default_tab: layout.default_tab,
    };
    let b = Breaker::on_grid(para, info, env, styles.char_grid_of(para));
    layout.lines.truncate(from);
    let lines = b.lines(
        std::sync::Arc::make_mut(&mut layout.measured),
        styles,
        layout.head.as_ref(),
        layout.hang,
        start,
        from,
        &mut |_, k| {
            let (shift, width) = interval(k);
            LineSpec {
                seg: Some((b.left + shift, width)),
                ..LineSpec::FIT
            }
        },
    );
    layout.lines.extend(lines);
}

/// How one line is laid out.
#[derive(Clone, Copy)]
struct LineSpec {
    end: End,
    /// The line's segment `(start, width)` in the column, when not the
    /// paragraph's margins.
    seg: Option<(i32, i32)>,
    /// The segment is the stored one.
    stored_seg: bool,
    /// Stored vertical metrics to keep: text height, baseline, spacing.
    metrics: Option<(i32, i32, i32)>,
    /// Imported inline tab widths stay authoritative: the line is not
    /// composed anew around an object that takes room in its text.
    cached_tabs: bool,
}

impl LineSpec {
    const FIT: LineSpec = LineSpec {
        end: End::Fit,
        seg: None,
        stored_seg: false,
        metrics: None,
        cached_tabs: false,
    };
}

/// The spec of the line starting at a wire position, given its index.
type LinePlan<'a> = dyn FnMut(u32, usize) -> LineSpec + 'a;

/// What breaking a paragraph's lines needs besides its measured items.
/// Narrowest line Hancom sets between paragraph margins.
const MIN_MARGIN_LINE: i32 = 1440;

/// The paragraph's left and right margins in a column `width` wide. Margins
/// that would leave a line narrower than [`MIN_MARGIN_LINE`] give way: the
/// line takes that width from the column start, as the segments Hancom
/// stores for such paragraphs show. A column without margins keeps its own
/// width, however narrow.
fn line_margins(left: i32, right: i32, width: i32) -> (i32, i32) {
    if (left != 0 || right != 0) && width - left - right < MIN_MARGIN_LINE {
        (0, width - MIN_MARGIN_LINE)
    } else {
        (left, right)
    }
}

struct Breaker<'p> {
    para: &'p Paragraph,
    env: ParaEnv,
    shape: ParaShape,
    /// Automatic spacing between scripts (Latin, digits).
    auto: (bool, bool),
    /// Character-unit basis of the paragraph's distances.
    basis: i32,
    fixed_baseline: bool,
    left: i32,
    right: i32,
    indent: i32,
    tab_attr: u32,
    tabs: Vec<(i32, u8)>,
    /// Imported inline widths survive horizontal head fitting.
    imported_tabs: bool,
    rules: Rules,
}

impl<'p> Breaker<'p> {
    fn new(para: &'p Paragraph, info: &DocInfo, env: ParaEnv) -> Self {
        let shape = info
            .para_shape(para.header.para_shape as u32)
            .cloned()
            .unwrap_or_default();
        let flags2 = shape.attr2.unwrap_or(0);
        let compat1 = crate::compat::effective(info)[1];
        let auto = (
            flags2 & (1 << 4) != 0 && compat1 & (1 << 9) == 0,
            flags2 & (1 << 5) != 0 && compat1 & (1 << 10) == 0,
        );
        // Character-unit distances are a percentage of half the reference
        // height, or of all of it under compatibility word 1 bit 2. The
        // reference is the first style's character shape for every
        // paragraph, whatever its own text size; 1000 without one.
        let reference = info
            .styles
            .first()
            .and_then(|s| info.char_shape(s.char_shape as u32))
            .map_or(1000, |c| c.base_size);
        let basis = if compat1 & (1 << 2) != 0 {
            reference
        } else {
            reference / 2
        };
        let tab_def = info.tab_defs.get(shape.tab_def as usize);
        let tabs = tab_def.map_or_else(Vec::new, |t| {
            t.tabs
                .iter()
                .map(|s| (unpack_distance(s.position, || basis), s.kind))
                .collect()
        });
        let (left, right) = line_margins(shape.left(basis), shape.right(basis), env.column_width);
        Breaker {
            para,
            env,
            auto,
            basis,
            fixed_baseline: compat1 & (1 << 5) != 0,
            left,
            right,
            indent: shape.first_indent(basis),
            tab_attr: tab_def.map_or(0, |t| t.attr),
            tabs,
            imported_tabs: false,
            rules: Rules {
                korean_by_char: shape.korean_break_by_char(),
                latin_by_char: shape.latin_break() == 2,
                condense: shape.min_space() as i32,
                fresh_blanks: para.line_segs.is_empty(),
            },
            shape,
        }
    }

    /// A breaker for a paragraph on a character grid (none when its pitch
    /// is 0): its lines hold whole cells only, and Korean words break
    /// between any two characters.
    fn on_grid(
        para: &'p Paragraph,
        info: &DocInfo,
        env: ParaEnv,
        grid: crate::style::CharGrid,
    ) -> Self {
        let b = Self::new(para, info, env);
        let grid = grid.pitch;
        if grid <= 0 {
            return b;
        }
        let cut = (env.column_width - b.left - b.right).rem_euclid(grid);
        let narrow = ParaEnv {
            column_width: env.column_width - cut,
            ..env
        };
        let mut b = Self::new(para, info, narrow);
        b.rules.korean_by_char = true;
        b.rules.latin_by_char = true;
        b
    }

    /// Breaks lines from item `item` to the paragraph end, the first of
    /// them being line `k`. `plan(at, k)` says how the line starting at wire
    /// position `at` ends, gives its segment `(start, width)` in the column
    /// when it is not the paragraph's margins, and whether that segment is
    /// the stored one.
    #[allow(clippy::too_many_arguments)]
    fn lines(
        &self,
        m: &mut Measured,
        styles: &mut Styles<'_>,
        head: Option<&HeadLayout>,
        hang: i32,
        mut item: usize,
        mut k: usize,
        plan: &mut LinePlan<'_>,
    ) -> Vec<Line> {
        let (left, right, indent) = (self.left, self.right, self.indent);
        let mut lines = Vec::new();
        loop {
            let first = k == 0;
            let head_width = if first {
                head.map_or(0, |h| h.width)
            } else {
                0
            };
            let offset = if first {
                indent.max(0) + head_width
            } else {
                (-indent).max(0).max(hang)
            };
            let spec = plan(m.items[item].0, k);
            let (seg_x, seg_width) = spec
                .seg
                .unwrap_or((left, (self.env.column_width - left - right).max(0)));
            let x = seg_x + offset;
            let width = (seg_width - offset).max(0);
            let tab_ctx = TabCtx {
                stops: &self.tabs,
                imported: self.imported_tabs && spec.cached_tabs,
                default: self.env.default_tab,
                origin: x,
                auto_left: (self.tab_attr & 1 != 0 && self.shape.indent < 0)
                    .then(|| left.wrapping_add(indent.wrapping_abs())),
                auto_right: (self.tab_attr & 2 != 0).then(|| self.env.column_width - right),
                end: x + width,
            };
            let mut line = break_line(
                m,
                &self.para.text,
                item,
                width,
                self.rules,
                &tab_ctx,
                spec.end,
            );
            line.x = x;
            line.width = width;
            line.seg_x = seg_x;
            line.seg_width = seg_width;
            line.stored = spec.stored_seg;
            let mut seed = if first { head.map_or(0, |h| h.size) } else { 0 };
            // A line holding nothing with an advance (anchors, section and
            // column definitions) still takes its characters' height.
            let items = &m.items[line.first_item as usize..line.end_item as usize];
            if let Some(&(pos, _)) = items.first()
                && items.iter().all(|&(_, k)| k == Kind::Zero)
            {
                seed = seed.max(styles.get(m.styles[pos as usize]).line_size);
            }
            metrics(&mut line, m, styles, &self.shape, self.basis, seed);
            // A replayed line keeps the vertical metrics Hancom stored: its
            // baseline, and its text height plus spacing as its advance.
            if let Some((text_height, baseline, spacing)) = spec.metrics {
                line.height = text_height;
                line.text_height = text_height;
                line.baseline = baseline;
                line.spacing = spacing;
            }
            if self.fixed_baseline && self.shape.line_spacing().0 == LineSpacingKind::Fixed {
                let advance = line.advance();
                line.height = advance;
                line.text_height = advance;
                line.baseline = mul_div(advance, 4, 5);
                line.spacing = 0;
            } else if self.fixed_baseline
                && self.shape.line_spacing().0 == LineSpacingKind::Percent
                && line.spacing < 0
            {
                line.baseline = line.baseline.min(mul_div(line.advance(), 4, 5));
            }
            item = line.end_item as usize;
            let last = line.last;
            lines.push(line);
            k += 1;
            if last || item >= m.items.len() {
                break;
            }
        }
        if let Some(l) = lines.last_mut() {
            l.last = true;
        }
        lines
    }
}

/// Stored lines to replay.
struct Replay {
    segs: Vec<hm_model::LineSeg>,
    /// The paragraph's margins `[left, right)` in the column.
    margins: (i32, i32),
    /// Lines starting before this wire position replay; the rest are
    /// laid out fresh.
    until: u32,
    /// Printing refits these lines horizontally while keeping their
    /// imported vertical metrics.
    fit_from: u32,
}

impl Replay {
    /// The stored segment of line `k` as a placement interval, when it lies
    /// within the margins (an object beside the line narrows it).
    fn segment(&self, k: usize) -> Option<(i32, i32)> {
        let (left, right) = self.margins;
        self.segs
            .get(k)
            .filter(|s| s.col_start >= left && s.width > 0 && s.col_start + s.width <= right)
            .map(|s| (s.col_start, s.width))
    }

    /// How line `k`, starting at wire position `at`, is laid out: replayed
    /// lines end where the next stored line starts and keep their stored
    /// vertical metrics. Fresh lines retain imported segments unless the
    /// paragraph's own square drawing must be anchored before its live
    /// exclusion bands can be composed. Body paragraphs with an authored
    /// automatic hanging head refit horizontally, including empty formats.
    /// Nonempty manual heads retain multi-line membership. Resolved numbered
    /// body heads refresh first-line metrics; later lines keep stored metrics.
    fn line(
        &self,
        at: u32,
        k: usize,
        wrap_square: bool,
        refit_head: bool,
        head_line: bool,
    ) -> LineSpec {
        let cached_tabs = at < self.until;
        if at >= self.until || refit_head && k >= self.segs.len() {
            if wrap_square {
                return LineSpec {
                    cached_tabs,
                    ..LineSpec::FIT
                };
            }
            let seg = self.segment(k);
            return LineSpec {
                seg,
                stored_seg: seg.is_some(),
                cached_tabs,
                ..LineSpec::FIT
            };
        }
        let j = if refit_head {
            k
        } else {
            self.segs
                .partition_point(|s| s.text_start <= at)
                .saturating_sub(1)
        };
        let s = &self.segs[j];
        let seg = self.segment(j);
        LineSpec {
            end: if refit_head || at >= self.fit_from {
                End::Fit
            } else {
                self.segs
                    .get(j + 1)
                    .map_or(End::At(None), |s| End::At(Some(s.text_start)))
            },
            seg,
            stored_seg: seg.is_some(),
            metrics: (!head_line || j > 0).then_some((s.text_height, s.baseline, s.spacing)),
            cached_tabs,
        }
    }
}

/// The lines Hancom stored for an unedited paragraph, when it replays
/// them: Hancom keeps a paragraph's imported line membership, so a stored
/// line may hold more or less than a fresh fit would, and keeps each
/// line's stored baseline and its text height plus spacing as its advance.
///
/// The first segment decides: it must span the paragraph's margins
/// (`[left, right)` in the column) exactly, or lie within them where an
/// object narrows the line, and be a whole line (not empty, the line's
/// last segment). Segments must start at 0, rise strictly within the text
/// and follow one another down; text on both sides of an object is laid
/// out fresh.
///
/// Hancom sizes drawing objects and equations anew when it opens a file
/// and lays out the paragraph again from the stored line holding the first
/// of them that takes room in its text: one set as a character, or one the
/// text wraps around on both sides. A table the text wraps around does the
/// same, and so does a table set as a character whose cells are laid out
/// fresh; other tables set as characters, objects in front of or behind
/// the text, or above and below it, leave the stored lines alone.
/// True when a table's cells hold paragraphs Hancom stored no lines for:
/// it sizes them anew, and with them the table.
fn fresh_cells(t: &hm_model::Table) -> bool {
    t.cells
        .iter()
        .any(|c| c.paras.iter().any(|p| p.line_segs.is_empty()))
}

/// True when a table set as a character at wire position `pos` is wider than
/// its stored line and something other than a section or column definition
/// comes before it on that line. Hancom breaks such a line again: the table
/// cannot follow anything, even a control without width, so it opens a line
/// of its own. Multiple inline tables also need fresh fitting when their
/// combined rigid widths exceed the stored line.
fn crowded_off_its_line(
    para: &Paragraph,
    segs: &[hm_model::LineSeg],
    pos: u32,
    o: &hm_model::ObjectCommon,
    measured: &Measured,
) -> bool {
    let Some(seg) = segs.iter().rfind(|s| s.text_start <= pos) else {
        return false;
    };
    let width = o.width + i32::from(o.margins[0]) + i32::from(o.margins[1]);
    let (tables_width, count) = measured
        .boxes
        .iter()
        .filter(|b| {
            b.pos >= seg.text_start
                && b.pos <= pos
                && para.controls[b.control as usize].as_table().is_some()
        })
        .fold((0i64, 0), |(w, n), b| (w + i64::from(b.width), n + 1));
    if count > 1 && tables_width > i64::from(seg.width) {
        return true;
    }
    width > seg.width
        && para.units().any(|(p, u)| {
            p >= seg.text_start && p < pos && !matches!(u, hm_model::Unit::Extended { code: 2, .. })
        })
}

fn resized_by_reflow(t: &hm_model::Table, l: &crate::table::TableLayout) -> bool {
    // A changed cell line count or line height (an equation Hancom sizes
    // anew) invalidates the inline host's cached height. A table's
    // authored height alone does not invalidate it.
    l.height != t.header.common.height
        && t.cells.iter().zip(&l.cells).any(|(c, l)| {
            c.paras.iter().zip(&l.paras).any(|(p, l)| {
                !p.line_segs.is_empty()
                    && (p.line_segs.len() != l.lines.len()
                        || p.line_segs
                            .iter()
                            .zip(&l.lines)
                            .any(|(s, l)| s.height > 0 && s.height != l.height))
            })
        })
}

#[allow(clippy::too_many_arguments)]
fn stored_lines(
    para: &Paragraph,
    left: i32,
    right: i32,
    endnote: bool,
    tables: &[(u32, std::sync::Arc<crate::table::TableLayout>)],
    measured: &Measured,
    bottom: bool,
    guides: bool,
) -> Option<Replay> {
    const EMPTY: u32 = 1 << 16;
    const LINE_LAST: u32 = 1 << 18;
    let segs = &para.line_segs;
    let first = segs.first()?;
    let clean = para.src.is_some()
        && first.text_start == 0
        && first.flags & EMPTY == 0
        && first.flags & LINE_LAST != 0
        && first.col_start >= left
        && first.width > 0
        && first.col_start + first.width <= right
        && segs
            .windows(2)
            .all(|w| w[0].text_start < w[1].text_start && (endnote || w[0].vpos != w[1].vpos))
        && segs.last().is_some_and(|s| s.text_start < para.len());
    if !clean {
        return None;
    }
    let object = para.units().find_map(|(pos, u)| match u {
        hm_model::Unit::Extended { index, .. } => para
            .controls
            .get(index as usize)
            .filter(|c| {
                c.object_common().is_some_and(|o| {
                    if let Some(t) = c.as_table() {
                        (!o.treat_as_char() && o.text_wrap() == 0)
                            || (o.treat_as_char()
                                && (fresh_cells(t)
                                    || tables
                                        .iter()
                                        .find(|(k, _)| *k == index)
                                        .is_some_and(|(_, l)| resized_by_reflow(t, l))
                                    || crowded_off_its_line(para, segs, pos, &o, measured)))
                    } else {
                        // In Word-compatible documents an object that keeps its stored
                        // size keeps the stored lines. Equations compare height only:
                        // their width comes from our own math layout.
                        if o.treat_as_char()
                            && c.as_object().is_some()
                            && bottom
                            && measured.boxes.iter().any(|b| {
                                b.control == index
                                    && b.height
                                        == o.height
                                            + i32::from(o.margins[2])
                                            + i32::from(o.margins[3])
                                    && (c.as_object().is_some_and(|o| o.equation().is_some())
                                        || b.width
                                            == o.width
                                                + i32::from(o.margins[0])
                                                + i32::from(o.margins[1]))
                            })
                        {
                            return false;
                        }
                        o.treat_as_char() || o.text_wrap() == 0
                    }
                })
            })
            .map(|_| pos),
        _ => None,
    });
    let line_start = |pos| {
        segs.iter()
            .map(|s| s.text_start)
            .rfind(|&s| s <= pos)
            .unwrap_or(0)
    };
    let until = object.map_or(u32::MAX, line_start);
    // Suppressed guides change print line breaks, but their imported
    // height and baseline remain authoritative. The editing screen shows
    // them where Hancom stored the lines.
    let mut prompts = Vec::new();
    if !guides {
        crate::annot::unfilled_prompts(para, &mut prompts);
    }
    let fit_from = prompts
        .iter()
        .map(|r| line_start(r.0))
        .min()
        .unwrap_or(u32::MAX);
    Some(Replay {
        segs: segs.clone(),
        margins: (left, right),
        until,
        fit_from,
    })
}

#[derive(Clone, Copy)]
struct Rules {
    korean_by_char: bool,
    latin_by_char: bool,
    /// Percent of the line's blank widths a candidate may borrow to fit
    /// (paragraph shape bits 9-15).
    condense: i32,
    /// Paragraphs without imported line membership use the fresh blank
    /// run rule. Imported spans retain their existing fallback policy.
    fresh_blanks: bool,
}

struct TabCtx<'a> {
    /// Explicit stops in stored order: position (column-relative) and kind.
    stops: &'a [(i32, u8)],
    /// The unchanged paragraph keeps its imported inline tab advances.
    imported: bool,
    default: i32,
    /// Line content start relative to the column. Tab stops and the default
    /// tab grid count from the column's left edge.
    origin: i32,
    /// Automatic stops of the tab definition: at the hanging indent and at
    /// the right margin.
    auto_left: Option<i32>,
    auto_right: Option<i32>,
    /// The right edge of the line's content (column-relative): a right
    /// stop past it acts at it.
    end: i32,
}

impl TabCtx<'_> {
    /// An applicable authored stop, with the same automatic-stop precedence
    /// as fresh fitting. No authored stop leaves imported widths authoritative.
    fn stop_advance(&self, current: i32, run_end: i32, dot: Option<i32>) -> Option<i32> {
        let mid = (current.wrapping_add(run_end)) / 2;
        let auto_left = self.auto_left.filter(|&s| current < s);
        let auto_right = self.auto_right.filter(|&s| run_end < s);
        for &(pos, kind) in self.stops {
            if pos <= current {
                continue;
            }
            if let Some(a) = auto_left.filter(|&a| pos > a) {
                return Some(a - current);
            }
            if let Some(a) = auto_right.filter(|&a| pos > a) {
                return Some((a - run_end).max(0));
            }
            match kind {
                0 => return Some(pos - current),
                1 if pos > run_end => return Some((pos.min(self.end) - run_end).max(0)),
                2 if pos > mid => return Some(pos - mid),
                3 => {
                    let anchor = dot.unwrap_or(run_end);
                    if pos > anchor {
                        let room = self.end - (run_end - anchor);
                        return Some((pos.min(room) - anchor).max(0));
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// The advance of a fresh tab. Authored stops use their left, right,
    /// center or decimal reference; automatic and default stops follow when
    /// no authored stop applies.
    fn advance(&self, current: i32, run_end: i32, dot: Option<i32>) -> i32 {
        if let Some(width) = self.stop_advance(current, run_end, dot) {
            return width;
        }
        if let Some(a) = self.auto_left.filter(|&s| current < s) {
            return a - current;
        }
        if let Some(a) = self.auto_right.filter(|&s| run_end < s) {
            return (a - run_end).max(0);
        }
        let d = self.default.max(1);
        (d.wrapping_add(current) / d).wrapping_mul(d) - current
    }
}

/// How an element takes part in choosing break positions.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Run {
    Blank,
    /// A unit of the Latin run class (Latin script slot, NBSP, ZWNBSP,
    /// non-breaking hyphen).
    Latin,
    /// Every other unit, inline objects included.
    Other,
    /// Controls that never open a line.
    Never,
}

fn run_of(kind: Kind) -> Run {
    match kind {
        Kind::Char(c)
            if matches!(c, '\u{a0}' | '\u{feff}' | '\u{2011}')
                || script_of(c) == Some(Script::Latin) =>
        {
            Run::Latin
        }
        Kind::Char(_) | Kind::Object(_) => Run::Other,
        Kind::Space | Kind::Tab => Run::Blank,
        Kind::Zero | Kind::ParaEnd | Kind::LineBreak => Run::Never,
    }
}

/// May a line start at item `j`?
///
/// A line can start after a blank, at a tab and at every boundary between
/// the Latin run class and other units. Inside a run it can start only when that
/// run's policy breaks by character: paragraph Latin mode 2 for Latin runs,
/// Korean bit 7 for the rest. An inline equation joins the Latin text it
/// touches into one word.
fn can_break_before(m: &Measured, j: usize, rules: Rules) -> bool {
    let items = &m.items;
    let cur = run_of(items[j].1);
    let Some((prev_kind, prev)) = items[..j]
        .iter()
        .rev()
        .map(|(_, k)| (*k, run_of(*k)))
        .find(|(_, r)| *r != Run::Never)
    else {
        return false;
    };
    // Unlike a space, a tab that does not fit can open the next line.
    if matches!(items[j].1, Kind::Tab) {
        return true;
    }
    match (prev, cur) {
        (_, Run::Never | Run::Blank) => false,
        (Run::Blank, _) => true,
        (Run::Other, Run::Latin) if is_equation(m, prev_kind) => rules.latin_by_char,
        (Run::Latin, Run::Other) if is_equation(m, items[j].1) => rules.latin_by_char,
        (Run::Latin, Run::Latin) => rules.latin_by_char,
        (Run::Other, Run::Other) => rules.korean_by_char,
        _ => true,
    }
}

fn is_equation(m: &Measured, kind: Kind) -> bool {
    let Kind::Object(k) = kind else {
        return false;
    };
    m.boxes.get(k as usize).is_some_and(|b| {
        m.objects
            .iter()
            .any(|(c, o)| *c == b.control && o.equation.is_some())
    })
}

/// Hancom's forbidden-character correction: move a break back (at most
/// twice) while the line would end with a forbidden-end character or the
/// next would start with a forbidden-start one. Each correction takes one
/// character in character-break mode, or finds the preceding legal word
/// boundary after a separator in word-break mode.
fn correct_forbidden(m: &Measured, from: usize, mut b: usize, rules: Rules) -> usize {
    let items = &m.items;
    let by_char = rules.korean_by_char || rules.latin_by_char;
    let class = |k: usize| match items[k].1 {
        Kind::Char(c) => break_class(c),
        _ => BreakClass::Other,
    };
    for _ in 0..2 {
        if b <= from + 1 || b >= items.len() {
            break;
        }
        let forbidden = class(b - 1) == BreakClass::Open || class(b) == BreakClass::Close;
        if !forbidden {
            break;
        }
        let previous = if by_char || can_break_before(m, b - 1, rules) {
            Some(b - 1)
        } else {
            // Script transitions inside a word are not word recovery points.
            items[from..b]
                .iter()
                .rposition(|&(_, kind)| matches!(kind, Kind::Space | Kind::Tab))
                .and_then(|blank| (from + blank + 1..b).find(|&j| can_break_before(m, j, rules)))
        };
        let Some(previous) = previous else {
            break;
        };
        b = previous;
    }
    b
}

/// Where a line ends.
#[derive(Clone, Copy, PartialEq, Eq)]
enum End {
    /// Where the content stops fitting the width.
    Fit,
    /// Before the item at a wire position (a stored line start), or at the
    /// paragraph end.
    At(Option<u32>),
}

/// Fills one line starting at item `from`.
fn break_line(
    m: &mut Measured,
    text: &[u16],
    from: usize,
    width: i32,
    rules: Rules,
    tabs: &TabCtx<'_>,
    rule: End,
) -> Line {
    let n = m.items.len();
    // Leading section and column definitions take no room: the first item
    // after them is consumed whatever its width, as the first item is.
    let first = from
        + m.items[from..]
            .iter()
            .take_while(|(p, k)| *k == Kind::Zero && text.get(*p as usize) == Some(&2))
            .count();
    let mut x = 0i32;
    let mut x_ink = 0i32;
    let mut blanks = 0i32;
    let mut measured = false;
    let mut best: Option<(usize, i32)> = None;
    let mut j = from;
    let mut forced = false;
    let mut last = false;
    let mut end = n;
    let mut overruns = 0;
    while j < n {
        let (pos, kind) = m.items[j];
        if let End::At(Some(stop)) = rule
            && j > from
            && pos >= stop
        {
            // A stored line may open with blanks; they keep their room.
            end = j;
            break;
        }
        match kind {
            Kind::ParaEnd => {
                end = j + 1;
                last = true;
                break;
            }
            Kind::LineBreak => {
                end = j + 1;
                forced = true;
                break;
            }
            Kind::Tab => {
                let at = tabs.origin + x;
                let dot = period_offset(m, j + 1).map(|d| at + d);
                let run_end = at + following_width(m, j + 1);
                let p = pos as usize;
                let stored = tabs
                    .imported
                    .then(|| {
                        let (&lo, &hi) = text.get(p + 1).zip(text.get(p + 2))?;
                        i32::try_from(u32::from(lo) | u32::from(hi) << 16).ok()
                    })
                    .flatten();
                let data = text.get(p + 3).copied().unwrap_or(0);
                let custom = data >> 8 == 2;
                let leader = data & 0xFF != 0;
                let (width, replayed) = match stored {
                    // Leader and empty custom fields seek a manual stop.
                    // Plain cached fields retain their stored gap.
                    Some(width) if custom && (leader || run_end == at) => {
                        match tabs.stop_advance(at, run_end, dot) {
                            Some(width) => (width, false),
                            None => (width, true),
                        }
                    }
                    Some(width) => (width, true),
                    None => (tabs.advance(at, run_end, dot), false),
                };
                m.advances[p] = width;
                match (m.fixed_tabs.binary_search(&pos), replayed && !custom) {
                    (Err(k), true) => m.fixed_tabs.insert(k, pos),
                    (Ok(k), false) => {
                        m.fixed_tabs.remove(k);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        let (border_left, border_right) = m.border_room(j, from, n);
        let w = m.advances[pos as usize] + border_left + border_right;
        if j > from && can_break_before(m, j, rules) {
            best = Some((j, x_ink));
        }
        let space = matches!(kind, Kind::Space);
        // Condensing lets a candidate borrow part of the blanks before it,
        // the percentage applied once to their sum.
        // Tabs take no part in it.
        let budget = if rules.condense > 0 && !matches!(kind, Kind::Tab) {
            blanks.wrapping_mul(rules.condense) / 100
        } else {
            0
        };
        // A character fits when its advance before character spacing does.
        let fit = match kind {
            Kind::Char(_) => m.natural[pos as usize] + border_left + border_right,
            _ => w,
        };
        // Every candidate closes its border, even when the next item
        // continues the same fill and will share that edge if it fits.
        let fit = fit
            + m.border(pos)
                .map_or(0, |(_, sides)| sides[1] - border_right);
        let ordinary = matches!(text.get(pos as usize), Some(&0x20 | &0x3000));
        // A terminal blank can cross the physical edge. If it opens the
        // blank run, the following blank hangs with it.
        if rule == End::Fit && space && ordinary && (x + fit > width || measured && x >= width) {
            let blank = |k: usize| {
                m.items.get(k).is_some_and(|&(p, kind)| {
                    kind == Kind::Space && matches!(text.get(p as usize), Some(&0x20 | &0x3000))
                })
            };
            let trailing = m.items[j..]
                .iter()
                .take_while(|i| !matches!(i.1, Kind::ParaEnd | Kind::LineBreak))
                .all(|i| i.1 == Kind::Space);
            if trailing && x + fit > width && blank(j + 1) {
                end = j + 1;
                if measured && j > from && !blank(j - 1) {
                    end += 1;
                    if m.items.get(end).is_some_and(|i| i.1 == Kind::ParaEnd) {
                        end += 1;
                        last = true;
                    }
                }
                break;
            }
            if !trailing && measured && x >= width {
                if rules.fresh_blanks && blank(j - 1) {
                    // A run already begun inside the line keeps only the
                    // blank starting at or crossing the physical edge.
                    if x > width {
                        end = if j > from + 1 && !blank(j - 2) {
                            j + 1
                        } else {
                            j
                        };
                        break;
                    }
                } else {
                    end = if !blank(j - 1) && blank(j + 1) {
                        j + 2
                    } else {
                        j + 1
                    };
                    break;
                }
            }
        }
        if rule == End::Fit && !space && j > first && x + fit > width + budget {
            overruns += 1;
            // Manuscript paper keeps up to three remaining ASCII letters
            // or digits on this line, and up to three forbidden starters.
            let suffix = matches!(kind, Kind::Char(c) if c.is_ascii_alphanumeric())
                && m.items[j + 1..]
                    .iter()
                    .take(3)
                    .any(|&(_, k)| !matches!(k, Kind::Char(c) if c.is_ascii_alphanumeric()));
            let closing = overruns < 4
                && matches!(kind, Kind::Char(c) if break_class(c) == BreakClass::Close);
            if m.manuscript && (suffix || closing) {
                x += w;
                x_ink = x;
                j += 1;
                continue;
            }
            end = match best {
                Some((b, _)) if b > from => b,
                _ => j,
            };
            end = correct_forbidden(m, from, end, rules);
            break;
        }
        x += w;
        // The condense budget counts ordinary blanks (U+0020, U+3000) once
        // something has been measured on the line, at their width before
        // character spacing; a tab starts it afresh.
        if matches!(kind, Kind::Tab) {
            blanks = 0;
        } else if space && ordinary && measured {
            blanks += if m.natural[pos as usize] > 0 {
                m.natural[pos as usize]
            } else {
                w
            };
        }
        if !space {
            x_ink = x;
        }
        // A fixed-width blank counts as measured content without adding
        // to the budget.
        if !space || text.get(pos as usize) == Some(&31) {
            measured = true;
        }
        j += 1;
    }
    let end = end.min(n).max(from + 1).min(n);
    // Trailing ordinary blanks hang past the line; no-break and fixed-width
    // blanks are content, as right and centered lines show.
    let used = content_width(m, from, end, |k| {
        let (pos, kind) = m.items[k];
        alignment_ink(m, (pos, kind))
            || (kind == Kind::Space && matches!(text.get(pos as usize), Some(&(30 | 31))))
    });
    let start_pos = m.items.get(from).map_or(0, |i| i.0);
    let end_pos = m
        .items
        .get(end)
        .map_or_else(|| m.advances.len() as u32, |i| i.0);
    Line {
        start: start_pos,
        end: end_pos,
        first_item: from as u32,
        end_item: end as u32,
        used,
        forced,
        last: last || end >= n,
        ..Default::default()
    }
}

/// Width of items from `from` up to the next tab or line end.
///
/// Like the used width, the last visible character counts without its
/// character spacing, so text after a right tab ends at the stop.
/// The width before the first period of the text after a tab, up to the
/// next tab or line end, when it has one.
fn period_offset(m: &Measured, from: usize) -> Option<i32> {
    let mut w = 0;
    for k in from..m.items.len() {
        let (pos, kind) = m.items[k];
        let (left, right) = m.border_room(k, from, m.items.len());
        match kind {
            Kind::Tab | Kind::LineBreak | Kind::ParaEnd => return None,
            Kind::Char('.') => return Some(w + left),
            _ => w += m.advances[pos as usize] + left + right,
        }
    }
    None
}

fn following_width(m: &Measured, from: usize) -> i32 {
    let end = m.items[from..]
        .iter()
        .position(|(_, k)| matches!(k, Kind::Tab | Kind::LineBreak | Kind::ParaEnd))
        .map_or(m.items.len(), |k| from + k);
    ink_width(m, from, end)
}

/// Content width of items `[from, end)` without trailing blanks. The last
/// visible character counts without its character spacing (Hancom leaves
/// terminal tracking out of the used width).
fn ink_width(m: &Measured, from: usize, end: usize) -> i32 {
    content_width(m, from, end, |k| alignment_ink(m, m.items[k]))
}

/// Width of items `[from, end)` up to the last one `content` accepts, that
/// one without its character spacing.
fn content_width(m: &Measured, from: usize, end: usize, content: impl Fn(usize) -> bool) -> i32 {
    let Some(last) = (from..end).rev().find(|&k| content(k)) else {
        return 0;
    };
    let body: i32 = (from..last)
        .map(|k| {
            let (left, right) = m.border_room(k, from, last + 1);
            m.advances[m.items[k].0 as usize] + left + right
        })
        .sum();
    let (pos, kind) = m.items[last];
    let tail = match kind {
        Kind::Char(_) => m.natural[pos as usize],
        _ => m.advances[pos as usize],
    };
    let (left, right) = m.border_room(last, from, last + 1);
    body + tail + left + right
}

fn is_ink(kind: Kind) -> bool {
    !matches!(
        kind,
        Kind::Space | Kind::ParaEnd | Kind::LineBreak | Kind::Zero
    )
}

/// Imported default gaps do not extend a trailing ink span. Internal tabs
/// still anchor their following text.
fn alignment_ink(m: &Measured, (pos, kind): (u32, Kind)) -> bool {
    is_ink(kind) && !(kind == Kind::Tab && m.fixed_tabs.binary_search(&pos).is_ok())
}

/// Line height, baseline and spacing.
///
/// Each run (the paragraph end included) contributes its logical size as
/// height and 85 percent of it as baseline; inline objects contribute their
/// height the same way. The line takes the maxima. Percent spacing uses the
/// tallest text run only: `MulDiv(h / 4, pct - 100, 100) * 4`. Fixed spacing
/// makes the advance the value, added spacing adds it, and at-least spacing
/// adds what the value exceeds the height by.
fn metrics(
    line: &mut Line,
    m: &Measured,
    styles: &Styles<'_>,
    shape: &ParaShape,
    basis: i32,
    seed: i32,
) {
    // A paragraph head's style seeds the first line.
    let mut height = seed;
    let mut baseline = mul_div(seed, 85, 100);
    let mut text = seed;
    for &(pos, kind) in &m.items[line.first_item as usize..line.end_item as usize] {
        let (h, b) = match kind {
            Kind::Object(i) => {
                // The object's run still sets the percent reference.
                text = text.max(styles.get(m.styles[pos as usize]).line_size);
                let bx = m.boxes[i as usize];
                (bx.height, bx.baseline)
            }
            Kind::Zero => continue,
            _ => {
                let h = styles.get(m.styles[pos as usize]).line_size;
                text = text.max(h);
                (h, mul_div(h, 85, 100))
            }
        };
        height = height.max(h);
        baseline = baseline.max(b);
    }
    let (kind, value) = shape.line_spacing();
    let spacing = match kind {
        LineSpacingKind::Percent => {
            let quarter = if text < 0 { (text + 3) >> 2 } else { text >> 2 };
            mul_div(quarter, value - 100, 100).wrapping_mul(4)
        }
        LineSpacingKind::Fixed => unpack_distance(value, || basis) - height,
        LineSpacingKind::Added => unpack_distance(value, || basis),
        LineSpacingKind::AtLeast => (unpack_distance(value, || basis) - height).max(0),
    };
    line.height = height;
    line.text_height = height;
    // Paragraph vertical alignment (bits 20-21) replaces the measured
    // baseline with the line top, middle or bottom.
    line.baseline = match shape.vertical_align() {
        1 => 0,
        2 => height / 2,
        3 => height,
        _ => baseline,
    };
    line.spacing = spacing;
}

/// One positioned element of a line, for painting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placed {
    pub pos: u32,
    pub kind: Kind,
    /// Left edge relative to the column.
    pub x: i32,
    pub advance: i32,
}

/// Places a line's items horizontally, applying alignment.
///
/// A tab before the final ink resets alignment opportunities. Blanks count
/// only after visible content, and character gaps are the visible items
/// after the tab less one. Positive slack goes out as whole quotients:
/// justify fills blanks (or character gaps when the line has no blank) except
/// on a paragraph's last line and lines ending in a line break; distribute
/// fills blanks and gaps alike on every line; divide fills blanks only.
/// Negative slack (a condensed line) shrinks each blank by at most a fifth of
/// the narrowest blank and the character gaps by the rest.
pub fn place(layout: &ParaLayout, line: &Line, out: &mut Vec<Placed>) {
    out.clear();
    let m = &layout.measured;
    let items = &m.items[line.first_item as usize..line.end_item as usize];
    let align = layout.align.unwrap_or(Align::Justify);
    let ink_end = items
        .iter()
        .rposition(|&item| alignment_ink(m, item))
        .map_or(0, |i| i + 1);
    let reset = items
        .iter()
        .rposition(|&(_, kind)| kind == Kind::Tab)
        .map_or(0, |i| (i + 1).min(ink_end));
    let first_visible = items[reset..ink_end]
        .iter()
        .position(|&item| alignment_ink(m, item))
        .map_or(ink_end, |i| reset + i);
    // Fixed-width blanks keep their width.
    let eligible_space = |k: usize| {
        k >= first_visible
            && k < ink_end
            && matches!(items[k].1, Kind::Space)
            && m.fixed_blanks.binary_search(&items[k].0).is_err()
    };
    let eligible_char =
        |k: usize| k >= first_visible && k + 1 < ink_end && alignment_ink(m, items[k]);
    let spaces = (0..items.len()).filter(|&k| eligible_space(k)).count() as i32;
    let gaps = (0..items.len()).filter(|&k| eligible_char(k)).count() as i32;
    // Manuscript alignment measures narrow characters inside a Latin word
    // at their natural advances. Packed words ending in a lone half-width
    // glyph close on a whole cell, as does the line's total width.
    // Painting still gives them cells unless two can share one.
    let used = if m.manuscript {
        let mut word = false;
        let mut packed = false;
        let mut paired = false;
        // Keep terminal fixed-width and no-break blanks in the used width.
        let mut width = line.used - ink_width(m, line.first_item as usize, line.end_item as usize);
        for (k, &(pos, kind)) in items[..ink_end].iter().enumerate() {
            if kind == Kind::Zero {
                continue;
            }
            let p = pos as usize;
            let narrow = matches!(kind, Kind::Char(c) if crate::measure::grid_narrow(c));
            if !word {
                packed = narrow;
                paired = false;
            }
            paired |= m.advances[p] < m.grid;
            let word_end = items[k + 1..ink_end]
                .iter()
                .find(|(_, kind)| *kind != Kind::Zero)
                .is_none_or(|&(_, kind)| run_of(kind) != Run::Latin);
            let terminal_half = word_end && m.grid_glyphs[p].1 * 2 <= m.grid;
            // The second glyph of a shared cell contributes only its own
            // width, including at the end of the word. Its paint advance
            // still completes the cell.
            let pair_tail = m.advances[p] < m.grid && m.grid_glyphs[p].0 == 0;
            width += if pair_tail
                || word && narrow && !terminal_half && !(packed && m.advances[p] < m.grid)
            {
                m.grid_glyphs[p].1
            } else {
                m.advances[p]
            };
            // An unpaired half-width glyph closes a packed word's last
            // cell. A pair tail keeps its fractional alignment width.
            if terminal_half && m.advances[p] >= m.grid && packed && paired {
                width = (width + m.grid - 1) / m.grid * m.grid;
            }
            word = run_of(kind) == Run::Latin;
        }
        (width + m.grid - 1) / m.grid * m.grid
    } else {
        line.used
    };
    let used = if layout.hang_last_forbidden
        && m.grid == 0
        && matches!(align, Align::Center | Align::Right)
    {
        match items.get(ink_end.saturating_sub(1)) {
            Some(&(pos, Kind::Char(c))) if break_class(c) == BreakClass::Close => {
                used - m.natural[pos as usize]
            }
            _ => used,
        }
    } else {
        used
    };
    let slack = line.width - used;
    let (mut offset, mut space_extra, mut char_extra) = (0, 0, 0);
    if m.grid > 0 {
        // On a character grid lines keep whole cells: no stretching, and a
        // centred line moves by whole cells.
        offset = match align {
            Align::Right => slack.max(0),
            Align::Center => (slack.max(0) >> 1) / m.grid * m.grid,
            _ => 0,
        };
    } else if slack >= 0 {
        match align {
            Align::Left => {}
            Align::Right => offset = slack,
            Align::Center => offset = slack >> 1,
            Align::Justify if line.last || line.forced => {}
            Align::Justify if spaces > 0 => space_extra = slack / spaces,
            Align::Justify if gaps > 0 => char_extra = slack / gaps,
            Align::Justify => {}
            Align::Distribute if spaces + gaps > 0 => {
                space_extra = slack / (spaces + gaps);
                char_extra = space_extra;
            }
            Align::Divide if spaces > 0 => space_extra = slack / spaces,
            Align::Distribute | Align::Divide => {}
        }
    } else {
        let narrowest = (0..items.len())
            .filter(|&k| eligible_space(k))
            .map(|k| m.advances[items[k].0 as usize])
            .filter(|&w| w > 0)
            .min();
        match narrowest {
            Some(min) if spaces > 0 => {
                let cap = (min + 4) / 5;
                space_extra = slack / spaces;
                if -space_extra > cap && gaps > 0 {
                    space_extra = -cap;
                    char_extra = (slack + spaces * cap) / gaps;
                }
            }
            _ if gaps > 0 => char_extra = slack / gaps,
            _ => {}
        }
    }
    let mut x = line.x + offset;
    for (k, &(pos, kind)) in items.iter().enumerate() {
        let (border_left, border_right) = m.border_room(
            line.first_item as usize + k,
            line.first_item as usize,
            line.end_item as usize,
        );
        let advance = m.advances[pos as usize];
        // A character on the grid sits in the middle of its cells.
        let (mut lead, glyph) = m
            .grid_glyphs
            .get(pos as usize)
            .copied()
            .unwrap_or((0, advance));
        // The suffix hanging beyond a manuscript line uses its natural
        // advances, starting exactly at the margin.
        let advance = if m.manuscript && x >= line.x + offset + line.width {
            lead = 0;
            glyph
        } else {
            advance
        };
        out.push(Placed {
            pos,
            kind,
            x: x + lead + border_left,
            advance: glyph,
        });
        x += advance + border_left + border_right;
        if eligible_space(k) {
            x += space_extra;
        } else if eligible_char(k) {
            x += char_extra;
        }
    }
}

/// Content span after alignment and condensation, for object clearance.
pub(crate) fn content_span(layout: &ParaLayout, line: &Line) -> (i32, i32) {
    let mut placed = Vec::new();
    place(layout, line, &mut placed);
    placed
        .iter()
        .filter(|g| is_ink(g.kind))
        .map(|g| (g.x, g.x + g.advance))
        .reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)))
        .unwrap_or((line.x, line.x))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hm_fonts::FontSystem;
    use hm_model::{LineSeg, PARA_END, SharedBytes};

    #[test]
    fn cached_plain_tab_fields_anchor_their_own_overflow() {
        use hm_model::{TabDef, TabStop};
        let fonts = FontSystem::new();
        let info = DocInfo {
            para_shapes: vec![ParaShape::default()],
            tab_defs: vec![TabDef {
                tabs: vec![TabStop {
                    position: 4800,
                    kind: 1,
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut p = Paragraph {
            text: vec![
                'A' as u16,
                9,
                700,
                0,
                2 << 8,
                0,
                0,
                0,
                9,
                'A' as u16,
                32,
                'A' as u16,
                PARA_END,
            ],
            line_segs: vec![LineSeg {
                width: 2500,
                text_height: 1000,
                baseline: 850,
                flags: 0x60000,
                ..Default::default()
            }],
            src: Some(SharedBytes::empty()),
            ..Default::default()
        };
        let placed = |p: &Paragraph| {
            let layout = format(
                p,
                &info,
                &mut Styles::new(&info, &fonts),
                ParaEnv {
                    column_width: 8000,
                    default_tab: 4000,
                },
                None,
            );
            let mut glyphs = Vec::new();
            place(&layout, &layout.lines[0], &mut glyphs);
            glyphs
        };
        let plain = placed(&p);
        let at = |pos| plain.iter().find(|g| g.pos == pos).unwrap();
        // The source gap remains fixed and earlier text stays untouched.
        // Only the following field condenses to its physical right edge,
        // as Hancom's right-aligned header fields do.
        assert_eq!(at(1).x, 500);
        assert_eq!(at(9).x, 1200);
        assert_eq!(at(11).x + at(11).advance, 2500);
        // A field that fits keeps the cached gap.
        let mut fits = p.clone();
        fits.line_segs[0].width = 3000;
        let kept = placed(&fits);
        assert_eq!(kept.iter().find(|g| g.pos == 9).unwrap().x, 1200);
        // An inline leader instead resolves the applicable manual stop.
        p.text[4] |= 3;
        let leader = placed(&p);
        assert_eq!(leader.iter().find(|g| g.pos == 9).unwrap().x, 900);
    }

    #[test]
    fn trailing_imported_tabs_keep_ink_alignment_without_stretching_prior_text() {
        let (info, fonts) = (DocInfo::default(), FontSystem::new());
        let mut text: Vec<u16> = "A A".encode_utf16().collect();
        text.extend([9, 700, 0, 1 << 8, 0, 0, 0, 9]);
        text.extend("A A".encode_utf16());
        text.extend([9, 700, 0, 1 << 8, 0, 0, 0, 9]);
        let next_line = text.len() as u32;
        text.extend(['Q' as u16, PARA_END]);
        let segment = |text_start, vpos| LineSeg {
            text_start,
            vpos,
            width: 4000,
            text_height: 1000,
            baseline: 850,
            flags: 0x60000,
            ..Default::default()
        };
        let p = Paragraph {
            text,
            line_segs: vec![segment(0, 0), segment(next_line, 1600)],
            src: Some(SharedBytes::empty()),
            ..Default::default()
        };
        let mut styles = Styles::new(&info, &fonts);
        let mut layout = format(
            &p,
            &info,
            &mut styles,
            ParaEnv {
                column_width: 8000,
                default_tab: 4000,
            },
            None,
        );
        let mut placed = Vec::new();
        place(&layout, &layout.lines[0], &mut placed);
        let at = |pos| placed.iter().find(|g| g.pos == pos).unwrap();
        // The trailing anchor keeps earlier blanks at their natural width.
        // Its empty gap is excluded from the alignment ink span.
        assert_eq!(at(13).x + at(13).advance, 3700);
        assert_eq!(at(14).x, 3700);
        assert_eq!(at(11).x - at(3).x, 700);
        assert_eq!(at(2).x, 1000);
        assert_eq!(at(13).x - at(12).x, at(12).advance);
        // Native type 0 tabs have the same field anchoring: blanks before
        // them do not share justification after the tab.
        let mut anchored = p.clone();
        anchored.text[6] = 0;
        anchored.text[17] = 0;
        let anchored_layout = format(
            &anchored,
            &info,
            &mut Styles::new(&info, &fonts),
            ParaEnv {
                column_width: 8000,
                default_tab: 4000,
            },
            None,
        );
        let mut before = Vec::new();
        place(&anchored_layout, &anchored_layout.lines[0], &mut before);
        assert_eq!(before.iter().find(|g| g.pos == 2).unwrap().x, 1000);
        assert_eq!(before.iter().find(|g| g.pos == 11).unwrap().x, 2200);
        // A live reflow drops that authority: its computed tab anchors the
        // next line without stretching the blank before it.
        reflow(&mut layout, &p, &info, &mut styles, 0, &mut |_| (0, 4000));
        let mut fresh = Vec::new();
        place(&layout, &layout.lines[0], &mut fresh);
        assert_eq!(fresh.iter().find(|g| g.pos == 2).unwrap().x, 1000);
    }

    #[test]
    fn imported_tabs_keep_cached_advances_until_a_manual_stop_applies() {
        use hm_model::{ImportedLineMembership, ParaHead, TabDef, TabStop};

        let fonts = FontSystem::new();
        let mut info = DocInfo {
            para_shapes: vec![ParaShape::default()],
            tab_defs: vec![TabDef::default()],
            ..Default::default()
        };
        let env = ParaEnv {
            column_width: 8000,
            default_tab: 4000,
        };
        let paragraph = |kind: u16, width: u16| Paragraph {
            text: vec![
                'A' as u16,
                9,
                width,
                0,
                kind << 8,
                0,
                0,
                0,
                9,
                'Q' as u16,
                PARA_END,
            ],
            line_segs: vec![LineSeg {
                width: 8000,
                text_height: 1000,
                baseline: 850,
                flags: 0x60000,
                ..Default::default()
            }],
            src: Some(SharedBytes::empty()),
            ..Default::default()
        };
        let format = |p: &Paragraph, info: &DocInfo, stored| {
            let mut styles = Styles::new(info, &fonts);
            styles.stored_lines = stored;
            super::format(p, info, &mut styles, env, None)
        };
        let after = |layout: &ParaLayout| {
            let mut placed = Vec::new();
            place(layout, &layout.lines[0], &mut placed);
            placed.iter().find(|g| g.pos == 9).unwrap().x
        };
        for kind in [0, 1, 2] {
            assert_eq!(after(&format(&paragraph(kind, 700), &info, true)), 1200);
        }
        assert_eq!(after(&format(&paragraph(1, 0), &info, true)), 500);
        let mut p = paragraph(1, 700);
        // The head is not stored, so tabs on its line follow their stops.
        info.para_shapes[0].attr1 = 2 << 23;
        let headed = format_body(
            &p,
            &info,
            &mut Styles::new(&info, &fonts),
            env,
            Some(&Head {
                image: None,
                text: "1".into(),
                info: ParaHead::default(),
            }),
        );
        assert_eq!(after(&headed), 4000);
        info.para_shapes[0].attr1 = 0;
        // The source fact is independent of our strict segment validation.
        p.line_segs[0].width = 9000;
        assert_eq!(after(&format(&p, &info, true)), 1200);
        p.line_segs[0].width = 8000;
        assert_eq!(after(&format(&p, &info, false)), 4000);
        p.src = None;
        assert_eq!(after(&format(&p, &info, true)), 4000);
        p.src = Some(SharedBytes::empty());
        p.line_segs.clear();
        assert_eq!(after(&format(&p, &info, true)), 4000);
        p = paragraph(1, 700);
        info.imported_line_membership = ImportedLineMembership::Recompose;
        assert_eq!(after(&format(&p, &info, true)), 4000);
        info.imported_line_membership = ImportedLineMembership::Unknown;
        // Plain gaps retain their width beside a manual stop. A leader
        // resolves that stop, while an exhausted stop keeps its cached gap.
        info.tab_defs[0].tabs = vec![TabStop {
            position: 6000,
            kind: 1,
            ..Default::default()
        }];
        assert_eq!(after(&format(&p, &info, true)), 1200);
        p = paragraph(2, 700);
        assert_eq!(after(&format(&p, &info, true)), 1200);
        p.text[4] |= 3;
        assert_eq!(after(&format(&p, &info, true)), 2500);
        info.tab_defs[0].tabs[0].position = 800;
        assert_eq!(after(&format(&p, &info, true)), 1200);
        // Live reflow deliberately recalculates the grid from the new line.
        info.tab_defs[0].tabs.clear();
        let mut styles = Styles::new(&info, &fonts);
        let mut layout = super::format(&p, &info, &mut styles, env, None);
        reflow(&mut layout, &p, &info, &mut styles, 0, &mut |_| (0, 8000));
        assert_eq!(after(&layout), 4000);
    }

    #[test]
    fn lines_recomposed_around_objects_compute_their_tabs_again() {
        use hm_model::{Control, ControlKind, CtrlId, ObjectCommon, ObjectControl, ObjectHeader};
        use hm_model::{TabDef, TabStop};

        let fonts = FontSystem::new();
        let info = DocInfo {
            para_shapes: vec![ParaShape::default()],
            tab_defs: vec![TabDef {
                tabs: vec![TabStop {
                    position: 12000,
                    kind: 1,
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let env = ParaEnv {
            column_width: 8000,
            default_tab: 4000,
        };
        let drawing = |attr| Control {
            kind: ControlKind::Object(Box::new(ObjectControl {
                id: CtrlId::SHAPE,
                header: ObjectHeader {
                    common: ObjectCommon {
                        attr,
                        width: 1000,
                        height: 1000,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                parts: Vec::new(),
            })),
            src: None,
        };
        // A drawing anchored before a right-tab field whose cached gap is
        // wider than the stop now leaves.
        let mut p = Paragraph {
            text: [
                11,
                0,
                0,
                0,
                0,
                0,
                0,
                11,
                'A' as u16,
                9,
                7000,
                0,
                2 << 8,
                0,
                0,
                0,
                9,
            ]
            .into_iter()
            .chain("BB".encode_utf16())
            .chain([PARA_END])
            .collect(),
            line_segs: vec![LineSeg {
                width: 8000,
                text_height: 1000,
                baseline: 850,
                flags: 0x60000,
                ..Default::default()
            }],
            src: Some(SharedBytes::empty()),
            controls: vec![drawing((2 << 3) | (4 << 15) | (2 << 18))],
            ..Default::default()
        };
        let field = |p: &Paragraph| {
            let layout = format_body(p, &info, &mut Styles::new(&info, &fonts), env, None);
            let mut placed = Vec::new();
            place(&layout, &layout.lines[0], &mut placed);
            let x = placed.iter().find(|g| g.pos == 17).map(|g| g.x);
            (layout.lines.len(), x)
        };
        // Square wrapping composes the line again: its tab reaches the stop
        // and the field stays on the line.
        assert_eq!(field(&p), (1, Some(5000)));
        // A malformed cache keeps that object-driven composition.
        p.line_segs[0].flags |= 1 << 16;
        assert_eq!(field(&p), (1, Some(5000)));
        p.line_segs[0].flags &= !(1 << 16);
        // Floating objects refresh tabs even when they do not wrap text.
        for wrap in [1, 2, 3] {
            p.controls = vec![drawing((2 << 3) | (4 << 15) | (2 << 18) | (wrap << 21))];
            assert_eq!(field(&p), (1, Some(5000)), "wrap {wrap}");
        }
        // Removing the visual object restores an imported gap that fits.
        p.controls.clear();
        p.text[10] = 6000;
        assert_eq!(field(&p), (1, Some(6500)));
        // One that would overrun the stored line keeps its gap; the field
        // after it condenses to the line as Hancom's does.
        p.text[10] = 7000;
        assert_eq!(field(&p), (1, Some(7500)));
        // The visual object refreshes earlier lines too, rather than only
        // the line that owns its anchor.
        p.controls = vec![drawing((2 << 3) | (4 << 15) | (2 << 18) | (2 << 21))];
        let prefix = [
            'A' as u16,
            9,
            7000,
            0,
            2 << 8,
            0,
            0,
            0,
            9,
            'B' as u16,
            'B' as u16,
            10,
        ];
        p.text.splice(0..0, prefix);
        p.line_segs.push(LineSeg {
            text_start: 12,
            vpos: 1600,
            ..p.line_segs[0].clone()
        });
        let layout = format_body(&p, &info, &mut Styles::new(&info, &fonts), env, None);
        let mut fields = Vec::new();
        for line in &layout.lines {
            let mut placed = Vec::new();
            place(&layout, line, &mut placed);
            fields.extend(
                placed
                    .iter()
                    .filter(|g| matches!(g.pos, 9 | 29))
                    .map(|g| (g.pos, g.x)),
            );
        }
        assert_eq!(fields, [(9, 5000), (29, 5000)]);
    }

    #[test]
    fn body_character_borders_take_room_at_each_physical_fragment() {
        use hm_model::{BorderFill, BorderLine, CharRun, CharShape, Control, ControlKind};
        use hm_model::{CtrlId, ObjectCommon, ObjectControl, ObjectHeader};
        let plain = CharShape {
            base_size: 1000,
            ratios: [100; 7],
            ..Default::default()
        };
        let boxed = CharShape {
            border_fill: Some(1),
            ..plain.clone()
        };
        let mut info = DocInfo {
            char_shapes: vec![
                plain,
                boxed.clone(),
                CharShape {
                    color: 0xFF,
                    ..boxed
                },
            ],
            border_fills: vec![BorderFill {
                sides: [BorderLine {
                    kind: BorderLine::SOLID,
                    width: 2,
                    color: 0,
                }; 4],
                ..Default::default()
            }],
            ..Default::default()
        };
        let fonts = FontSystem::new();
        let paragraph = |text: &str, start, split, stop| Paragraph {
            text: text.encode_utf16().chain([PARA_END]).collect(),
            runs: vec![
                CharRun { pos: 0, shape: 0 },
                CharRun {
                    pos: start,
                    shape: 1,
                },
                CharRun {
                    pos: split,
                    shape: 2,
                },
                CharRun {
                    pos: stop,
                    shape: 0,
                },
            ],
            ..Default::default()
        };
        let positions = |p: &Paragraph, info: &DocInfo, width| {
            let layout = super::format_body(
                p,
                info,
                &mut Styles::new(info, &fonts),
                ParaEnv {
                    column_width: width,
                    default_tab: 4000,
                },
                None,
            );
            layout
                .lines
                .iter()
                .map(|line| {
                    let mut placed = Vec::new();
                    place(&layout, line, &mut placed);
                    placed
                        .into_iter()
                        .filter(|g| matches!(g.kind, Kind::Char(_)))
                        .map(|g| (g.pos, g.x))
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        };
        // A color-only character-shape change keeps one bordered fragment.
        assert_eq!(
            positions(&paragraph("ABCD", 1, 2, 3), &info, 8000)[0],
            [(0, 0), (1, 542), (2, 1042), (3, 1584)]
        );
        assert_eq!(
            positions(&paragraph("BCD", 0, 1, 2), &info, 8000)[0],
            [(0, 42), (1, 542), (2, 1084)]
        );
        assert_eq!(
            positions(&paragraph("A\nBCD", 2, 3, 4), &info, 8000)[1],
            [(2, 42), (3, 542), (4, 1084)]
        );
        assert_eq!(
            positions(&paragraph("AAAA BCD", 5, 6, 7), &info, 2200)[1],
            [(5, 42), (6, 542), (7, 1084)]
        );
        // A fragment that continues on the next line takes fresh side room.
        let continued = positions(&paragraph("A\nBBBBBBCD", 2, 8, 9), &info, 2700);
        assert_eq!(continued[1].first(), Some(&(2, 42)));
        assert_eq!(continued[1].len(), 5);
        assert_eq!(continued[2], [(7, 42), (8, 542), (9, 1084)]);
        let mut tabbed = paragraph("A", 1, 2, 3);
        tabbed.text = ['A' as u16, 9, 0, 0, 1 << 8, 0, 0, 0, 9]
            .into_iter()
            .chain("BCD".encode_utf16())
            .chain([PARA_END])
            .collect();
        tabbed.runs = vec![
            CharRun { pos: 0, shape: 0 },
            CharRun { pos: 9, shape: 1 },
            CharRun { pos: 10, shape: 2 },
            CharRun { pos: 11, shape: 0 },
        ];
        assert_eq!(
            positions(&tabbed, &info, 8000)[0],
            [(0, 0), (9, 4042), (10, 4542), (11, 5084)]
        );
        // An inline object's separate composition omits character-border room.
        let mut object = paragraph("ABCD", 9, 10, 11);
        object.text.splice(0..0, [11, 0, 0, 0, 0, 0, 0, 11]);
        object.controls.push(Control {
            kind: ControlKind::Object(Box::new(ObjectControl {
                id: CtrlId::SHAPE,
                header: ObjectHeader {
                    common: ObjectCommon {
                        attr: 1 | (4 << 15) | (2 << 18),
                        width: 1000,
                        height: 1000,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                parts: Vec::new(),
            })),
            src: None,
        });
        assert_eq!(
            positions(&object, &info, 8000)[0],
            [(8, 1000), (9, 1500), (10, 2000), (11, 2500)]
        );
        info.border_fills[0].sides[0].kind = 0;
        info.border_fills[0].sides[1].kind = 0;
        assert_eq!(
            positions(&paragraph("ABCD", 1, 2, 3), &info, 8000)[0],
            [(0, 0), (1, 500), (2, 1000), (3, 1500)]
        );
    }

    #[test]
    fn memoized_border_contexts_match_their_uncached_composition() {
        use hm_model::{BorderFill, BorderLine, CharShape};
        let info = DocInfo {
            char_shapes: vec![CharShape {
                base_size: 1000,
                border_fill: Some(1),
                ..Default::default()
            }],
            border_fills: vec![BorderFill {
                sides: [BorderLine {
                    kind: 1,
                    width: 2,
                    color: 0,
                }; 4],
                ..Default::default()
            }],
            ..Default::default()
        };
        let fonts = FontSystem::new();
        let p = Paragraph {
            text: "B\r".encode_utf16().collect(),
            src: Some(SharedBytes::empty()),
            ..Default::default()
        };
        let env = ParaEnv {
            column_width: 8000,
            default_tab: 4000,
        };
        let positions = |body, styles: &mut Styles<'_>| {
            let laid = if body {
                format_body(&p, &info, styles, env, None)
            } else {
                format(&p, &info, styles, env, None)
            };
            let mut placed = Vec::new();
            place(&laid, &laid.lines[0], &mut placed);
            placed
                .into_iter()
                .filter(|g| matches!(g.kind, Kind::Char(_)))
                .map(|g| g.x)
                .collect::<Vec<_>>()
        };
        let expected = [
            positions(false, &mut Styles::new(&info, &fonts)),
            positions(true, &mut Styles::new(&info, &fonts)),
        ];
        assert_eq!(expected[1], [42]);
        for order in [[false, true, true, false], [true, false, false, true]] {
            let mut styles = Styles::new(&info, &fonts);
            styles.memo = Some(crate::incr::Memo::default());
            for body in order {
                assert_eq!(positions(body, &mut styles), expected[body as usize]);
            }
        }
    }

    #[test]
    fn fixed_narrow_heads_use_half_em_cells_before_body_text() {
        use hm_model::{CharShape, FaceName, HwpString, ParaHead};
        use std::sync::Arc;

        let mut fonts = FontSystem::new();
        fonts.add(
            "portable Noto subset",
            Arc::new(include_bytes!("../../hm-fonts/tests/fixtures/source.ttf").to_vec()),
        );
        let family = fonts.entries()[0].families[0].clone();
        for size in [950, 1200] {
            let mut info = DocInfo {
                faces: std::array::from_fn(|_| {
                    vec![FaceName {
                        name: HwpString::from_text(&family),
                        ..Default::default()
                    }]
                }),
                char_shapes: vec![CharShape {
                    base_size: size,
                    ..Default::default()
                }],
                para_shapes: vec![ParaShape {
                    attr1: 3 << 23,
                    ..Default::default()
                }],
                ..Default::default()
            };
            let p = Paragraph {
                text: "Q".encode_utf16().chain([PARA_END]).collect(),
                ..Default::default()
            };
            let origin = |info: &DocInfo, text: &str, attr| {
                let head = Head {
                    image: None,
                    text: text.into(),
                    info: ParaHead {
                        attr,
                        text_distance: 50,
                        ..Default::default()
                    },
                };
                let laid = format_body(
                    &p,
                    info,
                    &mut Styles::new(info, &fonts),
                    ParaEnv {
                        column_width: 100_000,
                        default_tab: 4000,
                    },
                    Some(&head),
                );
                let mut placed = Vec::new();
                place(&laid, &laid.lines[0], &mut placed);
                placed[0].x
            };
            // A fixed soft-hyphen bullet reserves a half-em cell plus the
            // authored 50 percent gap; other narrow bullets stay measured
            // unless they hang automatically.
            assert_eq!(origin(&info, "\u{ad}", 0), size);
            let instance = origin(&info, "A", 1 << 2);
            assert!(instance > size);
            assert_eq!(origin(&info, "A", 0), instance);
            assert_eq!(origin(&info, "A", 1 << 3), size);
            // Wide bullets retain their one-em minimum. Ordinary numbered
            // heads stay measured; automatic hanging uses its fixed cell.
            assert_eq!(origin(&info, "각", 0), size + size / 2);
            info.para_shapes[0].attr1 = 2 << 23;
            assert_eq!(origin(&info, "A", 0), instance);
            assert!(origin(&info, "AA", 0) > instance);
            assert_eq!(origin(&info, "A", 1 << 3), size);
            assert_eq!(origin(&info, "AA", 1 << 3), size);
            assert_eq!(origin(&info, "A", (1 << 3) | (1 << 2)), instance);
        }
    }

    #[test]
    fn head_widths_leave_out_the_last_glyphs_character_spacing() {
        use hm_model::{CharShape, FaceName, HwpString, ParaHead};
        use std::sync::Arc;

        let mut fonts = FontSystem::new();
        fonts.add(
            "portable Noto subset",
            Arc::new(include_bytes!("../../hm-fonts/tests/fixtures/source.ttf").to_vec()),
        );
        let family = fonts.entries()[0].families[0].clone();
        let info = |spacing: i8| DocInfo {
            faces: std::array::from_fn(|_| {
                vec![FaceName {
                    name: HwpString::from_text(&family),
                    ..Default::default()
                }]
            }),
            char_shapes: vec![CharShape {
                base_size: 1000,
                spacings: [spacing; 7],
                ..Default::default()
            }],
            para_shapes: vec![ParaShape {
                attr1: 2 << 23,
                ..Default::default()
            }],
            ..Default::default()
        };
        let p = Paragraph {
            text: "Q".encode_utf16().chain([PARA_END]).collect(),
            ..Default::default()
        };
        let origin = |info: &DocInfo, text: &str| {
            let head = Head {
                image: None,
                text: text.into(),
                info: ParaHead {
                    // Automatic hanging at the instance text's width.
                    attr: (1 << 3) | (1 << 2),
                    text_distance: 50,
                    ..Default::default()
                },
            };
            let laid = format(
                &p,
                info,
                &mut Styles::new(info, &fonts),
                ParaEnv {
                    column_width: 100_000,
                    default_tab: 4000,
                },
                Some(&head),
            );
            let mut placed = Vec::new();
            place(&laid, &laid.lines[0], &mut placed);
            (placed[0].x, laid.hang)
        };
        // Tight spacing narrows every glyph of the head but its last: a
        // one-glyph "-" keeps its full advance, so the text and the hanging
        // indent start where they would without spacing.
        let (loose, tight) = (info(0), info(-10));
        assert_eq!(origin(&tight, "A"), origin(&loose, "A"));
        let (x, hang) = origin(&tight, "AA");
        assert!(x < origin(&loose, "AA").0);
        assert!(x > origin(&tight, "A").0);
        assert_eq!(x, hang);
    }

    #[test]
    fn unfilled_click_here_guides_do_not_freeze_print_line_breaks() {
        use hm_model::{Control, ControlKind, CtrlId, Field, HwpString};
        let info = DocInfo {
            char_shapes: vec![hm_model::CharShape {
                base_size: 1000,
                ..Default::default()
            }],
            para_shapes: vec![ParaShape {
                attr1: 2 << 5,
                ..Default::default()
            }],
            ..Default::default()
        };
        let fonts = FontSystem::new();
        let mut p = Paragraph {
            text: [3; 8]
                .into_iter()
                .chain("AB".encode_utf16())
                .chain([4; 8])
                .chain("abcdef\r".encode_utf16())
                .collect(),
            controls: vec![Control {
                kind: ControlKind::Field(Box::new(Field {
                    id: CtrlId::from_chars(b"%clk"),
                    command: HwpString::from_text("Clickhere:set:22:Direction:wstring:2:AB"),
                    ..Default::default()
                })),
                src: None,
            }],
            line_segs: [0, 20]
                .into_iter()
                .enumerate()
                .map(|(k, text_start)| LineSeg {
                    text_start,
                    vpos: k as i32 * 1600,
                    text_height: 1300,
                    baseline: 1105,
                    spacing: 300,
                    width: 2000,
                    flags: 0x60000,
                    ..Default::default()
                })
                .collect(),
            src: Some(SharedBytes::empty()),
            ..Default::default()
        };
        let env = ParaEnv {
            column_width: 2000,
            default_tab: 4000,
        };
        let laid = |p: &Paragraph, guides: bool| {
            let mut styles = Styles::new(&info, &fonts);
            styles.guides = guides;
            let laid = format(p, &info, &mut styles, env, None);
            laid.lines
                .iter()
                .map(|l| (l.start, l.height, l.baseline))
                .collect::<Vec<_>>()
        };
        let lines = |p: &Paragraph| laid(p, false);
        assert_eq!(crate::annot::unprinted_ranges(&p), [(8, 10)]);
        assert_eq!(lines(&p), [(0, 1300, 1105), (22, 1300, 1105)]);
        // The editing screen shows the guide where Hancom stored the lines.
        assert_eq!(laid(&p, true), [(0, 1300, 1105), (20, 1300, 1105)]);
        let ControlKind::Field(field) = &mut p.controls[0].kind else {
            unreachable!()
        };
        field.attr |= 1 << 15;
        assert_eq!(lines(&p), [(0, 1300, 1105), (20, 1300, 1105)]);
    }

    /// A four-character paragraph whose stored lines break after two.
    fn para() -> Paragraph {
        let seg = |text_start| LineSeg {
            text_start,
            vpos: text_start as i32 * 1000,
            width: 100_000,
            flags: 0x60000,
            ..LineSeg::default()
        };
        Paragraph {
            text: "가나다라".encode_utf16().chain([PARA_END]).collect(),
            line_segs: vec![seg(0), seg(2)],
            src: Some(SharedBytes::from(Vec::new())),
            ..Paragraph::default()
        }
    }

    fn starts(stored_lines: bool, p: &Paragraph) -> Vec<u32> {
        let (info, fonts) = (DocInfo::default(), FontSystem::new());
        let mut styles = Styles::new(&info, &fonts);
        styles.stored_lines = stored_lines;
        let env = ParaEnv {
            column_width: 100_000,
            default_tab: 4000,
        };
        let pl = format(p, &info, &mut styles, env, None);
        pl.lines.iter().map(|l| l.start).collect()
    }

    #[test]
    fn unedited_paragraphs_keep_their_stored_lines() {
        let p = para();
        assert_eq!(starts(true, &p), [0, 2]);
        assert_eq!(starts(false, &p), [0]);
        // An edited paragraph, or a first segment wider than the margins
        // or not a whole line, breaks fresh; later segments do not matter.
        let edited = Paragraph {
            src: None,
            ..para()
        };
        assert_eq!(starts(true, &edited), [0]);
        let mut wide = para();
        wide.line_segs[0].width += 1;
        assert_eq!(starts(true, &wide), [0]);
        let mut empty = para();
        empty.line_segs[0].flags |= 1 << 16;
        assert_eq!(starts(true, &empty), [0]);
        let mut later = para();
        later.line_segs[1].width = 150_000;
        assert_eq!(starts(true, &later), [0, 2]);
    }

    #[test]
    fn fixed_spacing_compatibility_uses_the_advance_for_the_line_and_baseline() {
        let fonts = FontSystem::new();
        let mut info = DocInfo {
            compatible: Some(hm_model::CompatibleDocument {
                target: 2,
                ..Default::default()
            }),
            layout_compat: Some(hm_model::LayoutCompatibility {
                flags: [0x1e3, 0xcf_f8ff, 0x4f, 0x1ec, 0],
                ..Default::default()
            }),
            char_shapes: vec![hm_model::CharShape {
                base_size: 1600,
                ..Default::default()
            }],
            para_shapes: vec![hm_model::ParaShape {
                attr3: Some(1),
                line_spacing: Some(hm_model::units::pack_distance(1000) as u32),
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut p = Paragraph {
            text: vec!['A' as u16, PARA_END],
            ..Default::default()
        };
        let env = ParaEnv {
            column_width: 100_000,
            default_tab: 4000,
        };
        let line = |p: &Paragraph, info: &DocInfo| {
            format(p, info, &mut Styles::new(info, &fonts), env, None).lines[0]
        };
        let fresh = line(&p, &info);
        assert_eq!(
            (fresh.height, fresh.baseline, fresh.spacing),
            (1000, 800, 0)
        );
        p.src = Some(SharedBytes::empty());
        p.line_segs = vec![LineSeg {
            height: 2767,
            text_height: 2767,
            baseline: 2061,
            spacing: -747,
            width: 100_000,
            flags: 0x60000,
            ..Default::default()
        }];
        let imported = line(&p, &info);
        assert_eq!(
            (imported.height, imported.baseline, imported.spacing),
            (2020, 1616, 0)
        );
        info.compatible = None;
        let ordinary = line(&p, &info);
        assert_eq!(
            (ordinary.height, ordinary.baseline, ordinary.spacing),
            (2767, 2061, -747)
        );
    }

    #[test]
    fn compressed_percent_spacing_compatibility_caps_the_baseline_at_four_fifths_of_the_pitch() {
        let fonts = FontSystem::new();
        let mut info = DocInfo {
            compatible: Some(hm_model::CompatibleDocument {
                target: 2,
                ..Default::default()
            }),
            layout_compat: Some(hm_model::LayoutCompatibility {
                flags: [0x1e3, 0xcf_f8ff, 0x4f, 0x1ec, 0],
                ..Default::default()
            }),
            char_shapes: vec![hm_model::CharShape {
                base_size: 1100,
                ..Default::default()
            }],
            para_shapes: vec![hm_model::ParaShape {
                attr3: Some(0),
                line_spacing: Some(71),
                ..Default::default()
            }],
            ..Default::default()
        };
        let seg = |spacing| LineSeg {
            height: 1903,
            text_height: 1903,
            baseline: 1417,
            spacing,
            width: 100_000,
            flags: 0x60000,
            ..Default::default()
        };
        let mut p = Paragraph {
            text: vec!['A' as u16, PARA_END],
            src: Some(SharedBytes::empty()),
            line_segs: vec![seg(-552)],
            ..Default::default()
        };
        let env = ParaEnv {
            column_width: 100_000,
            default_tab: 4000,
        };
        let line = |p: &Paragraph, info: &DocInfo| {
            format(p, info, &mut Styles::new(info, &fonts), env, None).lines[0]
        };
        // A 71% line keeps its height and spacing; only the baseline moves up
        // to four fifths of the 1351 HU pitch.
        let compressed = line(&p, &info);
        assert_eq!(
            (compressed.height, compressed.baseline, compressed.spacing),
            (1903, 1081, -552)
        );
        // Lines that are not compressed keep the stored baseline.
        p.line_segs = vec![seg(0)];
        assert_eq!(line(&p, &info).baseline, 1417);
        p.line_segs = vec![seg(-552)];
        info.compatible = None;
        assert_eq!(line(&p, &info).baseline, 1417);
    }

    #[test]
    fn a_numbered_body_line_can_wrap_after_its_head() {
        let info = DocInfo {
            char_shapes: vec![hm_model::CharShape {
                base_size: 1000,
                ..Default::default()
            }],
            para_shapes: vec![ParaShape {
                attr1: (2 << 23) | (2 << 5),
                line_spacing_legacy: 160,
                ..Default::default()
            }],
            ..Default::default()
        };
        let fonts = FontSystem::new();
        let mut styles = Styles::new(&info, &fonts);
        let p = Paragraph {
            text: "abcdefgh".encode_utf16().chain([PARA_END]).collect(),
            line_segs: vec![LineSeg {
                width: 3000,
                text_height: 1000,
                baseline: 850,
                spacing: 600,
                flags: 0x260000,
                ..Default::default()
            }],
            src: Some(SharedBytes::empty()),
            ..Default::default()
        };
        let head = Head {
            image: None,
            text: "1.".into(),
            info: hm_model::ParaHead {
                attr: 4,
                text_distance: 50,
                ..Default::default()
            },
        };
        let env = ParaEnv {
            column_width: 3000,
            default_tab: 4000,
        };
        let nested = format(&p, &info, &mut styles, env, Some(&head));
        assert_eq!(nested.lines.len(), 1);
        let body = format_body(&p, &info, &mut styles, env, Some(&head));
        assert_eq!(body.lines.len(), 2);
        assert!(body.lines.iter().all(|l| l.used <= l.width));
        assert_eq!(body.lines[1].height, 1000);
        assert_eq!(body.lines.last().unwrap().end, p.len());
        assert_eq!(
            format(&p, &info, &mut styles, env, Some(&head)).lines,
            nested.lines
        );
    }

    #[test]
    fn body_heads_refresh_first_metrics_while_nested_lists_replay() {
        let info = DocInfo {
            char_shapes: vec![hm_model::CharShape {
                base_size: 1000,
                ..Default::default()
            }],
            para_shapes: vec![ParaShape {
                attr1: (2 << 23) | (2 << 5),
                line_spacing_legacy: 160,
                ..Default::default()
            }],
            ..Default::default()
        };
        let fonts = FontSystem::new();
        let mut styles = Styles::new(&info, &fonts);
        // Stored lines with stale metrics: 777 high where the text is 1000.
        let seg = |text_start, flags| LineSeg {
            text_start,
            vpos: text_start as i32 * 1000,
            width: 100_000,
            text_height: 777,
            baseline: 660,
            spacing: 466,
            flags,
            ..Default::default()
        };
        let p = Paragraph {
            text: "abcdefgh".encode_utf16().chain([PARA_END]).collect(),
            line_segs: vec![seg(0, 0x260000), seg(4, 0x60000)],
            src: Some(SharedBytes::empty()),
            ..Default::default()
        };
        let head = Head {
            image: None,
            text: "1.".into(),
            info: hm_model::ParaHead {
                attr: 4,
                text_distance: 50,
                ..Default::default()
            },
        };
        let env = ParaEnv {
            column_width: 100_000,
            default_tab: 4000,
        };
        let lines = |p: &Paragraph, styles: &mut Styles<'_>, head| -> Vec<(u32, i32)> {
            let laid = format_body(p, &info, styles, env, head);
            laid.lines.iter().map(|l| (l.start, l.height)).collect()
        };
        // Manual heads retain cached membership and refresh first-line metrics.
        assert_eq!(lines(&p, &mut styles, Some(&head)), [(0, 1000), (4, 777)]);
        // Valid empty formats refit; an unresolved definition keeps its cache.
        let empty = Head {
            text: String::new(),
            ..head.clone()
        };
        assert_eq!(lines(&p, &mut styles, Some(&empty)), [(0, 1000)]);
        assert_eq!(lines(&p, &mut styles, None), [(0, 777), (4, 777)]);
        // Generic nested lists keep their imported membership and metrics.
        assert_eq!(
            format(&p, &info, &mut styles, env, Some(&head))
                .lines
                .iter()
                .map(|l| (l.start, l.height))
                .collect::<Vec<_>>(),
            [(0, 777), (4, 777)]
        );
        // A single stored line is fitted again with fresh metrics.
        let single = Paragraph {
            line_segs: vec![seg(0, 0x260000)],
            ..p.clone()
        };
        assert_eq!(lines(&single, &mut styles, Some(&head)), [(0, 1000)]);
    }

    #[test]
    fn cell_reflow_refreshes_the_inline_table_hosts_height() {
        use hm_model::{
            Cell, Control, ControlKind, CtrlId, ObjectCommon, ObjectControl, ObjectHeader, Table,
        };
        let inline = |width, height| ObjectHeader {
            common: ObjectCommon {
                attr: 1,
                width,
                height,
                ..Default::default()
            },
            ..Default::default()
        };
        let seg = |text_start, vpos, text_height| LineSeg {
            text_start,
            vpos,
            width: 10000,
            text_height,
            baseline: text_height * 85 / 100,
            spacing: 600,
            flags: 0x60000,
            ..Default::default()
        };
        let cell = Paragraph {
            text: [
                vec![11; 8],
                "abcdef".encode_utf16().collect(),
                vec![PARA_END],
            ]
            .concat(),
            controls: vec![Control {
                kind: ControlKind::Object(Box::new(ObjectControl {
                    id: CtrlId::SHAPE,
                    header: inline(1000, 1000),
                    parts: Vec::new(),
                })),
                src: None,
            }],
            line_segs: vec![seg(0, 0, 1000), seg(11, 1600, 1000)],
            src: Some(SharedBytes::empty()),
            ..Default::default()
        };
        let table = Table {
            header: inline(10000, 2600),
            rows: 1,
            cols: 1,
            cells: vec![Cell {
                width: 10000,
                row_span: 1,
                col_span: 1,
                paras: vec![cell],
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut parent = Paragraph {
            text: [vec![11; 8], vec![PARA_END]].concat(),
            controls: vec![Control {
                kind: ControlKind::Table(Box::new(table)),
                src: None,
            }],
            line_segs: vec![seg(0, 0, 2600)],
            src: Some(SharedBytes::empty()),
            ..Default::default()
        };
        let (info, fonts) = (DocInfo::default(), FontSystem::new());
        let mut styles = Styles::new(&info, &fonts);
        let env = ParaEnv {
            column_width: 10000,
            default_tab: 4000,
        };
        let laid = format(&parent, &info, &mut styles, env, None);
        let table = laid.table(0).unwrap();
        assert_eq!(table.cells[0].paras[0].lines.len(), 1);
        assert_eq!(laid.lines[0].height, table.height);
        assert!(laid.lines[0].height < parent.line_segs[0].text_height);

        let ControlKind::Table(table) = &mut parent.controls[0].kind else {
            unreachable!()
        };
        table.cells[0].paras[0].line_segs.truncate(1);
        let kept = format(&parent, &info, &mut styles, env, None);
        assert_eq!(kept.lines[0].height, parent.line_segs[0].text_height);
    }

    #[test]
    fn a_cell_line_sized_anew_refreshes_the_inline_table_hosts_height() {
        use hm_model::{
            Cell, Control, ControlKind, CtrlId, ObjectCommon, ObjectControl, ObjectHeader, Table,
        };
        let inline = |width, height| ObjectHeader {
            common: ObjectCommon {
                attr: 1,
                width,
                height,
                ..Default::default()
            },
            ..Default::default()
        };
        let seg = |height| LineSeg {
            height,
            width: 10000,
            text_height: height,
            baseline: height * 85 / 100,
            spacing: 600,
            flags: 0x60000,
            ..Default::default()
        };
        // The cell's one line was stored taller than its object now makes
        // it, as Hancom for Windows stores a primed equation's line.
        let cell = |stored| Paragraph {
            text: [vec![11; 8], vec![PARA_END]].concat(),
            controls: vec![Control {
                kind: ControlKind::Object(Box::new(ObjectControl {
                    id: CtrlId::SHAPE,
                    header: inline(1000, 1000),
                    parts: Vec::new(),
                })),
                src: None,
            }],
            line_segs: vec![seg(stored)],
            src: Some(SharedBytes::empty()),
            ..Default::default()
        };
        let parent = |stored| Paragraph {
            text: [vec![11; 8], vec![PARA_END]].concat(),
            controls: vec![Control {
                kind: ControlKind::Table(Box::new(Table {
                    header: inline(10000, stored),
                    rows: 1,
                    cols: 1,
                    cells: vec![Cell {
                        width: 10000,
                        row_span: 1,
                        col_span: 1,
                        paras: vec![cell(stored)],
                        ..Default::default()
                    }],
                    ..Default::default()
                })),
                src: None,
            }],
            line_segs: vec![seg(stored)],
            src: Some(SharedBytes::empty()),
            ..Default::default()
        };
        let (info, fonts) = (DocInfo::default(), FontSystem::new());
        let mut styles = Styles::new(&info, &fonts);
        let env = ParaEnv {
            column_width: 10000,
            default_tab: 4000,
        };
        let laid = format(&parent(1400), &info, &mut styles, env, None);
        let table = laid.table(0).unwrap();
        assert!(table.height < 1400);
        assert_eq!(laid.lines[0].height, table.height);
        // A cell line that keeps its stored height keeps the host's line.
        let kept = format(&parent(1000), &info, &mut styles, env, None);
        assert_eq!(kept.lines[0].height, 1000);
    }

    #[test]
    fn inline_tables_must_fit_together_on_their_stored_line() {
        use hm_model::{Cell, Control, ControlKind, ObjectCommon, ObjectHeader, PageHide, Table};
        // Each table includes a 141-HU outer margin on either side.
        let table = |width| Table {
            header: ObjectHeader {
                common: ObjectCommon {
                    attr: 1,
                    width,
                    height: 5000,
                    margins: [141, 141, 141, 141],
                    ..Default::default()
                },
                ..Default::default()
            },
            rows: 1,
            cols: 1,
            cells: vec![Cell {
                width,
                height: 5000,
                row_span: 1,
                col_span: 1,
                paras: vec![Paragraph {
                    text: vec![PARA_END],
                    line_segs: vec![LineSeg {
                        text_height: 1000,
                        width: width - 1000,
                        flags: 0x60000,
                        ..Default::default()
                    }],
                    src: Some(SharedBytes::empty()),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        // The page hiding control, then the table, on one stored line.
        let host = |hide: bool, widths: &[i32]| {
            let mut text = Vec::new();
            let mut controls = Vec::new();
            if hide {
                text.extend([21; 8]);
                controls.push(Control {
                    kind: ControlKind::PageHide(PageHide::default()),
                    src: None,
                });
            }
            for &width in widths {
                text.extend([11; 8]);
                controls.push(Control {
                    kind: ControlKind::Table(Box::new(table(width))),
                    src: None,
                });
            }
            text.push(PARA_END);
            Paragraph {
                text,
                controls,
                line_segs: vec![LineSeg {
                    text_height: 5282,
                    baseline: 4490,
                    spacing: 600,
                    width: 10_000,
                    flags: 0x60000,
                    ..Default::default()
                }],
                src: Some(SharedBytes::empty()),
                ..Default::default()
            }
        };
        let (info, fonts) = (DocInfo::default(), FontSystem::new());
        let mut styles = Styles::new(&info, &fonts);
        let env = ParaEnv {
            column_width: 10_000,
            default_tab: 4000,
        };
        let heights = |p: &Paragraph, styles: &mut Styles<'_>| -> Vec<i32> {
            let laid = format(p, &info, styles, env, None);
            laid.lines.iter().map(|l| l.height).collect()
        };
        // The table opens a line of its own below the control.
        let crowded = heights(&host(true, &[9719]), &mut styles);
        assert_eq!(crowded.len(), 2);
        assert_eq!(crowded[1], 5282);
        // A table that fits, or one first on its line, keeps the stored line.
        assert_eq!(heights(&host(true, &[9718]), &mut styles), [5282]);
        assert_eq!(heights(&host(false, &[9719]), &mut styles), [5282]);
        // Each table fits alone; their outer widths decide whether they
        // may share the cached line.
        assert_eq!(heights(&host(false, &[4718, 4718]), &mut styles), [5282]);
        assert_eq!(
            heights(&host(false, &[4718, 4719]), &mut styles),
            [5282, 5282]
        );
    }

    #[test]
    fn memo_keeps_image_heads_distinct_from_empty_numbering_and_other_images() {
        let (info, fonts) = (DocInfo::default(), FontSystem::new());
        let mut styles = Styles::new(&info, &fonts);
        styles.memo = Some(Default::default());
        let para = Paragraph {
            text: vec!['Q' as u16, PARA_END],
            src: Some(SharedBytes::empty()),
            ..Default::default()
        };
        let env = ParaEnv {
            column_width: 10000,
            default_tab: 4000,
        };
        let mut head = Head {
            text: String::new(),
            info: hm_model::ParaHead {
                text_distance: 50,
                ..Default::default()
            },
            image: None,
        };
        assert_eq!(
            format(&para, &info, &mut styles, env, Some(&head)).lines[0].x,
            0
        );
        for bin in [1, 2, 1] {
            head.image = Some(ImageHead {
                bin,
                contrast: 0,
                brightness: 0,
                effect: 0,
            });
            let layout = format(&para, &info, &mut styles, env, Some(&head));
            assert_eq!(layout.lines[0].x, 1500);
            assert_eq!(layout.head.unwrap().image.unwrap().bin, bin);
        }
    }

    #[test]
    fn automatic_numbered_body_heads_refit_cached_spans_and_keep_later_metrics() {
        use hm_model::{CharRun, CharShape, ParaHead};

        let mut info = DocInfo {
            char_shapes: vec![
                CharShape {
                    base_size: 1000,
                    ..Default::default()
                },
                CharShape {
                    base_size: 1200,
                    ..Default::default()
                },
            ],
            para_shapes: vec![ParaShape {
                attr1: (20 << 9) | (1 << 7),
                line_spacing_legacy: 160,
                ..Default::default()
            }],
            ..Default::default()
        };
        let fonts = FontSystem::new();
        let env = ParaEnv {
            column_width: 7000,
            default_tab: 4000,
        };
        let head = Head {
            image: None,
            text: "01".into(),
            info: ParaHead {
                // Native automatic heads refit with their measured instance width.
                attr: (1 << 2) | (1 << 3),
                char_shape: 1,
                text_distance: 50,
                ..Default::default()
            },
        };
        let empty_head = Head {
            text: String::new(),
            ..head.clone()
        };
        let mut p = Paragraph {
            text: "가 나 다라마바\r".encode_utf16().collect(),
            runs: vec![CharRun { pos: 0, shape: 0 }],
            line_segs: vec![
                LineSeg {
                    width: 7000,
                    text_height: 1200,
                    baseline: 1020,
                    spacing: 720,
                    flags: 0x60000,
                    ..Default::default()
                },
                LineSeg {
                    text_start: 7,
                    vpos: 1920,
                    col_start: 1000,
                    width: 5000,
                    text_height: 1375,
                    baseline: 1138,
                    spacing: 625,
                    flags: 0x60000,
                    ..Default::default()
                },
            ],
            src: Some(SharedBytes::empty()),
            ..Default::default()
        };
        let body = |p: &Paragraph, info: &DocInfo, head| {
            let mut styles = Styles::new(info, &fonts);
            format_body(p, info, &mut styles, env, head).lines
        };
        for kind in [1, 2] {
            info.para_shapes[0].attr1 = (kind << 23) | (20 << 9) | (1 << 7);
            p.line_segs[1].text_start = 7;
            let lines = body(&p, &info, Some(&head));
            assert_eq!(
                lines.iter().map(|l| (l.start, l.end)).collect::<Vec<_>>(),
                [(0, 6), (6, 9)]
            );
            assert_eq!(
                (lines[1].height, lines[1].baseline, lines[1].spacing),
                (1375, 1138, 625)
            );
            assert_eq!((lines[1].x, lines[1].width), (2800, 3200));
            // A fitting cached line is recomputed too, rather than retained.
            p.line_segs[1].text_start = 5;
            assert_eq!(body(&p, &info, Some(&head))[0].end, 6);
            // Empty authored heads still refit at the full available width.
            assert_eq!(body(&p, &info, Some(&empty_head))[0].end, 9);
        }
        // Extra recomposed lines use the full live segment and fresh metrics,
        // rather than indexing or repeating the last cached physical line.
        let mut narrow = p.clone();
        for segment in &mut narrow.line_segs {
            segment.col_start = 0;
            segment.width = 4000;
        }
        let extra = format_body(
            &narrow,
            &info,
            &mut Styles::new(&info, &fonts),
            ParaEnv {
                column_width: 4000,
                ..env
            },
            Some(&head),
        )
        .lines;
        assert_eq!(
            extra.iter().map(|l| (l.start, l.end)).collect::<Vec<_>>(),
            [(0, 2), (2, 4), (4, 6), (6, 9)]
        );
        assert_eq!(
            (
                extra[2].width,
                extra[2].height,
                extra[2].baseline,
                extra[2].spacing
            ),
            (2200, 1000, 850, 600)
        );
        // Nested automatic hanging numbers refit their cached spans too,
        // as Hancom lays them out anew in cells; other nested heads keep
        // the imported membership.
        p.line_segs[1].text_start = 7;
        let nested = |head: &Head| {
            let mut styles = Styles::new(&info, &fonts);
            format(&p, &info, &mut styles, env, Some(head)).lines[0].end
        };
        assert_eq!(nested(&head), 6);
        let manual = Head {
            info: ParaHead {
                attr: 1 << 2,
                ..head.info
            },
            ..head.clone()
        };
        assert_eq!(nested(&manual), 7);
        // Removing the authored mode preserves the cache in body text too.
        info.para_shapes[0].attr1 = (20 << 9) | (1 << 7);
        assert_eq!(body(&p, &info, None)[0].end, 7);
    }

    #[test]
    fn bullet_body_membership_keeps_its_imported_endpoints() {
        use hm_model::{CharShape, ParaHead};
        let info = DocInfo {
            char_shapes: vec![CharShape {
                base_size: 1000,
                ..Default::default()
            }],
            para_shapes: vec![ParaShape {
                attr1: 3 << 23,
                ..Default::default()
            }],
            ..Default::default()
        };
        let fonts = FontSystem::new();
        let mut p = Paragraph {
            text: "가나다라마바사아자차\r".encode_utf16().collect(),
            line_segs: vec![
                LineSeg {
                    width: 5000,
                    text_height: 1000,
                    baseline: 850,
                    flags: 0x260000,
                    ..Default::default()
                },
                LineSeg {
                    text_start: 3,
                    vpos: 1600,
                    width: 5000,
                    text_height: 1000,
                    baseline: 850,
                    flags: 0x160000,
                    ..Default::default()
                },
            ],
            src: Some(SharedBytes::empty()),
            ..Default::default()
        };
        let head = Head {
            image: None,
            text: "◦".into(),
            info: ParaHead {
                attr: 8,
                text_distance: 10,
                ..Default::default()
            },
        };
        let env = ParaEnv {
            column_width: 5000,
            default_tab: 4000,
        };
        for cached in [2, 7] {
            p.line_segs[1].text_start = cached;
            let laid = format_body(&p, &info, &mut Styles::new(&info, &fonts), env, Some(&head));
            assert_eq!(laid.lines[0].end, cached);
            let mut fresh = Styles::new(&info, &fonts);
            fresh.stored_lines = false;
            assert_ne!(
                format_body(&p, &info, &mut fresh, env, Some(&head)).lines[0].end,
                cached
            );
        }
    }

    #[test]
    fn endnote_membership_is_independent_of_fragment_vertical_coordinates() {
        let (info, fonts) = (DocInfo::default(), FontSystem::new());
        let mut styles = Styles::new(&info, &fonts);
        let env = ParaEnv {
            column_width: 100_000,
            default_tab: 4000,
        };
        let mut p = para();
        let original = format_endnote(&p, &info, &mut styles, env, None).lines;
        p.line_segs[1].vpos = p.line_segs[0].vpos;
        assert_eq!(
            format_endnote(&p, &info, &mut styles, env, None).lines,
            original
        );
        assert_eq!(starts(true, &p), [0]);
        styles.stored_lines = false;
        assert_eq!(
            format_endnote(&p, &info, &mut styles, env, None)
                .lines
                .len(),
            1
        );
        styles.stored_lines = true;
        p.src = None;
        assert_eq!(
            format_endnote(&p, &info, &mut styles, env, None)
                .lines
                .len(),
            1
        );
    }

    #[test]
    fn square_drawing_anchors_are_composed_before_their_exclusion_spans() {
        use hm_model::{Control, ControlKind, CtrlId, ObjectCommon, ObjectControl, ObjectHeader};

        let mut p = para();
        p.text.splice(0..0, [11, 0, 0, 0, 0, 0, 0, 11]);
        p.line_segs[0].width = 2000;
        p.line_segs[1].text_start = 10;
        p.line_segs[1].width = 2000;
        let drawing = |attr| Control {
            kind: ControlKind::Object(Box::new(ObjectControl {
                id: CtrlId::SHAPE,
                header: ObjectHeader {
                    common: ObjectCommon {
                        attr,
                        width: 1000,
                        height: 1000,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                parts: Vec::new(),
            })),
            src: None,
        };
        let (info, fonts) = (DocInfo::default(), FontSystem::new());
        let env = ParaEnv {
            column_width: 100_000,
            default_tab: 4000,
        };
        p.controls = vec![drawing((2 << 3) | (4 << 15) | (2 << 18))];
        let mut styles = Styles::new(&info, &fonts);
        let pl = format_body(&p, &info, &mut styles, env, None);
        assert_eq!(pl.lines.len(), 1);
        assert_eq!(pl.lines[0].seg_width, env.column_width);

        let nested = format(&p, &info, &mut styles, env, None);
        assert!(nested.lines.len() > 1);
        assert_eq!(nested.lines[0].seg_width, 2000);

        // An inline drawing still keeps exclusion spans supplied by other
        // objects; refreshing its membership does not discard those spans.
        p.controls = vec![drawing(1 | (4 << 15) | (2 << 18))];
        let pl = format_body(&p, &info, &mut styles, env, None);
        assert!(pl.lines.len() > 1);
        assert_eq!(pl.lines[0].seg_width, 2000);
    }

    #[test]
    fn reflow_breaks_lines_again_in_their_own_segments() {
        let (info, fonts) = (DocInfo::default(), FontSystem::new());
        let mut styles = Styles::new(&info, &fonts);
        let p = Paragraph {
            text: "가나다라마바".encode_utf16().chain([PARA_END]).collect(),
            ..Paragraph::default()
        };
        let env = ParaEnv {
            column_width: 100_000,
            default_tab: 4000,
        };
        let mut pl = format(&p, &info, &mut styles, env, None);
        assert_eq!(pl.lines.len(), 1);
        assert!(pl.line_facts(&p).content(0));
        let em = pl.lines[0].used / 6;
        // Two characters fit each narrowed segment, shifted right by 500.
        reflow(&mut pl, &p, &info, &mut styles, 0, &mut |_| {
            (500, em * 2 + em / 2)
        });
        let starts: Vec<u32> = pl.lines.iter().map(|l| l.start).collect();
        assert_eq!(starts, [0, 2, 4]);
        assert!(pl.lines.iter().all(|l| l.seg_x == 500 && l.x == 500));
        assert!(pl.lines.last().is_some_and(|l| l.last));
        assert!(
            pl.lines
                .iter()
                .enumerate()
                .all(|(i, _)| pl.line_facts(&p).content(i))
        );
        // Lines before the first reflowed one stay.
        reflow(&mut pl, &p, &info, &mut styles, 1, &mut |_| (0, 100_000));
        let starts: Vec<u32> = pl.lines.iter().map(|l| l.start).collect();
        assert_eq!(starts, [0, 2]);
        assert!(
            pl.lines
                .iter()
                .enumerate()
                .all(|(i, _)| pl.line_facts(&p).content(i))
        );
    }

    #[test]
    fn square_clearance_uses_condensed_positions_but_keeps_object_overflow() {
        let mut pl = ParaLayout {
            measured: std::sync::Arc::new(Measured {
                items: vec![
                    (0, Kind::Char('가')),
                    (1, Kind::Space),
                    (2, Kind::Char('나')),
                    (3, Kind::ParaEnd),
                ],
                advances: vec![100, 100, 100, 0],
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut line = Line {
            end_item: 4,
            width: 250,
            used: 300,
            ..Default::default()
        };
        let band = crate::objects::Band {
            bottom: 1000,
            left: 250,
            right: 600,
            square: true,
            ..Default::default()
        };
        let clear = |span| crate::objects::clear_line(&[band], 0, (100, 100), span, None, 0, 600);
        assert_eq!(clear((0, line.used)).0, 1000);
        assert_eq!(content_span(&pl, &line), (0, 250));
        assert_eq!(clear(content_span(&pl, &line)).0, 0);

        pl.measured = std::sync::Arc::new(Measured {
            items: vec![(0, Kind::Object(0)), (1, Kind::ParaEnd)],
            advances: vec![300, 0],
            ..Default::default()
        });
        line.end_item = 2;
        assert_eq!(content_span(&pl, &line), (0, 300));
        assert_eq!(clear(content_span(&pl, &line)).0, 1000);
    }

    #[test]
    fn grids_break_latin_runs_and_manuscript_suffixes_hang_at_natural_width() {
        let (info, fonts) = (DocInfo::default(), FontSystem::new());
        let p = Paragraph {
            text: "가나다ABCD".encode_utf16().chain([PARA_END]).collect(),
            ..Paragraph::default()
        };
        let env = ParaEnv {
            column_width: 6000,
            default_tab: 4000,
        };
        for manuscript in [false, true] {
            let mut styles = Styles::new(&info, &fonts);
            styles.char_grid = (
                &p as *const Paragraph as usize,
                crate::style::CharGrid {
                    pitch: 2000,
                    manuscript,
                },
            );
            let pl = format(&p, &info, &mut styles, env, None);
            let starts: Vec<u32> = pl.lines.iter().map(|l| l.start).collect();
            assert_eq!(
                starts,
                if manuscript {
                    vec![0, 3]
                } else {
                    vec![0, 3, 6]
                }
            );
            let mut placed = Vec::new();
            place(&pl, &pl.lines[1], &mut placed);
            if manuscript {
                let last = placed.iter().find(|g| g.pos == 6).unwrap();
                assert_eq!(last.x, 6000);
                assert!(last.advance < 2000);
            } else {
                assert!(placed.iter().all(|g| g.x < 6000));
            }
        }
    }

    #[test]
    fn manuscript_alignment_measures_words_without_shrinking_their_paint_cells() {
        let m = Measured {
            items: "G0123"
                .chars()
                .enumerate()
                .map(|(p, c)| (p as u32, Kind::Char(c)))
                .collect(),
            advances: vec![2000; 5],
            natural: vec![2000; 5],
            grid_glyphs: vec![(400, 1200); 5],
            grid: 2000,
            manuscript: true,
            ..Measured::default()
        };
        let pl = ParaLayout {
            measured: std::sync::Arc::new(m),
            align: Some(Align::Right),
            ..ParaLayout::default()
        };
        let line = Line {
            end_item: 5,
            used: 10_000,
            width: 20_000,
            last: true,
            ..Line::default()
        };
        let mut placed = Vec::new();
        place(&pl, &line, &mut placed);
        // The first glyph takes a cell for alignment; the four following
        // narrow glyphs take 1200 HU each. 6800 rounds to 8000 HU.
        assert_eq!(
            placed.iter().map(|g| g.x).collect::<Vec<_>>(),
            [12_400, 14_400, 16_400, 18_400, 20_400]
        );
    }

    /// Equal advances isolate line membership from font selection. The
    /// object and zero-width control still occupy their eight wire units.
    fn wrapped_ends(text: &str, width: i32, by_char: bool) -> Vec<u32> {
        let mut m = Measured::default();
        let mut wire = Vec::new();
        for c in text.chars().chain(['\r']) {
            let (kind, units, advance) = match c {
                ' ' => (Kind::Space, 1, 50),
                '\t' => (Kind::Tab, 8, 0),
                '\n' => (Kind::LineBreak, 1, 0),
                '\r' => (Kind::ParaEnd, 1, 0),
                '\0' => (Kind::Zero, 8, 0),
                // An inline equation, and an object of another kind.
                '◆' | '■' => {
                    let k = m.boxes.len() as u32;
                    m.boxes.push(crate::measure::InlineBox {
                        pos: wire.len() as u32,
                        control: k,
                        width: 300,
                        height: 100,
                        baseline: 85,
                    });
                    if c == '◆' {
                        let eq = crate::objects::ObjectLayout {
                            equation: Some(Default::default()),
                            ..Default::default()
                        };
                        m.objects.push((k, std::sync::Arc::new(eq)));
                    }
                    (Kind::Object(k), 8, 300)
                }
                _ => (Kind::Char(c), 1, 100),
            };
            m.items.push((wire.len() as u32, kind));
            m.advances.push(advance);
            m.advances.resize(wire.len() + units, 0);
            wire.push(c as u16);
            wire.resize(wire.len() + units - 1, 0);
        }
        m.natural = m.advances.clone();
        let tabs = TabCtx {
            stops: &[],
            imported: false,
            default: 400,
            origin: 0,
            auto_left: None,
            auto_right: None,
            end: width,
        };
        let rules = Rules {
            korean_by_char: by_char,
            latin_by_char: by_char,
            condense: 0,
            fresh_blanks: true,
        };
        let mut ends = Vec::new();
        let mut from = 0;
        while from < m.items.len() {
            let line = break_line(&mut m, &wire, from, width, rules, &tabs, End::Fit);
            assert!(line.end_item as usize > from);
            ends.push(line.end);
            from = line.end_item as usize;
        }
        ends
    }

    #[test]
    fn forbidden_punctuation_recovers_the_preceding_word_boundary() {
        for (text, width, end) in [
            ("AB 가나다.", 550, 3),
            ("가 abcd。", 550, 2),
            ("AB 가나다。", 550, 3),
            ("가 abcd.", 550, 2),
            ("AB ◆이다.", 750, 3),
            ("AB 가나다(漢", 650, 6),
        ] {
            assert_eq!(wrapped_ends(text, width, false)[0], end, "{text}");
        }
        // Character breaking retains the existing two-character correction.
        assert_eq!(wrapped_ends("AB 가나다.", 550, true)[0], 5);
        // A word longer than the line must still make progress.
        assert_eq!(wrapped_ends("가나다.", 300, false), [3, 5]);
    }

    #[test]
    fn forbidden_recovery_respects_tabs_controls_and_explicit_line_ends() {
        assert_eq!(wrapped_ends("AB\t가나다.", 700, false)[0], 10);
        assert_eq!(wrapped_ends("AB 가\0나다.", 550, false)[0], 3);
        // A blank already supplies a boundary before a forbidden starter.
        assert_eq!(wrapped_ends("AB 가나다 `X", 600, false)[0], 7);
        assert_eq!(wrapped_ends("AB 가나다\t`X", 800, false)[0], 6);
        assert_eq!(wrapped_ends("AB 가나다 \0`X", 600, false)[0], 15);
        assert_eq!(wrapped_ends("AB \0가나다.", 550, false)[0], 11);
        // An overlong first word retains its emergency split at a closer.
        assert_eq!(wrapped_ends("구분(분기)", 500, false)[0], 5);
        assert_eq!(wrapped_ends("AB\n가 나다.", 350, false), [3, 5, 9]);
        assert_eq!(wrapped_ends("AB 가나다.", 650, false), [8]);
        assert_eq!(wrapped_ends("AB 가\r나다.", 550, false)[0], 5);
    }

    #[test]
    fn inline_objects_join_the_latin_text_they_touch() {
        // `(가)~(라)는` with the tilde as an equation wraps as one word.
        assert_eq!(wrapped_ends("가 (A)◆(B)는", 900, false)[0], 2);
        assert_eq!(wrapped_ends("가 (A)◆(B)는", 550, false)[0], 2);
        assert_eq!(wrapped_ends("가 (A)◆(B)는", 900, true)[0], 13);
        // Other inline objects keep their boundaries with Latin text.
        assert_eq!(wrapped_ends("가 (A)■(B)는", 900, false)[0], 13);
        assert_eq!(wrapped_ends("가 (A)■(B)는", 550, false)[0], 5);
    }

    #[test]
    fn a_blank_at_the_edge_ends_the_line() {
        let info = DocInfo {
            char_shapes: vec![hm_model::CharShape {
                base_size: 1000,
                shade_color: 0xFFFF_FFFF,
                ..Default::default()
            }],
            ..Default::default()
        };
        let fonts = FontSystem::new();
        let mut styles = Styles::new(&info, &fonts);
        // Line starts and the first line's used width.
        let mut lay = |t: &str, column_width| {
            let p = Paragraph {
                text: t.encode_utf16().chain([PARA_END]).collect(),
                ..Paragraph::default()
            };
            let env = ParaEnv {
                column_width,
                default_tab: 4000,
            };
            let pl = format(&p, &info, &mut styles, env, None);
            let starts: Vec<u32> = pl.lines.iter().map(|l| l.start).collect();
            (starts, pl.lines[0].used)
        };
        let w = lay("가나다라", 100_000).1;
        // The first blank reached at the edge takes the next one along; the
        // third starts the next line.
        assert_eq!(lay("가나다라   마바", w).0, [0, 6]);
        let blank = lay("가 나", 100_000).1 - 2 * lay("가", 100_000).1;
        assert_eq!(lay("가나다라      마바", w + blank / 2).0, [0, 6]);
        assert_eq!(lay("가나다라      마바", w + 2 * blank).0, [0, 7]);
        assert_eq!(
            lay("가나다라      마바", w + 2 * blank + blank / 2).0,
            [0, 7]
        );
        // At the paragraph end two blanks hang with the text; the third
        // starts another line, as it does before more text.
        assert_eq!(lay("가나다라  ", w).0, [0]);
        assert_eq!(lay("가나다라   ", w).0, [0, 6]);
    }
    #[test]
    fn trailing_blanks_wrap_at_the_physical_edge_and_keep_the_end_shape() {
        use hm_model::{CharRun, CharShape};
        let info = DocInfo {
            char_shapes: vec![
                CharShape {
                    base_size: 800,
                    ..CharShape::default()
                },
                CharShape {
                    base_size: 1000,
                    ..CharShape::default()
                },
            ],
            para_shapes: vec![ParaShape {
                line_spacing_legacy: 160,
                ..ParaShape::default()
            }],
            ..DocInfo::default()
        };
        let fonts = FontSystem::new();
        let mut styles = Styles::new(&info, &fonts);
        let mut lay = |text: &str, width| {
            let end = text.encode_utf16().count() as u32;
            let p = Paragraph {
                text: text.encode_utf16().chain([PARA_END]).collect(),
                runs: vec![CharRun { pos: 0, shape: 0 }, CharRun { pos: end, shape: 1 }],
                ..Paragraph::default()
            };
            format(
                &p,
                &info,
                &mut styles,
                ParaEnv {
                    column_width: width,
                    default_tab: 4000,
                },
                None,
            )
            .lines
            .iter()
            .map(|l| (l.start, l.height, l.spacing))
            .collect::<Vec<_>>()
        };
        // Native controls at two widths retain a larger CR on the final
        // line, including when the last blank hangs beyond the margin.
        for (width, edge) in [(51024, 128), (27024, 68)] {
            for count in [edge - 1, edge] {
                assert_eq!(lay(&" ".repeat(count), width), [(0, 1000, 600)]);
            }
            for count in [edge + 1, edge + 6] {
                assert_eq!(
                    lay(&" ".repeat(count), width),
                    [(0, 800, 480), (edge as u32, 1000, 600)]
                );
            }
            assert_eq!(lay("", width), [(0, 1000, 600)]);
        }
        // A visible prefix follows the same measured edge as blank-only
        // content; terminal CR height must not enlarge earlier lines.
        // Two terminal blanks remain together when the prefix leaves
        // less than one blank of room. A third starts the next line.
        for (width, n) in [(3544, 4), (4344, 5)] {
            let prefix = "가".repeat(n);
            for count in [1, 2] {
                assert_eq!(
                    lay(&(prefix.clone() + &" ".repeat(count)), width),
                    [(0, 1000, 600)]
                );
            }
            assert_eq!(
                lay(&(prefix + "   "), width),
                [(0, 800, 480), ((n + 2) as u32, 1000, 600)]
            );
        }
        assert_eq!(
            lay(&("가".repeat(63) + "   "), 51024),
            [(0, 800, 480), (65, 1000, 600)]
        );
    }

    #[test]
    fn character_unit_indents_follow_the_first_style_not_the_paragraph_text() {
        use hm_model::{CharRun, CharShape, Style};
        // A hanging indent of 15.01 characters, as an old package's
        // unit-aware branch stores -3001.
        let mut info = DocInfo {
            char_shapes: vec![
                CharShape {
                    base_size: 1000,
                    ..CharShape::default()
                },
                CharShape {
                    base_size: 1500,
                    ..CharShape::default()
                },
            ],
            para_shapes: vec![ParaShape {
                indent: -3001,
                ..ParaShape::default()
            }],
            styles: vec![Style::default()],
            ..DocInfo::default()
        };
        let p = Paragraph {
            text: "가".repeat(80).encode_utf16().chain([PARA_END]).collect(),
            runs: vec![CharRun { pos: 0, shape: 1 }],
            ..Paragraph::default()
        };
        let continuation = |info: &DocInfo| {
            let fonts = FontSystem::new();
            let mut styles = Styles::new(info, &fonts);
            let env = ParaEnv {
                column_width: 48188,
                default_tab: 4000,
            };
            let pl = format(&p, info, &mut styles, env, None);
            assert!(pl.lines.len() > 1);
            (pl.lines[0].x, pl.lines[1].x)
        };
        // Half of the first style's 1000, whatever the 1500 text size.
        assert_eq!(continuation(&info), (0, 7505));
        info.styles[0].char_shape = 1;
        assert_eq!(continuation(&info), (0, 11258));
        info.styles.clear();
        assert_eq!(continuation(&info), (0, 7505));
    }
}
