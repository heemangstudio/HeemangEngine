//! tiny-skia backend.

mod cache;

use hm_fonts::{FontSystem, PathOp};
use tiny_skia::{FillRule, Mask, Paint, PathBuilder, Pixmap, Stroke, Transform};

use crate::draw::{ImageItem, PathEl, PathItem};
use crate::image::{self, Effects};
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
    let mut pixmap = blank(dl, dpi)?;
    pixmap.fill(tiny_skia::Color::WHITE);
    Some(draw_on(pixmap, dl, fonts, dpi))
}

/// Draws a display list on a transparent pixmap.
pub fn render_clear(dl: &DisplayList, fonts: &FontSystem, dpi: f32) -> Option<Pixmap> {
    Some(draw_on(blank(dl, dpi)?, dl, fonts, dpi))
}

fn blank(dl: &DisplayList, dpi: f32) -> Option<Pixmap> {
    let w = px(dl.width, dpi).round().max(1.0) as u32;
    let h = px(dl.height, dpi).round().max(1.0) as u32;
    Pixmap::new(w, h)
}

fn draw_on(mut pixmap: Pixmap, dl: &DisplayList, fonts: &FontSystem, dpi: f32) -> Pixmap {
    let mut clips = Clips::default();
    for item in &dl.items {
        match item {
            Item::Clip { x, y, w, h } => {
                clips.push([px(*x, dpi), px(*y, dpi), px(x + w, dpi), px(y + h, dpi)]);
                continue;
            }
            Item::Unclip => {
                clips.pop();
                continue;
            }
            _ => {}
        }
        let clip = clips.mask(pixmap.width(), pixmap.height());
        match item {
            Item::Glyphs(run) => glyphs(&mut pixmap, run, fonts, dpi, clip),
            Item::Rect { x, y, w, h, color } => {
                if let Some(r) = tiny_skia::Rect::from_xywh(
                    px(*x, dpi),
                    px(*y, dpi),
                    px(*w, dpi).max(0.5),
                    px(*h, dpi).max(0.5),
                ) {
                    pixmap.fill_rect(r, &paint(*color), Transform::identity(), clip);
                }
            }
            Item::Line {
                x1,
                y1,
                x2,
                y2,
                stroke,
            } => line(&mut pixmap, (*x1, *y1, *x2, *y2), stroke, dpi, clip),
            Item::Fill { x, y, w, h, brush } => {
                fill(&mut pixmap, (*x, *y, *w, *h), brush, dpi, clip)
            }
            Item::Path(p) => path_item(&mut pixmap, p, dpi, clip),
            Item::Image(i) => image_item(&mut pixmap, i, fonts, dpi, clip),
            Item::Outline { run, width, dash } => {
                outline_glyphs(&mut pixmap, run, (*width, dash), fonts, dpi, clip)
            }
            Item::Clip { .. } | Item::Unclip => {}
        }
    }
    pixmap
}

/// The clip rectangles in force, each the intersection of those before it,
/// and a mask of the innermost one, built when an item first needs it.
#[derive(Default)]
struct Clips {
    /// Device `[left, top, right, bottom]`.
    stack: Vec<[f32; 4]>,
    mask: Option<Mask>,
    /// The pixel rows and columns the mask covers now.
    written: Option<[u32; 4]>,
    current: bool,
}

impl Clips {
    fn push(&mut self, r: [f32; 4]) {
        let r = match self.stack.last() {
            Some(o) => [
                r[0].max(o[0]),
                r[1].max(o[1]),
                r[2].min(o[2]),
                r[3].min(o[3]),
            ],
            None => r,
        };
        self.stack.push(r);
        self.current = false;
    }

    fn pop(&mut self) {
        self.stack.pop();
        self.current = false;
    }

    /// The mask of the innermost clip, `None` when nothing clips.
    fn mask(&mut self, w: u32, h: u32) -> Option<&Mask> {
        let r = *self.stack.last()?;
        if !self.current {
            if self.mask.is_none() {
                self.mask = Mask::new(w, h);
            }
            let mask = self.mask.as_mut()?;
            let data = mask.data_mut();
            if let Some([x0, y0, x1, y1]) = self.written.take() {
                for y in y0..y1 {
                    data[(y * w + x0) as usize..(y * w + x1) as usize].fill(0);
                }
            }
            // Partly covered pixels at the edges keep their share.
            let (l, t) = (r[0].clamp(0.0, w as f32), r[1].clamp(0.0, h as f32));
            let (rr, b) = (r[2].clamp(l, w as f32), r[3].clamp(t, h as f32));
            let (x0, y0) = (l.floor() as u32, t.floor() as u32);
            let (x1, y1) = (rr.ceil() as u32, b.ceil() as u32);
            for y in y0..y1 {
                let cy = (b.min(y as f32 + 1.0) - t.max(y as f32)).max(0.0);
                for x in x0..x1 {
                    let cx = (rr.min(x as f32 + 1.0) - l.max(x as f32)).max(0.0);
                    data[(y * w + x) as usize] = (cx * cy * 255.0).round() as u8;
                }
            }
            self.written = Some([x0, y0, x1, y1]);
            self.current = true;
        }
        self.mask.as_ref()
    }
}

/// Narrows `m` to `clip`.
fn and_mask(m: &mut Mask, clip: Option<&Mask>) {
    if let Some(c) = clip {
        for (a, b) in m.data_mut().iter_mut().zip(c.data()) {
            *a = ((*a as u16 * *b as u16) / 255) as u8;
        }
    }
}

fn glyphs(pixmap: &mut Pixmap, run: &GlyphRun, fonts: &FontSystem, dpi: f32, clip: Option<&Mask>) {
    let Some(face) = fonts.face(run.face) else {
        return;
    };
    let Some(path) = glyph_path(run, &face, dpi) else {
        return;
    };
    let p = paint(run.color);
    pixmap.fill_path(&path, &p, FillRule::Winding, Transform::identity(), clip);
    // Hancom draws synthetic bold as fill plus a stroke: 1/40 of the size
    // for outline fonts, one font unit (1/1000 of the size) for HFT glyphs.
    if run.fake_bold {
        let share = if face.is_hft() { 1000.0 } else { 40.0 };
        let stroke = Stroke {
            width: px(run.size, dpi) / share,
            ..Stroke::default()
        };
        pixmap.stroke_path(&path, &p, &stroke, Transform::identity(), clip);
    }
}

/// Strokes a run's glyph outlines with round joins and caps.
fn outline_glyphs(
    pixmap: &mut Pixmap,
    run: &GlyphRun,
    (width, dash): (i32, &[i32]),
    fonts: &FontSystem,
    dpi: f32,
    clip: Option<&Mask>,
) {
    let Some(face) = fonts.face(run.face) else {
        return;
    };
    let Some(path) = glyph_path(run, &face, dpi) else {
        return;
    };
    let mut stroke = Stroke {
        width: px(width, dpi).max(0.5),
        line_cap: tiny_skia::LineCap::Round,
        line_join: tiny_skia::LineJoin::Round,
        ..Stroke::default()
    };
    if !dash.is_empty() {
        stroke.dash =
            tiny_skia::StrokeDash::new(dash.iter().map(|&d| px(d, dpi).max(0.01)).collect(), 0.0);
    }
    pixmap.stroke_path(
        &path,
        &paint(run.color),
        &stroke,
        Transform::identity(),
        clip,
    );
}

/// The device path of a run's glyphs.
fn glyph_path(run: &GlyphRun, face: &hm_fonts::Face, dpi: f32) -> Option<tiny_skia::Path> {
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
    pb.finish()
}

