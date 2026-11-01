//! An object in a cell's paragraph paints by its wrapping: passport p1 (Hancom
//! PDF) draws the photo cell's text over the white ellipse behind it.

use hm_fonts::FontSystem;
use hm_model::{
    Cell, CharShape, Control, ControlKind, CtrlId, DocInfo, DocMeta, Document, Format, LineStyle,
    Matrix, ObjectCommon, ObjectControl, ObjectHeader, ObjectPart, PARA_END, PageDef, ParaShape,
    Paragraph, RectGeom, Section, SectionChild, SectionDef, Shape, ShapeComponent, ShapePart,
    ShapeStyle, Table,
};
use hm_render::{DisplayList, Item, build_page};

const SHADE: u32 = 0x0000_FF00;
const OUTLINE: u32 = 0x0000_00FF;

fn document(wrap: u32) -> Document {
    let shape = Control {
        kind: ControlKind::Object(Box::new(ObjectControl {
            id: CtrlId::SHAPE,
            header: ObjectHeader {
                common: ObjectCommon {
                    // Anchored to its paragraph.
                    attr: (2 << 3) | (wrap << 21),
                    width: 2000,
                    height: 1000,
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
                    initial_size: (2000, 1000),
                    size: (2000, 1000),
                    flip: 0,
                    rotation: 0,
                    rotation_center: (0, 0),
                    translation: Matrix::IDENTITY,
                    transforms: Vec::new(),
                    style: Some(ShapeStyle {
                        line: LineStyle {
                            color: OUTLINE,
                            width: 100,
                            attr: 1,
                            ..Default::default()
                        },
                        fill: Default::default(),
                        shadow: None,
                        instance_id: None,
                        reserved: None,
                        shadow_alpha: None,
                    }),
                    members: None,
                    group_instance: None,
                    tail: Default::default(),
                },
                parts: vec![ShapePart::Rect(RectGeom {
                    corners: [(0, 0), (2000, 0), (2000, 1000), (0, 1000)],
                    ..Default::default()
                })],
            }))],
        })),
        src: None,
    };
    let table = Table {
        header: ObjectHeader {
            common: ObjectCommon {
                attr: 1,
                width: 6000,
                height: 2000,
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
            height: 2000,
            paras: vec![Paragraph {
                text: [vec![11; 8], vec!['A' as u16, PARA_END]].concat(),
                controls: vec![shape],
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
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
                shade_color: SHADE,
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
                    Control {
                        kind: ControlKind::Table(Box::new(table)),
                        src: None,
                    },
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

fn order(dl: &DisplayList) -> (usize, usize) {
    let shade = dl
        .items
        .iter()
        .position(|i| matches!(i, Item::Rect { color, .. } if *color == SHADE));
    let object = dl
        .items
        .iter()
        .position(|i| matches!(i, Item::Path(p) if p.outline.is_some_and(|o| o.color == OUTLINE)));
    (
        shade.expect("the text's shade"),
        object.expect("the object"),
    )
}

#[test]
fn an_object_behind_a_cells_text_paints_under_it() {
    let fonts = FontSystem::new();
    for (wrap, under) in [(2, true), (3, false)] {
        let doc = document(wrap);
        let layout = hm_layout::layout(&doc, &fonts);
        let dl = build_page(&doc, &layout, &fonts, 0);
        let (shade, object) = order(&dl);
        assert_eq!(object < shade, under, "wrap {wrap}");
    }
}
