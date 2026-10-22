//! Painting objects: inline ones in their lines, floating ones at their
//! placements.
//!
//! Callers that lay out paragraphs in a frame (body, header, footer, note,
//! cell) paint inline objects with [`paint_inline`] while drawing a line and
//! floating ones with [`paint_placement`]. A drawing object is a tree of
//! shapes; each shape maps its own coordinates to its parent's through its
//! rendering matrix, and the top shape's coordinates start at the object's
//! box.

use hm_layout::math::MathLayout;
use hm_layout::objects::{ObjectLayout, Placement, TextFrame};
use hm_layout::{ParaLayout, Rect};
use hm_model::{
    Arrow, Control, CurveGeom, EllipseGeom, LineStyle, Matrix, Paragraph, Picture, Shape, ShapePart,
};

use crate::draw::{ImageItem, Outline, Path, PathItem};
use crate::image::Effects;
use crate::paint::{Dash, brushes};
use crate::text::Ctx;
use crate::{DisplayList, GlyphRun, Item};

/// The inner layout of control `control` of a paragraph.
fn inner(layout: &ParaLayout, control: u32) -> Option<&ObjectLayout> {
    layout
        .measured
        .objects
        .iter()
        .find(|(k, _)| *k == control)
        .map(|(_, o)| &**o)
}

/// Paints an object treated as a character: control `control` of a
/// paragraph, its box's top left (inside the outer margins) at `(x, y)`, on
/// a line whose baseline is at `baseline` (page coordinates).
pub(crate) fn paint_inline(
    dl: &mut DisplayList,
    ctx: &Ctx<'_>,
    para: &Paragraph,
    layout: &ParaLayout,
    control: u32,
    (x, y): (i32, i32),
    baseline: i32,
) {
    let Some(c) = para.controls.get(control as usize) else {
        return;
    };
    let inner = inner(layout, control);
    let (dx, dy) = inner.map_or((0, 0), ObjectLayout::object_offset);
    let Some(o) = c.object_common() else { return };
    let rect = Rect {
        x: x + dx,
        y: y + dy,
        w: o.width,
        h: o.height,
    };
    match inner.and_then(|i| i.equation.as_ref()) {
        // The equation's baseline is the line's.
        Some(m) => paint_math(dl, ctx.fonts, m, rect.x, baseline),
        None => paint_object(dl, ctx, c, rect, inner),
    }
    paint_caption(dl, ctx, c, inner, rect);
}

/// Paints an object's caption beside its box.
fn paint_caption(
    dl: &mut DisplayList,
    ctx: &Ctx<'_>,
    c: &Control,
    inner: Option<&ObjectLayout>,
    rect: Rect,
) {
    let (Some(inner), Some(cap)) = (inner, c.as_object().and_then(|o| o.caption())) else {
        return;
    };
    let (Some((cx, cy)), Some(cl)) = (inner.caption_offset((rect.w, rect.h)), &inner.caption)
    else {
        return;
    };
    let mut placed = Vec::new();
    for l in &cl.lines {
        let (Some(para), Some(pal)) = (
            cap.paras.get(l.para as usize),
            cl.paras.get(l.para as usize),
        ) else {
            continue;
        };
        crate::text::draw_line(
            dl,
            ctx,
            para,
            pal,
            l.line as usize,
            rect.x + cx,
            rect.y + cy + l.y,
            &mut placed,
        );
    }
}

/// Paints a floating object at its placement.
pub(crate) fn paint_placement(
    dl: &mut DisplayList,
    ctx: &Ctx<'_>,
    para: &Paragraph,
    layout: &ParaLayout,
    p: &Placement,
) {
    let Some(c) = para.controls.get(p.control as usize) else {
        return;
    };
    let inner = inner(layout, p.control);
    match inner.and_then(|i| i.equation.as_ref()) {
        Some(m) => {
            let baseline = p.rect.y + p.rect.h.min(m.ascent).max(0);
            paint_math(dl, ctx.fonts, m, p.rect.x, baseline);
        }
        None => paint_object(dl, ctx, c, p.rect, inner),
    }
    paint_caption(dl, ctx, c, inner, p.rect);
}