/// Strokes a segment with butt caps (round for dotted strokes).
fn line(
    pixmap: &mut Pixmap,
    (x1, y1, x2, y2): (i32, i32, i32, i32),
    s: &LineStroke,
    dpi: f32,
    clip: Option<&Mask>,
) {
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
    pixmap.stroke_path(&path, &paint(s.color), &stroke, Transform::identity(), clip);
}

/// A clip mask of a device rectangle.
fn rect_mask(pixmap: &Pixmap, r: tiny_skia::Rect) -> Option<tiny_skia::Mask> {
    let mut mask = tiny_skia::Mask::new(pixmap.width(), pixmap.height())?;
    let path = PathBuilder::from_rect(r);
    mask.fill_path(&path, FillRule::Winding, true, Transform::identity());
    Some(mask)
}

fn fill(
    pixmap: &mut Pixmap,
    (x, y, w, h): (i32, i32, i32, i32),
    brush: &Brush,
    dpi: f32,
    clip: Option<&Mask>,
) {
    let Some(r) = tiny_skia::Rect::from_xywh(px(x, dpi), px(y, dpi), px(w, dpi), px(h, dpi)) else {
        return;
    };
    fill_in(pixmap, r, brush, clip, dpi);
}

/// Paints a brush over `r`, limited to `clip` when given.
fn fill_in(
    pixmap: &mut Pixmap,
    r: tiny_skia::Rect,
    brush: &Brush,
    clip: Option<&tiny_skia::Mask>,
    dpi: f32,
) {
    match brush {
        Brush::Solid(c) => pixmap.fill_rect(r, &paint(*c), Transform::identity(), clip),
        Brush::Hatch { color, style } => hatch(pixmap, r, *color, *style, clip, dpi),
        Brush::Gradient(g) => gradient(pixmap, r, g, clip),
        Brush::Image {
            data: Some(data),
            mode,
            effect,
            brightness,
            contrast,
            ..
        } => {
            let fx = Effects {
                effect: *effect,
                brightness: *brightness,
                contrast: *contrast,
                ..Effects::default()
            };
            image_fill(pixmap, r, data, *mode, fx, dpi, clip);
        }
        Brush::Image { data: None, .. } => {}
    }
}

/// The intersection of a rectangle mask with an optional clip.
fn clip_mask(
    pixmap: &Pixmap,
    r: tiny_skia::Rect,
    clip: Option<&tiny_skia::Mask>,
) -> Option<tiny_skia::Mask> {
    let mut m = rect_mask(pixmap, r)?;
    and_mask(&mut m, clip);
    Some(m)
}

