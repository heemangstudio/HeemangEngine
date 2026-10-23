//! Border and fill painting shared by tables, paragraphs, pages and
//! drawing objects.
//!
//! Strokes are centered on the edge they draw. A border line kind expands
//! into one or more parallel strokes whose total thickness is the line's
//! width; horizontal edges reach half their width past the corners so
//! adjacent vertical edges meet them without gaps.

use hm_model::{BorderFill, BorderLine, Fill};

use crate::{DisplayList, Item};

/// How a stroke is broken into dashes. Lengths scale with the stroke width.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Dash {
    Solid,
    /// Long dashes (긴 점선).
    LongDash,
    /// Dots (점선).
    Dot,
    DashDot,
    DashDotDot,
    /// Dashes longer than [`Dash::LongDash`] pieces but with wider gaps.
    Dash,
    /// Round dots larger than [`Dash::Dot`].
    Circle,
    /// Round dots as wide as the stroke, each the stroke width times this
    /// many thousandths apart (Hancom's 큰 동그라미 border line).
    Beads(u16),
}

impl Dash {
    /// The on/off pattern in multiples of the stroke width, `None` when solid.
    pub fn pattern(self) -> Option<Vec<f32>> {
        Some(match self {
            Dash::Solid => return None,
            Dash::LongDash => vec![8.0, 4.0],
            Dash::Dot => vec![1.0, 2.0],
            Dash::DashDot => vec![8.0, 3.0, 1.0, 3.0],
            Dash::DashDotDot => vec![8.0, 3.0, 1.0, 3.0, 1.0, 3.0],
            Dash::Dash => vec![16.0, 6.0],
            Dash::Circle => vec![0.0, 2.5],
            Dash::Beads(apart) => vec![0.0, f32::from(apart) / 1000.0],
        })
    }

    /// Round caps turn zero-length dashes into dots.
    pub fn round(self) -> bool {
        matches!(self, Dash::Circle | Dash::Beads(_))
    }
}

/// A stroke: width in HU, COLORREF color, dash style.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Stroke {
    pub width: i32,
    pub color: u32,
    pub dash: Dash,
}

/// A gradient ready for painting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GradientBrush {
    /// 1 linear, 2 radial, 3 conical, 4 square.
    pub kind: u8,
    /// Degrees; 0 runs top to bottom.
    pub angle: i32,
    /// Center in percent of the box (radial, conical and square).
    pub center_x: i32,
    pub center_y: i32,
    /// Number of color bands; 0 or 1 paints smoothly.
    pub step: i32,
    /// Where the gradient turns back (percent of its length), 0 or 100 for
    /// a one-way gradient.
    pub step_center: i32,
    /// `(position percent, COLORREF)` stops in order.
    pub stops: Vec<(i32, u32)>,
}

/// What paints the inside of a shape.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Brush {
    Solid(u32),
    /// Hatch lines (style 1-6 as in [`hm_model::SolidFill::pattern`]) on a
    /// transparent background.
    Hatch {
        color: u32,
        style: u8,
    },
    Gradient(Box<GradientBrush>),
    /// A BinData picture. Backends without image support skip it.
    Image {
        bin_id: u16,
        /// The picture's bytes, filled from the document's BinData.
        data: Option<hm_model::SharedBytes>,
        mode: u8,
        effect: u8,
        brightness: i8,
        contrast: i8,
    },
}

/// The dash style of a border line kind.
fn dash_of(kind: u8) -> Dash {
    match kind {
        2 => Dash::LongDash,
        3 => Dash::Dot,
        4 => Dash::DashDot,
        5 => Dash::DashDotDot,
        6 => Dash::Dash,
        7 => Dash::Circle,
        _ => Dash::Solid,
    }
}

/// The parallel strokes of a border line as `(offset, width)` pairs, with
/// offsets from the edge toward its right-hand side (for a left-to-right
/// edge, downward). The bands fill the line's width without overlapping.
pub fn line_bands(line: &BorderLine) -> Vec<(i32, i32)> {
    let w = line.width_hu();
    if !line.visible() || w <= 0 {
        return Vec::new();
    }
    // Relative band thicknesses with gaps as negative entries. Hancom
    // draws a double line as two quarter-width strokes at the outer edges.
    let parts: &[i32] = match line.kind {
        8 => &[1, -2, 1],
        13 => &[1, -1, 1],
        9 => &[1, -1, 2],
        10 => &[2, -1, 1],
        11 => &[1, -1, 2, -1, 1],
        _ => &[1],
    };
    let total: i32 = parts.iter().map(|p| p.abs()).sum();
    let mut out = Vec::new();
    let mut at = 0;
    for &p in parts {
        let span = p.abs() * w / total;
        if p > 0 {
            out.push((at + span / 2 - w / 2, span.max(1)));
        }
        at += span;
    }
    out
}

