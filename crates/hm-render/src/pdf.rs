//! PDF backend: one page per display list, text as glyph outlines kept
//! once per document as form XObjects, rules and paths as vectors, and
//! pictures and brushes as image patches rendered at a chosen resolution.
//! Content streams draw in page HU, y down, under one flipping matrix.

use std::collections::HashMap;
use std::fmt::Write;

use hm_fonts::FontSystem;

use crate::draw::PathEl;
use crate::paint::Brush;
use crate::vector::{self, n, rgb};
use crate::{DisplayList, GlyphRun, Item};

/// How a glyph form paints its outline.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Mode {
    Fill,
    FillStroke,
    Stroke,
}

/// A PDF file being written object by object.
pub struct Writer<'a> {
    fonts: &'a FontSystem,
    patch_dpi: f32,
    out: Vec<u8>,
    offsets: Vec<usize>,
    pages: Vec<usize>,
    /// Glyph forms and pictures by resource name, with their objects.
    xobjects: Vec<(String, usize)>,
    glyphs: HashMap<(u32, u16, Mode), String>,
    alphas: Vec<(String, f32)>,
    resources: usize,
    pages_id: usize,
}

fn rg(c: u32) -> String {
    let (r, g, b) = rgb(c);
    format!(
        "{} {} {}",
        n(r as f32 / 255.0),
        n(g as f32 / 255.0),
        n(b as f32 / 255.0)
    )
}

fn deflate(data: &[u8]) -> Vec<u8> {
    miniz_oxide::deflate::compress_to_vec_zlib(data, 1)
}

impl<'a> Writer<'a> {
    /// Starts a document whose pictures and brushes are rendered at
    /// `patch_dpi`.
    pub fn new(fonts: &'a FontSystem, patch_dpi: f32) -> Self {
        let mut w = Writer {
            fonts,
            patch_dpi,
            out: b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec(),
            offsets: Vec::new(),
            pages: Vec::new(),
            xobjects: Vec::new(),
            glyphs: HashMap::new(),
            alphas: Vec::new(),
            resources: 0,
            pages_id: 0,
        };
        // Ids fixed up front: the page tree and the shared resources.
        w.pages_id = w.reserve();
        w.resources = w.reserve();
        w
    }

    fn reserve(&mut self) -> usize {
        self.offsets.push(0);
        self.offsets.len()
    }

    fn object(&mut self, id: usize, body: &[u8]) {
        self.offsets[id - 1] = self.out.len();
        self.out
            .extend_from_slice(format!("{id} 0 obj\n").as_bytes());
        self.out.extend_from_slice(body);
        self.out.extend_from_slice(b"\nendobj\n");
    }

    fn stream(&mut self, id: usize, dict: &str, data: &[u8]) {
        let z = deflate(data);
        let mut body = format!(
            "<<{dict} /Filter /FlateDecode /Length {}>>\nstream\n",
            z.len()
        )
        .into_bytes();
        body.extend_from_slice(&z);
        body.extend_from_slice(b"\nendstream");
        self.object(id, &body);
    }

    /// The resource name of a glyph's form, writing it on first use.
    fn glyph(&mut self, face: &hm_fonts::Face, key: (u32, u16, Mode)) -> String {
        if let Some(name) = self.glyphs.get(&key) {
            return name.clone();
        }
        let name = format!("G{}", self.glyphs.len());
        let id = self.reserve();
        let op = match key.2 {
            Mode::Fill => "f",
            Mode::FillStroke => "B",
            Mode::Stroke => "S",
        };
        let ops = format!("{}{op}", vector::outline_ops(face, key.1, false));
        let em = face.units_per_em.max(1) as i32 * 4;
        self.stream(
            id,
            &format!("/Type /XObject /Subtype /Form /BBox [-{em} -{em} {em} {em}]"),
            ops.as_bytes(),
        );
        self.glyphs.insert(key, name.clone());
        self.xobjects.push((name.clone(), id));
        name
    }

    fn alpha(&mut self, a: f32) -> String {
        if let Some((name, _)) = self.alphas.iter().find(|x| x.1 == a) {
            return name.clone();
        }
        let name = format!("A{}", self.alphas.len());
        self.alphas.push((name.clone(), a));
        name
    }

