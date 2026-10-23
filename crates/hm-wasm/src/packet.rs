//! A page as one versioned little-endian packet: a header, a block table
//! and blocks.
//!
//! ```text
//! header   "HMPK", u16 version, u16 0, u32 page, u32 content epoch,
//!          u32 layout epoch, u32 font generation, i32 width, i32 height
//!          (HU), u32 block count, count x (4-byte tag, u32 offset from the
//!          packet start, u32 length)
//! "PANT"   paint ops: u32 count, then per op a u8 tag and its record
//! "LINE"   line index (see [`crate::index`])
//! "GLYF"   glyphs the page paints: u32 n, n x (u32 face, u32 glyph)
//! ```
//!
//! Paint ops, coordinates in page HU:
//!
//! ```text
//! 1 glyphs   u32 face, i32 size, i32 ratio, u32 color, u8 flags (1 bold,
//!            2 italic, 4 HFT face), i32 baseline, u32 n, n x (u16 glyph,
//!            i32 x)
//! 2 rect     i32 x, y, w, h, u32 color
//! 3 line     i32 x1, y1, x2, y2, i32 width, u32 color, u8 dash
//! 4 path     u32 fill color (0xFF000000 none), i32 width (-1 no stroke),
//!            u32 stroke color, u8 dash, u8 round cap, u8 fill alpha (0
//!            opaque, 255 clear), u32 n, n x (u8 verb: 0 move x y, 1 line
//!            x y, 2 cubic x1 y1 x2 y2 x y, 3 close; coordinates i32)
//! 5 patch    u32 item, i32 x, y, w, h: pixels the player asks for with
//!            `Doc.patch` (pictures, gradients, hatches, picture fills)
//! 6 outline  glyphs as 1, then i32 width, u32 n, n x i32 dash lengths
//! 7 clip     i32 x, y, w, h: later ops up to the matching 8 stay inside
//! 8 unclip
//! ```
//!
//! Colors are COLORREF (`0x00BBGGRR`). Dashes, in multiples of the stroke
//! width: 0 solid, 1 long dash [8 4], 2 dot [1 2], 3 dash dot [8 3 1 3],
//! 4 dash dot dot [8 3 1 3 1 3], 5 dash [16 6], 6 circle [0 2.5] with
//! round caps. Strokes are at least half a device pixel wide; paths fill
//! even-odd, glyphs nonzero. Glyph outlines (`Engine.glyphOutlines`) scale
//! by size over em, x also by ratio percent, and italic leans 0.2 of the
//! height; synthetic bold adds a stroke of the size over 40 (over 1000 for
//! HFT faces).

use hm_fonts::FontSystem;
use hm_render::draw::PathEl;
use hm_render::{Brush, Dash, DisplayList, GlyphRun, Item};

pub const VERSION: u16 = 1;
const NO_FILL: u32 = 0xFF00_0000;

pub(crate) struct W(pub Vec<u8>);

