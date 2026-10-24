//! Display list and its backends.
//!
//! [`build_page`] turns one laid-out page into a [`DisplayList`] in integer
//! HU; [`raster::render`] draws it with tiny-skia at any dpi, [`svg::page`]
//! and [`pdf::Writer`] as vectors.

mod annot;
mod chart;
mod decor;
pub mod draw;
mod forms;
mod furniture;
pub mod image;
pub mod objects;
pub mod paint;
mod parabox;
pub mod pdf;
mod proof;
pub mod raster;
pub mod svg;
mod tabfill;
mod table;
mod text;
pub mod vector;

use hm_fonts::{FaceId, FontSystem};
use hm_layout::objects::{self as objects_layout, Layer};
use hm_layout::{DocLayout, Placed, PlacedTable};
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
    /// A filled and stroked path (drawing objects).
    Path(Box<draw::PathItem>),
    /// A picture.
    Image(Box<draw::ImageItem>),
    /// Glyphs stroked instead of filled (outline characters): stroke width
    /// and dash lengths in HU, solid when empty.
    Outline {
        run: GlyphRun,
        width: i32,
        dash: Vec<i32>,
    },
    /// Limits the items up to the matching [`Item::Unclip`] to a rectangle,
    /// inside any clip already in force.
    Clip {
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    },
    /// Ends the innermost [`Item::Clip`].
    Unclip,
}

/// One page of drawing commands in HU.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DisplayList {
    pub width: i32,
    pub height: i32,
    pub items: Vec<Item>,
}

/// Runs `draw` with what it adds limited to the rectangle `(x, y, w, h)`;
/// nothing is added when it draws nothing.
pub(crate) fn clipped(
    dl: &mut DisplayList,
    (x, y, w, h): (i32, i32, i32, i32),
    draw: impl FnOnce(&mut DisplayList),
) {
    let at = dl.items.len();
    dl.items.push(Item::Clip { x, y, w, h });
    draw(dl);
    if dl.items.len() == at + 1 {
        dl.items.pop();
    } else {
        dl.items.push(Item::Unclip);
    }
}

/// A line drawn on a page, for hit testing and the caret.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineBox {
    /// The paragraph's address, valid while the document is unchanged.
    pub para: usize,
    pub line: u32,
    /// Column origin and line top, page coordinates.
    pub x: i32,
    pub y: i32,
    pub height: i32,
    pub baseline: i32,
    /// `(wire position, page x)` of each item's left edge in order, then
    /// the line's end position at the right edge of its last item.
    pub stops: Vec<(u32, i32)>,
}

/// The visible box of a cell, including empty and nested cells.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellBox {
    /// The model cell's address, valid while the document is unchanged.
    pub cell: usize,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Default)]
struct IndexSinks<'a> {
    lines: Option<&'a std::cell::RefCell<Vec<LineBox>>>,
    cells: Option<&'a std::cell::RefCell<Vec<CellBox>>>,
}

/// Visible cell fragments in all paragraph lists, using the painter's actual
/// placement and clipping, without painting text or decoding pictures.
pub fn page_cells(
    doc: &Document,
    layout: &DocLayout,
    fonts: &FontSystem,
    index: usize,
) -> Vec<CellBox> {
    let cells = std::cell::RefCell::new(Vec::new());
    build(
        doc,
        layout,
        fonts,
        index,
        IndexSinks {
            lines: None,
            cells: Some(&cells),
        },
        true,
    );
    cells.into_inner()
}

/// Builds the display list of page `index`.
pub fn build_page(
    doc: &Document,
    layout: &DocLayout,
    fonts: &FontSystem,
    index: usize,
) -> DisplayList {
    build(doc, layout, fonts, index, IndexSinks::default(), false)
}

/// Builds the display list of page `index` with every line it draws.
pub fn build_page_indexed(
    doc: &Document,
    layout: &DocLayout,
    fonts: &FontSystem,
    index: usize,
) -> (DisplayList, Vec<LineBox>) {
    let sink = std::cell::RefCell::new(Vec::new());
    let dl = build(
        doc,
        layout,
        fonts,
        index,
        IndexSinks {
            lines: Some(&sink),
            cells: None,
        },
        false,
    );
    (dl, sink.into_inner())
}

