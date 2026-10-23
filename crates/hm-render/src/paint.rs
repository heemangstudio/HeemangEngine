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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
}

impl Dash {
    /// The on/off pattern in multiples of the stroke width, `None` when solid.
    pub fn pattern(self) -> Option<&'static [f32]> {
        Some(match self {
            Dash::Solid => return None,
            Dash::LongDash => &[8.0, 4.0],
            Dash::Dot => &[1.0, 2.0],
            Dash::DashDot => &[8.0, 3.0, 1.0, 3.0],
            Dash::DashDotDot => &[8.0, 3.0, 1.0, 3.0, 1.0, 3.0],
            Dash::Dash => &[16.0, 6.0],
            Dash::Circle => &[0.0, 2.5],
        })
    }

    /// Round caps turn zero-length dashes into dots.
    pub fn round(self) -> bool {
        self == Dash::Circle
    }
}

/// A stroke: width in HU, COLORREF color, dash style.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
        dl.items.push(Item::Line {
            x1: x1 + ox,
            y1: y1 + oy,
            x2: x2 + ox,
            y2: y2 + oy,
            stroke: Stroke { width, color, dash },
        });
    }
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
    let slash = bf.slash();
    if slash & 2 != 0 {
        border_line(dl, x, y2, x2, y, &line);
    }
    if slash & 1 != 0 {
        border_line(dl, x, y2, x2, y + h / 2, &line);
    }
    if slash & 4 != 0 {
        border_line(dl, x, y2, x + w / 2, y, &line);
    }
    let back = bf.backslash();
    if back & 2 != 0 {
        border_line(dl, x, y, x2, y2, &line);
    }
    if back & 1 != 0 {
        border_line(dl, x, y, x2, y + h / 2, &line);
    }
    if back & 4 != 0 {
        border_line(dl, x, y, x + w / 2, y2, &line);
    }
    if bf.center_line() {
        border_line(dl, x + w / 2, y, x + w / 2, y2, &line);
        border_line(dl, x, y + h / 2, x2, y + h / 2, &line);
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
