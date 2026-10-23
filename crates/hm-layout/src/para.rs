//! Paragraph formatting: line breaking, line metrics and horizontal placement.

use hm_model::units::{mul_div, unpack_distance};
use hm_model::{Align, DocInfo, LineSpacingKind, ParaShape, Paragraph, Script};

use crate::heads::Head;
use crate::measure::{Kind, Measured, advance, measure};
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
    /// Width the head takes from the first line, distance to the text included.
    pub width: i32,
    /// Line size of the head's style, which seeds the first line's metrics.
    pub size: i32,
}

/// A formatted paragraph for one column width.
#[derive(Clone, Debug, Default)]
pub struct ParaLayout {
    pub lines: Vec<Line>,
    pub head: Option<HeadLayout>,
    /// Shared with the copies [`crate::LayoutCache`] keeps for reuse.
    pub measured: std::sync::Arc<Measured>,
    /// Paragraph spacing before the first and after the last line.
    pub before: i32,
    pub after: i32,
    pub align: Option<Align>,
    /// Paragraph vertical alignment: 0 baseline, 1 top, 2 center, 3 bottom.
    pub valign: u32,
    /// The column width and section tab interval this layout was made for.
    pub column_width: i32,
    pub default_tab: i32,
    /// Indent of the lines after the first under an automatically hanging
    /// head.
    pub hang: i32,
    /// Layouts of the tables among the paragraph's controls, by control index.
    pub tables: Vec<(u32, crate::table::TableLayout)>,
}

impl ParaLayout {
    /// The layout of the table at control index `control`.
    pub fn table(&self, control: u32) -> Option<&crate::table::TableLayout> {
        self.tables
            .iter()
            .find(|(k, _)| *k == control)
            .map(|(_, t)| t)
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
    let cs = head
        .char_shape(info)
        .or_else(|| para.runs.last().map(|r| r.shape))
        .unwrap_or(0);
    let mut out = HeadLayout::default();
    let mut x = 0;
    for c in head.text.chars() {
        let style = styles.style_for(cs, script_of(c).unwrap_or(Script::Hangul), Some(c));
        out.glyphs.push((c, style, x));
        x += advance(styles.get(style), styles.face(style).map(|f| &**f), c);
        out.size = out.size.max(styles.get(style).line_size);
    }
    // Unless the head follows its instance width (attribute bit 2), it
    // takes at least one em of the paragraph's text size.
    let body = para
        .runs
        .first()
        .and_then(|r| info.char_shape(r.shape))
        .map_or(1000, |c| c.base_size);
    let text = if head.info.attr & (1 << 2) != 0 {
        x
    } else {
        x.max(body)
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
        format_fresh(para, info, styles, env, head)
    })
}

