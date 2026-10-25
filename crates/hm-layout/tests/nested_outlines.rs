use hm_layout::{DocLayout, LayoutCache, Options, ParaLayout};
use hm_model::*;

fn paragraph(controls: Vec<Control>) -> Paragraph {
    let mut text = Vec::new();
    for c in &controls {
        let code = if c.as_section().is_some() { 2 } else { 11 };
        text.extend([code, 0, 0, 0, 0, 0, 0, code]);
    }
    text.push(PARA_END);
    Paragraph {
        text,
        controls,
        runs: vec![CharRun { pos: 0, shape: 0 }],
        ..Default::default()
    }
}

fn table(para: Paragraph, width: i32) -> Control {
    Control {
        kind: ControlKind::Table(Box::new(Table {
            header: ObjectHeader {
                common: ObjectCommon {
                    attr: 1 | (4 << 15) | (2 << 18),
                    width,
                    height: 1000,
                    ..Default::default()
                },
                ..Default::default()
            },
            rows: 1,
            cols: 1,
            row_cells: vec![1],
            cells: vec![Cell {
                col_span: 1,
                row_span: 1,
                width,
                height: 1000,
                paras: vec![para],
                ..Default::default()
            }],
            ..Default::default()
        })),
        src: None,
    }
}

fn nested(layout: &DocLayout, section: u32) -> &ParaLayout {
    layout
        .para(section, 1)
        .table(0)
        .unwrap()
        .cells
        .first()
        .expect("outer cell laid out")
        .paras
        .first()
        .expect("cell paragraph laid out")
}

fn head_text(para: &ParaLayout) -> String {
    para.head
        .as_ref()
        .map(|h| h.glyphs.iter().map(|g| g.0).collect())
        .unwrap_or_default()
}

#[test]
fn section_outlines_precede_inline_tables_and_refresh_cached_nested_layouts() {
    let mut cell = paragraph(vec![table(paragraph(Vec::new()), 2000)]);
    cell.header.para_shape = 1;
    let sections = (1..=2)
        .map(|outline_numbering| Section {
            paras: vec![
                paragraph(vec![Control {
                    kind: ControlKind::Section(Box::new(SectionDef {
                        outline_numbering,
                        children: vec![SectionChild::Page(PageDef {
                            width: 12000,
                            height: 16000,
                            ..Default::default()
                        })],
                        ..Default::default()
                    })),
                    src: None,
                }]),
                paragraph(vec![table(cell.clone(), 8000)]),
            ],
            ..Default::default()
        })
        .collect();
    let original = Document {
        meta: DocMeta {
            format: Format::Hwpx,
            version: Version(0),
            flags: 0,
        },
        info: DocInfo {
            para_shapes: vec![
                ParaShape::default(),
                ParaShape {
                    attr1: 1 << 23,
                    ..Default::default()
                },
            ],
            char_shapes: vec![CharShape {
                base_size: 1000,
                ratios: [100; 7],
                ..Default::default()
            }],
            numberings: [5, 9]
                .into_iter()
                .map(|start| Numbering {
                    start: Some(start),
                    level_starts: Some([u32::from(start); 7]),
                    levels: vec![NumberingLevel {
                        format: HwpString::from_text("^1."),
                        head: ParaHead {
                            text_distance: 50,
                            ..Default::default()
                        },
                    }],
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        },
        sections,
        source: None,
        bins: None,
        diagnostics: Vec::new(),
    };
    let mut doc = hm_hwpx::read(SharedBytes::from(hm_hwpx::write_new(&original).unwrap())).unwrap();
    let fonts = hm_fonts::FontSystem::new();
    let mut cache = LayoutCache::new();
    let layout = hm_layout::layout_cached(&doc, &fonts, Options::default(), &mut cache);
    for (section, expected) in [(0, "5."), (1, "9.")] {
        let para = nested(&layout, section);
        assert_eq!(head_text(para), expected);
        let mut placed = Vec::new();
        hm_layout::place(para, &para.lines[0], &mut placed);
        let table = placed
            .iter()
            .find(|p| matches!(p.kind, hm_layout::Kind::Object(_)))
            .unwrap();
        assert!(table.x >= para.lines[0].seg_x + para.head.as_ref().unwrap().width);
    }
    hm_layout::layout_cached(&doc, &fonts, Options::default(), &mut cache);
    assert!(cache.stats().0 > 0, "the source paragraphs are cacheable");
    let ControlKind::Section(def) = &mut doc.sections[0].paras[0].controls[0].kind else {
        unreachable!()
    };
    def.outline_numbering = 2;
    doc.sections[0].paras[0].mark_all_dirty();
    let changed = hm_layout::layout_cached(&doc, &fonts, Options::default(), &mut cache);
    assert_eq!(head_text(nested(&changed, 0)), "9.");
}
