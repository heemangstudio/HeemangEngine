//! A table flowing in a text box's text: 1341000 p34 (Hancom PDF) draws it
//! where the box's lines leave room for it.

use hm_fonts::FontSystem;
use hm_model::{
    Cell, CharShape, Control, ControlKind, CtrlId, DocInfo, DocMeta, Document, Format, Matrix,
    ObjectCommon, ObjectControl, ObjectHeader, ObjectPart, PARA_END, PageDef, ParaShape, Paragraph,
    Section, SectionChild, SectionDef, Shape, ShapeComponent, ShapePart, Table, TextBox,
};

fn document() -> Document {
    let table = Table {
        header: ObjectHeader {
            common: ObjectCommon {
                // Anchored to its paragraph, top and bottom.
                attr: (2 << 3) | (1 << 21),
                width: 6000,
                height: 1200,
                ..Default::default()
            },
            ..Default::default()
        },
        rows: 1,
        cols: 1,
        cells: vec![Cell {
            col_span: 1,
            row_span: 1,
            width: 6000,
            height: 1200,
            paras: vec![Paragraph {
                text: vec!['A' as u16, PARA_END],
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    let text = TextBox {
        paras: vec![Paragraph {
            text: [vec![11; 8], vec![PARA_END]].concat(),
            controls: vec![Control {
                kind: ControlKind::Table(Box::new(table)),
                src: None,
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    let shape = Control {
        kind: ControlKind::Object(Box::new(ObjectControl {
            id: CtrlId::SHAPE,
            header: ObjectHeader {
                common: ObjectCommon {
                    attr: 1,
                    width: 8000,
                    height: 4000,
                    ..Default::default()
                },
                ..Default::default()
            },
            parts: vec![ObjectPart::Shape(Box::new(Shape {
                component: ShapeComponent {
                    kind: CtrlId::from_chars(b"$rec"),
                    doubled: true,
                    offset: (0, 0),
                    group_level: 0,
                    version: 0,
                    initial_size: (8000, 4000),
                    size: (8000, 4000),
                    flip: 0,
                    rotation: 0,
                    rotation_center: (0, 0),
                    translation: Matrix::IDENTITY,
                    transforms: Vec::new(),
                    style: None,
                    members: None,
                    group_instance: None,
                    tail: Default::default(),
                },
                parts: vec![ShapePart::Text(text)],
            }))],
        })),
        src: None,
    };
    let section = SectionDef {
        children: vec![SectionChild::Page(PageDef {
            width: 20000,
            height: 20000,
            ..Default::default()
        })],
        ..Default::default()
    };
    Document {
        meta: DocMeta {
            format: Format::Hwp5,
            version: Default::default(),
            flags: 0,
        },
        info: DocInfo {
            char_shapes: vec![CharShape {
                base_size: 1000,
                ratios: [100; 7],
                rel_sizes: [100; 7],
                ..Default::default()
            }],
            para_shapes: vec![ParaShape {
                line_spacing_legacy: 160,
                ..Default::default()
            }],
            ..Default::default()
        },
        sections: vec![Section {
            paras: vec![Paragraph {
                text: [vec![2; 8], vec![11; 8], vec![PARA_END]].concat(),
                controls: vec![
                    Control {
                        kind: ControlKind::Section(Box::new(section)),
                        src: None,
                    },
                    shape,
                ],
                ..Default::default()
            }],
            ..Default::default()
        }],
        source: None,
        bins: None,
        diagnostics: Vec::new(),
    }
}

#[test]
fn a_table_flowing_in_a_text_box_is_drawn() {
    let doc = document();
    let fonts = FontSystem::new();
    let layout = hm_layout::layout(&doc, &fonts);
    let cell = doc.sections[0].paras[0].controls[1]
        .as_object()
        .and_then(|o| o.shape())
        .and_then(|s| s.text())
        .and_then(|t| t.paras[0].controls[0].as_table())
        .map(|t| &t.cells[0] as *const Cell as usize)
        .unwrap();
    let cells = hm_render::page_cells(&doc, &layout, &fonts, 0);
    assert!(cells.iter().any(|c| c.cell == cell));
}
