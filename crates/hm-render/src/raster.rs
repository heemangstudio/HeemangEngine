//! tiny-skia backend.

use hm_fonts::{FontSystem, PathOp};
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Stroke, Transform};

use crate::paint::{Brush, GradientBrush, Stroke as LineStroke};
use crate::{DisplayList, GlyphRun, Item};

/// Pixels for `hu` at `dpi`.
fn px(hu: i32, dpi: f32) -> f32 {
    hu as f32 * dpi / 7200.0
}

fn paint(color: u32) -> Paint<'static> {
    let mut p = Paint::default();
    let (r, g, b) = (
        (color & 0xFF) as u8,
        ((color >> 8) & 0xFF) as u8,
        ((color >> 16) & 0xFF) as u8,
    );
    p.set_color_rgba8(r, g, b, 255);
    p.anti_alias = true;
    p
}

/// Draws a display list on white paper.
pub fn render(dl: &DisplayList, fonts: &FontSystem, dpi: f32) -> Option<Pixmap> {
    let w = px(dl.width, dpi).round().max(1.0) as u32;
    let h = px(dl.height, dpi).round().max(1.0) as u32;
    let mut pixmap = Pixmap::new(w, h)?;
    pixmap.fill(tiny_skia::Color::WHITE);
    for item in &dl.items {
        match item {
            Item::Glyphs(run) => glyphs(&mut pixmap, run, fonts, dpi),
            Item::Rect { x, y, w, h, color } => {
                if let Some(r) = tiny_skia::Rect::from_xywh(
                    px(*x, dpi),
                    px(*y, dpi),
                    px(*w, dpi).max(0.5),
                    px(*h, dpi).max(0.5),
                ) {
                    pixmap.fill_rect(r, &paint(*color), Transform::identity(), None);
                }
            }
            Item::Line {
                x1,
                y1,
                x2,
                y2,
                stroke,
            } => line(&mut pixmap, (*x1, *y1, *x2, *y2), stroke, dpi),
            Item::Fill { x, y, w, h, brush } => fill(&mut pixmap, (*x, *y, *w, *h), brush, dpi),
        }
    }
    Some(pixmap)
}

fn glyphs(pixmap: &mut Pixmap, run: &GlyphRun, fonts: &FontSystem, dpi: f32) {
    let Some(face) = fonts.face(run.face) else {
        return;
    };
    let skew = if run.italic { 0.2 } else { 0.0 };
    let base = px(run.baseline, dpi);
    let mut pb = PathBuilder::new();
    for &(glyph, x) in &run.glyphs {
        let ox = px(x, dpi);
        let outline = face.outline(glyph);
        // HFT families mix design ems across their files.
        let em = if outline.units_per_em > 0 {
            outline.units_per_em
        } else {
            face.units_per_em
        };
        let scale = px(run.size, dpi) / em as f32;
        let sx = scale * run.ratio as f32 / 100.0;
        // Font units, y up, to device pixels, y down; italic leans right.
        let map = |fx: f32, fy: f32| (ox + fx * sx + fy * scale * skew, base - fy * scale);
        for op in &outline.ops {
            match *op {
                PathOp::Move(x, y) => {
                    let (a, b) = map(x, y);
                    pb.move_to(a, b);
                }
                PathOp::Line(x, y) => {
                    let (a, b) = map(x, y);
                    pb.line_to(a, b);
                }
                PathOp::Quad(x1, y1, x, y) => {
                    let (a, b) = map(x1, y1);
                    let (c, d) = map(x, y);
                    pb.quad_to(a, b, c, d);
                }
                PathOp::Cubic(x1, y1, x2, y2, x, y) => {
                    let (a, b) = map(x1, y1);
                    let (c, d) = map(x2, y2);
                    let (e, f) = map(x, y);
                    pb.cubic_to(a, b, c, d, e, f);
                }
                PathOp::Close => pb.close(),
            }
        }
    }
    let Some(path) = pb.finish() else { return };
    let p = paint(run.color);
    pixmap.fill_path(&path, &p, FillRule::Winding, Transform::identity(), None);
    // Hancom draws synthetic bold as fill plus a stroke of 1/40 of the size.
    if run.fake_bold {
        let stroke = Stroke {
            width: px(run.size, dpi) / 40.0,
            ..Stroke::default()
        };
        pixmap.stroke_path(&path, &p, &stroke, Transform::identity(), None);
    }
}

