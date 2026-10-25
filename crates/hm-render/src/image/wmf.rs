//! Windows metafiles (MS-WMF) played onto a pixmap.
//!
//! The metafile's window (its first window origin and extent, else the
//! placeable header's bounds) maps onto the whole pixmap. Pens, brushes,
//! fonts, the device context stack, clipping rectangles, shapes, text and
//! device-independent bitmaps are played; raster operations other than
//! copying are approximated with blend modes.

use std::sync::Arc;

use hm_fonts::{FaceQuery, FontSystem, PathOp};
use tiny_skia::{
    BlendMode, FillRule, LineCap, LineJoin, Mask, Paint, PathBuilder, Pixmap, PixmapPaint, Stroke,
    StrokeDash, Transform,
};

use super::bmp;

fn u16at(d: &[u8], o: usize) -> u16 {
    d.get(o..o + 2)
        .map_or(0, |b| u16::from_le_bytes([b[0], b[1]]))
}

fn i16at(d: &[u8], o: usize) -> f32 {
    u16at(d, o) as i16 as f32
}

fn u32at(d: &[u8], o: usize) -> u32 {
    d.get(o..o + 4)
        .map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// A parsed metafile: where its records start and the logical rectangle
/// that fills the picture.
pub struct Wmf<'a> {
    d: &'a [u8],
    records: usize,
    objects: usize,
    /// Left, top, width and height in logical units.
    pub window: [f32; 4],
}

impl<'a> Wmf<'a> {
    pub fn parse(d: &'a [u8]) -> Option<Wmf<'a>> {
        let placeable = d.starts_with(&[0xD7, 0xCD, 0xC6, 0x9A]);
        let at = if placeable { 22 } else { 0 };
        let kind = u16at(d, at);
        if !(kind == 1 || kind == 2) || u16at(d, at + 2) != 9 {
            return None;
        }
        let mut wmf = Wmf {
            d,
            records: at + 18,
            objects: u16at(d, at + 10) as usize,
            window: [0.0; 4],
        };
        let (mut org, mut ext) = (None, None);
        for (f, p) in wmf.iter() {
            match f {
                0x020B if org.is_none() => org = Some((i16at(p, 2), i16at(p, 0))),
                0x020C if ext.is_none() => ext = Some((i16at(p, 2), i16at(p, 0))),
                _ => {}
            }
            if org.is_some() && ext.is_some() {
                break;
            }
        }
        wmf.window = match ext {
            Some((w, h)) if w != 0.0 && h != 0.0 => {
                let (x, y) = org.unwrap_or((0.0, 0.0));
                [x, y, w, h]
            }
            _ if placeable => {
                let (l, t, r, b) = (i16at(d, 6), i16at(d, 8), i16at(d, 10), i16at(d, 12));
                if r == l || b == t {
                    return None;
                }
                [l, t, r - l, b - t]
            }
            _ => return None,
        };
        Some(wmf)
    }

    /// Records as `(function, parameters)`.
    fn iter(&self) -> impl Iterator<Item = (u16, &'a [u8])> + 'a {
        let d = self.d;
        let mut i = self.records;
        std::iter::from_fn(move || {
            let words = u32at(d, i) as usize;
            let f = u16at(d, i + 4);
            if words < 3 || f == 0 || i + 6 > d.len() {
                return None;
            }
            let end = (i + words * 2).min(d.len());
            let p = &d[i + 6..end.max(i + 6)];
            i += words * 2;
            Some((f, p))
        })
    }

    /// Plays the metafile onto a `w` by `h` pixmap.
    pub fn render(&self, w: u32, h: u32, fonts: &FontSystem) -> Option<Pixmap> {
        let pixmap = Pixmap::new(w.max(1), h.max(1))?;
        let mut p = Player {
            pixmap,
            fonts,
            objects: vec![None; self.objects.max(1)],
            dc: Dc::new(self.window),
            stack: Vec::new(),
            vp: (w as f32, h as f32),
            mask: None,
            mask_version: u64::MAX,
            clip_seq: 0,
        };
        for (f, params) in self.iter() {
            p.record(f, params);
        }
        Some(p.pixmap)
    }
}

#[derive(Clone, Debug)]
struct Pen {
    style: u16,
    width: f32,
    color: u32,
}

#[derive(Clone, Debug)]
enum Brush {
    Null,
    Solid(u32),
    Hatch(u32, u16),
    Pattern(Arc<Pixmap>),
}

#[derive(Clone, Debug)]
struct Font {
    height: f32,
    escapement: f32,
    weight: u16,
    italic: bool,
    underline: bool,
    strike: bool,
    charset: u8,
    face: String,
}

#[derive(Clone, Debug)]
enum Obj {
    Pen(Pen),
    Brush(Brush),
    Font(Box<Font>),
    Other,
}

/// The clip: a rectangle and excluded rectangles, in device pixels.
#[derive(Clone, Debug, Default)]
struct Clip {
    rect: Option<[f32; 4]>,
    excludes: Arc<Vec<[f32; 4]>>,
    /// Changes whenever the clip does, to reuse the built mask.
    version: u64,
}

#[derive(Clone, Debug)]
struct Dc {
    pen: Pen,
    brush: Brush,
    font: Option<Box<Font>>,
    text_color: u32,
    bk_color: u32,
    bk_opaque: bool,
    align: u16,
    winding: bool,
    cur: (f32, f32),
    org: (f32, f32),
    ext: (f32, f32),
    /// The window the pixmap shows at its first mapping.
    base: [f32; 4],
    clip: Clip,
}

impl Dc {
    fn new(window: [f32; 4]) -> Dc {
        Dc {
            pen: Pen {
                style: 0,
                width: 0.0,
                color: 0,
            },
            brush: Brush::Solid(0xFF_FFFF),
            font: None,
            text_color: 0,
            bk_color: 0xFF_FFFF,
            bk_opaque: true,
            align: 0,
            winding: false,
            cur: (0.0, 0.0),
            org: (window[0], window[1]),
            ext: (window[2], window[3]),
            base: window,
            clip: Clip::default(),
        }
    }
}

struct Player<'f> {
    pixmap: Pixmap,
    fonts: &'f FontSystem,
    objects: Vec<Option<Obj>>,
    dc: Dc,
    stack: Vec<Dc>,
    /// Device size of the first window.
    vp: (f32, f32),
    mask: Option<Mask>,
    mask_version: u64,
    clip_seq: u64,
}

fn color(c: u32) -> tiny_skia::Color {
    tiny_skia::Color::from_rgba8(c as u8, (c >> 8) as u8, (c >> 16) as u8, 255)
}

impl Player<'_> {
    /// Device pixels per logical unit, signed.
    fn scale(&self) -> (f32, f32) {
        let d = &self.dc;
        (
            self.vp.0 / d.base[2] * d.base[2] / d.ext.0,
            self.vp.1 / d.base[3] * d.base[3] / d.ext.1,
        )
    }