/// Strokes a border line from `(x1, y1)` to `(x2, y2)`.
pub fn border_line(dl: &mut DisplayList, x1: i32, y1: i32, x2: i32, y2: i32, line: &BorderLine) {
    stroke_line(dl, (x1, y1, x2, y2), line, false);
}

/// Strokes a column or note separator from `(x1, y1)` to `(x2, y2)`:
/// Hancom draws a separator's large dots bigger and further apart than a
/// border's.
pub fn rule_line(dl: &mut DisplayList, x1: i32, y1: i32, x2: i32, y2: i32, line: &BorderLine) {
    stroke_line(dl, (x1, y1, x2, y2), line, true);
}

fn stroke_line(
    dl: &mut DisplayList,
    (x1, y1, x2, y2): (i32, i32, i32, i32),
    line: &BorderLine,
    rule: bool,
) {
    let dash = dash_of(line.kind);
    let color = line.color & 0x00FF_FFFF;
    let (dx, dy) = ((x2 - x1) as f64, (y2 - y1) as f64);
    let len = (dx * dx + dy * dy).sqrt();
    if len == 0.0 {
        return;
    }
    // Unit normal pointing to the right of the direction of travel.
    let (nx, ny) = (-dy / len, dx / len);
    for (off, width) in line_bands(line) {
        let ox = (nx * off as f64).round() as i32;
        let oy = (ny * off as f64).round() as i32;
        let (width, dash) = if dash == Dash::Circle {
            beads(width, rule)
        } else {
            (width, dash)
        };
        dl.items.push(Item::Line {
            x1: x1 + ox,
            y1: y1 + oy,
            x2: x2 + ox,
            y2: y2 + oy,
            stroke: Stroke { width, color, dash },
        });
    }
}

/// Hancom's 큰 동그라미 line of width `w`: filled dots and their spacing,
/// measured from Hancom's PDF output in its 12 HU device units. As a
/// border, a 56 HU line draws dots 7 units across every 10 units and a
/// 141 HU line dots 13 units across every 24; as a separator (`rule`), a
/// 141 HU line draws dots 19 units across every 43. Other widths follow
/// the line through the measured points.
fn beads(w: i32, rule: bool) -> (i32, Dash) {
    const UNIT: f64 = 12.0;
    let du = f64::from(w) / UNIT;
    let (across, apart) = if rule {
        (1.695 * du - 0.91, 4.66 * du - 11.8)
    } else {
        (0.847 * du + 3.04, 1.977 * du + 0.77)
    };
    let across = across.round().max(1.0);
    let apart = apart.round().max(across + 1.0);
    let width = (across * UNIT) as i32;
    (width, Dash::Beads((apart / across * 1000.0).round() as u16))
}

/// The brushes of a fill, background first.
pub fn brushes(fill: &Fill) -> Vec<Brush> {
    let mut out = Vec::new();
    if let Some(bg) = fill.background() {
        out.push(Brush::Solid(bg));
    }
    if let Some(s) = fill.solid.filter(|s| (1..=6).contains(&s.pattern)) {
        out.push(Brush::Hatch {
            color: s.pattern_color & 0x00FF_FFFF,
            style: s.pattern as u8,
        });
    }
    if let Some(g) = fill.gradient.as_ref().filter(|g| !g.colors.is_empty()) {
        let n = g.colors.len();
        let stops = g
            .colors
            .iter()
            .enumerate()
            .map(|(k, &c)| {
                let pos = match g.positions.get(k) {
                    Some(&p) if n > 2 => p,
                    _ if n > 1 => (k * 100 / (n - 1)) as i32,
                    _ => 0,
                };
                (pos, c & 0x00FF_FFFF)
            })
            .collect();
        out.push(Brush::Gradient(Box::new(GradientBrush {
            kind: g.kind,
            angle: g.angle,
            center_x: g.center_x,
            center_y: g.center_y,
            step: g.step,
            step_center: g.step_center.map_or(50, i32::from),
            stops,
        })));
    }
    if let Some(i) = fill.image.filter(|i| i.bin_id != 0) {
        out.push(Brush::Image {
            bin_id: i.bin_id,
            data: None,
            mode: i.mode,
            effect: i.effect,
            brightness: i.brightness,
            contrast: i.contrast,
        });
    }
    out
}