/// Paints a drawing object into its box.
fn paint_object(
    dl: &mut DisplayList,
    ctx: &Ctx<'_>,
    c: &Control,
    rect: Rect,
    inner: Option<&ObjectLayout>,
) {
    if let Some(form) = c.as_form() {
        let text = inner.and_then(|i| i.form.as_ref());
        crate::forms::paint(dl, ctx, form, rect, text);
        return;
    }
    let Some(shape) = c.as_object().and_then(|o| o.shape()) else {
        return;
    };
    let origin = Matrix([1.0, 0.0, rect.x as f64, 0.0, 1.0, rect.y as f64]);
    // An OLE object fills the object's box; its own sizes are placeholders.
    if let Some(ShapePart::Ole(o)) = shape.kind() {
        ole(dl, ctx, &origin, o, (rect.w, rect.h));
        return;
    }
    let frames = inner.map_or(&[][..], |i| i.frames.as_slice());
    let mut next = 0;
    paint_shape(dl, ctx, shape, &origin, frames, &mut next);
}

fn pt(m: &Matrix, p: (i32, i32)) -> (i32, i32) {
    let (x, y) = m.apply(p.0 as f64, p.1 as f64);
    (x.round() as i32, y.round() as i32)
}

/// Paints one shape and its group members; `parent` maps the shape's
/// parent coordinates to the page.
fn paint_shape(
    dl: &mut DisplayList,
    ctx: &Ctx<'_>,
    shape: &Shape,
    parent: &Matrix,
    frames: &[TextFrame],
    next: &mut usize,
) {
    let m = parent.mul(&shape.component.matrix());
    let style = shape.style();
    let mut path = Path::default();
    let mut closed = true;
    match shape.kind() {
        Some(ShapePart::Rect(r)) => rect_path(&mut path, &m, r.corners, r.round),
        Some(ShapePart::Ellipse(e)) => {
            closed = ellipse_path(&mut path, &m, e);
        }
        Some(ShapePart::Arc(a)) => {
            closed = arc_path(&mut path, &m, (a.center, a.axis1, a.axis2), a.attr & 0xFF);
        }
        Some(ShapePart::Line(l)) => {
            path.move_to(pt(&m, l.start));
            path.line_to(pt(&m, l.end));
            closed = false;
        }
        Some(ShapePart::Polygon(p)) => {
            for (k, &q) in p.points.iter().enumerate() {
                if k == 0 {
                    path.move_to(pt(&m, q));
                } else {
                    path.line_to(pt(&m, q));
                }
            }
            closed = p.points.len() > 2 && p.points.first() == p.points.last();
            if closed {
                path.close();
            }
        }
        Some(ShapePart::Curve(c)) => {
            closed = curve_path(&mut path, &m, c);
        }
        Some(ShapePart::Picture(p)) => picture(dl, ctx, &m, p),

        _ => {}
    }
    if !path.is_empty() {
        let fill = match style {
            Some(s) if closed => brushes(&s.fill),
            _ => Vec::new(),
        };
        let mut outline = style.and_then(|s| outline(&s.line));
        let fill_alpha = style.map_or(0, |s| s.fill.alpha.first().copied().unwrap_or(0));
        let ends = if closed { None } else { path_ends(&path) };
        // Double and triple lines: parallel strokes splitting the width.
        let bands = style
            .zip(outline)
            .map(|(s, o)| multi_line(&path, s.line.kind(), o))
            .unwrap_or_default();
        if !bands.is_empty() {
            outline = None;
        }
        if !fill.is_empty() || outline.is_some() {
            dl.items.push(Item::Path(Box::new(PathItem {
                path,
                fill,
                outline,
                fill_alpha,
            })));
        }
        for item in bands {
            dl.items.push(Item::Path(Box::new(item)));
        }
        if let (Some(s), Some((start, end)), Some(_)) = (style, ends, outline) {
            let (head, head_filled) = s.line.head();
            let (tail, tail_filled) = s.line.tail();
            arrowhead(dl, &s.line, start, head, head_filled, s.line.head_size());
            arrowhead(dl, &s.line, end, tail, tail_filled, s.line.tail_size());
        }
    }
    if let Some(t) = shape.text() {
        if let Some(frame) = frames.get(*next) {
            paint_text(dl, ctx, shape, t, &m, frame);
        }
        *next += 1;
    }
    for child in shape.children() {
        paint_shape(dl, ctx, child, &m, frames, next);
    }
}

