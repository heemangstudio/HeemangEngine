//! Paragraph formatting: line breaking, line metrics and horizontal placement.

use hm_model::units::{mul_div, unpack_distance};
use hm_model::{Align, DocInfo, LineSpacingKind, ParaShape, Paragraph};

use crate::measure::{Kind, Measured, measure};
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

/// A formatted paragraph for one column width.
#[derive(Clone, Debug, Default)]
pub struct ParaLayout {
    pub lines: Vec<Line>,
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

/// Formats one paragraph.
pub fn format(
    para: &Paragraph,
    info: &DocInfo,
    styles: &mut Styles<'_>,
    env: ParaEnv,
) -> ParaLayout {
    let shape = info
        .para_shape(para.header.para_shape as u32)
        .cloned()
        .unwrap_or_default();
    let mut m = measure(para, styles);
    let basis = para
        .runs
        .first()
        .and_then(|r| info.char_shape(r.shape))
        .map_or(1000, |c| c.base_size);
    let left = shape.left(basis);
    let right = shape.right(basis);
    let indent = shape.first_indent(basis);
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
    let mut lines = Vec::new();
    let mut item = 0usize;
    loop {
        let first = lines.is_empty();
        let offset = if first {
            indent.max(0)
        } else {
            (-indent).max(0)
        };
        let x = left + offset;
        let width = (env.column_width - left - right - offset).max(0);
        let tab_ctx = TabCtx {
            stops: tabs.as_deref().unwrap_or(&[]),
            default: env.default_tab,
            origin: left + offset,
        };
        let mut line = break_line(&mut m, item, width, rules, &tab_ctx);
        line.x = x;
        line.width = width;
        line.seg_x = left;
        line.seg_width = (env.column_width - left - right).max(0);
        metrics(&mut line, &m, styles, &shape, basis);
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
    stops: &'a [(i32, u8)],
    default: i32,
    /// Line content start relative to the column. Tab stops and the default
    /// tab grid count from the column's left edge.
    origin: i32,
}

impl TabCtx<'_> {
    /// The next stop strictly after `x` (relative to the column) and its kind.
    fn next(&self, x: i32) -> (i32, u8) {
        if let Some(&(p, k)) = self.stops.iter().find(|(p, _)| *p > x) {
            return (p, k);
        }
        let d = self.default.max(1);
        ((x / d + 1) * d, 0)
    }
}

fn class_of(kind: Kind) -> BreakClass {
    match kind {
        Kind::Char(c) => break_class(c),
        Kind::Space | Kind::Tab => BreakClass::Space,
        Kind::Object(_) => BreakClass::Ideograph,
        _ => BreakClass::Other,
    }
}

/// May a line start at item `j`?
fn can_break_before(items: &[(u32, Kind)], j: usize, rules: Rules) -> bool {
    let cur = class_of(items[j].1);
    let prev = class_of(items[j - 1].1);
    if matches!(items[j].1, Kind::Zero | Kind::ParaEnd | Kind::LineBreak) {
        return false;
    }
    match (prev, cur) {
        (_, BreakClass::Space) => false,
        (_, BreakClass::Close) => false,
        (BreakClass::Space, _) => true,
        (BreakClass::Open, _) => false,
        (BreakClass::Ideograph, _) | (_, BreakClass::Ideograph) => true,
        (BreakClass::Hangul, _) | (_, BreakClass::Hangul) => rules.korean_by_char,
        (BreakClass::Alnum, BreakClass::Alnum) => rules.latin_by_char,
        _ => false,
    }
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
                let (stop, kind) = tabs.next(at);
                let follow = || following_width(m, j + 1);
                let w = match kind {
                    1 => (stop - at - follow()).max(0),
                    2 => (stop - at - follow() / 2).max(0),
                    _ => stop - at,
                };
                m.advances[pos as usize] = w;
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

/// Content width of items `[from, end)` without trailing spaces.
fn ink_width(m: &Measured, from: usize, end: usize) -> i32 {
    let mut last_ink = from;
    for k in from..end {
        if !matches!(
            m.items[k].1,
            Kind::Space | Kind::ParaEnd | Kind::LineBreak | Kind::Zero
        ) {
            last_ink = k + 1;
        }
    }
    m.items[from..last_ink]
        .iter()
        .map(|(p, _)| m.advances[*p as usize])
        .sum()
}

/// Line height, baseline and spacing.
///
/// Each run (the paragraph end included) contributes its logical size as
/// height and 85 percent of it as baseline; inline objects contribute their
/// height the same way. The line takes the maxima. Percent spacing uses the
/// tallest text run only: `MulDiv(h / 4, pct - 100, 100) * 4`. Fixed spacing
/// makes the advance the value, added spacing adds it, and at-least spacing
/// adds what the value exceeds the height by.
fn metrics(line: &mut Line, m: &Measured, styles: &Styles<'_>, shape: &ParaShape, basis: i32) {
    let mut height = 0i32;
    let mut baseline = 0i32;
    let mut text = 0i32;
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
pub fn place(layout: &ParaLayout, line: &Line, out: &mut Vec<Placed>) {
    out.clear();
    let m = &layout.measured;
    let items = &m.items[line.first_item as usize..line.end_item as usize];
    let extra = (line.width - line.used).max(0);
    let align = layout.align.unwrap_or(Align::Justify);
    let ink_end = items
        .iter()
        .rposition(|(_, k)| {
            !matches!(
                k,
                Kind::Space | Kind::ParaEnd | Kind::LineBreak | Kind::Zero
            )
        })
        .map_or(0, |i| i + 1);
    let spaces = items[..ink_end]
        .iter()
        .filter(|(_, k)| matches!(k, Kind::Space))
        .count() as i32;
    let gaps = items[..ink_end]
        .iter()
        .filter(|(_, k)| !matches!(k, Kind::Zero))
        .count()
        .saturating_sub(1) as i32;
    let stretch = |spread_all: bool| -> (bool, i32) {
        if !spread_all && spaces > 0 {
            (false, spaces)
        } else {
            (true, gaps)
        }
    };
    let (offset, mode) = match align {
        Align::Left => (0, None),
        Align::Right => (extra, None),
        Align::Center => (extra / 2, None),
        Align::Justify if line.last || line.forced => (0, None),
        Align::Justify => (0, Some(stretch(false))),
        Align::Distribute => (0, Some(stretch(true))),
        Align::Divide => (0, Some(stretch(false))),
    };
    // An overfull line (condensed to fit) shrinks its blanks first, each by
    // at most a fifth of the narrowest blank, and its characters by the rest.
    let deficit = (line.used - line.width).max(0);
    let (mut space_cut, mut char_cut) = (0, 0);
    if deficit > 0 {
        let narrowest = items[..ink_end]
            .iter()
            .filter(|(p, k)| matches!(k, Kind::Space) && m.advances[*p as usize] > 0)
            .map(|(p, _)| m.advances[*p as usize])
            .min();
        let chars = items[..ink_end]
            .iter()
            .filter(|(_, k)| matches!(k, Kind::Char(_)))
            .count() as i32;
        match narrowest {
            Some(min) if spaces > 0 => {
                let cap = (min + 4) / 5;
                space_cut = deficit / spaces;
                if space_cut > cap && chars > 0 {
                    space_cut = cap;
                    char_cut = (deficit - cap * spaces) / chars;
                }
            }
            _ if chars > 0 => char_cut = deficit / chars,
            _ => {}
        }
    }
    let mut x = line.x + offset;
    let mut slot = 0i32;
    for (k, &(pos, kind)) in items.iter().enumerate() {
        let mut advance = m.advances[pos as usize];
        if k < ink_end {
            match kind {
                Kind::Space => advance -= space_cut,
                Kind::Char(_) => advance -= char_cut,
                _ => {}
            }
        }
        out.push(Placed {
            pos,
            kind,
            x,
            advance,
        });
        x += advance;
        if let Some((all, count)) = mode
            && count > 0
            && k + 1 < ink_end
            && deficit == 0
        {
            let takes = if all {
                !matches!(kind, Kind::Zero)
            } else {
                matches!(kind, Kind::Space)
            };
            if takes {
                // Even shares, the remainder going to the leftmost gaps.
                let share = extra / count + i32::from(slot < extra % count);
                x += share;
                slot += 1;
            }
        }
    }
}