    fn run(&mut self, s: &mut String, run: &GlyphRun, stroke: Option<(i32, &[i32])>) {
        let Some(face) = self.fonts.face(run.face) else {
            return;
        };
        let scale = vector::glyph_matrix(run, &face, 0, 0).0[3].abs().max(1e-6);
        let mode = match stroke {
            None if run.fake_bold => {
                let _ = writeln!(s, "{} w", n(vector::fake_bold_width(run, &face) / scale));
                Mode::FillStroke
            }
            None => Mode::Fill,
            Some((width, dash)) => {
                let _ = writeln!(s, "{} w 1 J 1 j", n(vector::stroke_width(width) / scale));
                if !dash.is_empty() {
                    let d: Vec<String> = dash.iter().map(|v| n(*v as f32 / scale)).collect();
                    let _ = writeln!(s, "[{}] 0 d", d.join(" "));
                }
                Mode::Stroke
            }
        };
        let _ = writeln!(s, "{c} rg {c} RG", c = rg(run.color));
        for &(glyph, x) in &run.glyphs {
            let name = self.glyph(&face, (run.face.0, glyph, mode));
            let (m, _) = vector::glyph_matrix(run, &face, glyph, x);
            let _ = writeln!(
                s,
                "q {} {} {} {} {} {} cm /{name} Do Q",
                n(m[0]),
                n(m[1]),
                n(m[2]),
                n(m[3]),
                n(m[4]),
                n(m[5])
            );
        }
        if stroke.is_some() {
            s.push_str("[] 0 d 0 J 0 j\n");
        }
    }

    fn patch(&mut self, s: &mut String, item: &Item) {
        let Some(((x, y, w, h), pixmap)) = vector::patch(item, self.fonts, self.patch_dpi) else {
            return;
        };
        let (pw, ph) = (pixmap.width(), pixmap.height());
        let id = self.image(
            pw,
            ph,
            pixmap.pixels().iter().map(|p| {
                let c = p.demultiply();
                [c.red(), c.green(), c.blue(), c.alpha()]
            }),
        );
        let name = format!("I{}", self.xobjects.len());
        self.xobjects.push((name.clone(), id));
        // The image's unit square, flipped back under the page's y-down
        // matrix.
        let _ = writeln!(s, "q {w} 0 0 -{h} {x} {} cm /{name} Do Q", y + h);
    }

    /// A picture at its own size, clipped to its frame when cropped: a
    /// JPEG as stored, others decoded.
    fn picture(&mut self, s: &mut String, i: &crate::draw::ImageItem) -> Option<()> {
        use crate::image::{Format, sniff};
        let jpeg = (sniff(&i.data) == Format::Jpeg)
            .then(|| vector::jpeg_components(&i.data))
            .flatten()
            .filter(|c| matches!(c, 1 | 3));
        let (id, size) = match (jpeg, crate::image::dimensions(&i.data)) {
            (Some(comps), Some((pw, ph))) => {
                let id = self.reserve();
                let space = if comps == 1 {
                    "DeviceGray"
                } else {
                    "DeviceRGB"
                };
                let mut body = format!(
                    "<</Type /XObject /Subtype /Image /Width {pw} /Height {ph} /ColorSpace /{space} /BitsPerComponent 8 /Filter /DCTDecode /Length {}>>\nstream\n",
                    i.data.len()
                )
                .into_bytes();
                body.extend_from_slice(&i.data);
                body.extend_from_slice(b"\nendstream");
                self.object(id, &body);
                (id, (pw, ph))
            }
            _ => {
                let r = crate::image::decode(&i.data)?;
                let id = self.image(
                    r.w,
                    r.h,
                    r.rgba.chunks_exact(4).map(|p| [p[0], p[1], p[2], p[3]]),
                );
                (id, (r.w, r.h))
            }
        };
        let name = format!("I{}", self.xobjects.len());
        self.xobjects.push((name.clone(), id));
        s.push_str("q ");
        if !vector::uncropped(i, size) {
            let c = vector::frame_corners(i);
            let _ = write!(
                s,
                "{} {} m {} {} l {} {} l {} {} l h W n ",
                c[0].0, c[0].1, c[1].0, c[1].1, c[2].0, c[2].1, c[3].0, c[3].1
            );
        }
        // Image space runs bottom up: flip the picture's t axis.
        let [a, b, c, d, e, f] = vector::picture_matrix(i, size);
        let _ = writeln!(
            s,
            "{} {} {} {} {} {} cm /{name} Do Q",
            n(a),
            n(d),
            n(-b),
            n(-e),
            n(b + c),
            n(e + f)
        );
        Some(())
    }