/// Paints the text of a shape inside its box less the text margins,
/// aligned vertically.
fn paint_text(
    dl: &mut DisplayList,
    ctx: &Ctx<'_>,
    shape: &Shape,
    t: &hm_model::TextBox,
    m: &Matrix,
    frame: &TextFrame,
) {
    let (w, h) = (
        shape.component.initial_size.0 as i32,
        shape.component.initial_size.1 as i32,
    );
    let corners = [pt(m, (0, 0)), pt(m, (w, 0)), pt(m, (w, h)), pt(m, (0, h))];
    let x0 = corners.iter().map(|p| p.0).min().unwrap_or(0);
    let y0 = corners.iter().map(|p| p.1).min().unwrap_or(0);
    let y1 = corners.iter().map(|p| p.1).max().unwrap_or(0);
    let [ml, _, mt, mb] = t.margins.unwrap_or([0; 4]).map(i32::from);
    let room = (y1 - y0) - mt - mb;
    let offset = match t.vertical_align() {
        1 => (room - frame.content) / 2,
        2 => room - frame.content,
        _ => 0,
    };
    let mut placed = Vec::new();
    for l in &frame.lines {
        let (Some(para), Some(pal)) = (
            t.paras.get(l.para as usize),
            frame.paras.get(l.para as usize),
        ) else {
            continue;
        };
        crate::text::draw_line(
            dl,
            ctx,
            para,
            pal,
            l.line as usize,
            x0 + ml,
            y0 + mt + offset + l.y,
            &mut placed,
        );
    }
}

/// The outline of a drawing object, when it draws one. Drawing objects
/// count line kinds from 1 for solid.
fn outline(l: &LineStyle) -> Option<Outline> {
    let dash = match l.kind() {
        0 => return None,
        2 => Dash::LongDash,
        3 => Dash::Dot,
        4 => Dash::DashDot,
        5 => Dash::DashDotDot,
        6 => Dash::Dash,
        7 => Dash::Circle,
        _ => Dash::Solid,
    };
    Some(Outline {
        width: l.width.max(0),
        color: l.color & 0x00FF_FFFF,
        dash,
        round_cap: l.cap() == 0,
    })
}

/// An open path's end points with the direction pointing out of the path
/// at each: `((point, direction), (point, direction))`.
#[allow(clippy::type_complexity)]
fn path_ends(path: &Path) -> Option<(((f64, f64), (f64, f64)), ((f64, f64), (f64, f64)))> {
    use crate::draw::PathEl;
    let mut pts: Vec<(f64, f64)> = Vec::new();
    for e in &path.els {
        match *e {
            PathEl::Move(x, y) | PathEl::Line(x, y) => pts.push((x as f64, y as f64)),
            PathEl::Cubic(a, b, c, d, x, y) => {
                pts.push((a as f64, b as f64));
                pts.push((c as f64, d as f64));
                pts.push((x as f64, y as f64));
            }
            PathEl::Close => {}
        }
    }
    let first = *pts.first()?;
    let last = *pts.last()?;
    // The nearest distinct points give the directions.
    let after = pts.iter().find(|p| **p != first)?;
    let before = pts.iter().rev().find(|p| **p != last)?;
    let dir = |from: (f64, f64), to: (f64, f64)| {
        let (dx, dy) = (to.0 - from.0, to.1 - from.1);
        let l = dx.hypot(dy).max(1e-9);
        (dx / l, dy / l)
    };
    Some(((first, dir(*after, first)), (last, dir(*before, last))))
}

