//! tiny-skia backend.

use hm_fonts::{FontSystem, PathOp};
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Stroke, Transform};

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

/// Renders a display list to PNG bytes.
pub fn render_png(dl: &DisplayList, fonts: &FontSystem, dpi: f32) -> Option<Vec<u8>> {
    render(dl, fonts, dpi)?.encode_png().ok()
}
