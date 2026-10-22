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
    pub measured: Measured,
    /// Paragraph spacing before the first and after the last line.
    pub before: i32,
    pub after: i32,
    pub align: Option<Align>,
    /// The column width this layout was made for.
    pub column_width: i32,
}

impl ParaLayout {
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
    out.width = x + head.distance(out.size) + head.info.width_adjust as i32;
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
    let shape = info
        .para_shape(para.header.para_shape as u32)
        .cloned()
        .unwrap_or_default();
    let flags2 = shape.attr2.unwrap_or(0);
    let compat1 = info.layout_compat.as_ref().map_or(0, |c| c.flags[1]);
    let auto = (
        flags2 & (1 << 4) != 0 && compat1 & (1 << 9) == 0,
        flags2 & (1 << 5) != 0 && compat1 & (1 << 10) == 0,
    );
    let mut m = measure(para, styles, auto);
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
    let left = shape.left(basis);
    let right = shape.right(basis);
    let indent = shape.first_indent(basis);
    let tab_attr = info
        .tab_defs
        .get(shape.tab_def as usize)
        .map_or(0, |t| t.attr);
    let tabs = info.tab_defs.get(shape.tab_def as usize).map(|t| {
        t.tabs
            .iter()
            .map(|s| (unpack_distance(s.position, || basis), s.kind))
            .collect::<Vec<_>>()
    });
    let rules = Rules {
        korean_by_char: shape.korean_break_by_char(),
        latin_by_char: shape.latin_break() == 2,
        condense: shape.min_space() as i32,
    };
    let head = head.map(|h| head_layout(h, para, info, styles));
    let mut lines = Vec::new();
    let mut item = 0usize;
    loop {
        let first = lines.is_empty();
        let head_width = if first {
            head.as_ref().map_or(0, |h| h.width)
        } else {
            0
        };
        let offset = if first {
            indent.max(0) + head_width
        } else {
            (-indent).max(0)
        };
        let x = left + offset;
        let width = (env.column_width - left - right - offset).max(0);
        let tab_ctx = TabCtx {
            stops: tabs.as_deref().unwrap_or(&[]),
            default: env.default_tab,
            origin: left + offset,
            auto_left: (tab_attr & 1 != 0 && shape.indent < 0)
                .then(|| left.wrapping_add(indent.wrapping_abs())),
            auto_right: (tab_attr & 2 != 0).then(|| env.column_width - right),
        };
        let mut line = break_line(&mut m, item, width, rules, &tab_ctx);
        line.x = x;
        line.width = width;
        line.seg_x = left;
        line.seg_width = (env.column_width - left - right).max(0);
        let seed = if first {
            head.as_ref().map_or(0, |h| h.size)
        } else {
            0
        };
        metrics(&mut line, &m, styles, &shape, basis, seed);
        item = line.end_item as usize;
        let last = line.last;
        lines.push(line);
        if last || item >= m.items.len() {
            break;
        }
    }
    if let Some(l) = lines.last_mut() {
        l.last = true;
    }
    ParaLayout {
        lines,
        head,
        measured: m,
        before: shape.before(basis),
        after: shape.after(basis),
        align: Some(shape.align()),
        column_width: env.column_width,
    }
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
                1 if pos > run_end => return pos - run_end,
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
/// A line can start after a blank and at every boundary between the Latin
/// run class and other units. Inside a run it can start only when that
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
    match (prev, cur) {
        (_, Run::Never | Run::Blank) => false,
        (Run::Blank, _) => true,
        (Run::Latin, Run::Latin) => rules.latin_by_char,
        (Run::Other, Run::Other) => rules.korean_by_char,
        _ => true,
    }
}

/// Hancom's forbidden-character correction: in character-break paragraphs,
/// move a break back (at most twice) while the line would end with a
/// forbidden-end character or the next would start with a forbidden-start one.
fn correct_forbidden(items: &[(u32, Kind)], from: usize, mut b: usize, rules: Rules) -> usize {
    if !(rules.korean_by_char || rules.latin_by_char) {
        return b;
    }
    let class = |k: usize| match items[k].1 {
        Kind::Char(c) => break_class(c),
        _ => BreakClass::Other,
    };
    for _ in 0..2 {
        if b <= from + 1 || b >= items.len() {
            break;
        }
        if class(b - 1) == BreakClass::Open || class(b) == BreakClass::Close {
            b -= 1;
        } else {
            break;
        }
    }
    b
}

/// Fills one line starting at item `from`.
fn break_line(m: &mut Measured, from: usize, width: i32, rules: Rules, tabs: &TabCtx<'_>) -> Line {
    let n = m.items.len();
    let mut x = 0i32;
    let mut x_ink = 0i32;
    let mut blanks = 0i32;
    let mut best: Option<(usize, i32)> = None;
    let mut j = from;
    let mut forced = false;
    let mut last = false;
    let mut end = n;
    while j < n {
        let (pos, kind) = m.items[j];
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
        let budget = if rules.condense > 0 {
            blanks.wrapping_mul(rules.condense) / 100
        } else {
            0
        };
        // A character fits when its advance before character spacing does.
        let fit = match kind {
            Kind::Char(_) => m.natural[pos as usize],
            _ => w,
        };
        if !space && j > from && x + fit > width + budget {
            end = match best {
                Some((b, _)) if b > from => b,
                _ => j,
            };
            end = correct_forbidden(&m.items, from, end, rules);
            break;
        }
        x += w;
        if space {
            blanks += w;
        } else {
            x_ink = x;
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
fn following_width(m: &Measured, from: usize) -> i32 {
    m.items[from..]
        .iter()
        .take_while(|(_, k)| !matches!(k, Kind::Tab | Kind::LineBreak | Kind::ParaEnd))
        .map(|(p, _)| m.advances[*p as usize])
        .sum()
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
    line.baseline = baseline;
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