/// An arrowhead at an end of a line pointing along `dir`. Hancom draws
/// the smallest class about 576 HU long and wide whatever the line width,
/// larger classes and thick lines bigger.
fn arrowhead(
    dl: &mut DisplayList,
    line: &LineStyle,
    ((x, y), (dx, dy)): ((f64, f64), (f64, f64)),
    kind: Arrow,
    filled: bool,
    size: u32,
) {
    if matches!(kind, Arrow::None) {
        return;
    }
    let base = 576f64.max(4.0 * line.width as f64);
    let class = |c: u32| [1.0, 1.5, 2.0][c.min(2) as usize];
    let (w, l) = (base * class(size / 3), base * class(size % 3));
    // Along the line (u) and across it (v).
    let at = |u: f64, v: f64| {
        (
            (x + dx * u - dy * v).round() as i32,
            (y + dy * u + dx * v).round() as i32,
        )
    };
    let mut path = Path::default();
    let mut solid = filled;
    match kind {
        Arrow::Arrow => {
            path.move_to(at(0.0, 0.0));
            path.line_to(at(-l, -w / 2.0));
            path.line_to(at(-l, w / 2.0));
            path.close();
            solid = true;
        }
        Arrow::Spear => {
            path.move_to(at(0.0, 0.0));
            path.line_to(at(-l, -w / 2.0));
            path.line_to(at(-l * 0.75, 0.0));
            path.line_to(at(-l, w / 2.0));
            path.close();
            solid = true;
        }
        Arrow::ConcaveArrow => {
            path.move_to(at(0.0, 0.0));
            path.line_to(at(-l, -w / 2.0));
            path.line_to(at(-l * 0.6, 0.0));
            path.line_to(at(-l, w / 2.0));
            path.close();
            solid = true;
        }
        Arrow::Diamond => {
            path.move_to(at(l / 2.0, 0.0));
            path.line_to(at(0.0, -w / 2.0));
            path.line_to(at(-l / 2.0, 0.0));
            path.line_to(at(0.0, w / 2.0));
            path.close();
        }
        Arrow::Square => {
            path.move_to(at(l / 2.0, -w / 2.0));
            path.line_to(at(l / 2.0, w / 2.0));
            path.line_to(at(-l / 2.0, w / 2.0));
            path.line_to(at(-l / 2.0, -w / 2.0));
            path.close();
        }
        Arrow::Circle => {
            const K: f64 = 0.552_284_8;
            let (a, b) = (l / 2.0, w / 2.0);
            path.move_to(at(a, 0.0));
            path.cubic_to(at(a, b * K), at(a * K, b), at(0.0, b));
            path.cubic_to(at(-a * K, b), at(-a, b * K), at(-a, 0.0));
            path.cubic_to(at(-a, -b * K), at(-a * K, -b), at(0.0, -b));
            path.cubic_to(at(a * K, -b), at(a, -b * K), at(a, 0.0));
            path.close();
        }
        Arrow::None => return,
    }
    let color = line.color & 0x00FF_FFFF;
    dl.items.push(Item::Path(Box::new(PathItem {
        path,
        fill: if solid {
            vec![crate::paint::Brush::Solid(color)]
        } else {
            vec![crate::paint::Brush::Solid(0x00FF_FFFF)]
        },
        outline: Some(Outline {
            width: line.width.max(0),
            color,
            dash: Dash::Solid,
            round_cap: false,
        }),
        fill_alpha: 0,
    })));
}

/// The parallel strokes of a multi-line outline kind (double, thin-thick,
/// thick-thin, thin-thick-thin, double wave) along a polygonal path, each
/// band offset from the path toward the right of travel and the bands
/// together as wide as the line. Empty for single lines and curved paths.
fn multi_line(path: &Path, kind: u32, o: Outline) -> Vec<PathItem> {
    use crate::draw::PathEl;
    let parts: &[i32] = match kind {
        8 | 13 => &[1, -1, 1],
        9 => &[1, -1, 2],
        10 => &[2, -1, 1],
        11 => &[1, -1, 2, -1, 1],
        _ => return Vec::new(),
    };
    // Polylines only, split into subpaths.
    let mut subs: Vec<(Vec<(f64, f64)>, bool)> = Vec::new();
    for e in &path.els {
        match *e {
            PathEl::Move(x, y) => subs.push((vec![(x as f64, y as f64)], false)),
            PathEl::Line(x, y) => match subs.last_mut() {
                Some(s) => s.0.push((x as f64, y as f64)),
                None => return Vec::new(),
            },
            PathEl::Close => {
                if let Some(s) = subs.last_mut() {
                    s.1 = true;
                }
            }
            PathEl::Cubic(..) => return Vec::new(),
        }
    }
    // A hairline still shows two strokes.
    let w = (o.width as f64).max(30.0);
    let total: i32 = parts.iter().map(|p| p.abs()).sum();
    let mut out = Vec::new();
    let mut at = 0.0;
    for &p in parts {
        let span = p.abs() as f64 * w / total as f64;
        if p > 0 {
            let off = at + span / 2.0 - w / 2.0;
            let mut band = Path::default();
            for (pts, closed) in &subs {
                let moved = offset_polyline(pts, *closed, off);
                for (k, q) in moved.iter().enumerate() {
                    let q = (q.0.round() as i32, q.1.round() as i32);
                    if k == 0 {
                        band.move_to(q);
                    } else {
                        band.line_to(q);
                    }
                }
                if *closed {
                    band.close();
                }
            }
            out.push(PathItem {
                path: band,
                fill: Vec::new(),
                outline: Some(Outline {
                    width: span.round().max(1.0) as i32,
                    ..o
                }),
                fill_alpha: 0,
            });
        }
        at += span;
    }
    out
}

