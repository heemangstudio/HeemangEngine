//! SVG backend: text as glyph outlines defined once and placed with
//! `<use>`, rules and paths as vectors, and pictures and brushes as PNG
//! patches rendered at a chosen resolution. Coordinates are page HU.

use std::collections::HashMap;
use std::fmt::Write;

use hm_fonts::FontSystem;

use crate::draw::PathEl;
use crate::paint::Brush;
use crate::vector::{self, n, rgb};
use crate::{DisplayList, GlyphRun, Item};

fn color(c: u32) -> String {
    let (r, g, b) = rgb(c);
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn path_data(els: &[PathEl]) -> String {
    let mut d = String::new();
    for e in els {
        let _ = match *e {
            PathEl::Move(x, y) => write!(d, "M{x} {y}"),
            PathEl::Line(x, y) => write!(d, "L{x} {y}"),
            PathEl::Cubic(a, b, c, e2, x, y) => write!(d, "C{a} {b} {c} {e2} {x} {y}"),
            PathEl::Close => write!(d, "Z"),
        };
    }
    d
}

fn dash_attr(out: &mut String, dash: Option<Vec<f32>>, round: bool) {
    if let Some(d) = dash {
        let list: Vec<String> = d.iter().map(|v| n(*v)).collect();
        let _ = write!(out, r#" stroke-dasharray="{}""#, list.join(" "));
    }
    if round {
        out.push_str(r#" stroke-linecap="round""#);
    }
}

struct Writer<'a> {
    fonts: &'a FontSystem,
    patch_dpi: f32,
    defs: String,
    body: String,
    glyphs: HashMap<(u32, u16), usize>,
    clips: usize,
}

impl Writer<'_> {
    /// The id of a glyph's outline in `<defs>`, defining it on first use.
    fn glyph(&mut self, face: &hm_fonts::Face, key: (u32, u16)) -> usize {
        let next = self.glyphs.len();
        *self.glyphs.entry(key).or_insert_with(|| {
            let _ = write!(
                self.defs,
                r#"<path id="g{next}" d="{}"/>"#,
                vector::outline_ops(face, key.1, true)
            );
            next
        })
    }

    fn run(&mut self, run: &GlyphRun, stroke: Option<(i32, &[i32])>) {
        let Some(face) = self.fonts.face(run.face) else {
            return;
        };
        let paint = match stroke {
            None if run.fake_bold => {
                // Stroke width in font units: the run's vertical scale.
                let scale = vector::glyph_matrix(run, &face, 0, 0).0[3].abs().max(1e-6);
                format!(
                    r#"fill="{c}" stroke="{c}" stroke-width="{}""#,
                    n(vector::fake_bold_width(run, &face) / scale),
                    c = color(run.color)
                )
            }
            None => format!(r#"fill="{}""#, color(run.color)),
            Some((width, dash)) => {
                let scale = vector::glyph_matrix(run, &face, 0, 0).0[3].abs().max(1e-6);
                let mut s = format!(
                    r#"fill="none" stroke="{}" stroke-width="{}" stroke-linecap="round" stroke-linejoin="round""#,
                    color(run.color),
                    n(vector::stroke_width(width) / scale)
                );
                if !dash.is_empty() {
                    let d: Vec<String> = dash.iter().map(|v| n(*v as f32 / scale)).collect();
                    let _ = write!(s, r#" stroke-dasharray="{}""#, d.join(" "));
                }
                s
            }
        };
        let _ = write!(self.body, "<g {paint}>");
        for &(glyph, x) in &run.glyphs {
            let id = self.glyph(&face, (run.face.0, glyph));
            let (m, _) = vector::glyph_matrix(run, &face, glyph, x);
            let _ = write!(
                self.body,
                r##"<use href="#g{id}" transform="matrix({} {} {} {} {} {})"/>"##,
                n(m[0]),
                n(m[1]),
                n(m[2]),
                n(m[3]),
                n(m[4]),
                n(m[5])
            );
        }
        self.body.push_str("</g>");
    }

    /// A picture as its own bytes (PNG, JPEG, GIF) or as PNG at its own
    /// size, clipped to its frame when cropped.
    fn picture(&mut self, i: &crate::draw::ImageItem) -> Option<()> {
        use crate::image::{Format, sniff};
        let stored = match sniff(&i.data) {
            Format::Png => Some("png"),
            Format::Jpeg => Some("jpeg"),
            Format::Gif => Some("gif"),
            _ => None,
        };
        let (mime, data, size) = match (stored, crate::image::dimensions(&i.data)) {
            (Some(mime), Some(size)) => (mime, i.data.to_vec(), size),
            _ => {
                let r = crate::image::decode(&i.data)?;
                let size = (r.w, r.h);
                ("png", r.into_pixmap()?.encode_png().ok()?, size)
            }
        };
        let m = vector::picture_matrix(i, size);
        let clip = !vector::uncropped(i, size);
        if clip {
            let id = self.clips;
            self.clips += 1;
            let pts: Vec<String> = vector::frame_corners(i)
                .iter()
                .map(|(x, y)| format!("{x},{y}"))
                .collect();
            let _ = write!(
                self.defs,
                r#"<clipPath id="c{id}"><polygon points="{}"/></clipPath>"#,
                pts.join(" ")
            );
            let _ = write!(self.body, r#"<g clip-path="url(#c{id})">"#);
        }
        let _ = write!(
            self.body,
            r#"<image width="1" height="1" preserveAspectRatio="none" transform="matrix({} {} {} {} {} {})" href="data:image/{mime};base64,{}"/>"#,
            n(m[0]),
            n(m[3]),
            n(m[1]),
            n(m[4]),
            n(m[2]),
            n(m[5]),
            base64(&data)
        );
        if clip {
            self.body.push_str("</g>");
        }
        Some(())
    }

    fn item(&mut self, item: &Item) {
        if let Some(i) = vector::plain_picture(item)
            && self.picture(i).is_some()
        {
            return;
        }
        if vector::patch_box(item).is_some() {
            if let Some(((x, y, w, h), pixmap)) = vector::patch(item, self.fonts, self.patch_dpi)
                && let Ok(png) = pixmap.encode_png()
            {
                let _ = write!(
                    self.body,
                    r#"<image x="{x}" y="{y}" width="{w}" height="{h}" preserveAspectRatio="none" href="data:image/png;base64,{}"/>"#,
                    base64(&png)
                );
            }
            return;
        }
        match item {
            Item::Glyphs(run) => self.run(run, None),
            Item::Outline { run, width, dash } => self.run(run, Some((*width, dash))),
            Item::Rect {
                x,
                y,
                w,
                h,
                color: c,
            } => {
                let _ = write!(
                    self.body,
                    r#"<rect x="{x}" y="{y}" width="{}" height="{}" fill="{}"/>"#,
                    (*w).max(1),
                    (*h).max(1),
                    color(*c)
                );
            }
            Item::Fill {
                x,
                y,
                w,
                h,
                brush: Brush::Solid(c),
            } => {
                let _ = write!(
                    self.body,
                    r#"<rect x="{x}" y="{y}" width="{w}" height="{h}" fill="{}"/>"#,
                    color(*c)
                );
            }
            Item::Line {
                x1,
                y1,
                x2,
                y2,
                stroke,
            } => {
                let width = stroke.width.max(1) as f32;
                let _ = write!(
                    self.body,
                    r#"<line x1="{x1}" y1="{y1}" x2="{x2}" y2="{y2}" stroke="{}" stroke-width="{}""#,
                    color(stroke.color),
                    n(width)
                );
                dash_attr(
                    &mut self.body,
                    vector::dashes(stroke.dash, width),
                    stroke.dash.round(),
                );
                self.body.push_str("/>");
            }
            Item::Path(p) => {
                let d = path_data(&p.path.els);
                let fill = match p.fill.first() {
                    Some(Brush::Solid(c)) => {
                        let mut s = format!(r#"fill="{}" fill-rule="evenodd""#, color(*c));
                        if p.fill_alpha > 0 {
                            let _ = write!(
                                s,
                                r#" fill-opacity="{}""#,
                                n(1.0 - p.fill_alpha as f32 / 255.0)
                            );
                        }
                        s
                    }
                    _ => r#"fill="none""#.to_string(),
                };
                let _ = write!(self.body, r#"<path d="{d}" {fill}"#);
                if let Some(o) = p.outline {
                    let width = vector::stroke_width(o.width);
                    let _ = write!(
                        self.body,
                        r#" stroke="{}" stroke-width="{}" stroke-miterlimit="4""#,
                        color(o.color),
                        n(width)
                    );
                    dash_attr(
                        &mut self.body,
                        vector::dashes(o.dash, width),
                        o.round_cap || o.dash.round(),
                    );
                }
                self.body.push_str("/>");
            }
            Item::Clip { x, y, w, h } => {
                let id = self.clips;
                self.clips += 1;
                let _ = write!(
                    self.defs,
                    r#"<clipPath id="c{id}"><rect x="{x}" y="{y}" width="{w}" height="{h}"/></clipPath>"#
                );
                let _ = write!(self.body, r#"<g clip-path="url(#c{id})">"#);
            }
            Item::Unclip => self.body.push_str("</g>"),
            Item::Fill { .. } | Item::Image(_) => {}
        }
    }
}

/// A page as an SVG document sized in inches, with pictures and brushes
/// rendered at `patch_dpi`.
pub fn page(dl: &DisplayList, fonts: &FontSystem, patch_dpi: f32) -> String {
    let (defs, body) = contents(dl, fonts, patch_dpi);
    format!(
        concat!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{}in" height="{}in" viewBox="0 0 {} {}">"#,
            r##"<rect width="100%" height="100%" fill="#fff"/><defs>{}</defs>{}</svg>"##
        ),
        n(dl.width as f32 / 7200.0),
        n(dl.height as f32 / 7200.0),
        dl.width,
        dl.height,
        defs,
        body
    )
}

/// A display list as a standalone SVG on a transparent background, sized
/// in CSS pixels at `dpi` (96 for one CSS pixel per screen pixel).
pub fn image(dl: &DisplayList, fonts: &FontSystem, dpi: f32) -> String {
    let (defs, body) = contents(dl, fonts, dpi);
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{}px" height="{}px" viewBox="0 0 {} {}"><defs>{}</defs>{}</svg>"#,
        n(dl.width as f32 * dpi / 7200.0),
        n(dl.height as f32 * dpi / 7200.0),
        dl.width,
        dl.height,
        defs,
        body
    )
}

/// The `<defs>` and body markup of a display list.
fn contents(dl: &DisplayList, fonts: &FontSystem, patch_dpi: f32) -> (String, String) {
    let mut w = Writer {
        fonts,
        patch_dpi,
        defs: String::new(),
        body: String::new(),
        glyphs: HashMap::new(),
        clips: 0,
    };
    let mut open = 0usize;
    for item in &dl.items {
        match item {
            Item::Clip { .. } => open += 1,
            Item::Unclip if open == 0 => continue,
            Item::Unclip => open -= 1,
            _ => {}
        }
        w.item(item);
    }
    for _ in 0..open {
        w.body.push_str("</g>");
    }
    (w.defs, w.body)
}

/// Standard base64 with padding.
pub(crate) fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let v = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for k in 0..4 {
            if k <= chunk.len() {
                out.push(T[(v >> (18 - 6 * k)) as usize & 63] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paint::{Dash, Stroke};

    #[test]
    fn base64_matches_the_standard_alphabet() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn rules_and_clips_become_svg_elements() {
        let dl = DisplayList {
            width: 7200,
            height: 3600,
            items: vec![
                Item::Clip {
                    x: 0,
                    y: 0,
                    w: 100,
                    h: 100,
                },
                Item::Line {
                    x1: 0,
                    y1: 10,
                    x2: 100,
                    y2: 10,
                    stroke: Stroke {
                        width: 20,
                        color: 0x0000FF,
                        dash: Dash::Dot,
                    },
                },
                Item::Unclip,
                Item::Unclip,
            ],
        };
        let svg = page(&dl, &FontSystem::new(), 96.0);
        assert!(svg.contains(r#"width="1in" height="0.5in""#));
        assert!(svg.contains(r##"stroke="#ff0000""##));
        assert!(svg.contains(r#"stroke-dasharray="20 40""#));
        assert_eq!(svg.matches("<g ").count(), svg.matches("</g>").count());
    }
}