/// Strokes a segment with butt caps (round for dotted strokes).
fn line(pixmap: &mut Pixmap, (x1, y1, x2, y2): (i32, i32, i32, i32), s: &LineStroke, dpi: f32) {
    let mut pb = PathBuilder::new();
    pb.move_to(px(x1, dpi), px(y1, dpi));
    pb.line_to(px(x2, dpi), px(y2, dpi));
    let Some(path) = pb.finish() else { return };
    // Hairlines stay visible at low resolutions.
    let width = px(s.width, dpi).max(0.5);
    let mut stroke = Stroke {
        width,
        ..Stroke::default()
    };
    if let Some(pattern) = s.dash.pattern() {
        let intervals: Vec<f32> = pattern.iter().map(|k| (k * width).max(0.01)).collect();
        stroke.dash = tiny_skia::StrokeDash::new(intervals, 0.0);
        if s.dash.round() {
            stroke.line_cap = tiny_skia::LineCap::Round;
        }
    }
    pixmap.stroke_path(&path, &paint(s.color), &stroke, Transform::identity(), None);
}

/// A clip mask of a device rectangle.
fn rect_mask(pixmap: &Pixmap, r: tiny_skia::Rect) -> Option<tiny_skia::Mask> {
    let mut mask = tiny_skia::Mask::new(pixmap.width(), pixmap.height())?;
    let path = PathBuilder::from_rect(r);
    mask.fill_path(&path, FillRule::Winding, true, Transform::identity());
    Some(mask)
}

fn fill(pixmap: &mut Pixmap, (x, y, w, h): (i32, i32, i32, i32), brush: &Brush, dpi: f32) {
    let Some(r) = tiny_skia::Rect::from_xywh(px(x, dpi), px(y, dpi), px(w, dpi), px(h, dpi)) else {
        return;
    };
    match brush {
        Brush::Solid(c) => pixmap.fill_rect(r, &paint(*c), Transform::identity(), None),
        Brush::Hatch { color, style } => hatch(pixmap, r, *color, *style, dpi),
        Brush::Gradient(g) => gradient(pixmap, r, g),
        // Pictures need the image decoder; nothing to draw without it.
        Brush::Image { .. } => {}
    }
}

/// Hatch lines about 1.6 mm apart, 0.25 mm wide.
fn hatch(pixmap: &mut Pixmap, r: tiny_skia::Rect, color: u32, style: u8, dpi: f32) {
    let Some(mask) = rect_mask(pixmap, r) else {
        return;
    };
    let gap = px(450, dpi).max(2.0);
    let stroke = Stroke {
        width: px(70, dpi).max(0.5),
        ..Stroke::default()
    };
    let (w, h) = (r.width(), r.height());
    let mut pb = PathBuilder::new();
    let mut lines = |dx: f32, dy: f32| {
        // Lines along (dx, dy), stepped across the box diagonal.
        let span = w + h;
        let mut t = -span;
        while t <= span {
            let (ox, oy) = if dx == 0.0 {
                (r.left() + t, r.top())
            } else if dy == 0.0 {
                (r.left(), r.top() + t)
            } else {
                (r.left() + t, r.top())
            };
            pb.move_to(ox - dx * span, oy - dy * span);
            pb.line_to(ox + dx * span, oy + dy * span);
            t += gap;
        }
    };
    match style {
        1 => lines(1.0, 0.0),
        2 => lines(0.0, 1.0),
        3 => lines(1.0, 1.0),
        4 => lines(-1.0, 1.0),
        5 => {
            lines(1.0, 0.0);
            lines(0.0, 1.0);
        }
        _ => {
            lines(1.0, 1.0);
            lines(-1.0, 1.0);
        }
    }
    if let Some(path) = pb.finish() {
        pixmap.stroke_path(
            &path,
            &paint(color),
            &stroke,
            Transform::identity(),
            Some(&mask),
        );
    }
}

/// The color at `t` (0 to 1) along a gradient's stops.
fn ramp(g: &GradientBrush, t: f32) -> u32 {
    let pos = t * 100.0;
    let stops = &g.stops;
    let Some(&(_, first)) = stops.first() else {
        return 0;
    };
    let mut prev = (stops[0].0 as f32, first);
    for &(p, c) in stops {
        let p = p as f32;
        if pos <= p {
            let span = (p - prev.0).max(1e-3);
            let k = ((pos - prev.0) / span).clamp(0.0, 1.0);
            return mix(prev.1, c, k);
        }
        prev = (p, c);
    }
    prev.1
}