/// Gives picture fills (of boxes and of drawing objects) their BinData
/// bytes from the document.
pub fn resolve_images(dl: &mut DisplayList, doc: &hm_model::Document) {
    let Some(bins) = doc.bins.as_ref() else {
        return;
    };
    let resolve = |brush: &mut Brush| {
        if let Brush::Image { bin_id, data, .. } = brush
            && data.is_none()
        {
            *data = bins.get(*bin_id);
        }
    };
    for item in &mut dl.items {
        match item {
            Item::Fill { brush, .. } => resolve(brush),
            // Drawing objects filled with a picture.
            Item::Path(p) => p.fill.iter_mut().for_each(resolve),
            _ => {}
        }
    }
}

/// Paints a fill over a rectangle.
pub fn fill_rect(dl: &mut DisplayList, x: i32, y: i32, w: i32, h: i32, fill: &Fill) {
    if w <= 0 || h <= 0 {
        return;
    }
    for brush in brushes(fill) {
        dl.items.push(Item::Fill { x, y, w, h, brush });
    }
}

/// Which sides of a box to draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sides {
    pub left: bool,
    pub right: bool,
    pub top: bool,
    pub bottom: bool,
}

impl Sides {
    pub const ALL: Sides = Sides {
        left: true,
        right: true,
        top: true,
        bottom: true,
    };
    pub const NONE: Sides = Sides {
        left: false,
        right: false,
        top: false,
        bottom: false,
    };
}

/// Draws the four sides of a border fill around a box, centered on its
/// edges: the vertical sides first, then the horizontal ones over them,
/// as Hancom does. A horizontal side reaches over the vertical side at
/// each end to its outer edge (a double line stops at its inner edge).
pub fn border_sides(
    dl: &mut DisplayList,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    bf: &BorderFill,
    sides: Sides,
) {
    let [l, r, _, _] = &bf.sides;
    let half = |s: &BorderLine| if s.visible() { s.width_hu() / 2 } else { 0 };
    let (a, z) = (half(l), half(r));
    border_sides_meeting(dl, x, y, w, h, bf, sides, [a, z, a, z]);
}

/// [`border_sides`] with the half widths of the vertical lines the top
/// and bottom sides meet at their ends, given as `[top left, top right,
/// bottom left, bottom right]`.
#[allow(clippy::too_many_arguments)]
pub fn border_sides_meeting(
    dl: &mut DisplayList,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    bf: &BorderFill,
    sides: Sides,
    meet: [i32; 4],
) {
    let [l, r, t, b] = &bf.sides;
    let (x2, y2) = (x + w, y + h);
    if sides.left && l.visible() {
        border_line(dl, x, y, x, y2, l);
    }
    if sides.right && r.visible() {
        border_line(dl, x2, y, x2, y2, r);
    }
    let reach = |half: i32, kind: u8| if kind == 8 { -half } else { half };
    for (on, line, at, ends) in [
        (sides.top, t, y, (meet[0], meet[1])),
        (sides.bottom, b, y2, (meet[2], meet[3])),
    ] {
        if on && line.visible() {
            let (a, z) = (reach(ends.0, line.kind), reach(ends.1, line.kind));
            border_line(dl, x - a, at, x2 + z, at, line);
        }
    }
}

