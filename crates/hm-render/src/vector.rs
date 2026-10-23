//! What vector backends (SVG, PDF and the canvas player) share: which
//! items they draw as vectors and which as pixel patches rendered here,
//! glyph outlines placed on the page, and dash patterns.

use hm_fonts::{Face, FontSystem, PathOp};

use crate::draw::{ImageItem, PathEl, PathItem};
use crate::image::{self, Format};
use crate::paint::{Brush, Dash};
use crate::{DisplayList, GlyphRun, Item};

/// The page box of an item vector backends draw as pixels (pictures,
/// brushes other than a solid color, paths filled with them), `None` for
/// items they draw themselves.
pub fn patch_box(item: &Item) -> Option<(i32, i32, i32, i32)> {
    match item {
        Item::Image(i) => {
            let [a, b, c, d, e, f] = i.frame;
            let xs = [c, a + c, b + c, a + b + c];
            let ys = [f, d + f, e + f, d + e + f];
            let (x0, x1) = (*xs.iter().min()?, *xs.iter().max()?);
            let (y0, y1) = (*ys.iter().min()?, *ys.iter().max()?);
            Some((x0, y0, x1 - x0, y1 - y0))
        }
        Item::Fill { x, y, w, h, brush } if !matches!(brush, Brush::Solid(_)) => {
            Some((*x, *y, *w, *h))
        }
        Item::Path(p) if !simple(p) => {
            let (x, y, w, h) = p.path.bounds();
            // Room for the outline's width.
            let pad = p.outline.map_or(0, |o| o.width.max(0)) + 50;
            Some((x - pad, y - pad, w + 2 * pad, h + 2 * pad))
        }
        _ => None,
    }
}

/// A path filled with at most one solid color, which vector backends draw.
pub fn simple(p: &PathItem) -> bool {
    p.fill.len() <= 1 && p.fill.iter().all(|b| matches!(b, Brush::Solid(_)))
}

/// An item moved by `(dx, dy)`; only patch items move.
pub fn shifted(item: &Item, dx: i32, dy: i32) -> Item {
    let mut it = item.clone();
    match &mut it {
        Item::Image(i) => {
            i.frame[2] += dx;
            i.frame[5] += dy;
        }
        Item::Fill { x, y, .. } => {
            *x += dx;
            *y += dy;
        }
        Item::Path(p) => {
            for e in &mut p.path.els {
                match e {
                    PathEl::Move(x, y) | PathEl::Line(x, y) => {
                        *x += dx;
                        *y += dy;
                    }
                    PathEl::Cubic(a, b, c, d, x, y) => {
                        *a += dx;
                        *b += dy;
                        *c += dx;
                        *d += dy;
                        *x += dx;
                        *y += dy;
                    }
                    PathEl::Close => {}
                }
            }
        }
        _ => {}
    }
    it
}

/// A patch item drawn alone at `dpi` on a transparent pixmap covering its
/// page box (`x, y, w, h` in HU).
pub fn patch(
    item: &Item,
    fonts: &FontSystem,
    dpi: f32,
) -> Option<((i32, i32, i32, i32), tiny_skia::Pixmap)> {
    let (x, y, w, h) = patch_box(item)?;
    if w <= 0 || h <= 0 {
        return None;
    }
    // A picture gains nothing past its own pixel density.
    let dpi = match item {
        Item::Image(i) => image::dimensions(&i.data).map_or(dpi, |(pw, ph)| {
            let density = (pw as f32 * 7200.0 / w as f32).max(ph as f32 * 7200.0 / h as f32);
            dpi.min(density * 1.25).max(24.0)
        }),
        _ => dpi,
    };
    let one = DisplayList {
        width: w,
        height: h,
        items: vec![shifted(item, -x, -y)],
    };
    let pixmap = crate::raster::render_clear(&one, fonts, dpi)?;
    // Nothing visible, nothing to embed.
    if pixmap.pixels().iter().all(|p| p.alpha() == 0) {
        return None;
    }
    Some(((x, y, w, h), pixmap))
}

