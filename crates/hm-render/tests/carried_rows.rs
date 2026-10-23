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

#[test]
fn authored_row_minimum_survives_three_cell_splits() {
    for (align, baselines) in [
        (0, [950, 950, 950, 950]),
        (1, [1350, 1300, 1300, 950]),
        (2, [1750, 1650, 1650, 950]),
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
        (1, [1350, 1300, 1300, 950]),
        (2, [1750, 1650, 1650, 950]),
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
