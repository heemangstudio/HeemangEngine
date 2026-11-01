//! Fragment bounds, line membership and alignment must agree after repeated
//! CELL splits, including the final fragment and the row following it.

use hm_fonts::FontSystem;
use hm_model::{
    Cell, CharShape, Control, ControlKind, DocInfo, DocMeta, Document, Format, ObjectCommon,
    ObjectHeader, PARA_END, PageDef, ParaShape, Paragraph, Section, SectionChild, SectionDef,
    Table,
};

fn document(align: u32, minimum: i32, count: usize) -> Document {
    let text = |lines: usize| Paragraph {
        text: (0..lines)
            .flat_map(|i| ['A' as u16, if i + 1 == lines { PARA_END } else { 10 }])
            .collect(),
        ..Default::default()
    };
    let table = Table {
        header: ObjectHeader {
            common: ObjectCommon {
                attr: (2 << 3) | (1 << 21),
                width: 6000,
                ..Default::default()
            },
            ..Default::default()
        },
        attr: 2,
        rows: 2,
        cols: 1,
        padding: [0, 0, 100, 100],
        cells: vec![
            Cell {
                list_attr: align << 5,
                col_span: 1,
                row_span: 1,
                width: 6000,
                height: minimum,
                paras: vec![text(count)],
                ..Default::default()
            },
            Cell {
                row: 1,
                col_span: 1,
                row_span: 1,
                width: 6000,
                height: 1200,
                paras: vec![text(1)],
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let section = SectionDef {
        children: vec![SectionChild::Page(PageDef {
            width: 10000,
            height: 10000,
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

fn check(align: u32, minimum: i32, count: usize, heights: &[i32], baselines: &[i32]) {
    let doc = document(align, minimum, count);
    let fonts = FontSystem::new();
    let layout = hm_layout::layout(&doc, &fonts);
    let table = doc.sections[0].paras[0].controls[1].as_table().unwrap();
    let cell = &table.cells[0];
    let following = &table.cells[1];
    let address = |p: &Paragraph| p as *const Paragraph as usize;
    let cell_address = |c: &Cell| c as *const Cell as usize;
    assert_eq!(layout.pages.len(), heights.len());
    for (i, page) in layout.pages.iter().enumerate() {
        let boxes = hm_render::page_cells(&doc, &layout, &fonts, i);
        let bounds = boxes.iter().find(|b| b.cell == cell_address(cell)).unwrap();
        assert_eq!((bounds.y, bounds.height), (0, heights[i]), "page {i}");
        let lines = hm_render::page_lines(&doc, &layout, &fonts, i);
        let lines: Vec<_> = lines
            .iter()
            .filter(|l| l.para == address(&cell.paras[0]))
            .collect();
        let last = i + 1 == heights.len();
        let end = if last { count } else { (i + 1) * 6 };
        assert_eq!(
            lines.iter().map(|l| l.line).collect::<Vec<_>>(),
            (i * 6..end).map(|l| l as u32).collect::<Vec<_>>(),
            "page {i}"
        );
        assert_eq!(lines[0].y + lines[0].baseline, baselines[i], "page {i}");
        assert_eq!(
            page.tables[0].height(),
            heights[i] + if last { 1200 } else { 0 }
        );
        if last {
            let bounds = boxes
                .iter()
                .find(|b| b.cell == cell_address(following))
                .unwrap();
            assert_eq!((bounds.y, bounds.height), (heights[i], 1200));
            let lines = hm_render::page_lines(&doc, &layout, &fonts, i);
            let line = lines
                .iter()
                .find(|l| l.para == address(&following.paras[0]))
                .unwrap();
            assert_eq!(line.y + line.baseline, heights[i] + 950);
        } else {
            assert!(boxes.iter().all(|b| b.cell != cell_address(following)));
        }
    }
}

// Every part of a cut cell, the first included, aligns between its top
// padding and the part's bottom less its bottom padding (Hancom probes
// g-valign and s-center-minh).
#[test]
fn authored_row_minimum_survives_three_cell_splits() {
    for (align, baselines) in [
        (0, [950, 950, 950, 950]),
        (1, [1300, 1300, 1300, 950]),
        (2, [1650, 1650, 1650, 950]),
    ] {
        check(align, 37000, 22, &[9900, 9900, 9900, 6000], &baselines);
    }
}

#[test]
fn content_limited_continuations_close_below_their_last_line() {
    for align in 0..=2 {
        check(align, 1000, 22, &[9200, 9200, 9200, 6000], &[950; 4]);
    }
}

#[test]
fn completed_content_closes_without_filling_an_unfinished_minimum() {
    for (align, baselines) in [
        (0, [950; 4]),
        (1, [1300, 1300, 1300, 950]),
        (2, [1650, 1650, 1650, 950]),
    ] {
        check(align, 100000, 22, &[9900, 9900, 9900, 6000], &baselines);
    }
}

#[test]
fn first_and_carried_fragments_use_the_same_strict_hundred_hu_boundary() {
    for (height, count) in [(9300, 5), (9301, 6)] {
        let mut doc = document(1, 37000, 22);
        let ControlKind::Section(section) = &mut doc.sections[0].paras[0].controls[0].kind else {
            unreachable!()
        };
        let SectionChild::Page(page) = &mut section.children[0] else {
            unreachable!()
        };
        page.height = height;
        let fonts = FontSystem::new();
        let layout = hm_layout::layout(&doc, &fonts);
        let table = doc.sections[0].paras[0].controls[1].as_table().unwrap();
        let address = &table.cells[0].paras[0] as *const Paragraph as usize;
        for i in 0..3 {
            let lines = hm_render::page_lines(&doc, &layout, &fonts, i);
            let membership: Vec<_> = lines
                .iter()
                .filter(|l| l.para == address)
                .map(|l| l.line)
                .collect();
            assert_eq!(
                membership,
                (i * count..(i + 1) * count)
                    .map(|l| l as u32)
                    .collect::<Vec<_>>()
            );
        }
    }
}

#[test]
fn occupied_fragments_exhaust_the_authored_minimum() {
    check(
        0,
        37000,
        34,
        &[9900, 9900, 9900, 9200, 9200, 6000],
        &[950; 6],
    );
}

#[test]
fn a_minimum_that_fits_no_longer_extends_content_limited_fragments() {
    check(
        0,
        39500,
        34,
        &[9900, 9900, 9900, 9200, 9200, 6000],
        &[950; 6],
    );
}

fn nested_document(wrap: u32) -> Document {
    let mut doc = document(0, 1200, 1);
    let mut nested = doc.sections[0].paras[0].controls[1]
        .as_table()
        .unwrap()
        .clone();
    nested.rows = 1;
    nested.cells.truncate(1);
    nested.header.common.attr = (2 << 3) | (wrap << 21);
    nested.header.common.width = 5000;
    nested.header.common.y_offset = 1300;
    nested.cells[0].width = 5000;
    nested.cells[0].paras[0].text = (0..12)
        .flat_map(|i| ['N' as u16, if i == 11 { PARA_END } else { 10 }])
        .collect();
    let ControlKind::Table(outer) = &mut doc.sections[0].paras[0].controls[1].kind else {
        unreachable!()
    };
    outer.rows = 1;
    outer.cells.truncate(1);
    let plain = outer.cells[0].paras[0].clone();
    outer.cells[0].paras = vec![
        plain.clone(),
        Paragraph {
            text: [vec![11; 8], vec!['H' as u16, PARA_END]].concat(),
            controls: vec![Control {
                kind: ControlKind::Table(Box::new(nested)),
                src: None,
            }],
            ..Default::default()
        },
        plain,
    ];
    doc
}

#[test]
fn nested_overlays_keep_their_anchor_without_moving_cell_text() {
    let fonts = FontSystem::new();
    for wrap in [0, 1, 2, 3] {
        let doc = nested_document(wrap);
        let layout = hm_layout::layout(&doc, &fonts);
        let outer = layout.para(0, 0).table(1).unwrap();
        let cell = &outer.cells[0];
        let ys: Vec<_> = cell.lines.iter().map(|l| l.y).collect();
        if wrap == 2 {
            assert!(cell.tables.is_empty());
            assert_eq!(ys, [0, 1600, 3200]);
            continue;
        }
        let nested = &cell.tables[0];
        assert_eq!(nested.y, 2900);
        assert!(!nested.cuts.is_empty());
        assert_eq!(cell.offset, 0);
        if wrap == 1 {
            assert!(ys[2] >= nested.y + nested.height);
        } else {
            assert_eq!(cell.content, nested.y + nested.height);
            assert_eq!(ys, [0, 1600, 3200]);
        }
    }
}

#[test]
fn nested_overlay_continuations_draw_each_content_line_once() {
    let doc = nested_document(3);
    let fonts = FontSystem::new();
    let layout = hm_layout::layout(&doc, &fonts);
    let outer = doc.sections[0].paras[0].controls[1].as_table().unwrap();
    let host = &outer.cells[0].paras[1];
    let nested = host.controls[0].as_table().unwrap();
    let text = &nested.cells[0].paras[0] as *const Paragraph as usize;
    let host = host as *const Paragraph as usize;
    assert!(layout.pages.len() > 1);
    let mut membership = Vec::new();
    let mut hosts = Vec::new();
    for page in 0..layout.pages.len() {
        let lines = hm_render::page_lines(&doc, &layout, &fonts, page);
        membership.extend(lines.iter().filter(|l| l.para == text).map(|l| l.line));
        hosts.extend(lines.iter().filter(|l| l.para == host).map(|l| (page, l.y)));
    }
    assert_eq!(membership, (0..12).collect::<Vec<_>>());
    assert_eq!(hosts, [(0, 1700)]);
    let control = &doc.sections[0].paras[0].controls[1];
    assert!(hm_layout::table::flows_in(control, 10000));
    assert!(!hm_layout::table::flows_in(
        &outer.cells[0].paras[1].controls[0],
        10000
    ));
}

fn merged_grid_document(align: u32, wrap: u32, offset: i32, merged: bool, mode: u32) -> Document {
    let mut doc = document(align, 6000, 1);
    let ControlKind::Table(table) = &mut doc.sections[0].paras[0].controls[1].kind else {
        unreachable!()
    };
    table.header.common.attr = (2 << 3) | (wrap << 21);
    table.header.common.height = 6000;
    table.header.common.y_offset = offset;
    table.attr = mode | 8;
    table.rows = 3;
    table.cols = 2;
    table.padding = [100; 4];
    let plain = table.cells[0].paras[0].clone();
    table.cells = vec![Cell {
        list_attr: align << 5,
        row_span: if merged { 3 } else { 1 },
        col_span: 1,
        width: 3000,
        height: if merged { 6000 } else { 2000 },
        paras: vec![plain.clone()],
        ..Default::default()
    }];
    table.cells.extend((0..3).map(|row| Cell {
        row,
        col: 1,
        row_span: 1,
        col_span: 1,
        width: 3000,
        height: 2000,
        list_attr: align << 5,
        paras: vec![plain.clone()],
        ..Default::default()
    }));
    if !merged {
        table.cells.extend((1..3).map(|row| Cell {
            row,
            row_span: 1,
            col_span: 1,
            width: 3000,
            height: 2000,
            paras: vec![plain.clone()],
            ..Default::default()
        }));
    }
    doc
}

#[test]
fn merged_row_edge_fragments_keep_physical_boxes_separate_from_logical_ink() {
    let fonts = FontSystem::new();
    for align in 0..=2 {
        for wrap in [0, 1] {
            for offset in [7800, 6750] {
                let doc = merged_grid_document(align, wrap, offset, true, 2);
                let layout = hm_layout::layout(&doc, &fonts);
                let table = doc.sections[0].paras[0].controls[1].as_table().unwrap();
                let first = &layout.pages[0].tables[0];
                assert_eq!((first.y, first.top, first.bottom), (offset, 0, 2000));
                assert_eq!(layout.pages[1].tables[0].top, 2000);
                assert_eq!(first.height(), 10000 - offset);
                let extra = 10000 - offset - 2000;
                let boxes = hm_render::page_cells(&doc, &layout, &fonts, 0);
                let lines = hm_render::page_lines(&doc, &layout, &fonts, 0);
                for cell in &table.cells[..2] {
                    let key = cell as *const Cell as usize;
                    let bounds = boxes.iter().find(|b| b.cell == key).unwrap();
                    assert_eq!((bounds.y, bounds.height), (offset, 10000 - offset));
                    let key = &cell.paras[0] as *const Paragraph as usize;
                    let line = lines.iter().find(|l| l.para == key).unwrap();
                    let expected = [100, 500 + extra / 2, 900 + extra][align as usize];
                    assert_eq!(line.y, offset + expected, "align={align}");
                    assert!(line.y + line.height <= bounds.y + bounds.height);
                }
                let mut membership: Vec<_> = (0..layout.pages.len())
                    .flat_map(|p| hm_render::page_lines(&doc, &layout, &fonts, p))
                    .filter_map(|l| {
                        table
                            .cells
                            .iter()
                            .position(|c| l.para == &c.paras[0] as *const Paragraph as usize)
                    })
                    .collect();
                membership.sort_unstable();
                assert_eq!(membership, [0, 1, 2, 3]);
            }
            for mode in [1, 2] {
                let doc = merged_grid_document(align, wrap, 7800, false, mode);
                let layout = hm_layout::layout(&doc, &fonts);
                assert_eq!(layout.pages[0].tables[0].height(), 2000);
            }
            let doc = merged_grid_document(align, wrap, 1000, true, 2);
            let layout = hm_layout::layout(&doc, &fonts);
            assert_eq!(layout.pages[0].tables[0].height(), 6000);
        }
    }
}

#[test]
fn merged_continuations_keep_physical_boxes_and_source_ownership() {
    let fonts = FontSystem::new();
    for align in 0..=2 {
        for wrap in [0, 1] {
            let mut doc = merged_grid_document(align, wrap, 0, true, 2);
            let ControlKind::Table(table) = &mut doc.sections[0].paras[0].controls[1].kind else {
                unreachable!()
            };
            table.rows = 12;
            table.header.common.height = 24000;
            table.header.common.margins[3] = 300;
            table.cells[0].row_span = 12;
            table.cells[0].height = 24000;
            table.cells[0].paras[0].text = (0..8)
                .flat_map(|i| ['A' as u16, if i == 7 { PARA_END } else { 10 }])
                .collect();
            let right = table.cells[1].clone();
            table.cells.truncate(1);
            table.cells.extend((0..12).map(|row| Cell {
                row,
                ..right.clone()
            }));
            let ControlKind::Section(section) = &mut doc.sections[0].paras[0].controls[0].kind
            else {
                unreachable!()
            };
            let SectionChild::Page(page) = &mut section.children[0] else {
                unreachable!()
            };
            page.height = 8500;
            let layout = hm_layout::layout(&doc, &fonts);
            let table = doc.sections[0].paras[0].controls[1].as_table().unwrap();
            assert_eq!(layout.pages.len(), 3);
            assert_eq!(layout.pages[0].tables[0].bottom, 8000);
            assert_eq!(layout.pages[1].tables[0].top, 8000);
            assert_eq!(layout.pages[1].tables[0].bottom, 16000);
            assert_eq!(layout.pages[1].tables[0].height(), 8200);
            assert_eq!(layout.pages[2].tables[0].height(), 8000);
            let closing = &table.cells[8];
            let boxes = hm_render::page_cells(&doc, &layout, &fonts, 1);
            let bounds = boxes
                .iter()
                .find(|b| b.cell == closing as *const Cell as usize)
                .unwrap();
            assert_eq!((bounds.y, bounds.height), (6000, 2200));
            let lines = hm_render::page_lines(&doc, &layout, &fonts, 1);
            let line = lines
                .iter()
                .find(|l| l.para == &closing.paras[0] as *const Paragraph as usize)
                .unwrap();
            assert_eq!(line.y, [6100, 6600, 7100][align as usize]);
            let spanning = &table.cells[0].paras[0] as *const Paragraph as usize;
            let first = lines.iter().find(|l| l.para == spanning).unwrap();
            assert_eq!(first.line, 5);
            assert_eq!(first.y, [100, 2000, 3900][align as usize]);
            let mut starts: Vec<_> = (0..layout.pages.len())
                .flat_map(|p| hm_render::page_lines(&doc, &layout, &fonts, p))
                .filter(|l| l.para == spanning)
                .map(|l| l.stops.first().unwrap().0)
                .collect();
            starts.sort_unstable();
            assert_eq!(starts, [0, 2, 4, 6, 8, 10, 12, 14]);
        }
    }
}

#[test]
fn ordinary_row_continuations_keep_their_natural_closing_boxes() {
    let fonts = FontSystem::new();
    for mode in [1, 2] {
        for wrap in [0, 1] {
            let mut doc = merged_grid_document(1, wrap, 0, false, mode);
            let ControlKind::Table(table) = &mut doc.sections[0].paras[0].controls[1].kind else {
                unreachable!()
            };
            table.rows = 12;
            table.header.common.height = 24000;
            table.header.common.margins[3] = 300;
            let left = table.cells[0].clone();
            let right = table.cells[1].clone();
            table.cells = (0..12)
                .flat_map(|row| {
                    [
                        Cell {
                            row,
                            ..left.clone()
                        },
                        Cell {
                            row,
                            ..right.clone()
                        },
                    ]
                })
                .collect();
            let ControlKind::Section(section) = &mut doc.sections[0].paras[0].controls[0].kind
            else {
                unreachable!()
            };
            let SectionChild::Page(page) = &mut section.children[0] else {
                unreachable!()
            };
            page.height = 8500;
            let layout = hm_layout::layout(&doc, &fonts);
            assert_eq!(layout.pages.len(), 3);
            let part = &layout.pages[1].tables[0];
            assert_eq!((part.top, part.bottom, part.height()), (8000, 16000, 8000));
            let table = doc.sections[0].paras[0].controls[1].as_table().unwrap();
            let closing = &table.cells[15];
            let boxes = hm_render::page_cells(&doc, &layout, &fonts, 1);
            let bounds = boxes
                .iter()
                .find(|b| b.cell == closing as *const Cell as usize)
                .unwrap();
            assert_eq!((bounds.y, bounds.height), (6000, 2000));
            assert_eq!(layout.pages[2].tables[0].height(), 8000);
        }
    }
}

#[test]
fn resizable_merged_fragments_close_above_the_cut_room() {
    let fonts = FontSystem::new();
    for wrap in [0, 1] {
        for align in 0..=2 {
            let mut doc = merged_grid_document(align, wrap, 7800, true, 2);
            let ControlKind::Table(table) = &mut doc.sections[0].paras[0].controls[1].kind else {
                unreachable!()
            };
            table.attr &= !8;
            let layout = hm_layout::layout(&doc, &fonts);
            let first = &layout.pages[0].tables[0];
            assert_eq!((first.top, first.bottom, first.height()), (0, 2000, 2099));
            let table = doc.sections[0].paras[0].controls[1].as_table().unwrap();
            let closing = &table.cells[1];
            let boxes = hm_render::page_cells(&doc, &layout, &fonts, 0);
            let bounds = boxes
                .iter()
                .find(|b| b.cell == closing as *const Cell as usize)
                .unwrap();
            assert_eq!((bounds.y, bounds.height), (7800, 2099));
            let lines = hm_render::page_lines(&doc, &layout, &fonts, 0);
            let line = lines
                .iter()
                .find(|l| l.para == &closing.paras[0] as *const Paragraph as usize)
                .unwrap();
            assert_eq!(line.y, 7800 + [100, 549, 999][align as usize]);
        }
    }
}

#[test]
fn merged_lines_that_fit_the_physical_tail_keep_one_source_owner() {
    let fonts = FontSystem::new();
    for align in 0..=2 {
        for wrap in [0, 1] {
            let mut doc = merged_grid_document(align, wrap, 3827, true, 2);
            doc.info.para_shapes[0].line_spacing_legacy = 150;
            let ControlKind::Table(table) = &mut doc.sections[0].paras[0].controls[1].kind else {
                unreachable!()
            };
            table.attr &= !8;
            table.rows = 4;
            table.padding = [283; 4];
            table.header.common.height = 6264;
            table.cells[0].row_span = 4;
            table.cells[0].height = 6264;
            let plain = table.cells[0].paras[0].clone();
            table.cells[0].paras = (0..4)
                .map(|i| Paragraph {
                    text: vec!['A' as u16 + i, PARA_END],
                    ..plain.clone()
                })
                .collect();
            let right = table.cells[1].clone();
            table.cells.truncate(1);
            table.cells.extend((0..4).map(|row| Cell {
                row,
                height: 1566,
                ..right.clone()
            }));
            let layout = hm_layout::layout(&doc, &fonts);
            let table = doc.sections[0].paras[0].controls[1].as_table().unwrap();
            let first = &layout.pages[0].tables[0];
            assert_eq!((first.top, first.bottom, first.height()), (0, 4698, 6072));
            assert_eq!(layout.pages[1].tables[0].top, 4698);
            let lines = hm_render::page_lines(&doc, &layout, &fonts, 0);
            let boxes = hm_render::page_cells(&doc, &layout, &fonts, 0);
            let bounds = boxes
                .iter()
                .find(|b| b.cell == &table.cells[0] as *const Cell as usize)
                .unwrap();
            assert_eq!((bounds.y, bounds.height), (3827, 6072));
            for (i, para) in table.cells[0].paras.iter().enumerate() {
                let key = para as *const Paragraph as usize;
                let line = lines
                    .iter()
                    .find(|l| l.para == key)
                    .expect("fitting merged line stays");
                assert_eq!(
                    line.y,
                    3827 + 283 + i as i32 * 1500 + [0, 3, 6][align as usize]
                );
                assert!(line.y + line.height <= bounds.y + bounds.height);
                let source: Vec<_> = (0..layout.pages.len())
                    .flat_map(|p| hm_render::page_lines(&doc, &layout, &fonts, p))
                    .filter(|l| l.para == key)
                    .map(|l| (l.stops.first().unwrap().0, l.stops.last().unwrap().0))
                    .collect();
                assert_eq!(source, [(0, 2)]);
            }
            let next = &table.cells[4].paras[0] as *const Paragraph as usize;
            assert!(!lines.iter().any(|l| l.para == next));
            assert!(
                hm_render::page_lines(&doc, &layout, &fonts, 1)
                    .iter()
                    .any(|l| l.para == next)
            );
        }
    }
}

#[test]
fn content_growth_can_fulfill_a_spanning_cells_minimum_before_its_last_row() {
    let fonts = FontSystem::new();
    for align in 0..=2 {
        for wrap in [0, 1] {
            let mut doc = merged_grid_document(align, wrap, 6850, true, 2);
            let ControlKind::Table(table) = &mut doc.sections[0].paras[0].controls[1].kind else {
                unreachable!()
            };
            table.attr &= !8;
            table.cells[0].height = 2000;
            table.cells[1].paras[0].text = "A\nB\r".encode_utf16().collect();
            let layout = hm_layout::layout(&doc, &fonts);
            let first = &layout.pages[0].tables[0];
            assert_eq!((first.top, first.bottom, first.height()), (0, 2800, 2800));
            let table = doc.sections[0].paras[0].controls[1].as_table().unwrap();
            let boxes = hm_render::page_cells(&doc, &layout, &fonts, 0);
            for cell in &table.cells[..2] {
                let bounds = boxes
                    .iter()
                    .find(|b| b.cell == cell as *const Cell as usize)
                    .unwrap();
                assert_eq!((bounds.y, bounds.height), (6850, 2800));
            }
            for (cell, count) in table.cells.iter().zip([1, 2, 1, 1]) {
                let source = &cell.paras[0] as *const Paragraph as usize;
                let mut lines: Vec<_> = (0..layout.pages.len())
                    .flat_map(|p| hm_render::page_lines(&doc, &layout, &fonts, p))
                    .filter(|l| l.para == source)
                    .map(|l| l.line)
                    .collect();
                lines.sort_unstable();
                assert_eq!(lines, (0..count).collect::<Vec<_>>());
            }
        }
    }
}

#[test]
fn physical_tails_spend_spanning_minima_without_changing_source_lines() {
    let fonts = FontSystem::new();
    for align in 0..=2 {
        for wrap in [0, 1] {
            for (minimum, remaining) in [(6000, 4000), (6500, 4300)] {
                let mut doc = merged_grid_document(align, wrap, 7800, true, 2);
                let ControlKind::Table(table) = &mut doc.sections[0].paras[0].controls[1].kind
                else {
                    unreachable!()
                };
                table.cells[0].height = minimum;
                let layout = hm_layout::layout(&doc, &fonts);
                assert_eq!(layout.pages.len(), 2);
                assert_eq!(layout.pages[0].tables[0].height(), 2200);
                assert_eq!(layout.pages[1].tables[0].height(), remaining);
                if let Some(projected) = &layout.pages[1].tables[0].projected {
                    let canonical = layout.para(0, 0).table(1).unwrap();
                    for (before, after) in canonical.cells.iter().zip(&projected.cells) {
                        assert_eq!(before.lines, after.lines);
                        for (before, after) in before.paras.iter().zip(&after.paras) {
                            assert_eq!(before.lines, after.lines);
                        }
                    }
                }
                let table = doc.sections[0].paras[0].controls[1].as_table().unwrap();
                for cell in &table.cells {
                    let source = &cell.paras[0] as *const Paragraph as usize;
                    let lines: Vec<_> = (0..layout.pages.len())
                        .flat_map(|p| hm_render::page_lines(&doc, &layout, &fonts, p))
                        .filter(|l| l.para == source)
                        .map(|l| {
                            (
                                l.line,
                                l.stops.first().unwrap().0,
                                l.stops.last().unwrap().0,
                            )
                        })
                        .collect();
                    assert_eq!(lines, [(0, 0, 2)]);
                }
            }
        }
    }
}

/// Every fragment that does not end the table keeps room for the whole
/// bottom caption below it, however many lines it has, while only the last
/// fragment paints it (probes f-caption-bottom, f-caption-bottom-3p).
#[test]
fn nonfinal_caption_fragments_keep_room_for_the_whole_caption() {
    let fonts = FontSystem::new();
    let mut first: Option<i32> = None;
    for caption_lines in [1, 3] {
        let mut doc = document(0, 0, 1);
        let ControlKind::Table(t) = &mut doc.sections[0].paras[0].controls[1].kind else {
            unreachable!()
        };
        let cell = t.cells[1].clone();
        t.rows = 16;
        t.cells = (0..16)
            .map(|row| Cell {
                row,
                ..cell.clone()
            })
            .collect();
        t.caption = Some(hm_model::Caption {
            count: 1,
            attr: Some(3),
            gap: Some(500),
            paras: vec![Paragraph {
                text: (0..caption_lines)
                    .flat_map(|i| {
                        [
                            'C' as u16,
                            if i + 1 == caption_lines { PARA_END } else { 10 },
                        ]
                    })
                    .collect(),
                ..Default::default()
            }],
            ..Default::default()
        });
        let layout = hm_layout::layout(&doc, &fonts);
        let fragment = &layout.pages[0].tables[0];
        assert!(!fragment.caption);
        let cap = layout.para(0, 0).table(1).unwrap().caption.clone().unwrap();
        let page = &layout.pages[0];
        let room = page.body.y + page.body.h - (fragment.y + fragment.height());
        assert!(room >= cap.gap + cap.height);
        if let Some(one_line) = first {
            assert!(fragment.bottom < one_line);
        } else {
            first = Some(fragment.bottom);
        }
        let caption = &doc.sections[0].paras[0].controls[1]
            .as_table()
            .unwrap()
            .caption
            .as_ref()
            .unwrap()
            .paras[0] as *const Paragraph as usize;
        let mut painted = 0;
        for page in 0..layout.pages.len() {
            for line in hm_render::page_lines(&doc, &layout, &fonts, page) {
                assert!(line.y + line.height <= layout.pages[page].height);
                if line.para == caption {
                    painted += 1;
                    assert_eq!(page + 1, layout.pages.len());
                }
            }
        }
        assert_eq!(painted, caption_lines);
    }
}

/// Above a bottom caption a fragment keeps the table's bottom margin and the
/// caption's gap on the page, and the fragment that ends the table keeps the
/// caption's text too (hwpspec tables 246 p39 and 270 p42: Hancom moves them
/// rather than end them in less room; table 328 p50 ends at row edge 9739).
#[test]
fn a_bottom_caption_keeps_its_room_while_a_table_continues() {
    let fonts = FontSystem::new();
    for wrap in [0, 1] {
        let mut doc = merged_grid_document(1, wrap, 3500, false, 2);
        let ControlKind::Table(table) = &mut doc.sections[0].paras[0].controls[1].kind else {
            unreachable!()
        };
        table.caption = Some(hm_model::Caption {
            count: 1,
            attr: Some(3),
            gap: Some(500),
            paras: vec![table.cells[0].paras[0].clone()],
            ..Default::default()
        });
        let layout = hm_layout::layout(&doc, &fonts);
        assert_eq!(layout.pages.len(), 2);
        assert_eq!(layout.pages[0].tables[0].bottom, 4000);
        assert_eq!(layout.pages[1].tables[0].height(), 2000);
        let table = doc.sections[0].paras[0].controls[1].as_table().unwrap();
        let source = &table.caption.as_ref().unwrap().paras[0] as *const Paragraph as usize;
        assert!(
            hm_render::page_lines(&doc, &layout, &fonts, 0)
                .iter()
                .all(|l| l.para != source)
        );
        assert_eq!(
            hm_render::page_lines(&doc, &layout, &fonts, 1)
                .iter()
                .filter(|l| l.para == source)
                .count(),
            1
        );
    }
}

/// A fragment that does not end the table keeps the caption's gap and text
/// below it (probes f-caption-bottom, f-caption-bottom-3p, Hancom 2022), so
/// a row cut on every page goes on over three pages here. The caption then
/// follows the carried final row on the last page.
#[test]
fn a_bottom_caption_stays_on_page_after_a_carried_final_row() {
    let fonts = FontSystem::new();
    let mut doc = document(0, 0, 11);
    let ControlKind::Table(table) = &mut doc.sections[0].paras[0].controls[1].kind else {
        unreachable!()
    };
    table.rows = 1;
    table.cells.truncate(1);
    table.caption = Some(hm_model::Caption {
        count: 1,
        attr: Some(3),
        gap: Some(500),
        paras: vec![Paragraph {
            text: vec!['C' as u16, PARA_END],
            ..Default::default()
        }],
        ..Default::default()
    });
    let layout = hm_layout::layout(&doc, &fonts);
    assert_eq!(layout.pages.len(), 3);
    let table = doc.sections[0].paras[0].controls[1].as_table().unwrap();
    let source = &table.cells[0].paras[0] as *const Paragraph as usize;
    let caption = &table.caption.as_ref().unwrap().paras[0] as *const Paragraph as usize;
    let mut kept = Vec::new();
    let mut captions = Vec::new();
    for page in 0..layout.pages.len() {
        for line in hm_render::page_lines(&doc, &layout, &fonts, page) {
            assert!(line.y + line.height <= layout.pages[page].height);
            if line.para == source {
                kept.push(line.line);
            }
            if line.para == caption {
                captions.push(page);
            }
        }
    }
    assert_eq!(kept, (0..11).collect::<Vec<_>>());
    assert_eq!(captions, [2]);
}

#[test]
fn completed_merged_cells_may_leave_the_table_margin_beyond_the_page() {
    let fonts = FontSystem::new();
    let minimum = 4 * 1000 + 3 * 300 + 2 * 283;
    for align in 0..=2 {
        for wrap in [0, 1] {
            for slack in [-4, -1, 0, 9, 132] {
                let mut doc = merged_grid_document(align, wrap, 10000 - minimum - slack, true, 2);
                doc.info.para_shapes[0].line_spacing_legacy = 130;
                let ControlKind::Table(table) = &mut doc.sections[0].paras[0].controls[1].kind
                else {
                    unreachable!()
                };
                table.padding = [283; 4];
                table.header.common.height = minimum + 1566;
                table.header.common.margins[3] = 141;
                table.cells[0].row_span = 2;
                table.cells[0].height = minimum;
                table.cells[0].paras[0].text = "A\nA\nA\nA\r".encode_utf16().collect();
                table.cells[1].height = minimum / 2;
                table.cells[2].height = minimum / 2;
                table.cells[3].height = 1566;
                let mut last = table.cells[3].clone();
                last.col = 0;
                table.cells.push(last);
                let layout = hm_layout::layout(&doc, &fonts);
                let table = doc.sections[0].paras[0].controls[1].as_table().unwrap();
                let key = &table.cells[0].paras[0] as *const Paragraph as usize;
                let first = &layout.pages[0].tables[0];
                if slack >= 0 {
                    assert_eq!((first.bottom, first.height()), (minimum, minimum));
                } else {
                    assert!(first.bottom < minimum);
                }
                let kept: Vec<_> = hm_render::page_lines(&doc, &layout, &fonts, 0)
                    .into_iter()
                    .filter(|l| l.para == key)
                    .map(|l| l.line)
                    .collect();
                assert_eq!(
                    kept,
                    if slack >= 0 {
                        vec![0, 1, 2, 3]
                    } else {
                        vec![0, 1, 2]
                    }
                );
                let all: Vec<_> = (0..layout.pages.len())
                    .flat_map(|p| hm_render::page_lines(&doc, &layout, &fonts, p))
                    .filter(|l| l.para == key)
                    .map(|l| l.line)
                    .collect();
                assert_eq!(all, [0, 1, 2, 3]);
                assert_eq!(layout.pages[1].tables[0].carried.is_none(), slack >= 0);
            }
        }
    }
}

fn first_line_document(
    align: u32,
    wrap: u32,
    slack: i32,
    scenario: &str,
    header: bool,
) -> Document {
    let offset = 10000 - 141 - 3400 - slack;
    let mut doc = merged_grid_document(align, wrap, offset, true, 2);
    let mut leading = doc.info.para_shapes[0].clone();
    leading.space_before = hm_model::units::pack_distance(200);
    leading.line_spacing_legacy = 130;
    doc.info.para_shapes.push(leading);
    let continued = matches!(scenario, "continued" | "continued blank");
    if continued {
        doc.info.para_shapes[0].line_spacing_legacy = 220;
    }
    let ControlKind::Table(table) = &mut doc.sections[0].paras[0].controls[1].kind else {
        unreachable!()
    };
    table.header.common.height = 4800;
    table.header.common.margins[3] = 141;
    table.cells[0].height = if continued { 4800 } else { 2000 };
    table.cells[0].row_span = if continued { 3 } else { 1 };
    table.cells[0].paras[0].text = match scenario {
        "continued" => "A\nB\r",
        "continued blank" => "\nB\r",
        _ => "H\r",
    }
    .encode_utf16()
    .collect();
    for c in &mut table.cells[2..4] {
        c.height = 1400;
        c.paras[0].header.para_shape = 1;
    }
    if scenario == "split" {
        table.cells[2].paras[0].text = "R\nS\r".encode_utf16().collect();
    }
    if !continued {
        let mut para = table.cells[2].paras[0].clone();
        para.text = if scenario == "blank first" {
            "\nN\r"
        } else {
            "N\r"
        }
        .encode_utf16()
        .collect();
        table.cells.push(Cell {
            row: 1,
            col: 0,
            row_span: if scenario == "blank first" { 1 } else { 2 },
            col_span: 1,
            width: 3000,
            height: if scenario == "blank first" {
                1400
            } else {
                2800
            },
            list_attr: align << 5,
            paras: vec![para],
            ..Default::default()
        });
    }
    if header {
        table.attr |= 4;
        table.cells[0].list_attr |= 1 << 18;
        table.cells[1].list_attr |= 1 << 18;
    }
    doc
}

#[test]
fn new_rows_with_no_fitting_first_item_move_whole() {
    let fonts = FontSystem::new();
    for scenario in ["whole", "split"] {
        for header in [false, true] {
            for align in 0..=2 {
                for wrap in [0, 1] {
                    for overflow in [1, 4, 103] {
                        let doc = first_line_document(align, wrap, -overflow, scenario, header);
                        let layout = hm_layout::layout(&doc, &fonts);
                        let table = doc.sections[0].paras[0].controls[1].as_table().unwrap();
                        let key = &table.cells[4].paras[0] as *const Paragraph as usize;
                        assert_eq!(layout.pages[0].tables[0].bottom, 2000);
                        assert!(
                            !hm_render::page_lines(&doc, &layout, &fonts, 0)
                                .iter()
                                .any(|l| l.para == key)
                        );
                        let source: Vec<_> = (1..layout.pages.len())
                            .flat_map(|p| hm_render::page_lines(&doc, &layout, &fonts, p))
                            .filter(|l| l.para == key)
                            .map(|l| (l.stops.first().unwrap().0, l.stops.last().unwrap().0))
                            .collect();
                        assert_eq!(source, [(0, 2)]);
                    }
                }
            }
        }
    }
}

#[test]
fn new_rows_keep_first_lines_by_physical_fit_without_spending_authored_slack() {
    let fonts = FontSystem::new();
    for scenario in ["whole", "split"] {
        for header in [false, true] {
            for align in 0..=2 {
                for wrap in [0, 1] {
                    for slack in [80, 62, 0, -1, -4, -103] {
                        let doc = first_line_document(align, wrap, slack, scenario, header);
                        let layout = hm_layout::layout(&doc, &fonts);
                        let table = doc.sections[0].paras[0].controls[1].as_table().unwrap();
                        let fresh = &table.cells[4].paras[0] as *const Paragraph as usize;
                        let first = &layout.pages[0].tables[0];
                        assert_eq!(
                            first.bottom,
                            if slack >= 0 { 3400 } else { 2000 },
                            "{scenario}, header={header}, align={align}, wrap={wrap}, slack={slack}"
                        );
                        let lines = hm_render::page_lines(&doc, &layout, &fonts, 0);
                        let selected: Vec<_> = lines.iter().filter(|l| l.para == fresh).collect();
                        assert_eq!(selected.len(), usize::from(slack >= 0));
                        if slack >= 0 {
                            // The leading 200 belongs to the kept content, under every alignment.
                            assert_eq!(selected[0].stops.first().unwrap().0, 0);
                            assert_eq!(selected[0].stops.last().unwrap().0, 2);
                            assert_eq!(
                                selected[0].y,
                                first.y + 2300,
                                "{scenario}, header={header}, align={align}, wrap={wrap}, slack={slack}"
                            );
                        }
                        let count = (0..layout.pages.len())
                            .flat_map(|p| hm_render::page_lines(&doc, &layout, &fonts, p))
                            .filter(|l| l.para == fresh)
                            .count();
                        assert_eq!(count, 1);
                    }
                }
            }
        }
    }
    for scenario in ["continued", "continued blank"] {
        let doc = first_line_document(1, 1, 80, scenario, false);
        let layout = hm_layout::layout(&doc, &fonts);
        let table = doc.sections[0].paras[0].controls[1].as_table().unwrap();
        let paragraph = &table.cells[0].paras[0] as *const Paragraph as usize;
        assert_eq!(layout.pages[0].tables[0].bottom, 2000);
        assert!(
            hm_render::page_lines(&doc, &layout, &fonts, 0)
                .iter()
                .all(|l| l.para != paragraph || l.line != 1)
        );
        let kept = (0..layout.pages.len())
            .flat_map(|p| hm_render::page_lines(&doc, &layout, &fonts, p))
            .filter(|l| l.para == paragraph && l.line == 1)
            .count();
        assert_eq!(kept, 1);
    }
    let doc = first_line_document(1, 1, 80, "blank first", false);
    let layout = hm_layout::layout(&doc, &fonts);
    let table = doc.sections[0].paras[0].controls[1].as_table().unwrap();
    let paragraph = &table.cells[4].paras[0] as *const Paragraph as usize;
    assert_eq!(layout.pages[0].tables[0].bottom, 3400);
    assert!(
        hm_render::page_lines(&doc, &layout, &fonts, 0)
            .iter()
            .any(|l| l.para == paragraph && l.line == 0)
    );
    assert!(
        hm_render::page_lines(&doc, &layout, &fonts, 0)
            .iter()
            .all(|l| l.para != paragraph || l.line != 1)
    );
    assert_eq!(
        (0..layout.pages.len())
            .flat_map(|p| hm_render::page_lines(&doc, &layout, &fonts, p))
            .filter(|l| l.para == paragraph && l.line == 1)
            .count(),
        1
    );
    for wrap in [0, 1] {
        let mut doc = merged_grid_document(1, wrap, 6609, false, 2);
        let ControlKind::Table(t) = &mut doc.sections[0].paras[0].controls[1].kind else {
            unreachable!()
        };
        t.header.common.margins[3] = 141;
        let layout = hm_layout::layout(&doc, &fonts);
        assert_eq!(layout.pages[0].tables[0].bottom, 2000);
    }
}

#[test]
fn resizable_rows_leave_the_cut_room_before_first_line_admission() {
    let fonts = FontSystem::new();
    for fixed in [false, true] {
        for scenario in ["whole", "split"] {
            for header in [false, true] {
                for align in 0..=2 {
                    for wrap in [0, 1] {
                        for slack in [101, 100, 80, 62, 0, -1, -4, -103] {
                            let mut doc = first_line_document(align, wrap, slack, scenario, header);
                            let ControlKind::Table(table) =
                                &mut doc.sections[0].paras[0].controls[1].kind
                            else {
                                unreachable!()
                            };
                            if !fixed {
                                table.attr &= !8;
                            }
                            let fresh = &table.cells[4].paras[0] as *const Paragraph as usize;
                            let layout = hm_layout::layout(&doc, &fonts);
                            let fits = slack >= if fixed { 0 } else { 101 };
                            assert_eq!(
                                layout.pages[0].tables[0].bottom,
                                if fits { 3400 } else { 2000 },
                                "fixed={fixed}, {scenario}, header={header}, align={align}, wrap={wrap}, slack={slack}"
                            );
                            let lines = hm_render::page_lines(&doc, &layout, &fonts, 0);
                            assert_eq!(
                                lines.iter().filter(|l| l.para == fresh).count(),
                                usize::from(fits)
                            );
                            let mut source: Vec<_> = (0..layout.pages.len())
                                .flat_map(|p| hm_render::page_lines(&doc, &layout, &fonts, p))
                                .filter(|l| l.para == fresh)
                                .map(|l| (l.stops.first().unwrap().0, l.stops.last().unwrap().0))
                                .collect();
                            source.sort_unstable();
                            assert_eq!(source, [(0, 2)]);
                        }
                    }
                }
            }
        }
    }
}

/// A continuation that carries cut cells at its top still closes its last
/// whole row at the physical page end when a merged cell runs on past it,
/// aligning that row's cells in the grown box.
#[test]
fn carried_continuations_close_merged_row_edges_at_the_page_end() {
    let fonts = FontSystem::new();
    for align in 0..=2 {
        let mut doc = merged_grid_document(align, 0, 0, true, 2);
        let ControlKind::Table(table) = &mut doc.sections[0].paras[0].controls[1].kind else {
            unreachable!()
        };
        table.rows = 12;
        table.header.common.height = 24000;
        table.header.common.margins[3] = 300;
        table.cells[0].row_span = 12;
        table.cells[0].height = 24000;
        let right = table.cells[1].clone();
        table.cells.truncate(1);
        table.cells.extend((0..12).map(|row| Cell {
            row,
            ..right.clone()
        }));
        table.cells[4].paras[0].text = (0..5)
            .flat_map(|i| ['A' as u16, if i == 4 { PARA_END } else { 10 }])
            .collect();
        let ControlKind::Section(section) = &mut doc.sections[0].paras[0].controls[0].kind else {
            unreachable!()
        };
        let SectionChild::Page(page) = &mut section.children[0] else {
            unreachable!()
        };
        page.height = 8500;
        let layout = hm_layout::layout(&doc, &fonts);
        let part = &layout.pages[1].tables[0];
        assert!(part.carried.is_some());
        assert_eq!(part.height(), 8200, "align={align}");
        let table = doc.sections[0].paras[0].controls[1].as_table().unwrap();
        let closing = &table.cells[5];
        let boxes = hm_render::page_cells(&doc, &layout, &fonts, 1);
        let bounds = boxes
            .iter()
            .find(|b| b.cell == closing as *const Cell as usize)
            .unwrap();
        assert_eq!(bounds.y + bounds.height, 8200);
        let lines = hm_render::page_lines(&doc, &layout, &fonts, 1);
        let line = lines
            .iter()
            .find(|l| l.para == &closing.paras[0] as *const Paragraph as usize)
            .unwrap();
        let slack = bounds.height - 200 - line.height;
        assert_eq!(
            line.y,
            bounds.y + 100 + [0, slack / 2, slack][align as usize]
        );
    }
}

/// A merged cell whose authored height outgrows the rows it spans fills
/// that rest where a page cuts it at a row edge: the cut row runs on empty
/// at the next page's top and the merged cell's last row keeps its height.
#[test]
fn merged_minimum_rest_runs_on_at_the_next_page_top() {
    let fonts = FontSystem::new();
    let mut doc = merged_grid_document(0, 0, 0, true, 2);
    let ControlKind::Table(table) = &mut doc.sections[0].paras[0].controls[1].kind else {
        unreachable!()
    };
    table.rows = 12;
    table.header.common.height = 30000;
    table.header.common.margins[3] = 300;
    table.cells[0].row_span = 12;
    table.cells[0].height = 30000;
    let right = table.cells[1].clone();
    table.cells.truncate(1);
    table.cells.extend((0..12).map(|row| Cell {
        row,
        ..right.clone()
    }));
    let ControlKind::Section(section) = &mut doc.sections[0].paras[0].controls[0].kind else {
        unreachable!()
    };
    let SectionChild::Page(page) = &mut section.children[0] else {
        unreachable!()
    };
    page.height = 8500;
    let layout = hm_layout::layout(&doc, &fonts);
    let first = &layout.pages[0].tables[0];
    assert_eq!(first.bottom, 8000);
    let part = &layout.pages[1].tables[0];
    let lead = part.carried.as_ref().map_or(0, |c| c.height);
    assert_eq!(first.tail + lead, 6000);
    assert_eq!(part.top, 8000);
    let table = doc.sections[0].paras[0].controls[1].as_table().unwrap();
    let closing = &table.cells[12];
    let page = (0..layout.pages.len())
        .find(|&p| {
            hm_render::page_cells(&doc, &layout, &fonts, p)
                .iter()
                .any(|b| b.cell == closing as *const Cell as usize)
        })
        .unwrap();
    let boxes = hm_render::page_cells(&doc, &layout, &fonts, page);
    let bounds = boxes
        .iter()
        .find(|b| b.cell == closing as *const Cell as usize)
        .unwrap();
    assert_eq!(bounds.height, 2000);
}

/// A cut row's rest dropped at a page end takes no part of a merged cell's
/// minimum: the merged cell's last row on the next page grows by it.
#[test]
fn a_dropped_row_rest_leaves_the_merged_minimum_to_later_rows() {
    let fonts = FontSystem::new();
    let mut doc = merged_grid_document(0, 0, 0, true, 2);
    let ControlKind::Table(table) = &mut doc.sections[0].paras[0].controls[1].kind else {
        unreachable!()
    };
    table.rows = 4;
    table.header.common.height = 8800;
    table.cells[0].row_span = 4;
    table.cells[0].height = 8800;
    let right = table.cells[1].clone();
    table.cells.truncate(1);
    table.cells.extend(
        [2000, 3000, 1200, 1200]
            .iter()
            .enumerate()
            .map(|(row, &h)| Cell {
                row: row as u16,
                height: h,
                ..right.clone()
            }),
    );
    let ControlKind::Section(section) = &mut doc.sections[0].paras[0].controls[0].kind else {
        unreachable!()
    };
    let SectionChild::Page(page) = &mut section.children[0] else {
        unreachable!()
    };
    page.height = 4500;
    let layout = hm_layout::layout(&doc, &fonts);
    let first = &layout.pages[0].tables[0];
    let next = &layout.pages[1].tables[0];
    let dropped = next.top - first.bottom;
    assert!(dropped > 0 && next.carried.is_none());
    let table = doc.sections[0].paras[0].controls[1].as_table().unwrap();
    let closing = &table.cells[4];
    let page = (0..layout.pages.len())
        .find(|&p| {
            hm_render::page_cells(&doc, &layout, &fonts, p)
                .iter()
                .any(|b| b.cell == closing as *const Cell as usize)
        })
        .unwrap();
    let bounds = hm_render::page_cells(&doc, &layout, &fonts, page)
        .into_iter()
        .find(|b| b.cell == closing as *const Cell as usize)
        .unwrap();
    assert_eq!(bounds.height, 2600 + dropped);
}

fn restarted_spanning_document(align: u32, wrap: u32) -> Document {
    use hm_model::{LineSeg, SharedBytes};
    let mut doc = document(align, 8000, 2);
    doc.info.para_shapes[0].line_spacing_legacy = 130;
    let ControlKind::Table(table) = &mut doc.sections[0].paras[0].controls[1].kind else {
        unreachable!()
    };
    table.header.common.attr = (2 << 3) | (wrap << 21);
    table.header.common.y_offset = 7800;
    table.header.common.margins = [0, 0, 0, 100];
    table.rows = 4;
    table.cols = 2;
    table.padding = [100; 4];
    table.cells = vec![Cell {
        list_attr: align << 5,
        row_span: 4,
        col_span: 1,
        width: 3000,
        height: 8000,
        paras: vec![Paragraph {
            text: vec!['A' as u16, 10, 'B' as u16, PARA_END],
            src: Some(SharedBytes::from(vec![1])),
            line_segs: [0, 2]
                .map(|text_start| LineSeg {
                    text_start,
                    vpos: 0,
                    height: 1000,
                    text_height: 1000,
                    baseline: 850,
                    spacing: 300,
                    width: 2800,
                    flags: (1 << 17) | (1 << 18),
                    ..Default::default()
                })
                .into(),
            ..Default::default()
        }],
        ..Default::default()
    }];
    table.cells.extend((0..4).map(|row| Cell {
        row,
        col: 1,
        row_span: 1,
        col_span: 1,
        width: 3000,
        height: 2000,
        paras: vec![Paragraph {
            text: vec!['X' as u16, PARA_END],
            ..Default::default()
        }],
        ..Default::default()
    }));
    doc
}

#[test]
fn spanning_restarts_keep_one_owner_in_paint_and_query_sinks() {
    let fonts = FontSystem::new();
    for align in 0..=2 {
        for wrap in [0, 1] {
            let doc = restarted_spanning_document(align, wrap);
            let layout = hm_layout::layout(&doc, &fonts);
            let table = doc.sections[0].paras[0].controls[1].as_table().unwrap();
            let cell = &table.cells[0];
            let para = &cell.paras[0] as *const Paragraph as usize;
            let cell_key = cell as *const Cell as usize;
            assert_eq!(layout.pages.len(), 2);
            let mut source_starts = Vec::new();
            for page in 0..2 {
                let (_, painted, _) = hm_render::build_page_indexed(&doc, &layout, &fonts, page);
                let queried = hm_render::page_lines(&doc, &layout, &fonts, page);
                let painted: Vec<_> = painted.into_iter().filter(|l| l.para == para).collect();
                let queried: Vec<_> = queried.into_iter().filter(|l| l.para == para).collect();
                assert_eq!(painted, queried, "align={align}, wrap={wrap}, page={page}");
                assert_eq!(
                    painted.iter().map(|l| l.line).collect::<Vec<_>>(),
                    [page as u32]
                );
                let boxes: Vec<_> = hm_render::page_cells(&doc, &layout, &fonts, page)
                    .into_iter()
                    .filter(|b| b.cell == cell_key)
                    .collect();
                assert_eq!(boxes.len(), 1);
                let bounds = &boxes[0];
                assert_eq!(
                    (bounds.y, bounds.height),
                    if page == 0 { (7800, 2000) } else { (0, 6000) }
                );
                let line = &painted[0];
                if page == 1 {
                    assert_eq!(
                        line.y,
                        [100, 2500, 4900][align as usize],
                        "align={align}, wrap={wrap}"
                    );
                }
                assert!(bounds.y <= line.y && line.y + line.height <= bounds.y + bounds.height);
                source_starts.push(line.stops.first().unwrap().0);
            }
            assert_eq!(source_starts, [0, 2]);
            assert_eq!(
                layout.para(0, 0).table(1).unwrap().cells[0].lines[1].y,
                1300
            );
        }
    }
}

/// A merged grid whose first column spans four rows with an authored
/// height above its rows', the row heights given, on a page `page` high.
fn merged_minimum_document(minimum: i32, rows: [i32; 4], lines: [usize; 4], page: u32) -> Document {
    let mut doc = merged_grid_document(0, 0, 0, true, 2);
    let ControlKind::Table(table) = &mut doc.sections[0].paras[0].controls[1].kind else {
        unreachable!()
    };
    table.rows = 4;
    table.header.common.height = minimum;
    table.cells[0].row_span = 4;
    table.cells[0].height = minimum;
    let right = table.cells[1].clone();
    table.cells.truncate(1);
    table
        .cells
        .extend(rows.iter().zip(lines).enumerate().map(|(row, (&h, n))| {
            Cell {
                row: row as u16,
                height: h,
                paras: document(0, 0, n).sections[0].paras[0].controls[1]
                    .as_table()
                    .unwrap()
                    .cells[0]
                    .paras
                    .clone(),
                ..right.clone()
            }
        }));
    let ControlKind::Section(section) = &mut doc.sections[0].paras[0].controls[0].kind else {
        unreachable!()
    };
    let SectionChild::Page(def) = &mut section.children[0] else {
        unreachable!()
    };
    def.height = page;
    doc
}

/// Heights cell `index` is drawn with, page by page.
fn drawn_heights(doc: &Document, layout: &hm_layout::DocLayout, index: usize) -> Vec<i32> {
    let fonts = FontSystem::new();
    let table = doc.sections[0].paras[0].controls[1].as_table().unwrap();
    let key = &table.cells[index] as *const Cell as usize;
    (0..layout.pages.len())
        .flat_map(|p| hm_render::page_cells(doc, layout, &fonts, p))
        .filter(|b| b.cell == key)
        .map(|b| b.height)
        .collect()
}

/// A row ending within the page end's cut room, in a table that keeps no
/// room above the page end, goes whole and grows to the page end; the merged
/// cell running past it still draws its authored height.
#[test]
fn a_row_ending_within_the_cut_room_goes_whole_to_the_page_end() {
    let fonts = FontSystem::new();
    let doc = merged_minimum_document(8800, [2000, 3000, 1200, 1200], [1; 4], 5050);
    let layout = hm_layout::layout(&doc, &fonts);
    let (first, next) = (&layout.pages[0].tables[0], &layout.pages[1].tables[0]);
    assert_eq!((first.bottom, first.tail, next.top), (5000, 50, 5000));
    assert_eq!(drawn_heights(&doc, &layout, 0).iter().sum::<i32>(), 8800);
}

/// Carried lines drawn taller than the rest of their row give that room to
/// a merged cell running past them: across its pages the merged cell draws
/// its authored height.
#[test]
fn carried_parts_drawn_taller_than_their_row_count_toward_merged_minima() {
    let fonts = FontSystem::new();
    for page in [4000, 4250] {
        let doc = merged_minimum_document(10000, [2000, 3000, 1200, 1200], [1, 3, 1, 1], page);
        let layout = hm_layout::layout(&doc, &fonts);
        let lead = layout.pages[1].tables[0].carried.as_ref().unwrap();
        assert!(lead.height > lead.until - lead.end, "page {page}");
        let merged: i32 = drawn_heights(&doc, &layout, 0).iter().sum();
        assert_eq!(merged, 10000, "page {page}");
    }
}
