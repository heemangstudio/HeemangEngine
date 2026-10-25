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

/// The box of an object or table drawn on a page, in drawing order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectBox {
    /// The control's address for drawing objects, the table's for tables;
    /// valid while the document is unchanged.
    pub key: usize,
    /// Page coordinates of the box without its outer margins; a table
    /// split over pages gives the fragment on this page.
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Records an object's or table's box when object boxes are wanted.
pub(crate) fn record_object(
    ctx: &text::Ctx<'_>,
    key: usize,
    (x, y, width, height): (i32, i32, i32, i32),
) {
    if let Some(sink) = ctx.objects {
        sink.borrow_mut().push(ObjectBox {
            key,
            x,
            y,
            width,
            height,
        });
    }
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
    objects: Option<&'a std::cell::RefCell<Vec<ObjectBox>>>,
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
            objects: None,
        },
        true,
    );
    cells.into_inner()
}

/// The objects and tables page `index` draws in every paragraph list,
/// bottom to top, without painting them.
pub fn page_objects(
    doc: &Document,
    layout: &DocLayout,
    fonts: &FontSystem,
    index: usize,
) -> Vec<ObjectBox> {
    let objects = std::cell::RefCell::new(Vec::new());
    build(
        doc,
        layout,
        fonts,
        index,
        IndexSinks {
            objects: Some(&objects),
            ..IndexSinks::default()
        },
        true,
    );
    objects.into_inner()
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
            objects: None,
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
            objects: None,
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
        objects: sinks.objects,
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
                paper_y: None,
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
        if !index_only {
            text::borders(
                &mut dl,
                &line_ctx,
                pal,
                &pal.lines[pl.line as usize],
                pl.x,
                pl.y,
                &placed,
            );
        }
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

#[cfg(test)]
mod body_text_tests {
    use super::*;
    use hm_model::{
        BorderFill, BorderLine, CharRun, CharShape, Control, ControlKind, DocMeta, Format, PageDef,
        ParaShape, Paragraph, Section, SectionChild, SectionDef,
    };

    #[test]
    fn character_borders_join_across_scripts_and_follow_the_logical_em() {
        let mut doc = Document {
            meta: DocMeta {
                format: Format::Hwp5,
                version: Default::default(),
                flags: 0,
            },
            info: Default::default(),
            source: None,
            bins: None,
            diagnostics: Vec::new(),
            sections: vec![Section {
                paras: vec![Paragraph {
                    text: "a가b".encode_utf16().chain([hm_model::PARA_END]).collect(),
                    runs: vec![CharRun { pos: 0, shape: 0 }, CharRun { pos: 2, shape: 1 }],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        let plain = CharShape {
            base_size: 1000,
            shade_color: u32::MAX,
            ..Default::default()
        };
        doc.info.char_shapes = vec![
            CharShape {
                border_fill: Some(1),
                offsets: [-20; 7],
                ..plain.clone()
            },
            plain,
        ];
        doc.info.para_shapes.push(ParaShape {
            line_spacing_legacy: 160,
            ..Default::default()
        });
        doc.info.border_fills.push(BorderFill {
            sides: [BorderLine {
                kind: 1,
                width: 2,
                color: 0,
            }; 4],
            ..Default::default()
        });
        doc.sections[0].paras[0].controls.push(Control {
            kind: ControlKind::Section(Box::new(SectionDef {
                children: vec![SectionChild::Page(PageDef {
                    width: 10000,
                    height: 10000,
                    ..Default::default()
                })],
                ..Default::default()
            })),
            src: None,
        });
        let fonts = FontSystem::new();
        let layout = hm_layout::layout(&doc, &fonts);
        let first = layout.pages[0].lines[0];
        let dl = build_page(&doc, &layout, &fonts, 0);
        let mut vertical = Vec::new();
        let mut horizontal = Vec::new();
        for item in &dl.items {
            if let Item::Line { x1, y1, x2, y2, .. } = *item {
                if x1 == x2 {
                    vertical.push(x1);
                }
                if y1 == y2 {
                    horizontal.push(y1);
                }
            }
        }
        vertical.sort();
        horizontal.sort();
        assert_eq!(vertical, [first.x, first.x + 1500]);
        assert_eq!(horizontal, [first.y, first.y + 1000]);
        assert_eq!(layout.para(0, 0).lines.len(), 1);
    }
}