impl W {
    pub fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    pub fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn i32(&mut self, v: i32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
}

fn dash(d: Dash) -> u8 {
    match d {
        Dash::Solid => 0,
        Dash::LongDash => 1,
        Dash::Dot => 2,
        Dash::DashDot => 3,
        Dash::DashDotDot => 4,
        Dash::Dash => 5,
        Dash::Circle => 6,
    }
}

fn glyphs(w: &mut W, r: &GlyphRun, fonts: &FontSystem) {
    let hft = fonts.face(r.face).is_some_and(|f| f.is_hft());
    w.u32(r.face.0);
    w.i32(r.size);
    w.i32(r.ratio);
    w.u32(r.color);
    w.u8(r.fake_bold as u8 | (r.italic as u8) << 1 | (hft as u8) << 2);
    w.i32(r.baseline);
    w.u32(r.glyphs.len() as u32);
    for &(g, x) in &r.glyphs {
        w.u16(g);
        w.i32(x);
    }
}

pub use hm_render::vector::{patch_box, shifted};

/// What a packet header carries besides the page.
pub struct Header {
    pub page: u32,
    pub content_epoch: u32,
    pub layout_epoch: u32,
    pub font_generation: u32,
}

/// Assembles a packet from its blocks.
pub fn packet(h: &Header, dl: &DisplayList, blocks: &[([u8; 4], Vec<u8>)]) -> Vec<u8> {
    let mut w = W(Vec::new());
    w.0.extend_from_slice(b"HMPK");
    w.u16(VERSION);
    w.u16(0);
    for v in [h.page, h.content_epoch, h.layout_epoch, h.font_generation] {
        w.u32(v);
    }
    w.i32(dl.width);
    w.i32(dl.height);
    w.u32(blocks.len() as u32);
    let mut at = w.0.len() + blocks.len() * 12;
    for (tag, b) in blocks {
        w.0.extend_from_slice(tag);
        w.u32(at as u32);
        w.u32(b.len() as u32);
        at += b.len();
    }
    for (_, b) in blocks {
        w.0.extend_from_slice(b);
    }
    w.0
}

/// The glyphs a page paints, each once: `(face, glyph)` in first use order.
pub fn glyph_block(dl: &DisplayList) -> Vec<u8> {
    let mut seen = std::collections::HashSet::new();
    let mut list = Vec::new();
    for item in &dl.items {
        let run = match item {
            Item::Glyphs(r) => r,
            Item::Outline { run, .. } => run,
            _ => continue,
        };
        for &(g, _) in &run.glyphs {
            if seen.insert((run.face.0, g)) {
                list.push((run.face.0, g as u32));
            }
        }
    }
    let mut w = W(Vec::with_capacity(4 + list.len() * 8));
    w.u32(list.len() as u32);
    for (f, g) in list {
        w.u32(f);
        w.u32(g);
    }
    w.0
}

/// The paint block of a page.
pub fn paint_block(dl: &DisplayList, fonts: &FontSystem) -> Vec<u8> {
    let mut w = W(Vec::with_capacity(8 + dl.items.len() * 32));
    w.u32(dl.items.len() as u32);
    for (k, item) in dl.items.iter().enumerate() {
        if let Some((x, y, bw, bh)) = patch_box(item) {
            w.u8(5);
            w.u32(k as u32);
            w.i32(x);
            w.i32(y);
            w.i32(bw);
            w.i32(bh);
            continue;
        }
        match item {
            Item::Glyphs(r) => {
                w.u8(1);
                glyphs(&mut w, r, fonts);
            }
            Item::Rect {
                x,
                y,
                w: rw,
                h,
                color,
            } => {
                w.u8(2);
                for v in [*x, *y, *rw, *h] {
                    w.i32(v);
                }
                w.u32(*color);
            }
            Item::Fill {
                x,
                y,
                w: rw,
                h,
                brush,
            } => {
                w.u8(2);
                for v in [*x, *y, *rw, *h] {
                    w.i32(v);
                }
                w.u32(match brush {
                    Brush::Solid(c) => *c,
                    _ => 0,
                });
            }
            Item::Line {
                x1,
                y1,
                x2,
                y2,
                stroke,
            } => {
                w.u8(3);
                for v in [*x1, *y1, *x2, *y2, stroke.width] {
                    w.i32(v);
                }
                w.u32(stroke.color);
                w.u8(dash(stroke.dash));
            }
            Item::Path(p) => {
                w.u8(4);
                w.u32(match p.fill.first() {
                    Some(Brush::Solid(c)) => *c,
                    _ => NO_FILL,
                });
                match p.outline {
                    Some(o) => {
                        w.i32(o.width);
                        w.u32(o.color);
                        w.u8(dash(o.dash));
                        w.u8(o.round_cap as u8);
                    }
                    None => {
                        w.i32(-1);
                        w.u32(0);
                        w.u8(0);
                        w.u8(0);
                    }
                }
                w.u8(p.fill_alpha);
                w.u32(p.path.els.len() as u32);
                for e in &p.path.els {
                    match *e {
                        PathEl::Move(x, y) => {
                            w.u8(0);
                            w.i32(x);
                            w.i32(y);
                        }
                        PathEl::Line(x, y) => {
                            w.u8(1);
                            w.i32(x);
                            w.i32(y);
                        }
                        PathEl::Cubic(a, b, c, d, x, y) => {
                            w.u8(2);
                            for v in [a, b, c, d, x, y] {
                                w.i32(v);
                            }
                        }
                        PathEl::Close => w.u8(3),
                    }
                }
            }
            Item::Outline { run, width, dash } => {
                w.u8(6);
                glyphs(&mut w, run, fonts);
                w.i32(*width);
                w.u32(dash.len() as u32);
                for &d in dash {
                    w.i32(d);
                }
            }
            Item::Clip { x, y, w: cw, h } => {
                w.u8(7);
                for v in [*x, *y, *cw, *h] {
                    w.i32(v);
                }
            }
            Item::Unclip => w.u8(8),
            // Pictures always go as patches.
            Item::Image(_) => {}
        }
    }
    w.0
}

/// Glyph outlines for `(face, glyph)` pairs, y up in font units:
///
/// ```text
/// u32 n, n x (u32 face, u32 glyph, f32 em, u32 count, count x f32)
/// ```
///
/// where the floats are verbs and coordinates: 0 move x y, 1 line x y,
/// 2 quad x1 y1 x y, 3 cubic x1 y1 x2 y2 x y, 4 close. An unknown face,
/// or a glyph whose outline the engine has not kept yet, gives em 0 and no
/// floats.
pub fn glyph_outlines(fonts: &FontSystem, pairs: &[u32]) -> Vec<u8> {
    use hm_fonts::PathOp::*;
    let mut w = W(Vec::new());
    let pairs: Vec<&[u32]> = pairs.chunks_exact(2).collect();
    w.u32(pairs.len() as u32);
    let mut floats: Vec<f32> = Vec::new();
    for p in pairs {
        let (face, glyph) = (p[0], p[1]);
        w.u32(face);
        w.u32(glyph);
        floats.clear();
        let mut em = 0;
        if let Some(f) = fonts.face(hm_fonts::FaceId(face)) {
            let o = f.outline(glyph as u16);
            // Not kept yet: em 0 tells the player to ask again after the
            // font generation moves.
            if !f.resident(glyph as u16) {
                w.0.extend_from_slice(&0f32.to_le_bytes());
                w.u32(0);
                continue;
            }
            // HFT families mix design ems across their files.
            em = if o.units_per_em > 0 {
                o.units_per_em
            } else {
                f.units_per_em
            };
            for op in &o.ops {
                match *op {
                    Move(x, y) => floats.extend_from_slice(&[0.0, x, y]),
                    Line(x, y) => floats.extend_from_slice(&[1.0, x, y]),
                    Quad(a, b, x, y) => floats.extend_from_slice(&[2.0, a, b, x, y]),
                    Cubic(a, b, c, d, x, y) => floats.extend_from_slice(&[3.0, a, b, c, d, x, y]),
                    Close => floats.push(4.0),
                }
            }
        }
        w.0.extend_from_slice(&(em as f32).to_le_bytes());
        w.u32(floats.len() as u32);
        for v in &floats {
            w.0.extend_from_slice(&v.to_le_bytes());
        }
    }
    w.0
}