/// A polyline moved `d` to the right of its direction of travel, corners
/// mitered.
fn offset_polyline(pts: &[(f64, f64)], closed: bool, d: f64) -> Vec<(f64, f64)> {
    let mut pts: Vec<(f64, f64)> = pts.to_vec();
    pts.dedup();
    if closed && pts.len() > 2 && pts.first() == pts.last() {
        pts.pop();
    }
    let n = pts.len();
    if n < 2 {
        return pts;
    }
    let normal = |a: (f64, f64), b: (f64, f64)| {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let l = dx.hypot(dy).max(1e-9);
        (-dy / l, dx / l)
    };
    (0..n)
        .map(|i| {
            let prev = if i > 0 {
                Some(pts[i - 1])
            } else if closed {
                Some(pts[n - 1])
            } else {
                None
            };
            let next = if i + 1 < n {
                Some(pts[i + 1])
            } else if closed {
                Some(pts[0])
            } else {
                None
            };
            let p = pts[i];
            match (prev, next) {
                (Some(a), Some(b)) => {
                    let (n1, n2) = (normal(a, p), normal(p, b));
                    let (mx, my) = (n1.0 + n2.0, n1.1 + n2.1);
                    let dot = 1.0 + n1.0 * n2.0 + n1.1 * n2.1;
                    if dot < 0.1 {
                        (p.0 + n1.0 * d, p.1 + n1.1 * d)
                    } else {
                        (p.0 + mx * d / dot, p.1 + my * d / dot)
                    }
                }
                (None, Some(b)) => {
                    let n1 = normal(p, b);
                    (p.0 + n1.0 * d, p.1 + n1.1 * d)
                }
                (Some(a), None) => {
                    let n1 = normal(a, p);
                    (p.0 + n1.0 * d, p.1 + n1.1 * d)
                }
                (None, None) => p,
            }
        })
        .collect()
}

/// A rectangle with corners rounded by `round` percent of its shorter side.
fn rect_path(path: &mut Path, m: &Matrix, c: [(i32, i32); 4], round: u8) {
    let (x0, y0) = c[0];
    let (x1, y1) = c[2];
    let r = ((x1 - x0).abs().min((y1 - y0).abs()) as i64 * round.min(50) as i64 / 100) as i32;
    if r <= 0 {
        path.move_to(pt(m, c[0]));
        for &q in &c[1..] {
            path.line_to(pt(m, q));
        }
        path.close();
        return;
    }
    let k = (r as f64 * 0.552_284_75).round() as i32;
    let (l, t, rt, b) = (x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1));
    path.move_to(pt(m, (l + r, t)));
    path.line_to(pt(m, (rt - r, t)));
    path.cubic_to(
        pt(m, (rt - r + k, t)),
        pt(m, (rt, t + r - k)),
        pt(m, (rt, t + r)),
    );
    path.line_to(pt(m, (rt, b - r)));
    path.cubic_to(
        pt(m, (rt, b - r + k)),
        pt(m, (rt - r + k, b)),
        pt(m, (rt - r, b)),
    );
    path.line_to(pt(m, (l + r, b)));
    path.cubic_to(
        pt(m, (l + r - k, b)),
        pt(m, (l, b - r + k)),
        pt(m, (l, b - r)),
    );
    path.line_to(pt(m, (l, t + r)));
    path.cubic_to(
        pt(m, (l, t + r - k)),
        pt(m, (l + r - k, t)),
        pt(m, (l + r, t)),
    );
    path.close();
}

/// An ellipse given by its center and two conjugate semi-axis vectors.
#[derive(Clone, Copy)]
struct Ellipse {
    c: (f64, f64),
    u: (f64, f64),
    v: (f64, f64),
}