    fn map(&self, x: f32, y: f32) -> (f32, f32) {
        let (sx, sy) = self.scale();
        ((x - self.dc.org.0) * sx, (y - self.dc.org.1) * sy)
    }

    fn rect(&self, l: f32, t: f32, r: f32, b: f32) -> [f32; 4] {
        let (x0, y0) = self.map(l, t);
        let (x1, y1) = self.map(r, b);
        [x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)]
    }

    fn add_object(&mut self, o: Obj) {
        if let Some(slot) = self.objects.iter_mut().find(|s| s.is_none()) {
            *slot = Some(o);
        } else {
            self.objects.push(Some(o));
        }
    }

    fn touch_clip(&mut self) {
        self.clip_seq += 1;
        self.dc.clip.version = self.clip_seq;
    }

    /// The clip as a mask, built again only when the clip changed.
    fn mask(&mut self) -> Option<&Mask> {
        let clip = &self.dc.clip;
        if clip.rect.is_none() && clip.excludes.is_empty() {
            return None;
        }
        if self.mask_version != clip.version || self.mask.is_none() {
            let (w, h) = (self.pixmap.width(), self.pixmap.height());
            let mut m = Mask::new(w, h)?;
            let data = m.data_mut();
            let span = |r: &[f32; 4]| {
                let x0 = r[0].round().clamp(0.0, w as f32) as usize;
                let x1 = r[2].round().clamp(0.0, w as f32) as usize;
                let y0 = r[1].round().clamp(0.0, h as f32) as usize;
                let y1 = r[3].round().clamp(0.0, h as f32) as usize;
                (x0, x1, y0, y1)
            };
            let (x0, x1, y0, y1) = clip
                .rect
                .map_or((0, w as usize, 0, h as usize), |r| span(&r));
            for y in y0..y1 {
                data[y * w as usize + x0..y * w as usize + x1].fill(255);
            }
            for r in clip.excludes.iter() {
                let (x0, x1, y0, y1) = span(r);
                for y in y0..y1 {
                    data[y * w as usize + x0..y * w as usize + x1].fill(0);
                }
            }
            self.mask = Some(m);
            self.mask_version = clip.version;
        }
        self.mask.as_ref()
    }

    fn fill_path(&mut self, path: &tiny_skia::Path) {
        let brush = self.dc.brush.clone();
        let rule = if self.dc.winding {
            FillRule::Winding
        } else {
            FillRule::EvenOdd
        };
        let mut paint = Paint {
            anti_alias: true,
            ..Paint::default()
        };
        let hatch;
        match &brush {
            Brush::Null => return,
            Brush::Solid(c) => paint.set_color(color(*c)),
            Brush::Hatch(c, style) => {
                let Some(pm) = hatch_tile(*c, *style) else {
                    return;
                };
                hatch = pm;
                paint.shader = tiny_skia::Pattern::new(
                    hatch.as_ref(),
                    tiny_skia::SpreadMode::Repeat,
                    tiny_skia::FilterQuality::Nearest,
                    1.0,
                    Transform::identity(),
                );
            }
            Brush::Pattern(pm) => {
                paint.shader = tiny_skia::Pattern::new(
                    pm.as_ref().as_ref(),
                    tiny_skia::SpreadMode::Repeat,
                    tiny_skia::FilterQuality::Nearest,
                    1.0,
                    Transform::identity(),
                );
            }
        }
        let mask = self.mask().cloned();
        self.pixmap
            .fill_path(path, &paint, rule, Transform::identity(), mask.as_ref());
    }

    fn stroke_path(&mut self, path: &tiny_skia::Path) {
        let pen = self.dc.pen.clone();
        // Style 5 is the null pen.
        if pen.style & 0xF == 5 {
            return;
        }
        let (sx, _) = self.scale();
        let width = (pen.width * sx.abs()).max(1.0);
        let mut stroke = Stroke {
            width,
            line_cap: match pen.style & 0xF00 {
                0x100 => LineCap::Square,
                0x200 => LineCap::Butt,
                _ => LineCap::Round,
            },
            line_join: match pen.style & 0xF000 {
                0x1000 => LineJoin::Bevel,
                0x2000 => LineJoin::Miter,
                _ => LineJoin::Round,
            },
            ..Stroke::default()
        };
        let unit = width.max(1.0);
        let dash: &[f32] = match pen.style & 0xF {
            1 => &[18.0, 6.0],
            2 => &[3.0, 3.0],
            3 => &[9.0, 6.0, 3.0, 6.0],
            4 => &[9.0, 3.0, 3.0, 3.0, 3.0, 3.0],
            _ => &[],
        };
        if !dash.is_empty() {
            stroke.dash = StrokeDash::new(dash.iter().map(|v| v * unit).collect(), 0.0);
        }
        let mut paint = Paint {
            anti_alias: true,
            ..Paint::default()
        };
        paint.set_color(color(pen.color));
        let mask = self.mask().cloned();
        self.pixmap
            .stroke_path(path, &paint, &stroke, Transform::identity(), mask.as_ref());
    }

    fn shape(&mut self, path: Option<tiny_skia::Path>, fill: bool) {
        let Some(path) = path else { return };
        if fill {
            self.fill_path(&path);
        }
        self.stroke_path(&path);
    }

    fn poly(&self, p: &[u8], at: usize, n: usize, close: bool, pb: &mut PathBuilder) {
        for k in 0..n {
            let (x, y) = self.map(i16at(p, at + 4 * k), i16at(p, at + 4 * k + 2));
            if k == 0 {
                pb.move_to(x, y);
            } else {
                pb.line_to(x, y);
            }
        }
        if close && n > 0 {
            pb.close();
        }
    }

    /// An ellipse arc from the radial through `start` to the one through
    /// `end`, counterclockwise as GDI draws them.
    fn arc(&self, p: &[u8], kind: u16) -> Option<tiny_skia::Path> {
        let (ye, xe, ys, xs) = (i16at(p, 0), i16at(p, 2), i16at(p, 4), i16at(p, 6));
        let r = self.rect(i16at(p, 14), i16at(p, 12), i16at(p, 10), i16at(p, 8));
        let (cx, cy) = ((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0);
        let (rx, ry) = ((r[2] - r[0]) / 2.0, (r[3] - r[1]) / 2.0);
        if rx <= 0.0 || ry <= 0.0 {
            return None;
        }
        let (sx, sy) = self.map(xs, ys);
        let (ex, ey) = self.map(xe, ye);
        let a0 = (-(sy - cy) / ry).atan2((sx - cx) / rx);
        let mut a1 = (-(ey - cy) / ry).atan2((ex - cx) / rx);
        if a1 <= a0 {
            a1 += std::f32::consts::TAU;
        }
        let steps = (((a1 - a0) / 0.05).ceil() as usize).clamp(4, 720);
        let mut pb = PathBuilder::new();
        let point = |a: f32| (cx + rx * a.cos(), cy - ry * a.sin());
        if kind == 0x081A {
            pb.move_to(cx, cy);
            let (x, y) = point(a0);
            pb.line_to(x, y);
        } else {
            let (x, y) = point(a0);
            pb.move_to(x, y);
        }
        for k in 1..=steps {
            let (x, y) = point(a0 + (a1 - a0) * k as f32 / steps as f32);
            pb.line_to(x, y);
        }
        if kind != 0x0817 {
            pb.close();
        }
        pb.finish()
    }

    fn record(&mut self, f: u16, p: &[u8]) {
        match f {
            // Device context state.
            0x001E => self.stack.push(self.dc.clone()),
            0x0127 => {
                let n = u16at(p, 0) as i16;
                let keep = if n < 0 {
                    self.stack.len().saturating_sub((-n) as usize)
                } else {
                    (n as usize).saturating_sub(1)
                };
                if keep < self.stack.len() {
                    self.stack.truncate(keep + 1);
                    if let Some(dc) = self.stack.pop() {
                        self.dc = dc;
                    }
                }
            }
            0x020B => self.dc.org = (i16at(p, 2), i16at(p, 0)),
            0x020C => {
                let (w, h) = (i16at(p, 2), i16at(p, 0));
                if w != 0.0 && h != 0.0 {
                    self.dc.ext = (w, h);
                }
            }
            0x020F => {
                self.dc.org.0 += i16at(p, 2);
                self.dc.org.1 += i16at(p, 0);
            }
            0x0410 => {
                let (yd, yn, xd, xn) = (i16at(p, 0), i16at(p, 2), i16at(p, 4), i16at(p, 6));
                if xd != 0.0 && yd != 0.0 {
                    let e = (self.dc.ext.0 * xn / xd, self.dc.ext.1 * yn / yd);
                    if e.0 != 0.0 && e.1 != 0.0 {
                        self.dc.ext = e;
                    }
                }
            }
            0x0209 => self.dc.text_color = u32at(p, 0),
            0x0201 => self.dc.bk_color = u32at(p, 0),
            0x0102 => self.dc.bk_opaque = u16at(p, 0) == 2,
            0x012E => self.dc.align = u16at(p, 0),
            0x0106 => self.dc.winding = u16at(p, 0) == 2,
            // Objects.
            0x02FA => self.add_object(Obj::Pen(Pen {
                style: u16at(p, 0),
                width: i16at(p, 2).abs(),
                color: u32at(p, 6),
            })),
            0x02FC => {
                let c = u32at(p, 2);
                let b = match u16at(p, 0) {
                    1 => Brush::Null,
                    2 => Brush::Hatch(c, u16at(p, 6)),
                    _ => Brush::Solid(c),
                };
                self.add_object(Obj::Brush(b));
            }
            0x0142 => {
                let b = bmp::decode_dib(p.get(4..).unwrap_or(&[]), None)
                    .and_then(|r| r.to_pixmap())
                    .map_or(Brush::Solid(0), |pm| Brush::Pattern(Arc::new(pm)));
                self.add_object(Obj::Brush(b));
            }
            0x02FB => {
                let name = p.get(18..).unwrap_or(&[]);
                let name = &name[..name.iter().position(|&b| b == 0).unwrap_or(name.len())];
                let (face, _, _) = encoding_rs::EUC_KR.decode(name);
                self.add_object(Obj::Font(Box::new(Font {
                    height: i16at(p, 0),
                    escapement: i16at(p, 4),
                    weight: u16at(p, 8),
                    italic: p.get(10).is_some_and(|&b| b != 0),
                    underline: p.get(11).is_some_and(|&b| b != 0),
                    strike: p.get(12).is_some_and(|&b| b != 0),
                    charset: p.get(13).copied().unwrap_or(0),
                    face: face.into_owned(),
                })));
            }
            0x00F7 | 0x06FF | 0x01F9 => self.add_object(Obj::Other),
            0x012D => {
                if let Some(Some(o)) = self.objects.get(u16at(p, 0) as usize) {
                    match o.clone() {
                        Obj::Pen(pen) => self.dc.pen = pen,
                        Obj::Brush(b) => self.dc.brush = b,
                        Obj::Font(font) => self.dc.font = Some(font),
                        Obj::Other => {}
                    }
                }
            }
            0x01F0 => {
                if let Some(slot) = self.objects.get_mut(u16at(p, 0) as usize) {
                    *slot = None;
                }
            }
            // Clipping.
            0x0416 => {
                let r = self.rect(i16at(p, 6), i16at(p, 4), i16at(p, 2), i16at(p, 0));
                let r = match self.dc.clip.rect {
                    Some(c) => [
                        c[0].max(r[0]),
                        c[1].max(r[1]),
                        c[2].min(r[2]),
                        c[3].min(r[3]),
                    ],
                    None => r,
                };
                self.dc.clip.rect = Some(r);
                self.touch_clip();
            }
            0x0415 => {
                let r = self.rect(i16at(p, 6), i16at(p, 4), i16at(p, 2), i16at(p, 0));
                Arc::make_mut(&mut self.dc.clip.excludes).push(r);
                self.touch_clip();
            }
            0x012C => {
                self.dc.clip = Clip::default();
                self.touch_clip();
            }
            // Lines and shapes.
            0x0214 => self.dc.cur = (i16at(p, 2), i16at(p, 0)),
            0x0213 => {
                let to = (i16at(p, 2), i16at(p, 0));
                let (a, b) = (self.map(self.dc.cur.0, self.dc.cur.1), self.map(to.0, to.1));
                let mut pb = PathBuilder::new();
                pb.move_to(a.0, a.1);
                pb.line_to(b.0, b.1);
                self.shape(pb.finish(), false);
                self.dc.cur = to;
            }
            0x0324 | 0x0325 => {
                let n = (u16at(p, 0) as usize).min(p.len().saturating_sub(2) / 4);
                let mut pb = PathBuilder::new();
                self.poly(p, 2, n, f == 0x0324, &mut pb);
                self.shape(pb.finish(), f == 0x0324);
            }
            0x0538 => {
                let polys = u16at(p, 0) as usize;
                let mut at = 2 + 2 * polys;
                let mut pb = PathBuilder::new();
                for k in 0..polys {
                    let n = (u16at(p, 2 + 2 * k) as usize).min(p.len().saturating_sub(at) / 4);
                    self.poly(p, at, n, true, &mut pb);
                    at += 4 * n;
                }
                self.shape(pb.finish(), true);
            }
            0x041B => {
                let r = self.rect(i16at(p, 6), i16at(p, 4), i16at(p, 2), i16at(p, 0));
                let path =
                    tiny_skia::Rect::from_ltrb(r[0], r[1], r[2], r[3]).map(PathBuilder::from_rect);
                self.shape(path, true);
            }
            0x061C => {
                let r = self.rect(i16at(p, 10), i16at(p, 8), i16at(p, 6), i16at(p, 4));
                let (sx, sy) = self.scale();
                let rx = (i16at(p, 2) * sx.abs() / 2.0).min((r[2] - r[0]) / 2.0);
                let ry = (i16at(p, 0) * sy.abs() / 2.0).min((r[3] - r[1]) / 2.0);
                self.shape(round_rect(r, rx, ry), true);
            }
            0x0418 => {
                let r = self.rect(i16at(p, 6), i16at(p, 4), i16at(p, 2), i16at(p, 0));
                let path = tiny_skia::Rect::from_ltrb(r[0], r[1], r[2], r[3])
                    .and_then(PathBuilder::from_oval);
                self.shape(path, true);
            }
            0x0817 | 0x081A | 0x0830 => {
                let path = self.arc(p, f);
                self.shape(path, f != 0x0817);
            }
            // Pictures and fills.
            0x061D => {
                let rop = u32at(p, 0);
                let r = self.rect(
                    i16at(p, 10),
                    i16at(p, 8),
                    i16at(p, 10) + i16at(p, 6),
                    i16at(p, 8) + i16at(p, 4),
                );
                self.pat_blt(rop, r);
            }
            0x0B41 | 0x0940 | 0x0F43 => self.blit(f, p),
            0x0A32 => self.ext_text_out(p),
            0x0521 => {
                let n = u16at(p, 0) as usize;
                let s = p.get(2..2 + n).unwrap_or(&[]);
                let at = 2 + n.div_ceil(2) * 2;
                let (y, x) = (i16at(p, at), i16at(p, at + 2));
                self.text(x, y, s, &[], None);
            }
            _ => {}
        }
    }

    fn pat_blt(&mut self, rop: u32, r: [f32; 4]) {
        let Some(rect) = tiny_skia::Rect::from_ltrb(r[0], r[1], r[2], r[3]) else {
            return;
        };
        match rop {
            // BLACKNESS and WHITENESS.
            0x0000_0042 | 0x00FF_0062 => {
                let mut paint = Paint::default();
                paint.set_color(color(if rop == 0x42 { 0 } else { 0xFF_FFFF }));
                let mask = self.mask().cloned();
                self.pixmap
                    .fill_rect(rect, &paint, Transform::identity(), mask.as_ref());
            }
            // Destination untouched.
            0x00AA_0029 => {}
            _ => {
                let path = PathBuilder::from_rect(rect);
                let saved = self.dc.winding;
                self.dc.winding = true;
                self.fill_path(&path);
                self.dc.winding = saved;
            }
        }
    }

    fn blit(&mut self, f: u16, p: &[u8]) {
        let rop = u32at(p, 0);
        // Field offsets: (src h, src w, src y, src x, dst h, dst w, dst y,
        // dst x, DIB).
        let (sh, sw, sy, sx, dh, dw, dy, dx, dib) = match f {
            0x0F43 => (
                i16at(p, 6),
                i16at(p, 8),
                i16at(p, 10),
                i16at(p, 12),
                i16at(p, 14),
                i16at(p, 16),
                i16at(p, 18),
                i16at(p, 20),
                22,
            ),
            0x0B41 => (
                i16at(p, 4),
                i16at(p, 6),
                i16at(p, 8),
                i16at(p, 10),
                i16at(p, 12),
                i16at(p, 14),
                i16at(p, 16),
                i16at(p, 18),
                20,
            ),
            _ => {
                let (h, w) = (i16at(p, 8), i16at(p, 10));
                (
                    h,
                    w,
                    i16at(p, 4),
                    i16at(p, 6),
                    h,
                    w,
                    i16at(p, 12),
                    i16at(p, 14),
                    16,
                )
            }
        };
        let r = self.rect(dx, dy, dx + dw, dy + dh);
        let Some(bits) = p.get(dib..).filter(|b| b.len() > 12) else {
            // The bitmap-less forms paint with the brush.
            self.pat_blt(rop, r);
            return;
        };
        let Some(raster) = bmp::decode_dib(bits, None) else {
            return;
        };
        let bottom_up = u32at(bits, 8) as i32 > 0 && u32at(bits, 0) != 12;
        let Some(src) = raster.to_pixmap() else {
            return;
        };
        let mirror_x = (sw < 0.0) != (dw < 0.0);
        let mirror_y = (sh < 0.0) != (dh < 0.0);
        let (iw, ih) = (src.width() as f32, src.height() as f32);
        let (sw, sh) = if sw == 0.0 || sh == 0.0 {
            (iw, ih)
        } else {
            (sw.abs(), sh.abs())
        };
        // Source rows of a bottom-up bitmap count from its bottom.
        let top = if bottom_up { ih - sy - sh } else { sy };
        let (w, h) = (r[2] - r[0], r[3] - r[1]);
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let (fx, fy) = (w / sw, h / sh);
        let mut t = Transform::from_row(fx, 0.0, 0.0, fy, r[0] - sx * fx, r[1] - top * fy);
        // DIB reflection follows the logical source/destination signs,
        // independently of the window's coordinate direction.
        if mirror_x {
            t = t.post_concat(Transform::from_row(-1.0, 0.0, 0.0, 1.0, r[0] + r[2], 0.0));
        }
        if mirror_y {
            t = t.post_concat(Transform::from_row(1.0, 0.0, 0.0, -1.0, 0.0, r[1] + r[3]));
        }
        let blend = match rop {
            0x0088_00C6 => BlendMode::Multiply,
            0x00EE_0086 => BlendMode::Lighten,
            0x0066_0046 => BlendMode::Difference,
            _ => BlendMode::SourceOver,
        };
        let paint = PixmapPaint {
            blend_mode: blend,
            quality: tiny_skia::FilterQuality::Bilinear,
            ..PixmapPaint::default()
        };
        let mut clip = Mask::new(self.pixmap.width(), self.pixmap.height());
        if let Some(m) = clip.as_mut() {
            let rect = tiny_skia::Rect::from_ltrb(r[0], r[1], r[2], r[3]);
            if let Some(rect) = rect {
                m.fill_path(
                    &PathBuilder::from_rect(rect),
                    FillRule::Winding,
                    false,
                    Transform::identity(),
                );
            }
            if let Some(dc_mask) = self.mask().cloned() {
                for (a, b) in m.data_mut().iter_mut().zip(dc_mask.data()) {
                    *a = ((*a as u16 * *b as u16) / 255) as u8;
                }
            }
        }
        self.pixmap
            .draw_pixmap(0, 0, src.as_ref(), &paint, t, clip.as_ref());
    }

    fn ext_text_out(&mut self, p: &[u8]) {
        let (y, x) = (i16at(p, 0), i16at(p, 2));
        let n = u16at(p, 4) as usize;
        let opts = u16at(p, 6);
        let mut at = 8;
        let mut rect = None;
        // ETO_OPAQUE or ETO_CLIPPED carry a rectangle.
        if opts & 6 != 0 {
            rect = Some(self.rect(
                i16at(p, at),
                i16at(p, at + 2),
                i16at(p, at + 4),
                i16at(p, at + 6),
            ));
            at += 8;
        }
        let s = p.get(at..at + n).unwrap_or(&[]);
        at += n.div_ceil(2) * 2;
        let dx: Vec<f32> = (0..n)
            .map(|k| at + 2 * k)
            .take_while(|&o| o + 2 <= p.len())
            .map(|o| i16at(p, o))
            .collect();
        if let (Some(r), true) = (rect, opts & 2 != 0) {
            if let Some(rect) = tiny_skia::Rect::from_ltrb(r[0], r[1], r[2], r[3]) {
                let mut paint = Paint::default();
                paint.set_color(color(self.dc.bk_color));
                let mask = self.mask().cloned();
                self.pixmap
                    .fill_rect(rect, &paint, Transform::identity(), mask.as_ref());
            }
        }
        let clip = rect.filter(|_| opts & 4 != 0);
        self.text(x, y, s, &dx, clip);
    }

    /// Text from the font's code page: Korean double-byte or single bytes.
    fn text(&mut self, x: f32, y: f32, s: &[u8], dx: &[f32], clip: Option<[f32; 4]>) {
        let Some(font) = self.dc.font.clone() else {
            return;
        };
        let symbol = font.charset == 2;
        // Characters with the number of bytes each takes.
        let mut chars: Vec<(char, usize)> = Vec::new();
        let mut i = 0;
        while i < s.len() {
            let b = s[i];
            if b >= 0x81 && i + 1 < s.len() && !symbol {
                let (t, _, bad) = encoding_rs::EUC_KR.decode(&s[i..i + 2]);
                let c = t.chars().next().unwrap_or('\u{fffd}');
                chars.push((if bad { '\u{fffd}' } else { c }, 2));
                i += 2;
            } else {
                let c = if symbol {
                    char::from_u32(0xF000 + b as u32).unwrap_or(' ')
                } else if b < 0x80 {
                    b as char
                } else {
                    encoding_rs::WINDOWS_1252
                        .decode(&s[i..i + 1])
                        .0
                        .chars()
                        .next()
                        .unwrap_or(' ')
                };
                chars.push((c, 1));
                i += 1;
            }
        }
        let Some(r) = self.fonts.resolve(&FaceQuery {
            name: &font.face,
            alternates: Vec::new(),
            bold: font.weight >= 600,
            italic: font.italic,
        }) else {
            return;
        };
        let Some(face) = self.fonts.face(r.face) else {
            return;
        };
        let upem = face.units_per_em.max(1) as f32;
        let (asc, desc) = (face.ascent as f32 / upem, face.descent.abs() as f32 / upem);
        // A positive height is the cell height, a negative one the em.
        let em = if font.height < 0.0 {
            -font.height
        } else if font.height > 0.0 {
            font.height / (asc + desc).max(0.5)
        } else {
            12.0
        };
        let (sx, sy) = self.scale();
        let (sx, sy) = (sx.abs(), sy.abs());
        let em_px = em * sy;
        // Logical advances: the Dx array when given, else the font's.
        let mut advances = Vec::with_capacity(chars.len());
        let mut k = 0;
        for &(c, bytes) in &chars {
            let from_dx: f32 = dx.iter().skip(k).take(bytes).sum();
            k += bytes;
            let adv = if dx.len() >= k {
                from_dx
            } else {
                face.glyph(c)
                    .map_or(em / 2.0, |g| g.advance as f32 / g.em.max(1) as f32 * em)
            };
            advances.push(adv);
        }
        let width: f32 = advances.iter().sum();
        let update_cp = self.dc.align & 1 != 0;
        let (mut ox, oy) = if update_cp { self.dc.cur } else { (x, y) };
        let start_x = ox;
        match self.dc.align & 6 {
            6 => ox -= width / 2.0,
            2 => ox -= width,
            _ => {}
        }
        // Baseline offset from the reference point.
        let base = match self.dc.align & 0x18 {
            0x18 => 0.0,
            8 => -desc * em,
            _ => asc * em,
        };
        if update_cp {
            self.dc.cur = (start_x + width, oy);
        }
        let (rx, ry) = self.map(x, y);
        let (px0, py0) = self.map(ox, oy);
        let angle = -font.escapement / 10.0;
        let rot = Transform::from_rotate_at(angle, rx, ry);
        let mut paint = Paint {
            anti_alias: true,
            ..Paint::default()
        };
        paint.set_color(color(self.dc.text_color));
        if self.dc.bk_opaque {
            let top = py0 + (base - asc * em) * sy;
            if let Some(rect) =
                tiny_skia::Rect::from_xywh(px0, top, width * sx, (asc + desc) * em * sy)
            {
                let mut bk = Paint::default();
                bk.set_color(color(self.dc.bk_color));
                let mask = self.mask().cloned();
                self.pixmap.fill_path(
                    &PathBuilder::from_rect(rect),
                    &bk,
                    FillRule::Winding,
                    rot,
                    mask.as_ref(),
                );
            }
        }
        let mut mask = self.mask().cloned();
        if let Some(c) = clip {
            let (w, h) = (self.pixmap.width(), self.pixmap.height());
            let mut m = mask.take().or_else(|| {
                let mut full = Mask::new(w, h)?;
                full.data_mut().fill(255);
                Some(full)
            });
            if let Some(m) = m.as_mut() {
                let data = m.data_mut();
                for yy in 0..h as usize {
                    for xx in 0..w as usize {
                        let (fx, fy) = (xx as f32 + 0.5, yy as f32 + 0.5);
                        if fx < c[0] || fx > c[2] || fy < c[1] || fy > c[3] {
                            data[yy * w as usize + xx] = 0;
                        }
                    }
                }
            }
            mask = m;
        }
        let skew = if font.italic && !self.fonts.entry(r.face).italic {
            -0.2
        } else {
            0.0
        };
        let mut pen = px0;
        let baseline = py0 + base * sy;
        for (&(c, _), adv) in chars.iter().zip(&advances) {
            if let Some(g) = face.glyph(c)
                && let Some(path) = outline_path(&face.outline(g.glyph))
            {
                let s = em_px / upem;
                let t = Transform::from_row(s, 0.0, skew * s, -s, pen, baseline).post_concat(rot);
                self.pixmap
                    .fill_path(&path, &paint, FillRule::Winding, t, mask.as_ref());
            }
            pen += adv * sx;
        }
        let line = |off: f32| {
            tiny_skia::Rect::from_xywh(
                px0,
                baseline + off * em_px,
                width * sx,
                (em_px / 20.0).max(1.0),
            )
        };
        for (on, off) in [(font.underline, 0.1), (font.strike, -0.3)] {
            if on && let Some(r) = line(off) {
                self.pixmap.fill_path(
                    &PathBuilder::from_rect(r),
                    &paint,
                    FillRule::Winding,
                    rot,
                    mask.as_ref(),
                );
            }
        }
    }
}

