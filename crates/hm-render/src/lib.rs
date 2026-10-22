//! Display list and its backends.
//!
//! [`build_page`] turns one laid-out page into a [`DisplayList`] in integer
//! HU; [`raster::render`] draws it with tiny-skia at any dpi.

pub mod objects;
pub mod paint;
mod parabox;
pub mod raster;

use hm_fonts::{FaceId, FontSystem};
use hm_layout::{DocLayout, Kind, Placed, place};
use hm_model::Document;

pub use paint::{Brush, Dash, GradientBrush, Stroke};

/// A run of glyphs sharing face, size and color, on one baseline.
#[derive(Clone, Debug, PartialEq)]
pub struct GlyphRun {
    pub face: FaceId,
    /// Em size in HU.
    pub size: i32,
    /// Horizontal scale percent.
    pub ratio: i32,
    /// COLORREF (`0x00BBGGRR`).
    pub color: u32,
    pub fake_bold: bool,
    pub italic: bool,
    pub baseline: i32,
    /// `(glyph id, x)` pairs, x in HU.
    pub glyphs: Vec<(u16, i32)>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Glyphs(GlyphRun),
    /// A filled rectangle (underlines, strikes, placeholders).
    Rect {
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        color: u32,
    },
    /// A stroked segment, centered on the line between its end points.
    Line {
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        stroke: Stroke,
    },
    /// A rectangle painted with a brush.
    Fill {
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        brush: Brush,
    },
}

/// One page of drawing commands in HU.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DisplayList {
    pub width: i32,
    pub height: i32,
    pub items: Vec<Item>,
}

/// Builds the display list of page `index`.
pub fn build_page(
    doc: &Document,
    layout: &DocLayout,
    fonts: &FontSystem,
    index: usize,
) -> DisplayList {
    let page = &layout.pages[index];
    let mut dl = DisplayList {
        width: page.width,
        height: page.height,
        items: Vec::new(),
    };
    parabox::draw(&mut dl, doc, layout, page);
    let floating = |dl: &mut DisplayList, front: bool| {
        let mut list: Vec<&hm_layout::objects::Placement> = page
            .objects
            .iter()
            .filter(|p| (p.layer == hm_layout::objects::Layer::Behind) != front)
            .collect();
        list.sort_by_key(|p| p.z);
        for p in list {
            let para = &doc.sections[page.section as usize].paras[p.para as usize];
            objects::paint_placement(dl, fonts, para, layout.para(page.section, p.para), p);
        }
    };
    floating(&mut dl, false);
    let mut placed: Vec<Placed> = Vec::new();
    for pl in &page.lines {
        let para_layout = layout.para(page.section, pl.para);
        let line = &para_layout.lines[pl.line as usize];
        place(para_layout, line, &mut placed);
        let m = &para_layout.measured;
        let para = &doc.sections[page.section as usize].paras[pl.para as usize];
        highlights(&mut dl, para, pl, line, &placed);
        shades(&mut dl, layout, para_layout, pl, line, &placed);
        let mut run: Option<(u32, GlyphRun)> = None;
        for p in &placed {
            let Kind::Char(ch) = p.kind else { continue };
            let si = m.styles[p.pos as usize];
            let style = &layout.styles[si as usize];
            let Some(face_id) = style.face else { continue };
            let Some(glyph) = fonts.face(face_id).and_then(|f| f.glyph(ch)) else {
                continue;
            };
            let baseline = pl.y + para_layout.run_baseline(line, style.line_size) + style.offset;
            let fresh = run
                .as_ref()
                .is_none_or(|(s, r)| *s != si || r.baseline != baseline);
            if fresh {
                if let Some((_, r)) = run.take() {
                    dl.items.push(Item::Glyphs(r));
                }
                run = Some((
                    si,
                    GlyphRun {
                        face: face_id,
                        size: style.size,
                        ratio: style.ratio,
                        color: style.color,
                        fake_bold: style.fake_bold,
                        italic: style.italic,
                        baseline,
                        glyphs: Vec::new(),
                    },
                ));
            }
            run.as_mut()
                .unwrap()
                .1
                .glyphs
                .push((glyph.glyph, pl.x + p.x));
        }
        if let Some((_, r)) = run.take() {
            dl.items.push(Item::Glyphs(r));
        }
        objects::paint_inline(
            &mut dl,
            fonts,
            para,
            para_layout,
            line,
            &placed,
            (pl.x, pl.y),
        );
        if pl.line == 0
            && let Some(head) = &para_layout.head
        {
            let origin = pl.x + line.x - head.width;
            for &(ch, si, gx) in &head.glyphs {
                let style = &layout.styles[si as usize];
                let Some(face_id) = style.face else { continue };
                let Some(glyph) = fonts.face(face_id).and_then(|f| f.glyph(ch)) else {
                    continue;
                };
                dl.items.push(Item::Glyphs(GlyphRun {
                    face: face_id,
                    size: style.size,
                    ratio: style.ratio,
                    color: style.color,
                    fake_bold: style.fake_bold,
                    italic: style.italic,
                    baseline: pl.y + para_layout.run_baseline(line, style.line_size) + style.offset,
                    glyphs: vec![(glyph.glyph, origin + gx)],
                }));
            }
        }
        decorations(&mut dl, layout, para_layout, pl, line, &placed);
    }
    floating(&mut dl, true);
    dl
}