impl Ellipse {
    fn new(center: (i32, i32), a1: (i32, i32), a2: (i32, i32)) -> Ellipse {
        let c = (center.0 as f64, center.1 as f64);
        Ellipse {
            c,
            u: (a1.0 as f64 - c.0, a1.1 as f64 - c.1),
            v: (a2.0 as f64 - c.0, a2.1 as f64 - c.1),
        }
    }

    fn at(&self, t: f64) -> (f64, f64) {
        (
            self.c.0 + self.u.0 * t.cos() + self.v.0 * t.sin(),
            self.c.1 + self.u.1 * t.cos() + self.v.1 * t.sin(),
        )
    }

    fn tangent(&self, t: f64) -> (f64, f64) {
        (
            -self.u.0 * t.sin() + self.v.0 * t.cos(),
            -self.u.1 * t.sin() + self.v.1 * t.cos(),
        )
    }

    /// The parameter of point `p`.
    fn param(&self, p: (i32, i32)) -> f64 {
        let (px, py) = (p.0 as f64 - self.c.0, p.1 as f64 - self.c.1);
        let (u, v) = (self.u, self.v);
        let det = u.0 * v.1 - u.1 * v.0;
        if det.abs() < 1e-9 {
            return 0.0;
        }
        let cos = (px * v.1 - py * v.0) / det;
        let sin = (u.0 * py - u.1 * px) / det;
        sin.atan2(cos)
    }

    /// Appends the arc from parameter `a0` to `a1` as cubic pieces.
    fn arc(&self, path: &mut Path, m: &Matrix, (a0, a1): (f64, f64), start: bool) {
        let p = |q: (f64, f64)| pt(m, (q.0.round() as i32, q.1.round() as i32));
        let n = (((a1 - a0).abs() / std::f64::consts::FRAC_PI_2).ceil() as usize).max(1);
        let step = (a1 - a0) / n as f64;
        let k = 4.0 / 3.0 * (step / 4.0).tan();
        if start {
            path.move_to(p(self.at(a0)));
        } else {
            path.line_to(p(self.at(a0)));
        }
        for i in 0..n {
            let (t0, t1) = (a0 + step * i as f64, a0 + step * (i + 1) as f64);
            let (p0, p1) = (self.at(t0), self.at(t1));
            let (d0, d1) = (self.tangent(t0), self.tangent(t1));
            path.cubic_to(
                p((p0.0 + k * d0.0, p0.1 + k * d0.1)),
                p((p1.0 - k * d1.0, p1.1 - k * d1.1)),
                p(p1),
            );
        }
    }

    /// An arc closed as a pie (1), a chord (2) or left open (0). Returns
    /// whether the path closes.
    fn finish(&self, path: &mut Path, m: &Matrix, span: (f64, f64), kind: u32) -> bool {
        match kind {
            1 => {
                path.move_to(pt(m, (self.c.0.round() as i32, self.c.1.round() as i32)));
                self.arc(path, m, span, false);
                path.close();
                true
            }
            2 => {
                self.arc(path, m, span, true);
                path.close();
                true
            }
            _ => {
                self.arc(path, m, span, true);
                false
            }
        }
    }
}

/// An ellipse, or the arc it was turned into. Returns whether it closes.
fn ellipse_path(path: &mut Path, m: &Matrix, e: &EllipseGeom) -> bool {
    let el = Ellipse::new(e.center, e.axis1, e.axis2);
    if !e.is_arc() {
        el.arc(path, m, (0.0, std::f64::consts::TAU), true);
        path.close();
        return true;
    }
    let a0 = el.param(e.start);
    let mut a1 = el.param(e.end);
    if a1 <= a0 {
        a1 += std::f64::consts::TAU;
    }
    el.finish(path, m, (a0, a1), e.arc_kind())
}

/// An arc object: the quarter of the ellipse from the first axis end to
/// the second.
fn arc_path(
    path: &mut Path,
    m: &Matrix,
    (center, a1, a2): ((i32, i32), (i32, i32), (i32, i32)),
    kind: u32,
) -> bool {
    Ellipse::new(center, a1, a2).finish(path, m, (0.0, std::f64::consts::FRAC_PI_2), kind)
}