fn round_rect(r: [f32; 4], rx: f32, ry: f32) -> Option<tiny_skia::Path> {
    if rx <= 0.0 || ry <= 0.0 {
        return tiny_skia::Rect::from_ltrb(r[0], r[1], r[2], r[3]).map(PathBuilder::from_rect);
    }
    const K: f32 = 0.552_284_8;
    let (l, t, rr, b) = (r[0], r[1], r[2], r[3]);
    let mut pb = PathBuilder::new();
    pb.move_to(l + rx, t);
    pb.line_to(rr - rx, t);
    pb.cubic_to(rr - rx + rx * K, t, rr, t + ry - ry * K, rr, t + ry);
    pb.line_to(rr, b - ry);
    pb.cubic_to(rr, b - ry + ry * K, rr - rx + rx * K, b, rr - rx, b);
    pb.line_to(l + rx, b);
    pb.cubic_to(l + rx - rx * K, b, l, b - ry + ry * K, l, b - ry);
    pb.line_to(l, t + ry);
    pb.cubic_to(l, t + ry - ry * K, l + rx - rx * K, t, l + rx, t);
    pb.close();
    pb.finish()
}

/// A glyph outline as a path in font units (y up).
fn outline_path(o: &hm_fonts::Outline) -> Option<tiny_skia::Path> {
    let mut pb = PathBuilder::new();
    for op in &o.ops {
        match *op {
            PathOp::Move(x, y) => pb.move_to(x, y),
            PathOp::Line(x, y) => pb.line_to(x, y),
            PathOp::Quad(a, b, x, y) => pb.quad_to(a, b, x, y),
            PathOp::Cubic(a, b, c, d, x, y) => pb.cubic_to(a, b, c, d, x, y),
            PathOp::Close => pb.close(),
        }
    }
    pb.finish()
}