/// A picture vector backends embed whole, as stored or decoded at its own
/// size, placed by [`picture_matrix`] and clipped to its frame: a raster
/// format without effects.
pub fn plain_picture(item: &Item) -> Option<&ImageItem> {
    let Item::Image(i) = item else {
        return None;
    };
    let raster = !matches!(
        image::sniff(&i.data),
        Format::Wmf | Format::Emf | Format::Unknown
    );
    (raster && i.effects.is_none()).then_some(i.as_ref())
}

/// The map `[a, b, c, d, e, f]` taking a picture of `pw` by `ph` pixels,
/// as its unit square with `(s, t)` from the top left, to page HU
/// `(a s + b t + c, d s + e t + f)`, so its visible part fills the frame.
pub fn picture_matrix(i: &ImageItem, size: (u32, u32)) -> [f32; 6] {
    let [cl, ct, cr, cb] = crate::raster::crop_fractions(i, size);
    let (cw, ch) = ((cr - cl).max(1e-6), (cb - ct).max(1e-6));
    let [a, b, c, d, e, f] = i.frame.map(|v| v as f32);
    [
        a / cw,
        b / ch,
        c - a * cl / cw - b * ct / ch,
        d / cw,
        e / ch,
        f - d * cl / cw - e * ct / ch,
    ]
}

/// The corners of a picture's frame on the page, in order around it.
pub fn frame_corners(i: &ImageItem) -> [(i32, i32); 4] {
    let [a, b, c, d, e, f] = i.frame;
    [
        (c, f),
        (a + c, d + f),
        (a + b + c, d + e + f),
        (b + c, e + f),
    ]
}

/// Whether the frame shows the whole picture, so no clip is needed.
pub fn uncropped(i: &ImageItem, size: (u32, u32)) -> bool {
    let [l, t, r, b] = crate::raster::crop_fractions(i, size);
    l.abs() < 1e-4 && t.abs() < 1e-4 && (r - 1.0).abs() < 1e-4 && (b - 1.0).abs() < 1e-4
}

/// The components of a JPEG's frame (1 gray, 3 color, 4 CMYK).
pub fn jpeg_components(data: &[u8]) -> Option<u8> {
    let mut at = 2;
    while at + 4 <= data.len() {
        if data[at] != 0xFF {
            return None;
        }
        let marker = data[at + 1];
        let len = u16::from_be_bytes([data[at + 2], data[at + 3]]) as usize;
        if (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            return data.get(at + 9).copied();
        }
        at += 2 + len;
    }
    None
}

/// How a glyph of a run sits on the page: `[a, b, c, d, e, f]` taking font
/// units `(u, v)`, y up, to page HU `(a u + c v + e, b u + d v + f)`.
pub fn glyph_matrix(run: &GlyphRun, face: &Face, glyph: u16, x: i32) -> ([f32; 6], u16) {
    let outline = face.outline(glyph);
    // HFT families mix design ems across their files.
    let em = if outline.units_per_em > 0 {
        outline.units_per_em
    } else {
        face.units_per_em
    };
    let scale = run.size as f32 / em.max(1) as f32;
    let sx = scale * run.ratio as f32 / 100.0;
    let skew = if run.italic { 0.2 } else { 0.0 };
    (
        [sx, 0.0, scale * skew, -scale, x as f32, run.baseline as f32],
        em,
    )
}

/// The width Hancom strokes synthetic bold with, in HU: 1/40 of the size
/// for outline fonts, 1/1000 for HFT glyphs.
pub fn fake_bold_width(run: &GlyphRun, face: &Face) -> f32 {
    run.size as f32 / if face.is_hft() { 1000.0 } else { 40.0 }
}