/// Hatch lines about 1.6 mm apart, 0.25 mm wide.
fn hatch(
    pixmap: &mut Pixmap,
    r: tiny_skia::Rect,
    color: u32,
    style: u8,
    clip: Option<&tiny_skia::Mask>,
    dpi: f32,
) {
    let Some(mask) = clip_mask(pixmap, r, clip) else {
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

/// Maps a position along the gradient (0 to 1) to the ramp, the step
/// center taking the ramp's midpoint (Hancom draws a plain ramp at its
/// default 50), and quantizes it to the step count.
fn level(g: &GradientBrush, t: f32) -> f32 {
    let c = (g.step_center as f32 / 100.0).clamp(0.01, 0.99);
    let v = if t <= c {
        0.5 * t / c
    } else {
        0.5 + 0.5 * (t - c) / (1.0 - c)
    };
    if g.step > 1 {
        let n = g.step as f32;
        ((v * n).floor().min(n - 1.0) / (n - 1.0)).clamp(0.0, 1.0)
    } else {
        v.clamp(0.0, 1.0)
    }
}

/// Paints a gradient as bands, the way Hancom draws it.
fn gradient(
    pixmap: &mut Pixmap,
    r: tiny_skia::Rect,
    g: &GradientBrush,
    clip: Option<&tiny_skia::Mask>,
) {
    let Some(mask) = clip_mask(pixmap, r, clip) else {
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
                    // The first color sits at the center.
                    let p = paint(ramp(g, level(g, 1.0 - t)));
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

fn build_path(p: &crate::draw::Path, dpi: f32) -> Option<tiny_skia::Path> {
    let mut pb = PathBuilder::new();
    for e in &p.els {
        match *e {
            PathEl::Move(x, y) => pb.move_to(px(x, dpi), px(y, dpi)),
            PathEl::Line(x, y) => pb.line_to(px(x, dpi), px(y, dpi)),
            PathEl::Cubic(a, b, c, d, x, y) => pb.cubic_to(
                px(a, dpi),
                px(b, dpi),
                px(c, dpi),
                px(d, dpi),
                px(x, dpi),
                px(y, dpi),
            ),
            PathEl::Close => pb.close(),
        }
    }
    pb.finish()
}

/// Coverage subsamples per pixel across and down. A PDF rasterizer at
/// eight bits of antialiasing counts the points of a 17 by 15 grid inside
/// the shape, so a pixel's coverage is that count out of 255.
const SAMPLES_X: f32 = 17.0;
const SAMPLES_Y: f32 = 15.0;

/// The solid outline of an axis-aligned rectangle at Hancom's PDF
/// coordinates, painted with the sampled coverage of a PDF rasterizer: an edge at device position `v`
/// starts or ends at subsample `floor(v * n)`, and each pixel takes the
/// count of covered subsample points. The four sides are disjoint bands
/// with mitred corners. Returns false for any other outline, and for a
/// stroke under a fifth of a pixel, which such rasterizers widen.
fn sampled_rect_outline(
    pixmap: &mut Pixmap,
    path: &crate::draw::Path,
    outline: &crate::draw::Outline,
    dpi: f32,
    clip: Option<&Mask>,
) -> bool {
    let els = match path.els.as_slice() {
        [els @ .., PathEl::Line(x, y), PathEl::Close]
            if els.len() == 4 && els.first() == Some(&PathEl::Move(*x, *y)) =>
        {
            els
        }
        [els @ .., PathEl::Close] => els,
        _ => return false,
    };
    let [
        PathEl::Move(a, b),
        PathEl::Line(c, d),
        PathEl::Line(e, f),
        PathEl::Line(g, h),
    ] = els
    else {
        return false;
    };
    let axis_aligned =
        (a == g && c == e && b == d && f == h) || (a == c && e == g && b == h && d == f);
    // Hancom writes drawing paths in whole 1/600 inch units, scaled by
    // 0.99996 about the page's top left corner, as it places pictures.
    let unit = dpi / 600.0;
    let grid = |v: i32| (px(v, dpi) / unit).round() * unit * 0.99996;
    let width = px(outline.width, dpi) * 0.99996;
    if outline.dash != crate::paint::Dash::Solid || !axis_aligned || width < 0.2 {
        return false;
    }
    let half = width / 2.0;
    let (l, t, r, bt) = (
        grid(*a.min(e)),
        grid(*b.min(f)),
        grid(*a.max(e)),
        grid(*b.max(f)),
    );
    if l >= r || t >= bt {
        return false;
    }
    let (pw, ph) = (pixmap.width() as i64, pixmap.height() as i64);
    let sub = |v: f32, n: f32| (f64::from(v) * f64::from(n)).floor() as i64;
    // Bands in subsample units: [x0, x1) x [y0, y1).
    let band = |x0: f32, y0: f32, x1: f32, y1: f32| {
        (
            sub(x0, SAMPLES_X),
            sub(y0, SAMPLES_Y),
            sub(x1, SAMPLES_X),
            sub(y1, SAMPLES_Y),
        )
    };
    let (ol, ot, or, ob) = (l - half, t - half, r + half, bt + half);
    let bands = if t + half >= bt - half || l + half >= r - half {
        vec![band(ol, ot, or, ob)]
    } else {
        vec![
            band(ol, ot, or, t + half),
            band(ol, bt - half, or, ob),
            band(ol, t + half, l + half, bt - half),
            band(r - half, t + half, or, bt - half),
        ]
    };
    let (sx, sy) = (SAMPLES_X as i64, SAMPLES_Y as i64);
    let x0 = (bands.iter().map(|b| b.0).min().unwrap_or(0).div_euclid(sx)).clamp(0, pw);
    let y0 = (bands.iter().map(|b| b.1).min().unwrap_or(0).div_euclid(sy)).clamp(0, ph);
    let x1 = ((bands.iter().map(|b| b.2).max().unwrap_or(0) + sx - 1).div_euclid(sx)).clamp(0, pw);
    let y1 = ((bands.iter().map(|b| b.3).max().unwrap_or(0) + sy - 1).div_euclid(sy)).clamp(0, ph);
    if x0 >= x1 || y0 >= y1 {
        return true;
    }
    let Some(mut layer) = Pixmap::new((x1 - x0) as u32, (y1 - y0) as u32) else {
        return false;
    };
    let lw = (x1 - x0) as usize;
    let mut cover = vec![0u32; lw * (y1 - y0) as usize];
    let overlap =
        |lo: i64, hi: i64, cell: i64, n: i64| (hi.min((cell + 1) * n) - lo.max(cell * n)).max(0);
    for &(bx0, by0, bx1, by1) in &bands {
        for y in by0.div_euclid(sy).max(y0)..((by1 + sy - 1).div_euclid(sy)).min(y1) {
            let ky = overlap(by0, by1, y, sy);
            if ky == 0 {
                continue;
            }
            for x in bx0.div_euclid(sx).max(x0)..((bx1 + sx - 1).div_euclid(sx)).min(x1) {
                let kx = overlap(bx0, bx1, x, sx);
                cover[(y - y0) as usize * lw + (x - x0) as usize] += (kx * ky) as u32;
            }
        }
    }
    let (cr, cg, cb) = (
        outline.color & 0xFF,
        (outline.color >> 8) & 0xFF,
        (outline.color >> 16) & 0xFF,
    );
    for (dst, &k) in layer.pixels_mut().iter_mut().zip(&cover) {
        let a = k.min(255);
        let pre = |c: u32| ((c * a + 127) / 255) as u8;
        if let Some(c) =
            tiny_skia::PremultipliedColorU8::from_rgba(pre(cr), pre(cg), pre(cb), a as u8)
        {
            *dst = c;
        }
    }
    pixmap.draw_pixmap(
        x0 as i32,
        y0 as i32,
        layer.as_ref(),
        &tiny_skia::PixmapPaint::default(),
        Transform::identity(),
        clip,
    );
    true
}

/// A drawing object: brushes inside the path, then its outline.
fn path_item(pixmap: &mut Pixmap, it: &PathItem, dpi: f32, clip: Option<&Mask>) {
    let Some(path) = build_path(&it.path, dpi) else {
        return;
    };
    if !it.fill.is_empty() {
        let Some(mut mask) = tiny_skia::Mask::new(pixmap.width(), pixmap.height()) else {
            return;
        };
        mask.fill_path(&path, FillRule::EvenOdd, true, Transform::identity());
        if it.fill_alpha > 0 {
            let keep = 255 - it.fill_alpha as u16;
            mask.data_mut()
                .iter_mut()
                .for_each(|v| *v = ((*v as u16 * keep) / 255) as u8);
        }
        and_mask(&mut mask, clip);
        let b = path.bounds();
        for brush in &it.fill {
            fill_in(pixmap, b, brush, Some(&mask), dpi);
        }
    }
    if let Some(o) = &it.outline {
        if sampled_rect_outline(pixmap, &it.path, o, dpi, clip) {
            return;
        }
        let width = px(o.width, dpi).max(0.5);
        let mut stroke = Stroke {
            width,
            line_cap: if o.round_cap {
                tiny_skia::LineCap::Round
            } else {
                tiny_skia::LineCap::Butt
            },
            line_join: tiny_skia::LineJoin::Miter,
            ..Stroke::default()
        };
        if let Some(pattern) = o.dash.pattern() {
            let intervals: Vec<f32> = pattern.iter().map(|k| (k * width).max(0.01)).collect();
            stroke.dash = tiny_skia::StrokeDash::new(intervals, 0.0);
            if o.dash.round() {
                stroke.line_cap = tiny_skia::LineCap::Round;
            }
        }
        pixmap.stroke_path(&path, &paint(o.color), &stroke, Transform::identity(), clip);
    }
}

/// A decoded picture with its effects applied.
fn picture(data: &hm_model::SharedBytes, fx: Effects) -> Option<std::sync::Arc<Pixmap>> {
    let key = cache::Key::data(data);
    cache::get_or(cache::Key::Decoded(key, fx), || decoded(data, fx))
}

fn decoded(data: &[u8], fx: Effects) -> Option<Pixmap> {
    // Pixels plus the decoder's own planes, roughly as large again.
    let bytes = image::dimensions(data).map_or(0, |(w, h)| w as usize * h as usize * 8);
    cache::decoding(bytes, || decode_effects(data, fx))
}

fn decode_effects(data: &[u8], fx: Effects) -> Option<Pixmap> {
    image::decode(data).and_then(|mut r| {
        fx.apply(&mut r);
        r.into_pixmap()
    })
}

/// The pixel size of a decoded picture, decoding it only when it was
/// never decoded before.
fn picture_size(data: &hm_model::SharedBytes, fx: Effects) -> Option<(u32, u32)> {
    let key = cache::Key::data(data);
    cache::dims(key, fx)
        .or_else(|| image::dimensions(data))
        .or_else(|| picture(data, fx).map(|p| (p.width(), p.height())))
}

/// A metafile played at `w` by `h` pixels with its effects.
fn metafile(
    data: &hm_model::SharedBytes,
    fx: Effects,
    (w, h): (u32, u32),
    fonts: &FontSystem,
) -> Option<std::sync::Arc<Pixmap>> {
    let key = cache::Key::data(data);
    cache::get_or(cache::Key::Played(key, fx, w, h), || {
        image::wmf::Wmf::parse(data)
            .and_then(|m| m.render(w, h, fonts))
            .and_then(|pm| {
                if fx.is_none() {
                    return Some(pm);
                }
                let mut r = image::Raster::new(pm.width(), pm.height());
                for (d, s) in r.rgba.chunks_exact_mut(4).zip(pm.pixels()) {
                    let c = s.demultiply();
                    d.copy_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
                }
                fx.apply(&mut r);
                r.into_pixmap()
            })
    })
}

/// A picture sampled at `w` by `h` pixels. Decode the full picture only
/// when this size was not made before.
fn shrunk(
    data: &hm_model::SharedBytes,
    fx: Effects,
    (w, h): (u32, u32),
    cubic: bool,
) -> Option<std::sync::Arc<Pixmap>> {
    let key = cache::Key::data(data);
    let shrunk_key = cache::Key::Shrunk(key, fx, w, h, cubic);
    let sample = |src: &Pixmap| {
        if cubic {
            resample(src, w, h)
        } else {
            resample_tent(src, w, h)
        }
    };
    if let Some(hit) = cache::peek(shrunk_key) {
        return hit;
    }
    // The full picture is only kept when it is drawn at its own size too:
    // a large photograph shown small would otherwise hold its decoded
    // pixels in the cache for nothing.
    if let Some(hit) = cache::peek(cache::Key::Decoded(key, fx)) {
        let src = hit?;
        let Some(small) = sample(&src) else {
            return Some(src);
        };
        return cache::get_or(shrunk_key, || Some(small));
    }
    // Decoded and reduced under one reservation: the full pixels live
    // only until the reduction is made.
    let bytes = image::dimensions(data).map_or(0, |(pw, ph)| pw as usize * ph as usize * 8);
    let small = cache::decoding(bytes, || {
        let src = decode_effects(data, fx)?;
        Some(sample(&src).unwrap_or(src))
    });
    cache::get_or(shrunk_key, || small)
}

/// Device-size sampling of premultiplied pixels. Reduction uses cubic
/// smoothstep; an enlarged axis selects linear interpolation on both axes.
fn resample(src: &Pixmap, w: u32, h: u32) -> Option<Pixmap> {
    if w > src.width() || h > src.height() {
        return resample_linear(src, w, h);
    }
    let (sw, sh) = (src.width() as usize, src.height() as usize);
    let (w, h) = (w.max(1) as usize, h.max(1) as usize);
    // Stronger reductions also involve decoder subsampling in the PDF
    // path. Keep their existing filter until that stage is reproduced.
    if sw > w * 2 || sh > h * 2 {
        return resample_tent(src, w as u32, h as u32);
    }
    let taps = |n_src: usize, n_dst: usize| {
        let scale = n_src as f32 / n_dst as f32;
        let support = scale.max(1.0);
        (0..n_dst)
            .map(|i| {
                let center = (i as f32 + 0.5) * scale;
                let lo = (center - support).floor().max(0.0) as usize;
                let hi = ((center + support).ceil() as usize).min(n_src);
                let mut v: Vec<(usize, i32)> = (lo..hi)
                    .filter_map(|k| {
                        let d = ((k as f32 + 0.5) - center).abs() / support;
                        let weight = 1.0 - 3.0 * d * d + 2.0 * d * d * d;
                        let weight = (256.0 * weight / support).round() as i32;
                        (d < 1.0 && weight > 0).then_some((k, weight))
                    })
                    .collect();
                let sum: i32 = v.iter().map(|t| t.1).sum();
                if let Some((k, _)) = v
                    .iter()
                    .enumerate()
                    .max_by_key(|(k, t)| (t.1, std::cmp::Reverse(*k)))
                {
                    v[k].1 += 256 - sum;
                }
                v
            })
            .collect::<Vec<_>>()
    };
    let (tx, ty) = (taps(sw, w), taps(sh, h));
    let mut out = Pixmap::new(w as u32, h as u32)?;
    let data = out.data_mut();
    // Keep only the horizontally sampled rows needed by one output row.
    let mut rows = std::collections::VecDeque::<(usize, Vec<u8>)>::new();
    for (y, vertical) in ty.iter().enumerate() {
        let Some(&(first, _)) = vertical.first() else {
            continue;
        };
        while rows.front().is_some_and(|r| r.0 < first) {
            rows.pop_front();
        }
        for &(sy, _) in vertical {
            if rows.back().is_some_and(|r| r.0 >= sy) {
                continue;
            }
            let line = &src.data()[sy * sw * 4..(sy + 1) * sw * 4];
            let mut row = vec![0; w * 4];
            for (x, horizontal) in tx.iter().enumerate() {
                for c in 0..4 {
                    let sum: i32 = horizontal
                        .iter()
                        .map(|&(sx, weight)| i32::from(line[sx * 4 + c]) * weight)
                        .sum();
                    row[x * 4 + c] = ((sum + 128) >> 8).clamp(0, 255) as u8;
                }
            }
            rows.push_back((sy, row));
        }
        for x in 0..w * 4 {
            let sum: i32 = vertical
                .iter()
                .zip(&rows)
                .map(|(&(_, weight), (_, row))| i32::from(row[x]) * weight)
                .sum();
            data[y * w * 4 + x] = ((sum + 128) >> 8).clamp(0, 255) as u8;
        }
    }
    Some(out)
}

/// Hancom PDF enlargement probes use a truncated 14-bit source step and
/// truncate channel values after each axis, including mixed-axis scaling.
fn resample_linear(src: &Pixmap, w: u32, h: u32) -> Option<Pixmap> {
    // An oversized frame can extend far beyond the page. Keep shader
    // sampling when its device pixels would exceed the cache's entry limit.
    if u64::from(w) * u64::from(h) > (cache::BUDGET / 8) as u64 {
        return None;
    }
    let mut out = Pixmap::new(w, h)?;
    let axis = |source: u32, target: u32| {
        let step = (i64::from(source) << 14) / i64::from(target);
        (0..target)
            .map(|i| {
                let pos = i64::from(i) * step + step / 2 - (1 << 13);
                let first = pos >> 14;
                (
                    first.clamp(0, i64::from(source) - 1) as usize,
                    (first + 1).clamp(0, i64::from(source) - 1) as usize,
                    (pos & 16383) as i32,
                )
            })
            .collect::<Vec<_>>()
    };
    let tx = axis(src.width(), w);
    let ty = axis(src.height(), h);
    let width = w as usize * 4;
    let stride = src.width() as usize * 4;
    let lerp = |a: u8, b: u8, weight: i32| {
        (i32::from(a) + (((i32::from(b) - i32::from(a)) * weight) >> 14)) as u8
    };
    // Only the two source rows used by this output row need storage.
    let mut rows = [(usize::MAX, vec![0; width]), (usize::MAX, vec![0; width])];
    for (y, &(top, bottom, weight)) in ty.iter().enumerate() {
        for sy in [top, bottom] {
            if rows.iter().any(|r| r.0 == sy) {
                continue;
            }
            let slot = usize::from(rows[0].0 == top || rows[0].0 == bottom);
            let row = &mut rows[slot];
            row.0 = sy;
            let line = &src.data()[sy * stride..(sy + 1) * stride];
            for (x, &(left, right, weight)) in tx.iter().enumerate() {
                for c in 0..4 {
                    row.1[x * 4 + c] = lerp(line[left * 4 + c], line[right * 4 + c], weight);
                }
            }
        }
        let a = &rows[usize::from(rows[0].0 != top)].1;
        let b = &rows[usize::from(rows[0].0 != bottom)].1;
        for (x, d) in out.data_mut()[y * width..(y + 1) * width]
            .iter_mut()
            .enumerate()
        {
            *d = lerp(a[x], b[x], weight);
        }
    }
    Some(out)
}

/// Separable tent-filter reduction of premultiplied pixels.
fn resample_tent(src: &Pixmap, w: u32, h: u32) -> Option<Pixmap> {
    let (sw, sh) = (src.width() as usize, src.height() as usize);
    let (w, h) = (w.max(1) as usize, h.max(1) as usize);
    // Weights of the source samples for each destination sample.
    let taps = |n_src: usize, n_dst: usize| -> Vec<Vec<(usize, f32)>> {
        let scale = n_src as f32 / n_dst as f32;
        let support = scale.max(1.0);
        (0..n_dst)
            .map(|i| {
                let center = (i as f32 + 0.5) * scale;
                let lo = (center - support).floor().max(0.0) as usize;
                let hi = ((center + support).ceil() as usize).min(n_src);
                let mut v: Vec<(usize, f32)> = (lo..hi)
                    .map(|k| {
                        let d = ((k as f32 + 0.5) - center).abs() / support;
                        (k, (1.0 - d).max(0.0))
                    })
                    .filter(|t| t.1 > 0.0)
                    .collect();
                let sum: f32 = v.iter().map(|t| t.1).sum();
                if sum > 0.0 {
                    v.iter_mut().for_each(|t| t.1 /= sum);
                }
                v
            })
            .collect()
    };
    let (tx, ty) = (taps(sw, w), taps(sh, h));
    let px = src.data();
    // One destination row at a time: the source rows it covers are summed
    // into `row`, then reduced across, so no reduced copy of the whole
    // picture is ever held.
    let mut row = vec![0f32; sw * 4];
    let mut out = Pixmap::new(w as u32, h as u32)?;
    let data = out.data_mut();
    for (y, t) in ty.iter().enumerate() {
        row.fill(0.0);
        for &(k, wt) in t {
            let line = &px[k * sw * 4..(k + 1) * sw * 4];
            for (d, &s) in row.iter_mut().zip(line) {
                *d += s as f32 * wt;
            }
        }
        for (x, t) in tx.iter().enumerate() {
            let mut acc = [0f32; 4];
            for &(k, wt) in t {
                for c in 0..4 {
                    acc[c] += row[k * 4 + c] * wt;
                }
            }
            let a = acc[3].round().clamp(0.0, 255.0);
            for c in 0..3 {
                data[(y * w + x) * 4 + c] = acc[c].round().clamp(0.0, a) as u8;
            }
            data[(y * w + x) * 4 + 3] = a as u8;
        }
    }
    Some(out)
}

/// A picture mapped onto its frame, cropped.
/// The visible part of a raster picture of `pw` by `ph` pixels as
/// fractions of the whole: left, top, right, bottom.
pub fn crop_fractions(it: &ImageItem, (pw, ph): (u32, u32)) -> [f32; 4] {
    let fractions = |iw: f32, ih: f32| {
        if it.crop == [0; 4] || iw <= 0.0 || ih <= 0.0 {
            [0.0, 0.0, 1.0, 1.0]
        } else {
            [
                it.crop[0] as f32 / iw,
                it.crop[1] as f32 / ih,
                it.crop[2] as f32 / iw,
                it.crop[3] as f32 / ih,
            ]
        }
    };
    // Crops count HU of the picture's original size, or of its pixels at
    // the file's resolution when the original size is missing. A crop far
    // off that size whose far corner has the picture's proportions (to a
    // pixel) spans the whole picture: files that saved a picture resampled
    // after the crop was set keep the old extent.
    let (dx, dy) = image::resolution(&it.data);
    let (iw, ih) = (pw as f32 * 7200.0 / dx, ph as f32 * 7200.0 / dy);
    let (cr, cb) = (it.crop[2] as f32, it.crop[3] as f32);
    let whole = cr > 0.0
        && cb > 0.0
        && ((cr - iw).abs() > 2.0 * 7200.0 / dx || (cb - ih).abs() > 2.0 * 7200.0 / dy)
        && ((cr * ph as f32 / cb - pw as f32).abs() <= 1.0
            || (cb * pw as f32 / cr - ph as f32).abs() <= 1.0);
    let inferred_whole = it.natural == (0, 0)
        && it.crop[0] == 0
        && it.crop[1] == 0
        && cr > 0.0
        && cb > 0.0
        && (cr / iw * pw as f32).round() == pw as f32
        && (cb / ih * ph as f32).round() == ph as f32;
    if it.natural.0 > 0 && it.natural.1 > 0 {
        fractions(it.natural.0 as f32, it.natural.1 as f32)
    } else if whole || inferred_whole {
        fractions(cr, cb)
    } else {
        fractions(iw, ih)
    }
}

fn image_item(
    pixmap: &mut Pixmap,
    it: &ImageItem,
    fonts: &FontSystem,
    dpi: f32,
    clip: Option<&Mask>,
) {
    let raw = it.frame.map(|v| px(v, dpi));
    // Hancom places upright pictures on its drawing grid of 1/600 inch: the
    // frame and the whole picture behind it each start on the nearest grid
    // line and span a whole number of grid units. Its PDF output then
    // scales them by 0.99996 about the page's top left corner, which moves
    // an edge on a pixel boundary just inside the pixel before it.
    let upright = raw[1].abs() < 1e-3 && raw[3].abs() < 1e-3 && raw[0] > 0.0 && raw[4] > 0.0;
    let unit = dpi / 600.0;
    let grid = |v: f32| (v / unit).round() * unit * 0.99996;
    let f = if upright {
        [
            grid(raw[0]),
            0.0,
            grid(raw[2]),
            0.0,
            grid(raw[4]),
            grid(raw[5]),
        ]
    } else {
        raw
    };
    let is_metafile = image::sniff(&it.data) == image::Format::Wmf;
    // The crop as fractions of the whole picture, `iw` by `ih` HU.
    let fractions = |iw: f32, ih: f32| {
        if it.crop == [0; 4] || iw <= 0.0 || ih <= 0.0 {
            [0.0, 0.0, 1.0, 1.0]
        } else {
            [
                it.crop[0] as f32 / iw,
                it.crop[1] as f32 / ih,
                it.crop[2] as f32 / iw,
                it.crop[3] as f32 / ih,
            ]
        }
    };
    let (pm, [cl, ct, cr, cb], sampled) = if is_metafile {
        let fr = fractions(it.natural.0 as f32, it.natural.1 as f32);
        // Play the whole picture at the size its visible part is drawn.
        let (cw, ch) = ((fr[2] - fr[0]).max(1e-3), (fr[3] - fr[1]).max(1e-3));
        let w = (f[0].hypot(f[3]) / cw).round().clamp(1.0, 6000.0) as u32;
        let h = (f[1].hypot(f[4]) / ch).round().clamp(1.0, 6000.0) as u32;
        let Some(pm) = metafile(&it.data, it.effects, (w, h), fonts) else {
            return;
        };
        (pm, fr, false)
    } else {
        let Some((pw, ph)) = picture_size(&it.data, it.effects) else {
            return;
        };
        let fr = crop_fractions(it, (pw, ph));
        // Fit the picture to its device pixels before placing it.
        let (cw, ch) = ((fr[2] - fr[0]).max(1e-3), (fr[3] - fr[1]).max(1e-3));
        // An upright picture is reduced straight to the device pixels it
        // covers, as PDF rasterizers do, and then placed pixel for pixel.
        let device = (upright && fr == [0.0, 0.0, 1.0, 1.0])
            .then(|| device_box(raw, (fr[0], fr[1]), (cw, ch), grid))
            .filter(|&(_, _, w, h)| w * 2.0 > pw as f32 && h * 2.0 > ph as f32);
        let (w, h) = match device {
            Some((_, _, gw, gh)) => (gw.max(1.0) as u32, gh.max(1.0) as u32),
            None => (
                (f[0].hypot(f[3]) / cw).round().max(1.0) as u32,
                (f[1].hypot(f[4]) / ch).round().max(1.0) as u32,
            ),
        };
        let pm = if device.is_some() && (w, h) != (pw, ph) {
            shrunk(&it.data, it.effects, (w, h), true)
        } else if (w as f32) < pw as f32 * 0.95 && (h as f32) < ph as f32 * 0.95 {
            shrunk(&it.data, it.effects, (w.min(pw), h.min(ph)), false)
        } else {
            picture(&it.data, it.effects)
        };
        let Some(pm) = pm else { return };
        (pm, fr, device.is_some())
    };
    let (iw, ih) = (pm.width() as f32, pm.height() as f32);
    let (cw, ch) = ((cr - cl).max(1e-6), (cb - ct).max(1e-6));
    // Picture pixels to the unit square of the visible part, then to the page.
    let to_unit = Transform::from_row(
        1.0 / (iw * cw),
        0.0,
        0.0,
        1.0 / (ih * ch),
        -cl / cw,
        -ct / ch,
    );
    let to_page = Transform::from_row(f[0], f[3], f[1], f[4], f[2], f[5]);
    // An upright picture is snapped to whole device pixels the way PDF
    // rasterizers fit images: the whole picture, before its crop, grows to
    // the pixels it touches; the frame still clips it.
    let device = upright.then(|| device_box(raw, (cl, ct), (cw, ch), grid));
    let snapped = device.map(|(gx0, gy0, gw, gh)| {
        // Picture pixels to the page, then back through the path's transform.
        let direct = Transform::from_row(gw / iw, 0.0, 0.0, gh / ih, gx0, gy0);
        to_page.invert().map(|inv| direct.post_concat(inv))
    });
    // A picture already reduced to its device pixels needs no filtering.
    let exact = sampled && device.is_some_and(|(_, _, gw, gh)| gw == iw && gh == ih);
    // The path's transform carries the shader along: the pattern only maps
    // picture pixels to the unit square.
    let shader = tiny_skia::Pattern::new(
        pm.as_ref().as_ref(),
        tiny_skia::SpreadMode::Pad,
        if exact {
            tiny_skia::FilterQuality::Nearest
        } else if sampled && device.is_some_and(|(_, _, gw, gh)| gw >= iw && gh >= ih) {
            tiny_skia::FilterQuality::Bilinear
        } else {
            tiny_skia::FilterQuality::Bicubic
        },
        1.0,
        snapped.flatten().unwrap_or(to_unit),
    );
    let mut p = Paint {
        shader,
        anti_alias: true,
        ..Paint::default()
    };
    p.blend_mode = tiny_skia::BlendMode::SourceOver;
    let Some(unit) = tiny_skia::Rect::from_xywh(0.0, 0.0, 1.0, 1.0) else {
        return;
    };
    let path = PathBuilder::from_rect(unit);
    match soft_mask(pixmap, it.effects.soft, to_page) {
        Some(mut soft) => {
            and_mask(&mut soft, clip);
            pixmap.fill_path(&path, &p, FillRule::Winding, to_page, Some(&soft));
        }
        None => pixmap.fill_path(&path, &p, FillRule::Winding, to_page, clip),
    }
}

/// The device pixels an upright picture covers before its crop: its frame
/// `raw` (page pixels), crop origin and size as fractions, each edge on the
/// drawing grid and then grown out to whole pixels. Returns the left, top,
/// width and height.
fn device_box(
    raw: [f32; 6],
    (cl, ct): (f32, f32),
    (cw, ch): (f32, f32),
    grid: impl Fn(f32) -> f32,
) -> (f32, f32, f32, f32) {
    let (fw, fh) = (raw[0] / cw, raw[4] / ch);
    let (x0, y0) = (grid(raw[2] - cl * fw), grid(raw[5] - ct * fh));
    let (fw, fh) = (grid(fw), grid(fh));
    let (gx0, gy0) = (x0.floor(), y0.floor());
    (gx0, gy0, (x0 + fw).ceil() - gx0, (y0 + fh).ceil() - gy0)
}

/// A mask fading a picture out toward the sides of its frame, the unit
/// square mapped by `to_page`.
fn soft_mask(pixmap: &Pixmap, (sx, sy): (u16, u16), to_page: Transform) -> Option<Mask> {
    if (sx, sy) == (0, 0) {
        return None;
    }
    let inv = to_page.invert()?;
    let (w, h) = (pixmap.width(), pixmap.height());
    let mut m = tiny_skia::Mask::new(w, h)?;
    let (ru, rv) = (
        (sx as f32 / 1000.0).max(1e-4),
        (sy as f32 / 1000.0).max(1e-4),
    );
    let corners = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)].map(|(u, v)| {
        let mut p = [tiny_skia::Point::from_xy(u, v)];
        to_page.map_points(&mut p);
        p[0]
    });
    let x0 = corners
        .iter()
        .map(|p| p.x)
        .fold(f32::MAX, f32::min)
        .floor()
        .max(0.0) as u32;
    let x1 = corners
        .iter()
        .map(|p| p.x)
        .fold(f32::MIN, f32::max)
        .ceil()
        .min(w as f32) as u32;
    let y0 = corners
        .iter()
        .map(|p| p.y)
        .fold(f32::MAX, f32::min)
        .floor()
        .max(0.0) as u32;
    let y1 = corners
        .iter()
        .map(|p| p.y)
        .fold(f32::MIN, f32::max)
        .ceil()
        .min(h as f32) as u32;
    let data = m.data_mut();
    for y in y0..y1 {
        for x in x0..x1 {
            let mut p = [tiny_skia::Point::from_xy(x as f32 + 0.5, y as f32 + 0.5)];
            inv.map_points(&mut p);
            let (u, v) = (p[0].x, p[0].y);
            let t = (u.min(1.0 - u) / ru)
                .min(v.min(1.0 - v) / rv)
                .clamp(0.0, 1.0);
            data[(y * w + x) as usize] = (t * t * (3.0 - 2.0 * t) * 255.0).round() as u8;
        }
    }
    Some(m)
}

/// ZOOM's fitted document box, before choosing a device sampler.
fn zoom_box(r: tiny_skia::Rect, (iw, ih): (u32, u32), dpi: f32) -> tiny_skia::Rect {
    // Brush sources have their natural 96dpi size, 75HU per pixel. Restore
    // document units at this pixel boundary before rounding the percentage.
    let hu = |v: f32| (f64::from(v) * 7200.0 / f64::from(dpi)).round();
    let percent = (hu(r.width()) * 100.0 / (f64::from(iw) * 75.0))
        .min(hu(r.height()) * 100.0 / (f64::from(ih) * 75.0))
        .ceil();
    let scale = (percent / 100.0 * f64::from(dpi) / 96.0) as f32;
    let (w, h) = (iw as f32 * scale, ih as f32 * scale);
    tiny_skia::Rect::from_xywh(
        r.left() + ((r.width() - w) / 2.0).max(0.0),
        r.top() + ((r.height() - h) / 2.0).max(0.0),
        w,
        h,
    )
    .unwrap_or(r)
}

/// A picture brush over a rectangle: tiled, stretched or placed.
fn image_fill(
    pixmap: &mut Pixmap,
    r: tiny_skia::Rect,
    data: &hm_model::SharedBytes,
    mode: u8,
    fx: Effects,
    dpi: f32,
    clip: Option<&tiny_skia::Mask>,
) {
    let Some(pm) = picture(data, fx) else { return };
    let (iw, ih) = (pm.width() as f32, pm.height() as f32);
    let bounds = r;
    // ZOOM rounds its fit to whole percent and centers any spare space.
    let r = if mode == 15 {
        zoom_box(r, (pm.width(), pm.height()), dpi)
    } else {
        r
    };
    let (spread, t) = match mode {
        // Stretched to the box.
        5 | 15 => (
            tiny_skia::SpreadMode::Pad,
            Transform::from_row(r.width() / iw, 0.0, 0.0, r.height() / ih, r.left(), r.top()),
        ),
        // Tiled from the top left at the picture's own size (96 dpi).
        0..=4 => (
            tiny_skia::SpreadMode::Repeat,
            Transform::from_translate(r.left(), r.top()),
        ),
        // Placed at a side or the center at its own size.
        _ => {
            let (hx, vy) = match mode {
                6 => (1, 1),
                7 => (1, 0),
                8 => (1, 2),
                9 => (0, 1),
                10 => (0, 0),
                11 => (0, 2),
                12 => (2, 1),
                13 => (2, 0),
                _ => (2, 2),
            };
            let x = r.left() + (r.width() - iw) * hx as f32 / 2.0;
            let y = r.top() + (r.height() - ih) * vy as f32 / 2.0;
            (tiny_skia::SpreadMode::Pad, Transform::from_translate(x, y))
        }
    };
    let bounded = (mode == 5
        && image::sniff(data) == image::Format::Png
        && r.width() >= iw
        && r.height() >= ih)
        .then(|| bounded_brush(&pm, r, pixmap.width(), pixmap.height()))
        .flatten();
    let shader = match &bounded {
        Some((p, x, y)) => tiny_skia::Pattern::new(
            p.as_ref(),
            tiny_skia::SpreadMode::Pad,
            tiny_skia::FilterQuality::Nearest,
            1.0,
            Transform::from_translate(*x, *y),
        ),
        None => tiny_skia::Pattern::new(
            pm.as_ref().as_ref(),
            spread,
            tiny_skia::FilterQuality::Bicubic,
            1.0,
            t,
        ),
    };
    let p = Paint {
        shader,
        anti_alias: true,
        ..Paint::default()
    };
    if let Some(r) = r.intersect(&bounds) {
        pixmap.fill_rect(r, &p, Transform::identity(), clip);
    }
}

/// Cubic enlargement bounded by the colors of its source neighborhood.
/// This retains the cubic edges without dark halos around light details.
fn bounded_brush(
    src: &Pixmap,
    r: tiny_skia::Rect,
    width: u32,
    height: u32,
) -> Option<(Pixmap, f32, f32)> {
    let x0 = r.left().floor().max(0.0);
    let y0 = r.top().floor().max(0.0);
    let w = (r.right().ceil().min(width as f32) - x0).max(0.0) as u32;
    let h = (r.bottom().ceil().min(height as f32) - y0).max(0.0) as u32;
    let mut dst = Pixmap::new(w, h)?;
    let sx = r.width() / src.width() as f32;
    let sy = r.height() / src.height() as f32;
    let p = Paint {
        shader: tiny_skia::Pattern::new(
            src.as_ref(),
            tiny_skia::SpreadMode::Pad,
            tiny_skia::FilterQuality::Bicubic,
            1.0,
            Transform::from_row(sx, 0.0, 0.0, sy, r.left() - x0, r.top() - y0),
        ),
        ..Paint::default()
    };
    dst.fill_rect(
        tiny_skia::Rect::from_xywh(0.0, 0.0, w as f32, h as f32)?,
        &p,
        Transform::identity(),
        None,
    );
    for y in 0..h {
        let iy = ((y as f32 + y0 + 0.5 - r.top()) / sy - 0.5).floor() as i32;
        for x in 0..w {
            let ix = ((x as f32 + x0 + 0.5 - r.left()) / sx - 0.5).floor() as i32;
            let (mut lo, mut hi) = ([255u8; 4], [0u8; 4]);
            for yy in iy - 1..=iy + 2 {
                for xx in ix - 1..=ix + 2 {
                    let k = ((yy.clamp(0, src.height() as i32 - 1) as u32 * src.width()
                        + xx.clamp(0, src.width() as i32 - 1) as u32)
                        * 4) as usize;
                    for c in 0..4 {
                        lo[c] = lo[c].min(src.data()[k + c]);
                        hi[c] = hi[c].max(src.data()[k + c]);
                    }
                }
            }
            let k = ((y * w + x) * 4) as usize;
            let pixel = &mut dst.data_mut()[k..k + 4];
            for c in 0..4 {
                pixel[c] = pixel[c].clamp(lo[c], hi[c]);
            }
            for c in 0..3 {
                pixel[c] = pixel[c].min(pixel[3]);
            }
        }
    }
    Some((dst, x0, y0))
}

/// Renders a display list to PNG bytes.
pub fn render_png(dl: &DisplayList, fonts: &FontSystem, dpi: f32) -> Option<Vec<u8>> {
    render(dl, fonts, dpi)?.encode_png().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_picture_brushes_preserve_aspect_and_center_inside_the_box() {
        for (w, h, expected) in [(4, 2, (0, 2, 8, 6)), (2, 4, (2, 0, 6, 8))] {
            let mut source = Pixmap::new(w, h).unwrap();
            source.fill(tiny_skia::Color::from_rgba8(80, 120, 160, 255));
            let data = hm_model::SharedBytes::from(source.encode_png().unwrap());
            let mut target = Pixmap::new(8, 8).unwrap();
            image_fill(
                &mut target,
                tiny_skia::Rect::from_xywh(0.0, 0.0, 8.0, 8.0).unwrap(),
                &data,
                15,
                Effects::default(),
                96.0,
                None,
            );
            for y in 0..8 {
                for x in 0..8 {
                    let inside =
                        x >= expected.0 && x < expected.2 && y >= expected.1 && y < expected.3;
                    assert_eq!(target.pixel(x, y).unwrap().alpha() != 0, inside);
                }
            }
        }
    }

    #[test]
    fn zoom_brush_document_geometry_survives_the_device_sampler_boundary() {
        // Native old-executable SVG geometry for source665×237 in this
        // authored box. It remains83% even when DPI changes the sampler.
        for dpi in [72.0, 96.0, 200.0] {
            let box_ = tiny_skia::Rect::from_xywh(
                px(7810, dpi),
                px(11569, dpi),
                px(44794, dpi),
                px(14583, dpi),
            )
            .unwrap();
            let fitted = zoom_box(box_, (665, 237), dpi);
            for (actual, native) in [
                (fitted.left(), 9508.875),
                (fitted.top(), 11569.0),
                (fitted.width(), 41396.25),
                (fitted.height(), 14753.25),
            ] {
                assert!((actual * 7200.0 / dpi - native).abs() < 0.02);
            }
        }
    }

    #[test]
    fn zoom_picture_brush_clips_rounded_overflow_to_its_authored_box() {
        let mut source = Pixmap::new(665, 237).unwrap();
        source.fill(tiny_skia::Color::from_rgba8(80, 120, 160, 255));
        let actual = render_clear(
            &DisplayList {
                width: 60000,
                height: 18000,
                items: vec![Item::Fill {
                    x: 0,
                    y: 0,
                    w: 60000,
                    h: 17700,
                    brush: Brush::Image {
                        bin_id: 1,
                        data: Some(hm_model::SharedBytes::from(source.encode_png().unwrap())),
                        mode: 15,
                        effect: 0,
                        brightness: 0,
                        contrast: 0,
                    },
                }],
            },
            &FontSystem::new(),
            96.0,
        )
        .unwrap();
        // Fit rounds from99.58% to100%, making the source one pixel
        // taller than the box. That extra source row must stay clipped.
        assert_eq!(actual.pixel(100, 235).unwrap().alpha(), 255);
        for y in 236..240 {
            for x in 0..800 {
                assert_eq!(actual.pixel(x, y).unwrap().alpha(), 0);
            }
        }
    }

    #[test]
    fn enlarged_image_brushes_keep_the_source_color_range() {
        let mut src = Pixmap::new(4, 1).unwrap();
        src.data_mut().copy_from_slice(&[
            255, 255, 255, 255, 96, 128, 160, 255, 96, 128, 160, 255, 255, 255, 255, 255,
        ]);
        let data = hm_model::SharedBytes::from(src.encode_png().unwrap());
        let mut dst = Pixmap::new(32, 32).unwrap();
        dst.fill(tiny_skia::Color::WHITE);
        image_fill(
            &mut dst,
            tiny_skia::Rect::from_xywh(0.0, 0.0, 32.0, 32.0).unwrap(),
            &data,
            5,
            Effects::default(),
            96.0,
            None,
        );
        assert!(
            dst.pixels()
                .iter()
                .all(|p| p.red() >= 96 && p.green() >= 128 && p.blue() >= 160 && p.alpha() == 255)
        );
    }

    #[test]
    fn reduced_picture_keeps_the_hancom_line_probe_samples() {
        let mut src = Pixmap::new(196, 196).unwrap();
        for y in 0..196usize {
            for x in 0..196usize {
                let ink =
                    (8..188).contains(&x) && x % 16 == 8 || (8..188).contains(&y) && y % 16 == 8;
                let value = if ink { 0 } else { 255 };
                src.data_mut()[(y * 196 + x) * 4..(y * 196 + x + 1) * 4]
                    .copy_from_slice(&[value, value, value, 255]);
            }
        }
        let dl = DisplayList {
            width: 14400,
            height: 14400,
            items: vec![Item::Image(Box::new(ImageItem {
                data: hm_model::SharedBytes::from(src.encode_png().unwrap()),
                frame: [5669, 0, 7200, 0, 5669, 7200],
                crop: [0; 4],
                natural: (14700, 14700),
                effects: Effects::default(),
            }))],
        };
        let pm = render(&dl, &FontSystem::new(), 200.0).unwrap();
        // Interior samples from the same 196-pixel grid exported through
        // Hancom and rasterized at 200 dpi, covering 159 device pixels.
        let red = |x: u32, y: u32| pm.pixel(199 + x, 199 + y).unwrap().red();
        assert_eq!(
            [red(6, 3), red(7, 3), red(19, 3), red(20, 3)],
            [129, 183, 124, 189]
        );
        assert_eq!(
            [red(6, 6), red(7, 6), red(6, 7), red(7, 7)],
            [66, 93, 93, 132]
        );
    }

    #[test]
    fn legacy_whole_picture_extents_paint_like_explicit_source_dimensions() {
        for alpha in [128, 255] {
            let mut src = Pixmap::new(728, 729).unwrap();
            for y in 0..729usize {
                for x in 0..728usize {
                    let pixel = if x % 16 == 8 || y % 16 == 8 {
                        [0, 0, 0, alpha]
                    } else {
                        [255; 4]
                    };
                    src.data_mut()[(y * 728 + x) * 4..(y * 728 + x + 1) * 4]
                        .copy_from_slice(&pixel);
                }
            }
            let data = hm_model::SharedBytes::from(src.encode_png().unwrap());
            for frame in [[29112, 0, 0, 0, 29160, 0], [0, -29160, 29160, 29112, 0, 0]] {
                let draw = |natural, crop| {
                    render_clear(
                        &DisplayList {
                            width: 30000,
                            height: 30000,
                            items: vec![Item::Image(Box::new(ImageItem {
                                data: data.clone(),
                                frame,
                                crop,
                                natural,
                                effects: Effects::default(),
                            }))],
                        },
                        &FontSystem::new(),
                        200.0,
                    )
                    .unwrap()
                    .data()
                    .to_vec()
                };
                // The same image in legacy HWP and native HWPX: a stored height
                // of 54660 HU ends at source pixel 728.8, which rounds to 729.
                let whole = draw((54600, 54660), [0, 0, 54600, 54660]);
                assert!(
                    draw((0, 0), [0, 0, 54600, 54660]) == whole,
                    "missing source dimensions changed the full image"
                );
                assert!(
                    draw((0, 0), [75, 0, 54600, 54660]) != whole,
                    "nonzero crop origins must retain their crop"
                );
                assert!(
                    draw((0, 0), [0, 0, 54600, 54615]) != whole,
                    "an intentional fractional crop must remain cropped"
                );
                assert!(
                    draw((54600, 54675), [0, 0, 54600, 54660]) != whole,
                    "explicit source dimensions remain authoritative"
                );
                if alpha == 128 {
                    assert!(whole.chunks_exact(4).any(|p| p[3] > 0 && p[3] < 255));
                }
            }
        }
    }

    #[test]
    fn a_large_reduction_keeps_constant_premultiplied_color() {
        let mut src = Pixmap::new(4096, 1).unwrap();
        for p in src.data_mut().chunks_exact_mut(4) {
            p.copy_from_slice(&[10, 20, 30, 64]);
        }
        let pm = resample(&src, 1, 1).unwrap();
        assert_eq!(pm.data(), &[10, 20, 30, 64]);
    }

    #[test]
    fn enlarged_picture_keeps_the_hancom_line_probe_samples() {
        let mut src = Pixmap::new(118, 118).unwrap();
        for y in 0..118usize {
            for x in 0..118usize {
                let ink =
                    (8..110).contains(&x) && x % 16 == 8 || (8..110).contains(&y) && y % 16 == 8;
                let value = if ink { 0 } else { 255 };
                src.data_mut()[(y * 118 + x) * 4..(y * 118 + x + 1) * 4]
                    .copy_from_slice(&[value, value, value, 255]);
            }
        }
        let dl = DisplayList {
            width: 43200,
            height: 14400,
            items: vec![Item::Image(Box::new(ImageItem {
                data: hm_model::SharedBytes::from(src.encode_png().unwrap()),
                frame: [5669, 0, 31200, 0, 5669, 7200],
                crop: [0; 4],
                natural: (8880, 8880),
                effects: Effects::default(),
            }))],
        };
        let pm = render(&dl, &FontSystem::new(), 200.0).unwrap();
        // The second grid in the Hancom probe covers 158 by 159 pixels.
        let red = |x: u32, y: u32| pm.pixel(866 + x, 199 + y).unwrap().red();
        assert_eq!(
            [red(10, 3), red(11, 3), red(10, 10), red(11, 11)],
            [167, 22, 118, 0]
        );
    }

    #[test]
    fn oversized_enlargement_reuses_the_source_across_draws() {
        let src = Pixmap::new(1, 1).unwrap();
        let data = hm_model::SharedBytes::from(src.encode_png().unwrap());
        let fx = Effects::default();
        picture(&data, fx).unwrap();
        for _ in 0..2 {
            let sampled = shrunk(&data, fx, (6000, 6000), true).unwrap();
            assert_eq!((sampled.width(), sampled.height()), (1, 1));
        }
    }

    #[test]
    fn clips_limit_what_they_enclose() {
        // A 1-inch page at 72 dpi: one pixel is 100 HU.
        let red = |x, y, w, h| Item::Rect {
            x,
            y,
            w,
            h,
            color: 0x0000FF,
        };
        let dl = DisplayList {
            width: 7200,
            height: 7200,
            items: vec![
                Item::Clip {
                    x: 1000,
                    y: 1000,
                    w: 4000,
                    h: 4000,
                },
                // Nested clips intersect.
                Item::Clip {
                    x: 3000,
                    y: 0,
                    w: 7200,
                    h: 7200,
                },
                red(0, 0, 7200, 2000),
                Item::Unclip,
                red(0, 4000, 7200, 3200),
                Item::Unclip,
                red(0, 6800, 400, 400),
            ],
        };
        let pm = render(&dl, &FontSystem::new(), 72.0).unwrap();
        let is_red = |x: u32, y: u32| {
            pm.pixel(x, y)
                .is_some_and(|p| p.red() == 255 && p.green() == 0)
        };
        assert!(is_red(40, 15) && !is_red(20, 15) && !is_red(60, 15) && !is_red(40, 5));
        assert!(is_red(15, 45) && !is_red(5, 45) && !is_red(15, 55));
        assert!(is_red(2, 70));
    }
}
