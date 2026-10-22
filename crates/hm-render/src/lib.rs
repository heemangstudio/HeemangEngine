//! Display list and its backends.
//!
//! [`build_page`] turns one laid-out page into a [`DisplayList`] in integer
//! HU; [`raster::render`] draws it with tiny-skia at any dpi.

pub mod paint;
mod parabox;
pub mod raster;
mod table;
mod text;

use hm_fonts::{FaceId, FontSystem};
use hm_layout::{DocLayout, Placed};
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
    let ctx = text::Ctx {
        doc,
        styles: &layout.styles,
        fonts,
    };
    let mut placed: Vec<Placed> = Vec::new();
    let paras = &doc.sections[page.section as usize].paras;
    for pl in &page.lines {
        let pal = layout.para(page.section, pl.para);
        let para = &paras[pl.para as usize];
        text::draw_line(
            &mut dl,
            &ctx,
            para,
            pal,
            pl.line as usize,
            pl.x,
            pl.y,
            &mut placed,
        );
    }
    dl
}