/// An 8 by 8 hatch tile: horizontal, vertical, two diagonals, cross and
/// diagonal cross.
fn hatch_tile(c: u32, style: u16) -> Option<Pixmap> {
    let mut pm = Pixmap::new(8, 8)?;
    let px =
        tiny_skia::PremultipliedColorU8::from_rgba(c as u8, (c >> 8) as u8, (c >> 16) as u8, 255)?;
    let pixels = pm.pixels_mut();
    for y in 0..8usize {
        for x in 0..8usize {
            let on = match style {
                0 => y == 4,
                1 => x == 4,
                2 => (x + y) % 8 == 7,
                3 => x == y,
                4 => x == 4 || y == 4,
                _ => x == y || (x + y) % 8 == 7,
            };
            if on {
                pixels[y * 8 + x] = px;
            }
        }
    }
    Some(pm)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(f: u16, params: &[i16]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend(((3 + params.len()) as u32).to_le_bytes());
        v.extend(f.to_le_bytes());
        for p in params {
            v.extend(p.to_le_bytes());
        }
        v
    }

    #[test]
    fn bitmap_mirroring_uses_logical_extents_with_either_window_direction() {
        let mut dib = vec![0; 40];
        dib[..4].copy_from_slice(&40u32.to_le_bytes());
        dib[4..8].copy_from_slice(&2i32.to_le_bytes());
        dib[8..12].copy_from_slice(&2i32.to_le_bytes());
        dib[12..14].copy_from_slice(&1u16.to_le_bytes());
        dib[14..16].copy_from_slice(&24u16.to_le_bytes());
        // Bottom-up BGR rows: blue/white below red/green, padded to DWORDs.
        dib.extend([255, 0, 0, 255, 255, 255, 0, 0, 0, 0, 255, 0, 255, 0, 0, 0]);
        let colors = [(255, 0, 0), (0, 255, 0), (0, 0, 255), (255, 255, 255)];
        for function in [0x0B41, 0x0F43] {
            for (wx, wy) in [(2, 2), (2, -2), (-2, 2), (-2, -2)] {
                for (dw, dh) in [(2, 2), (2, -2), (-2, 2), (-2, -2)] {
                    let mut d = vec![1, 0, 9, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
                    d.extend(record(0x020B, &[0, 0]));
                    d.extend(record(0x020C, &[wy, wx]));
                    let (dx, dy) = (
                        if (dw < 0) == (wx < 0) { 0 } else { wx },
                        if (dh < 0) == (wy < 0) { 0 } else { wy },
                    );
                    let mut params = vec![0x0020, 0x00CC]; // SRCCOPY.
                    if function == 0x0F43 {
                        params.push(0); // DIB_RGB_COLORS.
                    }
                    params.extend([2, 2, 0, 0, dh, dw, dy, dx]);
                    params.extend(
                        dib.chunks_exact(2)
                            .map(|p| i16::from_le_bytes([p[0], p[1]])),
                    );
                    d.extend(record(function, &params));
                    d.extend(record(0, &[]));
                    let pm = Wmf::parse(&d)
                        .unwrap()
                        .render(20, 20, &FontSystem::new())
                        .unwrap();
                    for (y, py) in [4, 15].into_iter().enumerate() {
                        for (x, px) in [4, 15].into_iter().enumerate() {
                            let sx = if dw < 0 { 1 - x } else { x };
                            let sy = if dh < 0 { 1 - y } else { y };
                            let expected = colors[sy * 2 + sx];
                            let p = pm.pixel(px, py).unwrap().demultiply();
                            assert_eq!(
                                (p.red(), p.green(), p.blue()),
                                expected,
                                "function={function:x}, window=({wx},{wy}), destination=({dw},{dh})"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_filled_rectangle_covers_its_window_quarter() {
        let mut d = vec![1, 0, 9, 0, 0, 3, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0];
        d.extend(record(0x020B, &[0, 0]));
        d.extend(record(0x020C, &[100, 100]));
        // A red solid brush and the null pen.
        d.extend(record(0x02FC, &[0, 0x00FF, 0, 0]));
        d.extend(record(0x012D, &[0]));
        d.extend(record(0x02FA, &[5, 0, 0, 0, 0]));
        d.extend(record(0x012D, &[1]));
        // Rectangle: bottom, right, top, left.
        d.extend(record(0x041B, &[50, 50, 0, 0]));
        d.extend(record(0, &[]));
        let wmf = Wmf::parse(&d).unwrap();
        assert_eq!(wmf.window, [0.0, 0.0, 100.0, 100.0]);
        let pm = wmf.render(20, 20, &FontSystem::new()).unwrap();
        let at = |x: u32, y: u32| pm.pixel(x, y).unwrap();
        assert_eq!((at(4, 4).red(), at(4, 4).alpha()), (255, 255));
        assert_eq!(at(15, 15).alpha(), 0);
    }
}