    /// An image object from straight RGBA pixels, with a soft mask when
    /// any pixel is not opaque.
    fn image(&mut self, w: u32, h: u32, pixels: impl Iterator<Item = [u8; 4]>) -> usize {
        let mut color = Vec::with_capacity((w * h * 3) as usize);
        let mut alpha = Vec::with_capacity((w * h) as usize);
        for [r, g, b, a] in pixels {
            color.extend_from_slice(&[r, g, b]);
            alpha.push(a);
        }
        let mask = if alpha.iter().all(|&a| a == 255) {
            String::new()
        } else {
            let mask = self.reserve();
            self.stream(
                mask,
                &format!(
                    "/Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceGray /BitsPerComponent 8"
                ),
                &alpha,
            );
            format!(" /SMask {mask} 0 R")
        };
        let id = self.reserve();
        self.stream(
            id,
            &format!(
                "/Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceRGB /BitsPerComponent 8{mask}"
            ),
            &color,
        );
        id
    }

    fn item(&mut self, s: &mut String, item: &Item) {
        if let Some(i) = vector::plain_picture(item)
            && self.picture(s, i).is_some()
        {
            return;
        }
        if vector::patch_box(item).is_some() {
            self.patch(s, item);
            return;
        }
        match item {
            Item::Glyphs(run) => self.run(s, run, None),
            Item::Outline { run, width, dash } => self.run(s, run, Some((*width, dash))),
            Item::Rect { x, y, w, h, color } => {
                let _ = writeln!(
                    s,
                    "{} rg {x} {y} {} {} re f",
                    rg(*color),
                    w.max(&1),
                    h.max(&1)
                );
            }
            Item::Fill {
                x,
                y,
                w,
                h,
                brush: Brush::Solid(c),
            } => {
                let _ = writeln!(s, "{} rg {x} {y} {w} {h} re f", rg(*c));
            }
            Item::Line {
                x1,
                y1,
                x2,
                y2,
                stroke,
            } => {
                let width = stroke.width.max(1) as f32;
                let _ = write!(s, "{} RG {} w ", rg(stroke.color), n(width));
                if let Some(d) = vector::dashes(stroke.dash, width) {
                    let d: Vec<String> = d.iter().map(|v| n(*v)).collect();
                    let _ = write!(s, "[{}] 0 d ", d.join(" "));
                }
                let cap = if stroke.dash.round() { 1 } else { 0 };
                let _ = writeln!(s, "{cap} J {x1} {y1} m {x2} {y2} l S [] 0 d 0 J");
            }
            Item::Path(p) => {
                let mut d = String::new();
                for e in &p.path.els {
                    let _ = match *e {
                        PathEl::Move(x, y) => writeln!(d, "{x} {y} m"),
                        PathEl::Line(x, y) => writeln!(d, "{x} {y} l"),
                        PathEl::Cubic(a, b, c, e2, x, y) => {
                            writeln!(d, "{a} {b} {c} {e2} {x} {y} c")
                        }
                        PathEl::Close => writeln!(d, "h"),
                    };
                }
                if let Some(Brush::Solid(c)) = p.fill.first() {
                    s.push_str("q ");
                    if p.fill_alpha > 0 {
                        let gs = self.alpha(1.0 - p.fill_alpha as f32 / 255.0);
                        let _ = write!(s, "/{gs} gs ");
                    }
                    let _ = writeln!(s, "{} rg\n{d}f* Q", rg(*c));
                }
                if let Some(o) = p.outline {
                    let width = vector::stroke_width(o.width);
                    let _ = write!(s, "{} RG {} w ", rg(o.color), n(width));
                    if let Some(dash) = vector::dashes(o.dash, width) {
                        let dash: Vec<String> = dash.iter().map(|v| n(*v)).collect();
                        let _ = write!(s, "[{}] 0 d ", dash.join(" "));
                    }
                    let cap = if o.round_cap || o.dash.round() { 1 } else { 0 };
                    let _ = writeln!(s, "{cap} J\n{d}S [] 0 d 0 J");
                }
            }
            Item::Clip { x, y, w, h } => {
                let _ = writeln!(s, "q {x} {y} {w} {h} re W n");
            }
            Item::Unclip => s.push_str("Q\n"),
            Item::Fill { .. } | Item::Image(_) => {}
        }
    }

