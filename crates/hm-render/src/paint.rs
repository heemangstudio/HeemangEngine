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
    /// Border lines (cell, paragraph, page and character borders) of kinds
    /// 2 to 6 as Hancom Mac prints them, in units of 22/15 of the line's
    /// width: dashes 5 on 3 off, dots 1 on 1.5 off, and long dashes of 10
    /// with dots of 1 between gaps of 3.
    BorderDash,
    BorderDot,
    BorderDashDot,
    BorderDashDotDot,
    BorderLongDash,
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
            Dash::BorderDash => vec![5.0 * U, 3.0 * U],
            Dash::BorderDot => vec![U, 1.5 * U],
            Dash::BorderDashDot => vec![10.0 * U, 3.0 * U, U, 3.0 * U],
            Dash::BorderDashDotDot => vec![10.0 * U, 3.0 * U, U, 3.0 * U, U, 3.0 * U],
            Dash::BorderLongDash => vec![10.0 * U, 3.0 * U],
        })
    }

    /// Round caps turn zero-length dashes into dots.
    pub fn round(self) -> bool {
        matches!(self, Dash::Circle | Dash::Beads(_))
    }
}

/// The unit of Hancom's border dash patterns, in line widths.
const U: f32 = 22.0 / 15.0;

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
    /// transparent background: Hancom's 8 by 8 pattern of 100 HU cells,
    /// repeating from `origin` (the page's bottom left corner, where Hancom
    /// anchors it).
    Hatch {
        color: u32,
        style: u8,
        origin: (i32, i32),
        /// The face color under a translucent hatch and the fill's transparency (0 opaque,
        /// 255 clear). Hancom Mac makes the 8 by 8 tile from the face and the lines over it,
        /// each at the fill's opacity, and paints the tile at that opacity again.
        face: Option<(u32, u8)>,
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
fn border_dash(kind: u8) -> Dash {
    match kind {
        2 => Dash::BorderDash,
        3 => Dash::BorderDot,
        4 => Dash::BorderDashDot,
        5 => Dash::BorderDashDotDot,
        6 => Dash::BorderLongDash,
        7 => Dash::Circle,
        _ => Dash::Solid,
    }
}

/// The dash style of a separator line kind.
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
    // Hancom keeps the mixed double line's centers at -40% and +25%
    // of its width when swapping the thin (20%) and thick (50%) strokes.
    if matches!(line.kind, 9 | 10) {
        let (thin, thick) = ((w / 5).max(1), (w / 2).max(1));
        let (a, b) = if line.kind == 9 {
            (thin, thick)
        } else {
            (thick, thin)
        };
        return vec![(-w * 2 / 5, a), (w / 4, b)];
    }
    // Relative band thicknesses with gaps as negative entries. Hancom
    // draws a double line as two quarter-width strokes at the outer edges.
    let parts: &[i32] = match line.kind {
        8 => &[1, -2, 1],
        13 => &[1, -1, 1],
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
    let dash = if rule {
        dash_of(line.kind)
    } else {
        border_dash(line.kind)
    };
    let color = line.color & 0x00FF_FFFF;
    let (dx, dy) = ((x2 - x1) as f64, (y2 - y1) as f64);
    let len = (dx * dx + dy * dy).sqrt();
    if len == 0.0 {
        return;
    }
    if !rule {
        match line.kind {
            12 => return wave(dl, (x1, y1, x2, y2), line.width_hu(), color, 0),
            // A double wave repeats the wave three strokes further along and
            // down or right (Hancom Mac PDF of HWP 5 kind 13).
            13 => {
                let w = line.width_hu();
                wave(dl, (x1, y1, x2, y2), w, color, 0);
                return wave(dl, (x1, y1, x2, y2), w, color, 3 * grid(w / 4));
            }
            // Hancom Mac prints no 3D border.
            14..=17 => return,
            _ => {}
        }
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

/// Rounds to Hancom Mac's 12 HU device grid.
fn grid(v: i32) -> i32 {
    ((v + 6) / 12 * 12).max(12)
}

/// A wave border of width `w` as Hancom Mac prints it: a zigzag of 45
/// degree strokes a quarter of the width thick, as tall as the width,
/// with a 12 HU flat at each peak. It lies on the upper or left side of
/// the edge, starting at its outer peak; `shift` moves it that far along
/// the line and down or right.
fn wave(
    dl: &mut DisplayList,
    (x1, y1, x2, y2): (i32, i32, i32, i32),
    w: i32,
    color: u32,
    shift: i32,
) {
    let (amp, s) = (grid(w), grid(w / 4));
    let (dx, dy) = ((x2 - x1) as f64, (y2 - y1) as f64);
    let len = (dx * dx + dy * dy).sqrt();
    let (ux, uy) = (dx / len, dy / len);
    // Toward the top or the left of the page.
    let (nx, ny) = (-uy.abs(), -ux.abs());
    let stroke = Stroke {
        width: s,
        color,
        dash: Dash::Solid,
    };
    let at = |t: f64, o: f64| {
        let (t, o) = (t + f64::from(shift), o - f64::from(shift));
        (
            x1 + (ux * t + nx * o).round() as i32,
            y1 + (uy * t + ny * o).round() as i32,
        )
    };
    let (outer, inner) = (f64::from(amp - s / 2), f64::from(-s / 2));
    // A shifted wave ends where the line does.
    let len = len - f64::from(shift);
    let (rise, flat) = (f64::from(amp), 12.0);
    let mut t = 0.0;
    let mut high = true;
    while t < len {
        let (o0, o1) = if high { (outer, inner) } else { (inner, outer) };
        let t1 = (t + rise).min(len);
        let o_end = o0 + (o1 - o0) * (t1 - t) / rise;
        let ((ax, ay), (bx, by)) = (at(t, o0), at(t1, o_end));
        dl.items.push(Item::Line {
            x1: ax,
            y1: ay,
            x2: bx,
            y2: by,
            stroke,
        });
        if t1 >= len {
            break;
        }
        let t2 = (t1 + flat).min(len);
        let ((ax, ay), (bx, by)) = (at(t1, o1), at(t2, o1));
        dl.items.push(Item::Line {
            x1: ax,
            y1: ay,
            x2: bx,
            y2: by,
            stroke,
        });
        t = t2;
        high = !high;
    }
}

/// Hancom's 큰 동그라미 line of width `w`: filled dots and their spacing,
/// measured from Hancom's PDF output in its 12 HU device units. As a
/// border, the dots are the width (on that grid) plus about 18 HU across,
/// every twice the width. As a separator (`rule`), a 141 HU line draws dots
/// 19 units across every 43; other widths follow the line through the
/// measured points.
fn beads(w: i32, rule: bool) -> (i32, Dash) {
    const UNIT: f64 = 12.0;
    if !rule {
        let across = grid(w) + 18;
        let apart = 2 * grid(w);
        return (across, Dash::Beads((apart * 1000 / across) as u16));
    }
    let du = f64::from(w) / UNIT;
    let (across, apart) = (1.695 * du - 0.91, 4.66 * du - 11.8);
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
            origin: (0, 0),
            face: None,
        });
    }
    // Hancom Mac paints nothing for step 0 (probes grad3b, grad3d).
    if let Some(g) = fill
        .gradient
        .as_ref()
        .filter(|g| !g.colors.is_empty() && g.step != 0)
    {
        // With more than two colors, Hancom Mac's smooth ramps (step above 100) run from the
        // first color to the second; its bands (step 1 to 100) and conical fans print black
        // (probes grad3, grad3b-d; also Hancom's own HWP 5 of them, whose positions are 0).
        let banded = g.kind == 3 || (1..=100).contains(&g.step);
        let colors: Vec<u32> = match g.colors.as_slice() {
            [_, _, _, ..] if banded => vec![0, 0],
            [a, b, ..] => vec![*a, *b],
            [a] => vec![*a],
            [] => Vec::new(),
        };
        let n = colors.len();
        let stops = colors
            .iter()
            .enumerate()
            .map(|(k, &c)| {
                let pos = if n > 1 { (k * 100 / (n - 1)) as i32 } else { 0 };
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
/// bytes from the document, and hatches their page anchor.
pub fn resolve_images(dl: &mut DisplayList, doc: &hm_model::Document) {
    let bins = doc.bins.as_ref();
    let page_bottom = dl.height;
    let resolve = |brush: &mut Brush| match brush {
        Brush::Image { bin_id, data, .. } if data.is_none() => {
            *data = bins.and_then(|b| b.get(*bin_id));
        }
        Brush::Hatch { origin, .. } => *origin = (0, page_bottom),
        _ => {}
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

/// The transparency (0 opaque, 255 clear) Hancom gives a fill's color and
/// hatch: the alpha of its solid part. Gradients print opaque.
pub fn solid_alpha(fill: &Fill) -> u8 {
    fill.solid.and(fill.alpha.first().copied()).unwrap_or(0)
}

/// Paints a fill over a rectangle. A translucent color or hatch is a
/// rectangle path with the fill's alpha.
pub fn fill_rect(dl: &mut DisplayList, x: i32, y: i32, w: i32, h: i32, fill: &Fill) {
    if w <= 0 || h <= 0 {
        return;
    }
    let alpha = solid_alpha(fill);
    let mut all = brushes(fill);
    // A translucent face and hatch paint as one tile (Hancom Mac, hatch-alpha probe).
    if alpha > 0
        && let [Brush::Solid(bg), Brush::Hatch { face, .. }, ..] = all.as_mut_slice()
    {
        *face = Some((*bg, alpha));
        all.remove(0);
    }
    for brush in all {
        if alpha > 0 && matches!(brush, Brush::Solid(_) | Brush::Hatch { .. }) {
            let mut path = crate::draw::Path::default();
            path.move_to((x, y));
            path.line_to((x + w, y));
            path.line_to((x + w, y + h));
            path.line_to((x, y + h));
            path.close();
            dl.items.push(Item::Path(Box::new(crate::draw::PathItem {
                path,
                fill: vec![brush],
                outline: None,
                fill_alpha: alpha,
            })));
        } else {
            dl.items.push(Item::Fill { x, y, w, h, brush });
        }
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
    border_sides_shared(dl, (x, y, w, h), bf, sides, meet, Sides::NONE);
}

/// [`border_sides_meeting`] for a table cell whose `shared` sides it shares
/// with a neighbor: of a double or triple line there, the cell draws only
/// the bands on its own side, as each cell's inner bands make the grid
/// (Hancom Mac PDF); the outer ones are its neighbor's.
pub fn border_sides_shared(
    dl: &mut DisplayList,
    (x, y, w, h): (i32, i32, i32, i32),
    bf: &BorderFill,
    sides: Sides,
    meet: [i32; 4],
    shared: Sides,
) {
    let [l, r, t, b] = &bf.sides;
    let (x2, y2) = (x + w, y + h);
    // Double and triple lines meeting at a corner join band by band, each
    // band out to the matching band of the other side, outer with outer
    // (Hancom Mac PDF): every band of a box closes into a rectangle.
    let mitred = |a: &BorderLine, c: &BorderLine| {
        a.visible() && c.visible() && !matches!(a.kind, 12..=17) && !matches!(c.kind, 12..=17) && {
            let n = line_bands(a).len();
            n > 1 && n == line_bands(c).len()
        }
    };
    let (vl, vr, ht, hb) = (
        across(l, true),
        across(r, true),
        across(t, false),
        across(b, false),
    );
    // Band `k` counted from the outside of a side: the top and left sides
    // face up and left, the others down and right.
    let outer = |bands: &[(i32, i32)], k: usize, low: bool| {
        if low {
            bands[k]
        } else {
            bands[bands.len() - 1 - k]
        }
    };
    for (on, line, at, bands, low, inner) in [
        (sides.left, l, x, &vl, true, shared.left),
        (sides.right, r, x2, &vr, false, shared.right),
    ] {
        if !on || !line.visible() {
            continue;
        }
        if !mitred(line, t) && !mitred(line, b) {
            border_line(dl, at, y, at, y2, line);
            continue;
        }
        for k in 0..bands.len() {
            let (off, width) = outer(bands, k, low);
            if inner && (off < 0) == low && off != 0 {
                continue;
            }
            let top = if mitred(line, t) {
                let (o, bw) = outer(&ht, k, true);
                y + o - bw / 2
            } else {
                y
            };
            let bottom = if mitred(line, b) {
                let (o, bw) = outer(&hb, k, false);
                y2 + o + bw / 2
            } else {
                y2
            };
            band(dl, (at + off, top, at + off, bottom), width, line);
        }
    }
    let reach = |half: i32, kind: u8| if kind == 8 { -half } else { half };
    for (on, line, at, ends, bands, low, inner) in [
        (sides.top, t, y, (meet[0], meet[1]), &ht, true, shared.top),
        (
            sides.bottom,
            b,
            y2,
            (meet[2], meet[3]),
            &hb,
            false,
            shared.bottom,
        ),
    ] {
        if !on || !line.visible() {
            continue;
        }
        if !mitred(line, l) && !mitred(line, r) {
            let (a, z) = (reach(ends.0, line.kind), reach(ends.1, line.kind));
            border_line(dl, x - a, at, x2 + z, at, line);
            continue;
        }
        for k in 0..bands.len() {
            let (off, width) = outer(bands, k, low);
            if inner && (off < 0) == low && off != 0 {
                continue;
            }
            let left = if mitred(line, l) {
                let (o, bw) = outer(&vl, k, true);
                x + o - bw / 2
            } else {
                x - reach(ends.0, line.kind)
            };
            let right = if mitred(line, r) {
                let (o, bw) = outer(&vr, k, false);
                x2 + o + bw / 2
            } else {
                x2 + reach(ends.1, line.kind)
            };
            band(dl, (left, at + off, right, at + off), width, line);
        }
    }
}

/// The bands of a side as `(offset, width)` across it, offsets growing
/// rightward for a vertical side and downward for a horizontal one.
fn across(line: &BorderLine, vertical: bool) -> Vec<(i32, i32)> {
    let mut bands = line_bands(line);
    if vertical {
        // A downward stroke's right-hand side is the page's left.
        bands.iter_mut().for_each(|b| b.0 = -b.0);
    }
    bands.sort_by_key(|b| b.0);
    bands
}

/// One solid band of a double or triple border line.
fn band(
    dl: &mut DisplayList,
    (x1, y1, x2, y2): (i32, i32, i32, i32),
    width: i32,
    line: &BorderLine,
) {
    dl.items.push(Item::Line {
        x1,
        y1,
        x2,
        y2,
        stroke: Stroke {
            width,
            color: line.color & 0x00FF_FFFF,
            dash: Dash::Solid,
        },
    });
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
    fn mixed_double_lines_swap_widths_without_moving_their_centers() {
        let fonts = hm_fonts::FontSystem::new();
        for (kind, ink) in [
            (9, [false, false, true, true]),
            (10, [true, true, false, true]),
        ] {
            let mut dl = DisplayList {
                width: 8000,
                height: 4000,
                items: Vec::new(),
            };
            border_line(&mut dl, 2000, 2000, 6000, 2000, &line(kind, 13));
            let pixmap = crate::raster::render(&dl, &fonts, 144.0).unwrap();
            for (y, black) in [30, 35, 41, 44].into_iter().zip(ink) {
                let pixel = pixmap.pixel(80, y).unwrap();
                let rgb = (pixel.red(), pixel.green(), pixel.blue());
                assert_eq!(rgb, if black { (0, 0, 0) } else { (255, 255, 255) });
            }
        }
    }

    /// Note separators of every line type Hancom's 주석 모양 dialog offers
    /// draw as that type: dash-dot
    /// and dash-dot-dot patterns, long dashes, round dots, and two or three
    /// parallel strokes for the mixed double and triple lines.
    #[test]
    fn separators_draw_every_line_type() {
        let strokes = |kind: u8| {
            let mut dl = DisplayList::default();
            rule_line(&mut dl, 1000, 2000, 15000, 2000, &line(kind, 4));
            dl.items
                .iter()
                .filter_map(|i| match i {
                    Item::Line { y1, stroke, .. } => Some((*y1, stroke.width, stroke.dash)),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let dash = |kind: u8| {
            let s = strokes(kind);
            assert_eq!(s.len(), 1, "kind {kind}");
            s[0].2
        };
        assert_eq!(dash(1), Dash::Solid);
        assert_eq!(dash(4), Dash::DashDot);
        assert_eq!(dash(5), Dash::DashDotDot);
        assert_eq!(dash(6), Dash::Dash);
        assert!(matches!(dash(7), Dash::Beads(_)));
        for (kind, bands) in [(8, 2), (9, 2), (10, 2), (11, 3)] {
            let s = strokes(kind);
            assert_eq!(s.len(), bands, "kind {kind}");
            assert!(s.iter().all(|b| b.2 == Dash::Solid));
            assert!(s.windows(2).all(|w| w[0].0 < w[1].0), "kind {kind}");
        }
        // Thin then thick, thick then thin.
        let (slim_thick, thick_slim) = (strokes(9), strokes(10));
        assert!(slim_thick[0].1 < slim_thick[1].1);
        assert!(thick_slim[0].1 > thick_slim[1].1);
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