fn mix(a: u32, b: u32, k: f32) -> u32 {
    let ch = |c: u32, s: u32| ((c >> s) & 0xFF) as f32;
    let one = |s: u32| ((ch(a, s) + (ch(b, s) - ch(a, s)) * k).round() as u32) << s;
    one(0) | one(8) | one(16)
}

/// Maps a position along the gradient (0 to 1) to the ramp, turning back at
/// the step center and quantizing to the step count.
fn level(g: &GradientBrush, t: f32) -> f32 {
    let c = g.step_center as f32 / 100.0;
    let v = if c <= 0.0 {
        1.0 - t
    } else if c >= 1.0 {
        t
    } else if t <= c {
        t / c
    } else {
        (1.0 - t) / (1.0 - c)
    };
    if g.step > 1 {
        let n = g.step as f32;
        ((v * n).floor().min(n - 1.0) / (n - 1.0)).clamp(0.0, 1.0)
    } else {
        v.clamp(0.0, 1.0)
    }
}

/// Paints a gradient as bands, the way Hancom draws it.
fn gradient(pixmap: &mut Pixmap, r: tiny_skia::Rect, g: &GradientBrush) {
    let Some(mask) = rect_mask(pixmap, r) else {
        return;
    };
    let bands = if g.step > 1 { g.step.min(256) * 2 } else { 128 } as usize;
    let (cx, cy) = (
        r.left() + r.width() * g.center_x.clamp(0, 100) as f32 / 100.0,
        r.top() + r.height() * g.center_y.clamp(0, 100) as f32 / 100.0,
    );
    match g.kind {
        // Radial and square: shapes shrinking toward the center.
        2 | 4 => {
            let reach = [
                (r.left(), r.top()),
                (r.right(), r.top()),
                (r.left(), r.bottom()),
                (r.right(), r.bottom()),
            ]
            .iter()
            .map(|&(x, y)| {
                if g.kind == 2 {
                    ((x - cx).powi(2) + (y - cy).powi(2)).sqrt()
                } else {
                    (x - cx).abs().max((y - cy).abs())
                }
            })
            .fold(0.0f32, f32::max);
            for k in 0..bands {
                let t = k as f32 / bands as f32;
                let rad = reach * (1.0 - t);
                let path = if g.kind == 2 {
                    PathBuilder::from_circle(cx, cy, rad.max(0.01))
                } else {
                    tiny_skia::Rect::from_xywh(cx - rad, cy - rad, rad * 2.0, rad * 2.0)
                        .map(PathBuilder::from_rect)
                };
                if let Some(path) = path {
                    let p = paint(ramp(g, level(g, t)));
                    pixmap.fill_path(
                        &path,
                        &p,
                        FillRule::Winding,
                        Transform::identity(),
                        Some(&mask),
                    );
                }
            }
        }
        // Linear (and conical, approximated): bands across the direction.
        _ => {
            let a = (g.angle as f32).to_radians();
            let (dx, dy) = (a.sin(), a.cos());
            let (mx, my) = (r.left() + r.width() / 2.0, r.top() + r.height() / 2.0);
            let half = (r.width() * dx.abs() + r.height() * dy.abs()) / 2.0;
            let long = r.width() + r.height();
            let step = 2.0 * half / bands as f32;
            for k in 0..bands {
                let t0 = -half + step * k as f32;
                let (sx, sy) = (mx + dx * t0, my + dy * t0);
                let (ex, ey) = (sx + dx * (step + 0.5), sy + dy * (step + 0.5));
                let mut pb = PathBuilder::new();
                pb.move_to(sx - dy * long, sy + dx * long);
                pb.line_to(sx + dy * long, sy - dx * long);
                pb.line_to(ex + dy * long, ey - dx * long);
                pb.line_to(ex - dy * long, ey + dx * long);
                pb.close();
                if let Some(path) = pb.finish() {
                    let t = (k as f32 + 0.5) / bands as f32;
                    let p = paint(ramp(g, level(g, t)));
                    pixmap.fill_path(
                        &path,
                        &p,
                        FillRule::Winding,
                        Transform::identity(),
                        Some(&mask),
                    );
                }
            }
        }
    }
}

/// Renders a display list to PNG bytes.
pub fn render_png(dl: &DisplayList, fonts: &FontSystem, dpi: f32) -> Option<Vec<u8>> {
    render(dl, fonts, dpi)?.encode_png().ok()
}