    /// Adds a page drawn from a display list.
    pub fn page(&mut self, dl: &DisplayList) {
        let (wpt, hpt) = (dl.width as f32 / 100.0, dl.height as f32 / 100.0);
        let mut s = format!("0.01 0 0 -0.01 0 {} cm\n", n(hpt));
        let mut open = 0usize;
        for item in &dl.items {
            match item {
                Item::Clip { .. } => open += 1,
                Item::Unclip if open == 0 => continue,
                Item::Unclip => open -= 1,
                _ => {}
            }
            self.item(&mut s, item);
        }
        for _ in 0..open {
            s.push_str("Q\n");
        }
        let content = self.reserve();
        self.stream(content, "", s.as_bytes());
        let id = self.reserve();
        let body = format!(
            "<</Type /Page /Parent {} 0 R /MediaBox [0 0 {} {}] /Resources {} 0 R /Contents {content} 0 R>>",
            self.pages_id,
            n(wpt),
            n(hpt),
            self.resources
        );
        self.object(id, body.as_bytes());
        self.pages.push(id);
    }

    /// The finished file.
    pub fn finish(mut self) -> Vec<u8> {
        let mut res = String::from("<</XObject <<");
        for (name, id) in &self.xobjects {
            let _ = write!(res, "/{name} {id} 0 R ");
        }
        res.push_str(">> /ExtGState <<");
        for (name, a) in &self.alphas {
            let _ = write!(res, "/{name} <</ca {}>> ", n(*a));
        }
        res.push_str(">>>>");
        let resources = self.resources;
        self.object(resources, res.as_bytes());
        let kids: Vec<String> = self.pages.iter().map(|p| format!("{p} 0 R")).collect();
        let tree = format!(
            "<</Type /Pages /Kids [{}] /Count {}>>",
            kids.join(" "),
            self.pages.len()
        );
        let pages_id = self.pages_id;
        self.object(pages_id, tree.as_bytes());
        let catalog = self.reserve();
        self.object(
            catalog,
            format!("<</Type /Catalog /Pages {pages_id} 0 R>>").as_bytes(),
        );
        let xref = self.out.len();
        let mut tail = format!("xref\n0 {}\n0000000000 65535 f \n", self.offsets.len() + 1);
        for o in &self.offsets {
            let _ = writeln!(tail, "{o:010} 00000 n ");
        }
        let _ = write!(
            tail,
            "trailer\n<</Size {} /Root {catalog} 0 R>>\nstartxref\n{xref}\n%%EOF\n",
            self.offsets.len() + 1
        );
        self.out.extend_from_slice(tail.as_bytes());
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_a_well_formed_file() {
        let fonts = FontSystem::new();
        let mut w = Writer::new(&fonts, 96.0);
        for _ in 0..2 {
            w.page(&DisplayList {
                width: 59528,
                height: 84188,
                items: vec![Item::Rect {
                    x: 7200,
                    y: 7200,
                    w: 1000,
                    h: 50,
                    color: 0,
                }],
            });
        }
        let pdf = w.finish();
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.starts_with("%PDF-1.4"));
        assert!(text.contains("/Count 2"));
        assert!(text.contains("/MediaBox [0 0 595.28 841.88]"));
        // Every xref offset points at its object.
        let xref = text.rfind("xref\n").unwrap();
        for (k, line) in text[xref..]
            .lines()
            .skip(3)
            .take_while(|l| l.ends_with(" n "))
            .enumerate()
        {
            let off: usize = line[..10].parse().unwrap();
            assert!(pdf[off..].starts_with(format!("{} 0 obj", k + 1).as_bytes()));
        }
    }
}