/// One outline command in font units, y up, as text: SVG path syntax
/// (`M x y`, `Q`, `C`, `Z`) when `svg`, PDF operators otherwise (quads
/// raised to cubics).
pub fn outline_ops(face: &Face, glyph: u16, svg: bool) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    let mut at = (0.0f32, 0.0f32);
    for op in &face.outline(glyph).ops {
        let _ = match *op {
            PathOp::Move(x, y) if svg => write!(s, "M{} {}", n(x), n(y)),
            PathOp::Line(x, y) if svg => write!(s, "L{} {}", n(x), n(y)),
            PathOp::Quad(a, b, x, y) if svg => {
                write!(s, "Q{} {} {} {}", n(a), n(b), n(x), n(y))
            }
            PathOp::Cubic(a, b, c, d, x, y) if svg => {
                write!(s, "C{} {} {} {} {} {}", n(a), n(b), n(c), n(d), n(x), n(y))
            }
            PathOp::Close if svg => write!(s, "Z"),
            PathOp::Move(x, y) => writeln!(s, "{} {} m", n(x), n(y)),
            PathOp::Line(x, y) => writeln!(s, "{} {} l", n(x), n(y)),
            PathOp::Quad(a, b, x, y) => {
                let c1 = (at.0 + 2.0 / 3.0 * (a - at.0), at.1 + 2.0 / 3.0 * (b - at.1));
                let c2 = (x + 2.0 / 3.0 * (a - x), y + 2.0 / 3.0 * (b - y));
                writeln!(
                    s,
                    "{} {} {} {} {} {} c",
                    n(c1.0),
                    n(c1.1),
                    n(c2.0),
                    n(c2.1),
                    n(x),
                    n(y)
                )
            }
            PathOp::Cubic(a, b, c, d, x, y) => {
                writeln!(s, "{} {} {} {} {} {} c", n(a), n(b), n(c), n(d), n(x), n(y))
            }
            PathOp::Close => writeln!(s, "h"),
        };
        at = match *op {
            PathOp::Move(x, y)
            | PathOp::Line(x, y)
            | PathOp::Quad(_, _, x, y)
            | PathOp::Cubic(_, _, _, _, x, y) => (x, y),
            PathOp::Close => at,
        };
    }
    s
}

/// A number with at most two decimals and no trailing zeros.
pub fn n(v: f32) -> String {
    let r = (v * 100.0).round() / 100.0;
    if r == r.trunc() {
        format!("{}", r as i64)
    } else {
        let s = format!("{r:.2}");
        s.trim_end_matches('0').to_string()
    }
}

/// A stroke's width in HU, hairlines widened to stay visible.
pub fn stroke_width(width: i32) -> f32 {
    width.max(7) as f32
}

/// On/off lengths in HU for a dash style at stroke width `width`.
pub fn dashes(dash: Dash, width: f32) -> Option<Vec<f32>> {
    Some(
        dash.pattern()?
            .iter()
            .map(|k| (k * width).max(0.01))
            .collect(),
    )
}

/// RGB of a COLORREF (`0x00BBGGRR`).
pub fn rgb(color: u32) -> (u8, u8, u8) {
    (
        (color & 0xFF) as u8,
        ((color >> 8) & 0xFF) as u8,
        ((color >> 16) & 0xFF) as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cropped_picture_fills_its_frame_with_the_visible_part() {
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13];
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&100u32.to_be_bytes());
        png.extend_from_slice(&50u32.to_be_bytes());
        let mut it = ImageItem {
            data: hm_model::SharedBytes::from(png),
            frame: [2000, 0, 700, 0, 1000, 900],
            crop: [0; 4],
            effects: Default::default(),
            natural: (4000, 2000),
        };
        assert!(uncropped(&it, (100, 50)));
        assert_eq!(
            picture_matrix(&it, (100, 50)),
            [2000.0, 0.0, 700.0, 0.0, 1000.0, 900.0]
        );
        // The right half shown: the whole picture is twice the frame's
        // width, its left edge a frame width to the left.
        it.crop = [2000, 0, 4000, 2000];
        assert!(!uncropped(&it, (100, 50)));
        assert_eq!(
            picture_matrix(&it, (100, 50)),
            [4000.0, 0.0, -1300.0, 0.0, 1000.0, 900.0]
        );
        assert_eq!(
            frame_corners(&it),
            [(700, 900), (2700, 900), (2700, 1900), (700, 1900)]
        );
    }

    #[test]
    fn numbers_drop_trailing_zeros() {
        assert_eq!(n(12.0), "12");
        assert_eq!(n(1.5), "1.5");
        assert_eq!(n(-0.125), "-0.13");
        assert_eq!(n(3.999), "4");
    }
}