/// Draws a border fill's diagonals and center lines inside a box.
///
/// Slash lines run from the bottom left toward the top right, backslash
/// lines from the top left toward the bottom right; shape bits 3 and 6 add
/// lines to the middle of an edge, crooked variants and rotation stay
/// approximations of the straight forms.
pub fn diagonals(dl: &mut DisplayList, x: i32, y: i32, w: i32, h: i32, bf: &BorderFill) {
    let d = &bf.diagonal;
    if !d.visible() {
        return;
    }
    let line = BorderLine {
        kind: d.kind,
        width: d.width,
        color: d.color,
    };
    let (x2, y2) = (x + w, y + h);
    // A crooked diagonal bends into a level step across the middle fifth
    // of the cell (attribute bit 8 for the slash, bit 9 for the backslash,
    // unless the center line bit gives those bits to the center line).
    let crooked = |dl: &mut DisplayList, (ax, ay): (i32, i32), (bx, by): (i32, i32)| {
        let my = y + h / 2;
        let (p, q) = (x + w * 2 / 5, x + w * 3 / 5);
        let (m1, m2) = if ax < bx { (p, q) } else { (q, p) };
        let (m1, m2) = if ax == x { (m1, m2) } else { (m2, m1) };
        border_line(dl, ax, ay, m1, my, &line);
        border_line(dl, m1, my, m2, my, &line);
        border_line(dl, m2, my, bx, by, &line);
    };
    // Hancom for Mac draws the corner-to-corner diagonal alone when a cell
    // asks for it, whatever other diagonals its shape bits add.
    let slash = bf.slash();
    if slash & 2 != 0 {
        if !bf.center_line() && bf.attr & (1 << 8) != 0 {
            crooked(dl, (x, y2), (x2, y));
        } else {
            border_line(dl, x, y2, x2, y, &line);
        }
    } else {
        if slash & 1 != 0 {
            border_line(dl, x, y2, x2, y + h / 2, &line);
        }
        if slash & 4 != 0 {
            border_line(dl, x, y2, x + w / 2, y, &line);
        }
    }
    let back = bf.backslash();
    if back & 2 != 0 {
        if !bf.center_line() && bf.attr & (1 << 9) != 0 {
            crooked(dl, (x, y), (x2, y2));
        } else {
            border_line(dl, x, y, x2, y2, &line);
        }
    } else {
        if back & 1 != 0 {
            border_line(dl, x, y, x2, y + h / 2, &line);
        }
        if back & 4 != 0 {
            border_line(dl, x, y, x + w / 2, y2, &line);
        }
    }
    // With the center line bit set, bits 8-9 pick its lines: 1 level
    // across the middle, 2 upright, 3 both.
    if bf.center_line() {
        let kind = (bf.attr >> 8) & 3;
        if kind & 1 != 0 {
            border_line(dl, x, y + h / 2, x2, y + h / 2, &line);
        }
        if kind & 2 != 0 {
            border_line(dl, x + w / 2, y, x + w / 2, y2, &line);
        }
    }
}

/// Fill, diagonals and sides of a border fill around a box, in paint order.
pub fn border_box(
    dl: &mut DisplayList,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    bf: &BorderFill,
    sides: Sides,
) {
    if let Some(f) = &bf.fill {
        fill_rect(dl, x, y, w, h, f);
    }
    diagonals(dl, x, y, w, h, bf);
    border_sides(dl, x, y, w, h, bf, sides);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(kind: u8, width: u8) -> BorderLine {
        BorderLine {
            kind,
            width,
            color: 0,
        }
    }

    #[test]
    fn bands_fill_the_line_width() {
        // 0.5 mm solid: one band of 141 HU on the edge.
        assert_eq!(line_bands(&line(1, 7)), vec![(0, 141)]);
        // A 0.7 mm double line: quarter-width strokes centered 3/8 of the
        // width either side of the edge (Hancom PDF: 48 HU strokes 144 HU
        // apart, on its 12 HU grid).
        let b = line_bands(&line(8, 9));
        assert_eq!(b, vec![(-75, 49), (73, 49)]);
        // No line, no bands.
        assert!(line_bands(&line(0, 7)).is_empty());
    }

    #[test]
    fn box_sides_extend_horizontals_past_corners() {
        let bf = BorderFill {
            sides: [line(1, 9); 4],
            ..Default::default()
        };
        let mut dl = DisplayList::default();
        border_sides(&mut dl, 1000, 2000, 5000, 3000, &bf, Sides::ALL);
        let lines: Vec<_> = dl
            .items
            .iter()
            .filter_map(|i| match i {
                Item::Line { x1, y1, x2, y2, .. } => Some((*x1, *y1, *x2, *y2)),
                _ => None,
            })
            .collect();
        assert_eq!(
            lines,
            vec![
                (1000, 2000, 1000, 5000),
                (6000, 2000, 6000, 5000),
                (901, 2000, 6099, 2000),
                (901, 5000, 6099, 5000)
            ]
        );
    }
}