/// Highlighter range tags (kind 2, end exclusive) behind the glyphs.
fn highlights(
    dl: &mut DisplayList,
    para: &hm_model::Paragraph,
    pl: &hm_layout::PlacedLine,
    line: &hm_layout::Line,
    placed: &[Placed],
) {
    for tag in para.range_tags.iter().filter(|t| t.tag >> 24 == 2) {
        for p in placed
            .iter()
            .filter(|p| p.pos >= tag.start && p.pos < tag.end)
        {
            dl.items.push(Item::Rect {
                x: pl.x + p.x,
                y: pl.y,
                w: p.advance,
                h: line.height,
                color: tag.tag & 0x00FF_FFFF,
            });
        }
    }
}

/// Character shading behind the glyphs, one rectangle per character.
fn shades(
    dl: &mut DisplayList,
    layout: &DocLayout,
    para_layout: &hm_layout::ParaLayout,
    pl: &hm_layout::PlacedLine,
    line: &hm_layout::Line,
    placed: &[Placed],
) {
    let m = &para_layout.measured;
    for p in placed {
        if !matches!(p.kind, Kind::Char(_) | Kind::Space) {
            continue;
        }
        if let Some(color) = layout.styles[m.styles[p.pos as usize] as usize].shade {
            dl.items.push(Item::Rect {
                x: pl.x + p.x,
                y: pl.y,
                w: p.advance,
                h: line.height,
                color,
            });
        }
    }
}

/// Underlines and strikes, one rectangle per styled stretch.
fn decorations(
    dl: &mut DisplayList,
    layout: &DocLayout,
    pl_layout: &hm_layout::ParaLayout,
    pl: &hm_layout::PlacedLine,
    line: &hm_layout::Line,
    placed: &[Placed],
) {
    let m = &pl_layout.measured;
    for p in placed {
        if !matches!(p.kind, Kind::Char(_) | Kind::Space) {
            continue;
        }
        let style = &layout.styles[m.styles[p.pos as usize] as usize];
        let thickness = (style.size / 15).max(7);
        let baseline = pl.y + pl_layout.run_baseline(line, style.line_size);
        if style.underline == 1 {
            let y = baseline + style.size / 10;
            dl.items.push(Item::Rect {
                x: pl.x + p.x,
                y,
                w: p.advance,
                h: thickness,
                color: style.underline_color,
            });
        }
        if style.strike {
            let y = baseline - style.size * 3 / 10;
            dl.items.push(Item::Rect {
                x: pl.x + p.x,
                y,
                w: p.advance,
                h: thickness,
                color: style.color,
            });
        }
    }
}