fn format_fresh(
    para: &Paragraph,
    info: &DocInfo,
    styles: &mut Styles<'_>,
    env: ParaEnv,
    head: Option<&Head>,
) -> ParaLayout {
    let b = Breaker::new(para, info, env);
    let tables = crate::table::layout_controls(para, info, styles, env.default_tab);
    let sizes: Vec<(u32, i32, i32)> = tables
        .iter()
        .map(|(k, t)| {
            let (w, h) = t.outer_size();
            (*k, w, h)
        })
        .collect();
    let mut m = measure(para, styles, b.auto, &sizes);
    let stored = styles
        .stored_lines
        .then(|| stored_lines(para, b.left, env.column_width - b.right))
        .flatten();
    let auto_hang = head.is_some_and(|h| h.info.attr & (1 << 3) != 0);
    let head = head.map(|h| head_layout(h, para, info, styles));
    // A head with automatic hanging (head attribute bit 3) indents the
    // following lines by its width.
    let hang = head.as_ref().filter(|_| auto_hang).map_or(0, |h| h.width);
    let lines = b.lines(
        &mut m,
        styles,
        head.as_ref(),
        hang,
        0,
        0,
        &mut |at, k| match &stored {
            Some(r) => r.line(at, k),
            None => LineSpec::FIT,
        },
    );
    ParaLayout {
        lines,
        head,
        measured: std::sync::Arc::new(m),
        before: b.shape.before(b.basis),
        after: b.shape.after(b.basis),
        align: Some(b.shape.align()),
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
    let env = ParaEnv {
        column_width: layout.column_width,
        default_tab: layout.default_tab,
    };
    let b = Breaker::new(para, info, env);
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
}

impl LineSpec {
    const FIT: LineSpec = LineSpec {
        end: End::Fit,
        seg: None,
        stored_seg: false,
        metrics: None,
    };
}

/// The spec of the line starting at a wire position, given its index.
type LinePlan<'a> = dyn FnMut(u32, usize) -> LineSpec + 'a;

/// What breaking a paragraph's lines needs besides its measured items.
struct Breaker<'p> {
    para: &'p Paragraph,
    env: ParaEnv,
    shape: ParaShape,
    /// Automatic spacing between scripts (Latin, digits).
    auto: (bool, bool),
    /// Character-unit basis of the paragraph's distances.
    basis: i32,
    left: i32,
    right: i32,
    indent: i32,
    tab_attr: u32,
    tabs: Vec<(i32, u8)>,
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
        // height, or of all of it under compatibility word 1 bit 2.
        let reference = para
            .runs
            .first()
            .and_then(|r| info.char_shape(r.shape))
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
        Breaker {
            para,
            env,
            auto,
            basis,
            left: shape.left(basis),
            right: shape.right(basis),
            indent: shape.first_indent(basis),
            tab_attr: tab_def.map_or(0, |t| t.attr),
            tabs,
            rules: Rules {
                korean_by_char: shape.korean_break_by_char(),
                latin_by_char: shape.latin_break() == 2,
                condense: shape.min_space() as i32,
            },
            shape,
        }
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
    /// vertical metrics; lines laid out fresh keep the segment stored for
    /// the line at their place, which objects beside it narrow alike.
    fn line(&self, at: u32, k: usize) -> LineSpec {
        if at >= self.until {
            let seg = self.segment(k);
            return LineSpec {
                seg,
                stored_seg: seg.is_some(),
                ..LineSpec::FIT
            };
        }
        let j = self
            .segs
            .partition_point(|s| s.text_start <= at)
            .saturating_sub(1);
        let s = &self.segs[j];
        let seg = self.segment(j);
        LineSpec {
            end: End::At(self.segs.get(j + 1).map(|s| s.text_start)),
            seg,
            stored_seg: seg.is_some(),
            metrics: Some((s.text_height, s.baseline, s.spacing)),
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
/// of them.
fn stored_lines(para: &Paragraph, left: i32, right: i32) -> Option<Replay> {
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
            .all(|w| w[0].text_start < w[1].text_start && w[0].vpos != w[1].vpos)
        && segs.last().is_some_and(|s| s.text_start < para.len());
    if !clean {
        return None;
    }
    let object = para.units().find_map(|(pos, u)| match u {
        hm_model::Unit::Extended { index, .. } => para
            .controls
            .get(index as usize)
            .filter(|c| c.as_table().is_none() && c.object_common().is_some())
            .map(|_| pos),
        _ => None,
    });
    let until = object.map_or(u32::MAX, |pos| {
        segs.iter()
            .map(|s| s.text_start)
            .rfind(|&s| s <= pos)
            .unwrap_or(0)
    });
    Some(Replay {
        segs: segs.clone(),
        margins: (left, right),
        until,
    })
}

#[derive(Clone, Copy)]
struct Rules {
    korean_by_char: bool,
    latin_by_char: bool,
    /// Percent of the line's blank widths a candidate may borrow to fit
    /// (paragraph shape bits 9-15).
    condense: i32,
}

struct TabCtx<'a> {
    /// Explicit stops in stored order: position (column-relative) and kind.
    stops: &'a [(i32, u8)],
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
    /// The advance of a tab at `current` whose following text, up to the next
    /// tab or line end, ends at `run_end` (all column-relative).
    ///
    /// Stops are tried in stored order and must lie strictly beyond the tab's
    /// reference point: `current` for left, `run_end` for right, their
    /// midpoint for center and decimal. An active automatic stop takes
    /// precedence over an explicit stop beyond it. Without a usable stop the
    /// section's tab interval applies.
    fn advance(&self, current: i32, run_end: i32) -> i32 {
        let mid = (current.wrapping_add(run_end)) / 2;
        let auto_left = self.auto_left.filter(|&s| current < s);
        let auto_right = self.auto_right.filter(|&s| run_end < s);
        for &(pos, kind) in self.stops {
            if pos <= current {
                continue;
            }
            if let Some(a) = auto_left.filter(|&a| pos > a) {
                return a - current;
            }
            if let Some(a) = auto_right.filter(|&a| pos > a) {
                return (a - run_end).max(0);
            }
            match kind {
                0 => return pos - current,
                1 if pos > run_end => return (pos.min(self.end) - run_end).max(0),
                2 | 3 if pos > mid => return pos - mid,
                _ => {}
            }
        }
        if let Some(a) = auto_left {
            return a - current;
        }
        if let Some(a) = auto_right {
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
/// Korean bit 7 for the rest.
fn can_break_before(items: &[(u32, Kind)], j: usize, rules: Rules) -> bool {
    let cur = run_of(items[j].1);
    let Some(prev) = items[..j]
        .iter()
        .rev()
        .map(|(_, k)| run_of(*k))
        .find(|r| *r != Run::Never)
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
        (Run::Latin, Run::Latin) => rules.latin_by_char,
        (Run::Other, Run::Other) => rules.korean_by_char,
        _ => true,
    }
}

/// Hancom's forbidden-character correction: move a break back (at most
/// twice) while the line would end with a forbidden-end character or the
/// next would start with a forbidden-start one. Character-break paragraphs
/// may break anywhere; word-break ones only where a line may start.
fn correct_forbidden(items: &[(u32, Kind)], from: usize, mut b: usize, rules: Rules) -> usize {
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
        if forbidden && (by_char || can_break_before(items, b - 1, rules)) {
            b -= 1;
        } else {
            break;
        }
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
                m.advances[pos as usize] = tabs.advance(at, at + following_width(m, j + 1));
            }
            _ => {}
        }
        let w = m.advances[pos as usize];
        if j > from && can_break_before(&m.items, j, rules) {
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
            Kind::Char(_) => m.natural[pos as usize],
            _ => w,
        };
        let ordinary = matches!(text.get(pos as usize), Some(&0x20 | &0x3000));
        // An ordinary blank reached at the physical edge ends the line
        // whatever budget is left, unless only blanks follow it to the line
        // end; the first blank of a run takes the next one along.
        if rule == End::Fit && space && ordinary && measured && x >= width {
            let blank = |k: usize| {
                m.items.get(k).is_some_and(|&(p, kind)| {
                    kind == Kind::Space && matches!(text.get(p as usize), Some(&0x20 | &0x3000))
                })
            };
            let trailing = m.items[j..]
                .iter()
                .take_while(|i| !matches!(i.1, Kind::ParaEnd | Kind::LineBreak))
                .all(|i| i.1 == Kind::Space);
            if !trailing {
                end = if !blank(j - 1) && blank(j + 1) {
                    j + 2
                } else {
                    j + 1
                };
                break;
            }
        }
        if rule == End::Fit && !space && j > first && x + fit > width + budget {
            end = match best {
                Some((b, _)) if b > from => b,
                _ => j,
            };
            end = correct_forbidden(&m.items, from, end, rules);
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
    let used = ink_width(m, from, end);
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
    let Some(last) = (from..end).rev().find(|&k| is_ink(m.items[k].1)) else {
        return 0;
    };
    let body: i32 = m.items[from..last]
        .iter()
        .map(|(p, _)| m.advances[*p as usize])
        .sum();
    let (pos, kind) = m.items[last];
    let tail = match kind {
        Kind::Char(_) => m.natural[pos as usize],
        _ => m.advances[pos as usize],
    };
    body + tail
}

fn is_ink(kind: Kind) -> bool {
    !matches!(
        kind,
        Kind::Space | Kind::ParaEnd | Kind::LineBreak | Kind::Zero
    )
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
/// Opportunities follow Hancom's producer: a tab resets them, blanks count
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
        .rposition(|(_, k)| is_ink(*k))
        .map_or(0, |i| i + 1);
    let reset = items[..ink_end]
        .iter()
        .rposition(|(_, k)| matches!(k, Kind::Tab))
        .map_or(0, |i| i + 1);
    let first_visible = items[reset..ink_end]
        .iter()
        .position(|(_, k)| is_ink(*k))
        .map_or(ink_end, |i| reset + i);
    let eligible_space =
        |k: usize| k >= first_visible && k < ink_end && matches!(items[k].1, Kind::Space);
    let eligible_char = |k: usize| k >= first_visible && k + 1 < ink_end && is_ink(items[k].1);
    let spaces = (0..items.len()).filter(|&k| eligible_space(k)).count() as i32;
    let gaps = (0..items.len()).filter(|&k| eligible_char(k)).count() as i32;
    let slack = line.width - line.used;
    let (mut offset, mut space_extra, mut char_extra) = (0, 0, 0);
    if slack >= 0 {
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
        let advance = m.advances[pos as usize];
        out.push(Placed {
            pos,
            kind,
            x,
            advance,
        });
        x += advance;
        if eligible_space(k) {
            x += space_extra;
        } else if eligible_char(k) {
            x += char_extra;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hm_fonts::FontSystem;
    use hm_model::{LineSeg, PARA_END, SharedBytes};

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
        let em = pl.lines[0].used / 6;
        // Two characters fit each narrowed segment, shifted right by 500.
        reflow(&mut pl, &p, &info, &mut styles, 0, &mut |_| {
            (500, em * 2 + em / 2)
        });
        let starts: Vec<u32> = pl.lines.iter().map(|l| l.start).collect();
        assert_eq!(starts, [0, 2, 4]);
        assert!(pl.lines.iter().all(|l| l.seg_x == 500 && l.x == 500));
        assert!(pl.lines.last().is_some_and(|l| l.last));
        // Lines before the first reflowed one stay.
        reflow(&mut pl, &p, &info, &mut styles, 1, &mut |_| (0, 100_000));
        let starts: Vec<u32> = pl.lines.iter().map(|l| l.start).collect();
        assert_eq!(starts, [0, 2]);
    }

    #[test]
    fn a_blank_at_the_edge_ends_the_line() {
        let (info, fonts) = (DocInfo::default(), FontSystem::new());
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
        // Blanks running to the paragraph end stay on the line.
        assert_eq!(lay("가나다라   ", w).0, [0]);
    }
}
