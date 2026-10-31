use hm_fonts::FontSystem;
use hm_model::*;

fn paragraph(text: &str, cached: bool, reset: bool) -> Paragraph {
    let mut p = Paragraph {
        header: ParaHeader {
            para_shape: 1,
            ..Default::default()
        },
        text: text.encode_utf16().chain([PARA_END]).collect(),
        ..Default::default()
    };
    if cached {
        p.src = Some(SharedBytes::from(vec![1]));
        p.line_segs = vec![
            LineSeg {
                text_start: 0,
                vpos: 200,
                height: 1000,
                text_height: 1000,
                baseline: 850,
                spacing: 300,
                col_start: 0,
                width: 5000,
                flags: 2490368,
            },
            LineSeg {
                text_start: 2,
                vpos: if reset { 0 } else { 1500 },
                height: 1000,
                text_height: 1000,
                baseline: 850,
                spacing: 300,
                col_start: 0,
                width: 5000,
                flags: 1441792,
            },
        ];
    }
    p
}

fn document(spans: (u16, u16), reset: bool, source: bool, edited: bool, split: u32) -> Document {
    let mut cells = Vec::new();
    let cell = |row, col, span, height, paras| Cell {
        row,
        col,
        row_span: span,
        col_span: 1,
        width: 5000,
        height,
        list_attr: 1 << 5,
        paras,
        ..Default::default()
    };
    for col in 0..3 {
        cells.push(cell(
            0,
            col,
            1,
            8034,
            vec![Paragraph {
                text: "prefix\r".encode_utf16().collect(),
                ..Default::default()
            }],
        ));
    }
    let mut merged = paragraph("A\nB", true, reset);
    if !source {
        merged.src = None;
    }
    if edited {
        merged.text = "AA\nBB\r".encode_utf16().collect();
        merged.mark_all_dirty();
    }
    cells.push(cell(
        1,
        0,
        spans.0,
        if spans.0 == 25 { 66273 } else { 7000 },
        vec![merged],
    ));
    cells.push(cell(
        1,
        1,
        spans.1,
        if spans.1 == 7 { 11674 } else { 4000 },
        vec![paragraph("C", false, false)],
    ));
    for row in 1..=spans.0 {
        if row > spans.1 {
            cells.push(cell(row, 1, 1, 1482, vec![paragraph("C", false, false)]));
        }
        cells.push(cell(
            row,
            2,
            1,
            1482,
            vec![if row == 1 {
                paragraph("D\nE", true, reset)
            } else {
                paragraph("D", false, false)
            }],
        ));
    }
    let table = Table {
        header: ObjectHeader {
            common: ObjectCommon {
                attr: (2 << 3) | (1 << 21),
                width: 15000,
                ..Default::default()
            },
            ..Default::default()
        },
        attr: split,
        rows: spans.0 + 1,
        cols: 3,
        padding: [0, 0, 141, 141],
        cells,
        ..Default::default()
    };
    let section = SectionDef {
        children: vec![SectionChild::Page(PageDef {
            width: 20000,
            height: 10000,
            ..Default::default()
        })],
        ..Default::default()
    };
    Document {
        meta: DocMeta {
            format: Format::Hwp5,
            version: Version(0x05010100),
            flags: 0,
        },
        info: DocInfo {
            char_shapes: vec![CharShape {
                base_size: 1000,
                ratios: [100; 7],
                rel_sizes: [100; 7],
                ..Default::default()
            }],
            para_shapes: vec![
                ParaShape {
                    line_spacing_legacy: 100,
                    ..Default::default()
                },
                ParaShape {
                    space_before: units::pack_distance(200),
                    line_spacing_legacy: 130,
                    ..Default::default()
                },
            ],
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

fn source_lines(doc: &Document, layout: &hm_layout::DocLayout, cell: usize) -> Vec<(usize, u32)> {
    let fonts = FontSystem::new();
    let table = doc.sections[0].paras[0].controls[1].as_table().unwrap();
    let id = &table.cells[cell].paras[0] as *const Paragraph as usize;
    (0..layout.pages.len())
        .flat_map(|p| {
            hm_render::page_lines(doc, layout, &fonts, p)
                .into_iter()
                .filter(move |l| l.para == id)
                .map(move |l| (p, l.line))
        })
        .collect()
}

#[test]
fn a_new_merged_cell_keeps_its_fitting_source_prefix_before_a_cached_page_restart() {
    for spans in [(25, 7), (3, 2)] {
        let doc = document(spans, true, true, false, 2);
        let layout = hm_layout::layout(&doc, &FontSystem::new());
        for (page, placed) in layout.pages.iter().take(2).enumerate() {
            let pt = &placed.tables[0];
            eprintln!(
                "ownership page={page} top={} bottom={} height={} carried={:?}",
                pt.top,
                pt.bottom,
                pt.height(),
                pt.carried
            );
        }
        for cell in [3, 5] {
            assert_eq!(
                source_lines(&doc, &layout, cell),
                [(0, 0), (1, 1)],
                "spans={spans:?},cell={cell}"
            );
        }
        let first = &layout.pages[0].tables[0];
        assert!(first.bottom > 8034);
        assert!(first.height() >= 8034 + 1482 && first.height() <= 10000);
        let carried = layout.pages[1].tables[0].carried.as_ref().unwrap();
        assert!(
            carried
                .cells
                .iter()
                .any(|&(k, first)| k == 3 && first.is_some())
        );
    }
}

#[test]
fn merged_prefix_admission_requires_an_unchanged_cell_cache_restart_and_cell_splitting() {
    for (name, reset, source, edited, split) in [
        ("monotonic", false, true, false, 2),
        ("no-source", true, false, false, 2),
        ("edited", true, true, true, 2),
        ("ROWS", true, true, false, 1),
        ("Never", true, true, false, 0),
    ] {
        // The ROWS group fits a fresh column, so its existing oversized-row
        // fallback does not turn this negative into an inside-cell cut.
        let spans = if split == 2 { (25, 7) } else { (3, 2) };
        let doc = document(spans, reset, source, edited, split);
        let layout = hm_layout::layout(&doc, &FontSystem::new());
        assert_eq!(source_lines(&doc, &layout, 3), [(1, 0), (1, 1)], "{name}");
        if split == 0 {
            // A table that never splits goes whole to the next page even
            // when taller than it, anchored in the first paragraph too
            // (probe c-none-taller-first, Hancom 2022 on windows1).
            assert!(layout.pages[0].tables.is_empty(), "{name}");
            let t = &layout.pages[1].tables[0];
            assert_eq!((t.top, t.bottom), (0, 15034), "{name}");
        } else {
            assert_eq!(layout.pages[0].tables[0].bottom, 8034, "{name}");
        }
    }
}

#[test]
fn a_restart_does_not_pull_a_merged_cells_later_rows_into_the_first_row() {
    let mut doc = document((25, 7), true, true, false, 2);
    let ControlKind::Table(table) = &mut doc.sections[0].paras[0].controls[1].kind else {
        unreachable!()
    };
    let p = &mut table.cells[3].paras[0];
    p.text = "A\nB\nC\r".encode_utf16().collect();
    p.line_segs.push(LineSeg {
        text_start: 4,
        vpos: 1300,
        ..p.line_segs[1]
    });
    let layout = hm_layout::layout(&doc, &FontSystem::new());
    assert_eq!(source_lines(&doc, &layout, 3), [(1, 0), (1, 1), (1, 2)]);
    assert_eq!(layout.pages[0].tables[0].bottom, 8034);
}

#[test]
fn a_source_paragraph_without_its_cached_boundary_uses_fresh_admission() {
    let mut doc = document((25, 7), true, true, false, 2);
    let ControlKind::Table(table) = &mut doc.sections[0].paras[0].controls[1].kind else {
        unreachable!()
    };
    table.cells[3].paras[0].line_segs.clear();
    assert!(table.cells[3].paras[0].src.is_some());
    let layout = hm_layout::layout(&doc, &FontSystem::new());
    assert_eq!(source_lines(&doc, &layout, 3), [(1, 0), (1, 1)]);
    assert_eq!(layout.pages[0].tables[0].bottom, 8034);
}

#[test]
fn a_carried_merged_prefix_paints_after_its_fill_and_keeps_hit_geometry() {
    let mut doc = document((25, 7), true, true, false, 2);
    let ControlKind::Table(table) = &mut doc.sections[0].paras[0].controls[1].kind else {
        unreachable!()
    };
    table.spacing = 80;
    table.cells[3].paras[0].text = "A\nQ\r".encode_utf16().collect();
    table.cells[3].border_fill = 1;
    let paragraph = &table.cells[3].paras[0] as *const Paragraph as usize;
    let cell = &table.cells[3] as *const Cell as usize;
    doc.info.border_fills.push(BorderFill {
        fill: Some(Fill {
            kind: 1,
            solid: Some(SolidFill {
                background: 0xFFFFFF,
                pattern_color: 0,
                pattern: -1,
            }),
            ..Default::default()
        }),
        ..Default::default()
    });
    let mut fonts = FontSystem::new();
    assert_eq!(
        fonts.add(
            "fixture",
            std::sync::Arc::new(
                include_bytes!("../../hm-fonts/tests/fixtures/source.ttf").to_vec()
            )
        ),
        1
    );
    let layout = hm_layout::layout(&doc, &fonts);
    let lines = hm_render::page_lines(&doc, &layout, &fonts, 1);
    let line = lines
        .iter()
        .find(|l| l.para == paragraph && l.line == 1)
        .unwrap();
    assert_eq!(line.stops.first().unwrap().0, 2);
    assert_eq!(line.stops.last().unwrap().0, 4);
    let cells = hm_render::page_cells(&doc, &layout, &fonts, 1);
    let bounds = cells.iter().find(|b| b.cell == cell).unwrap();
    assert!(line.y >= bounds.y && line.y + line.height <= bounds.y + bounds.height);
    let dl = hm_render::build_page(&doc, &layout, &fonts, 1);
    let image = hm_render::raster::render(&dl, &fonts, 144.0).unwrap();
    let scale = |hu: i32| (hu.max(0) as f32 * 144.0 / 7200.0) as u32;
    let ink = (scale(line.y)..scale(line.y + line.height))
        .flat_map(|y| (scale(bounds.x)..scale(bounds.x + bounds.width)).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            image
                .pixel(x, y)
                .is_some_and(|p| p.red() < 240 || p.green() < 240 || p.blue() < 240)
        })
        .count();
    assert!(
        ink > 0,
        "the cell fill must not hide its carried source glyph"
    );
}