/// A curve: straight segments, and cubic segments whose two control points
/// follow their start point. Returns whether it closes.
fn curve_path(path: &mut Path, m: &Matrix, c: &CurveGeom) -> bool {
    let pts = &c.points;
    if pts.is_empty() {
        return false;
    }
    path.move_to(pt(m, pts[0]));
    let mut i = 0;
    while i + 1 < pts.len() {
        if c.segments.get(i) == Some(&1) && i + 3 < pts.len() {
            path.cubic_to(pt(m, pts[i + 1]), pt(m, pts[i + 2]), pt(m, pts[i + 3]));
            i += 3;
        } else {
            path.line_to(pt(m, pts[i + 1]));
            i += 1;
        }
    }
    let closed = pts.len() > 2 && pts.first() == pts.last();
    if closed {
        path.close();
    }
    closed
}

/// A picture: its visible part stretched over the image rectangle, then
/// through the shape's matrix; the border on top.
fn picture(dl: &mut DisplayList, ctx: &Ctx<'_>, m: &Matrix, p: &Picture) {
    let [c0, c1, _, c3] = p.corners;
    let o = pt(m, c0);
    let xv = pt(m, c1);
    let yv = pt(m, c3);
    let frame = [xv.0 - o.0, yv.0 - o.0, o.0, xv.1 - o.1, yv.1 - o.1, o.1];
    if let Some(data) = ctx.doc.bins.as_ref().and_then(|b| b.get(p.image.bin_id)) {
        dl.items.push(Item::Image(Box::new(ImageItem {
            data,
            frame,
            crop: p.crop,
            effects: Effects {
                effect: p.image.effect,
                brightness: p.image.brightness,
                contrast: p.image.contrast,
                alpha: p.alpha(),
            },
            natural: p.original_size().unwrap_or((0, 0)),
        })));
    }
    if let Some(ol) = outline(&p.border).filter(|_| p.border.width > 0) {
        let mut path = Path::default();
        rect_path(&mut path, m, p.corners, 0);
        dl.items.push(Item::Path(Box::new(PathItem {
            path,
            fill: Vec::new(),
            outline: Some(ol),
            fill_alpha: 0,
        })));
    }
}

/// An OLE object: its saved presentation picture stretched over its box,
/// or its chart drawn there.
fn ole(dl: &mut DisplayList, ctx: &Ctx<'_>, m: &Matrix, o: &hm_model::Ole, (w, h): (i32, i32)) {
    let Some(view) = ctx.doc.bins.as_ref().and_then(|b| b.ole(o.bin_id)) else {
        return;
    };
    let p0 = pt(m, (0, 0));
    match &*view {
        hm_model::chart::OleView::Picture(data) => {
            let px = pt(m, (w, 0));
            let py = pt(m, (0, h));
            dl.items.push(Item::Image(Box::new(ImageItem {
                data: data.clone(),
                frame: [
                    px.0 - p0.0,
                    py.0 - p0.0,
                    p0.0,
                    px.1 - p0.1,
                    py.1 - p0.1,
                    p0.1,
                ],
                crop: [0; 4],
                effects: Effects::default(),
                natural: (0, 0),
            })));
        }
        hm_model::chart::OleView::Chart(c) => {
            let rect = Rect {
                x: p0.0,
                y: p0.1,
                w,
                h,
            };
            crate::chart::draw(dl, ctx.fonts, c, rect);
        }
    }
}

/// Paints an equation with its origin at `x` and its baseline at `y`.
pub fn paint_math(
    dl: &mut DisplayList,
    fonts: &hm_fonts::FontSystem,
    m: &MathLayout,
    x: i32,
    y: i32,
) {
    use hm_layout::math::MathItem;
    for it in &m.items {
        match *it {
            MathItem::Glyph {
                face,
                glyph,
                size,
                x: gx,
                y: gy,
                x_scale,
                bold,
                italic,
                color,
                ..
            } => {
                if fonts.face(face).is_none() {
                    continue;
                }
                dl.items.push(Item::Glyphs(GlyphRun {
                    face,
                    size,
                    ratio: x_scale,
                    color,
                    fake_bold: bold,
                    italic,
                    baseline: y + gy,
                    glyphs: vec![(glyph, x + gx)],
                }));
            }
            MathItem::Rule {
                x: rx,
                y: ry,
                w,
                h,
                color,
            } => dl.items.push(Item::Rect {
                x: x + rx,
                y: y + ry,
                w,
                h,
                color,
            }),
        }
    }
}