/// The lines page `index` draws, without painting them: for hit testing
/// pages that are not on screen.
pub fn page_lines(
    doc: &Document,
    layout: &DocLayout,
    fonts: &FontSystem,
    index: usize,
) -> Vec<LineBox> {
    let sink = std::cell::RefCell::new(Vec::new());
    build(
        doc,
        layout,
        fonts,
        index,
        IndexSinks {
            lines: Some(&sink),
            cells: None,
        },
        true,
    );
    sink.into_inner()
}

fn build(
    doc: &Document,
    layout: &DocLayout,
    fonts: &FontSystem,
    index: usize,
    sinks: IndexSinks<'_>,
    index_only: bool,
) -> DisplayList {
    let page = &layout.pages[index];
    let mut dl = DisplayList {
        width: page.width,
        height: page.height,
        items: Vec::new(),
    };
    let ctx = text::Ctx {
        doc,
        styles: &layout.styles,
        fonts,
        page: Some(page),
        body_anchor: None,
        lines: sinks.lines,
        cells: sinks.cells,
        index_only,
    };
    let mut placed: Vec<Placed> = Vec::new();
    furniture::draw_under(&mut dl, &ctx, layout, page, &mut placed);
    parabox::draw(&mut dl, doc, layout, page);
    // Floating tables share the object layers: behind the text, or in
    // front of it, stacked with the drawing objects by z-order.
    let section = &doc.sections[page.section as usize];
    let table_layer = |t: &PlacedTable| {
        let o = section
            .paras
            .get(t.para as usize)
            .and_then(|p| p.controls.get(t.control as usize))
            .and_then(|c| c.object_common());
        match o {
            Some(o) if !o.treat_as_char() => (objects_layout::layer_of(&o), o.z_order),
            _ => (Layer::Text, 0),
        }
    };
    enum Floating<'a> {
        Object(&'a objects_layout::Placement),
        Table(&'a PlacedTable),
    }
    let floating = |dl: &mut DisplayList, front: bool| {
        let wanted = if front { Layer::Front } else { Layer::Behind };
        // Objects beside the text come before those in front of it, each
        // layer by z-order.
        let mut list: Vec<((bool, i32), Floating<'_>)> = page
            .objects
            .iter()
            .filter(|p| (p.layer == Layer::Behind) != front)
            .map(|p| ((p.layer == Layer::Front, p.z), Floating::Object(p)))
            .collect();
        for t in &page.tables {
            let (layer, z) = table_layer(t);
            if layer == wanted {
                list.push(((layer == Layer::Front, z), Floating::Table(t)));
            }
        }
        list.sort_by_key(|e| e.0);
        for (_, item) in list {
            match item {
                Floating::Object(p) => {
                    let para = &section.paras[p.para as usize];
                    let pal = layout.para(page.section, p.para);
                    objects::paint_placement(dl, &ctx, para, pal, p);
                }
                Floating::Table(t) => table::draw_fragment(dl, &ctx, layout, page.section, t),
            }
        }
    };
    floating(&mut dl, false);
    for t in &page.tables {
        if !matches!(table_layer(t).0, Layer::Behind | Layer::Front) {
            table::draw_fragment(&mut dl, &ctx, layout, page.section, t);
        }
    }
    let paras = &section.paras;
    for pl in &page.lines {
        let pal = layout.para(page.section, pl.para);
        let para = &paras[pl.para as usize];
        let line_ctx = text::Ctx {
            body_anchor: Some(objects_layout::Anchor {
                paper: (page.width, page.height),
                paper_x: None,
                body: page.body,
                column: (pl.x, pal.column_width),
                para_top: pl.anchor_y,
                para_left: pl.x + hm_layout::table::para_left(&doc.info, para),
            }),
            ..ctx
        };
        text::draw_line(
            &mut dl,
            &line_ctx,
            para,
            pal,
            pl.line as usize,
            pl.x,
            pl.y,
            &mut placed,
        );
    }
    floating(&mut dl, true);
    furniture::draw_over(&mut dl, &ctx, page, &mut placed);
    if index_only {
        dl.items.clear();
    } else {
        paint::resolve_images(&mut dl, doc);
    }
    dl
}
