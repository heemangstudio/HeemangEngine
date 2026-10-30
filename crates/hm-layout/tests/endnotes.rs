//! Endnote composition measured against Mac Hancom exports.

mod common;

use hm_layout::{FrameKind, ListRef};
use hm_model::{
    ColumnDef, Control, ControlKind, DocInfo, DocMeta, Document, Format, Note, ObjectCommon,
    ObjectControl, ObjectHeader, PageDef, ParaList, Paragraph, Section, SectionChild, SectionDef,
};

fn drawing(height: i32, offset: i32, carry: bool) -> Control {
    Control {
        kind: ControlKind::Object(Box::new(ObjectControl {
            id: hm_model::CtrlId::SHAPE,
            header: ObjectHeader {
                common: ObjectCommon {
                    attr: (2 << 3)
                        | (2 << 8)
                        | (4 << 15)
                        | (2 << 18)
                        | (1 << 21)
                        | if carry { 1 << 13 } else { 0 },
                    width: 1000,
                    height,
                    y_offset: offset,
                    ..Default::default()
                },
                ..Default::default()
            },
            parts: Vec::new(),
        })),
        src: None,
    }
}

fn paragraph(controls: Vec<Control>) -> Paragraph {
    let mut text = Vec::new();
    for c in &controls {
        let code = match &c.kind {
            ControlKind::Section(_) | ControlKind::Columns(_) => 2,
            ControlKind::Note(_) => 17,
            _ => 11,
        };
        text.extend([code, 0, 0, 0, 0, 0, 0, code]);
    }
    text.push(hm_model::PARA_END);
    Paragraph {
        text,
        controls,
        ..Default::default()
    }
}

fn note_document(paras: Vec<Paragraph>) -> Document {
    let section = Control {
        kind: ControlKind::Section(Box::new(SectionDef {
            children: vec![SectionChild::Page(PageDef {
                width: 10000,
                height: 10000,
                ..Default::default()
            })],
            ..Default::default()
        })),
        src: None,
    };
    let note = Control {
        kind: ControlKind::Note(Box::new(Note {
            endnote: true,
            list: ParaList {
                paras,
                ..Default::default()
            },
            ..Default::default()
        })),
        src: None,
    };
    Document {
        meta: DocMeta {
            format: Format::Hwp5,
            version: Default::default(),
            flags: 0,
        },
        info: DocInfo {
            para_shapes: vec![hm_model::ParaShape {
                line_spacing_legacy: 100,
                ..Default::default()
            }],
            ..Default::default()
        },
        sections: vec![Section {
            paras: vec![paragraph(vec![
                section,
                Control {
                    kind: ControlKind::Columns(ColumnDef {
                        attr: (2 << 2) | (1 << 12),
                        ..Default::default()
                    }),
                    src: None,
                },
                note,
            ])],
            ..Default::default()
        }],
        source: None,
        bins: None,
        diagnostics: Vec::new(),
    }
}

fn endnote_fonts() -> hm_fonts::FontSystem {
    std::env::var("HEEMANG_FONTS")
        .map(|spec| common::fonts(&spec))
        .unwrap_or_default()
}

fn floating_endnote_table(height: i32) -> Control {
    use hm_model::{Cell, Table};
    let text = || Paragraph {
        text: vec!['A' as u16, hm_model::PARA_END],
        ..Default::default()
    };
    Control {
        kind: ControlKind::Table(Box::new(Table {
            header: ObjectHeader {
                common: ObjectCommon {
                    attr: (2 << 3) | (2 << 8) | (1 << 13) | (4 << 15) | (2 << 18) | (1 << 21),
                    width: 2000,
                    height,
                    margins: [100, 200, 300, 400],
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
                width: 2000,
                height,
                paras: vec![text()],
                ..Default::default()
            }],
            ..Default::default()
        })),
        src: None,
    }
}

#[test]
fn floating_endnote_tables_follow_their_host_column_and_precede_later_text() {
    let fonts = endnote_fonts();
    // Old-engine exports consume the same band with flowWithText disabled.
    for (prefix, flow_with_text) in [(0, true), (7, true), (8, true), (0, false), (7, false)] {
        let text = || Paragraph {
            text: vec!['A' as u16, hm_model::PARA_END],
            ..Default::default()
        };
        let mut table = floating_endnote_table(2500);
        if !flow_with_text {
            let ControlKind::Table(t) = &mut table.kind else {
                unreachable!()
            };
            t.header.common.attr &= !(1 << 13);
        }
        let mut paras: Vec<_> = (0..prefix).map(|_| text()).collect();
        paras.push(paragraph(vec![table]));
        paras.push(text());
        let doc = note_document(paras);
        let bytes = hm_hwpx::write_new(&doc).unwrap();
        let doc = hm_hwpx::read(hm_model::SharedBytes::from(bytes)).unwrap();
        let layout = hm_layout::layout(&doc, &fonts);
        let tables: Vec<_> = layout
            .pages
            .iter()
            .enumerate()
            .flat_map(|(pi, page)| page.frames.iter().map(move |frame| (pi, frame)))
            .flat_map(|(pi, frame)| frame.tables.iter().map(move |table| (pi, frame, table)))
            .collect();
        assert_eq!(tables.len(), 1, "the floating table's grid must be painted");
        let (page, frame, table) = tables[0];
        let host = frame
            .lines
            .iter()
            .find(|l| l.para as usize == prefix)
            .unwrap();
        assert_eq!(table.x, frame.rect.x + 100);
        assert_eq!(table.y, host.y + 300);
        assert_eq!(table.height(), 2500);
        assert!(table.y >= layout.pages[page].body.y);
        assert!(table.y + table.height() <= layout.pages[page].body.y + layout.pages[page].body.h);
        let (later_page, later_frame, later) = layout
            .pages
            .iter()
            .enumerate()
            .flat_map(|(pi, page)| page.frames.iter().map(move |frame| (pi, frame)))
            .flat_map(|(pi, frame)| frame.lines.iter().map(move |line| (pi, frame, line)))
            .find(|(_, _, line)| line.para as usize == prefix + 1)
            .unwrap();
        if later_page == page && later_frame.rect.x == frame.rect.x {
            assert!(later.y >= table.y + table.height() + 400);
        } else {
            assert!((later_page, later_frame.rect.x) > (page, frame.rect.x));
        }
        if prefix > 0 {
            assert!(
                frame.rect.x > layout.pages[page].body.x,
                "the host moved to the next column"
            );
        }
        assert!(layout.pages.iter().all(|p| p.tables.is_empty()));
    }
}

#[test]
fn a_table_at_the_end_of_an_endnote_reserves_its_bottom_margin_before_the_next_note() {
    let fonts = endnote_fonts();
    let mut doc = note_document(vec![paragraph(vec![floating_endnote_table(2500)])]);
    doc.sections[0].paras[0].controls.push(Control {
        kind: ControlKind::Note(Box::new(Note {
            endnote: true,
            list: ParaList {
                paras: vec![Paragraph {
                    text: vec!['B' as u16, hm_model::PARA_END],
                    ..Default::default()
                }],
                ..Default::default()
            },
            ..Default::default()
        })),
        src: None,
    });
    let controls = doc.sections[0].paras[0].controls.clone();
    doc.sections[0].paras[0] = paragraph(controls);
    let bytes = hm_hwpx::write_new(&doc).unwrap();
    let reopened = hm_hwpx::read(hm_model::SharedBytes::from(bytes)).unwrap();
    for doc in [doc, reopened] {
        let layout = hm_layout::layout(&doc, &fonts);
        let notes: Vec<_> = layout
            .pages
            .iter()
            .flat_map(|p| &p.frames)
            .filter(|f| f.kind == FrameKind::Endnotes)
            .collect();
        let table = &notes[0].tables[0];
        assert_eq!((table.x, table.y, table.height()), (100, 1300, 2500));
        assert_eq!(notes[1].lines[0].y, table.y + table.height() + 400);
        assert_eq!(notes[1].lines[0].y, 4200);
    }
}

#[test]
fn endnote_table_captions_and_exact_bottom_margins_participate_in_flow() {
    let fonts = endnote_fonts();
    for side in [2, 3] {
        let mut control = floating_endnote_table(2500);
        let ControlKind::Table(table) = &mut control.kind else {
            unreachable!()
        };
        table.caption = Some(hm_model::Caption {
            attr: Some(side),
            gap: Some(200),
            max_width: Some(2000),
            count: 1,
            paras: vec![Paragraph {
                text: vec!['C' as u16, hm_model::PARA_END],
                ..Default::default()
            }],
            ..Default::default()
        });
        let doc = note_document(vec![
            paragraph(vec![control]),
            Paragraph {
                text: vec!['B' as u16, hm_model::PARA_END],
                ..Default::default()
            },
        ]);
        let layout = hm_layout::layout(&doc, &fonts);
        let frame = layout
            .pages
            .iter()
            .flat_map(|p| &p.frames)
            .find(|f| !f.tables.is_empty())
            .unwrap();
        let grid = &frame.tables[0];
        let table = frame.paras[0].table(0).unwrap();
        let outer_bottom = grid.y - table.grid_offset().1 + table.outer_size().1;
        assert!(grid.caption);
        let later = frame.lines.iter().find(|l| l.para == 1).unwrap();
        assert!(later.y >= outer_bottom + 400);
        assert!(frame.rect.y + frame.rect.h >= outer_bottom);
    }
    let doc = note_document(vec![paragraph(vec![floating_endnote_table(8300)])]);
    let layout = hm_layout::layout(&doc, &fonts);
    let grids: Vec<_> = layout
        .pages
        .iter()
        .flat_map(|p| &p.frames)
        .flat_map(|f| &f.tables)
        .collect();
    assert_eq!(grids.len(), 1);
    assert_eq!(
        (grids[0].x, grids[0].y, grids[0].height()),
        (100, 1300, 8300)
    );
}

#[test]
fn an_endnote_table_taller_than_a_column_keeps_every_row_and_control() {
    let fonts = endnote_fonts();
    for split in [0, 1, 2] {
        let mut control = floating_endnote_table(12000);
        let ControlKind::Table(table) = &mut control.kind else {
            unreachable!()
        };
        table.attr = split;
        if split == 1 {
            table.rows = 2;
            table.row_cells = vec![1, 1];
            table.cells[0].height = 6000;
            let mut second = table.cells[0].clone();
            second.row = 1;
            table.cells.push(second);
        }
        let doc = note_document(vec![
            paragraph(vec![control]),
            Paragraph {
                text: vec!['B' as u16, hm_model::PARA_END],
                ..Default::default()
            },
        ]);
        let layout = hm_layout::layout(&doc, &fonts);
        let frames: Vec<_> = layout
            .pages
            .iter()
            .flat_map(|p| &p.frames)
            .filter(|f| f.kind == FrameKind::Endnotes)
            .collect();
        let grids: Vec<_> = frames.iter().flat_map(|f| &f.tables).collect();
        assert_eq!(
            grids.len(),
            1,
            "split mode {split}: the whole table remains present"
        );
        assert_eq!(grids[0].height(), 12000);
        assert_eq!(grids[0].top, 0);
        assert_eq!(grids[0].bottom, 12000);
        let table = frames.iter().find(|f| !f.tables.is_empty()).unwrap().paras[0]
            .table(0)
            .unwrap();
        assert_eq!(table.cells.len(), if split == 1 { 2 } else { 1 });
        assert_eq!(frames.iter().flat_map(|f| &f.lines).count(), 2);
    }
}

#[test]
fn endnotes_inside_nested_tables_keep_their_place_in_the_final_list() {
    let note = |text: &str| Control {
        kind: ControlKind::Note(Box::new(Note {
            endnote: true,
            list: ParaList {
                paras: vec![Paragraph {
                    text: format!("{text}\r").encode_utf16().collect(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            ..Default::default()
        })),
        src: None,
    };
    let table = |control| Control {
        kind: ControlKind::Table(Box::new(hm_model::Table {
            rows: 1,
            cols: 1,
            header: ObjectHeader {
                common: ObjectCommon {
                    attr: 1,
                    width: 3000,
                    height: 1000,
                    ..Default::default()
                },
                ..Default::default()
            },
            cells: vec![hm_model::Cell {
                width: 3000,
                height: 1000,
                row_span: 1,
                col_span: 1,
                paras: vec![paragraph(vec![control])],
                ..Default::default()
            }],
            ..Default::default()
        })),
        src: None,
    };
    let mut text_box = drawing(1000, 0, false);
    let ControlKind::Object(o) = &mut text_box.kind else {
        unreachable!()
    };
    o.parts
        .push(hm_model::ObjectPart::Shape(Box::new(hm_model::Shape {
            component: hm_model::ShapeComponent {
                kind: hm_model::CtrlId::from_chars(b"$rec"),
                doubled: true,
                offset: (0, 0),
                group_level: 0,
                version: 0,
                initial_size: (3000, 1000),
                size: (3000, 1000),
                flip: 0,
                rotation: 0,
                rotation_center: (0, 0),
                translation: hm_model::Matrix::IDENTITY,
                transforms: Vec::new(),
                style: None,
                members: None,
                group_instance: None,
                tail: Default::default(),
            },
            parts: vec![hm_model::ShapePart::Text(hm_model::TextBox {
                paras: vec![paragraph(vec![table(note("second"))])],
                ..Default::default()
            })],
        })));
    for nested in [table(table(note("second"))), table(text_box)] {
        let mut doc = note_document(Vec::new());
        let mut controls = doc.sections[0].paras[0].controls[..2].to_vec();
        controls.extend([note("first"), nested, note("third")]);
        doc.sections[0].paras = vec![paragraph(controls)];
        let layout = hm_layout::layout(&doc, &hm_fonts::FontSystem::new());
        let texts: Vec<_> = layout
            .pages
            .iter()
            .flat_map(|p| &p.frames)
            .filter(|f| f.kind == FrameKind::Endnotes)
            .map(|f| {
                let paras = f.list.paras(&doc).expect("the endnote reference resolves");
                String::from_utf16_lossy(&paras[0].text)
            })
            .collect();
        assert_eq!(texts, ["first\r", "second\r", "third\r"]);
    }
}

#[test]
fn carried_endnote_drawings_and_resumed_text_fit_their_columns() {
    for (heights, resume) in [
        (&[6000, 6000][..], false),
        (&[6000, 6000][..], true),
        (&[9500][..], true),
    ] {
        let mut paras = vec![paragraph(
            heights.iter().map(|&h| drawing(h, 4000, true)).collect(),
        )];
        if resume {
            paras.extend((0..10).map(|_| paragraph(Vec::new())));
        }
        let expected_lines = paras.len();
        let doc = note_document(paras);
        let layout = hm_layout::layout(&doc, &hm_fonts::FontSystem::new());
        let mut objects = 0;
        let mut lines = 0;
        for (pi, page) in layout.pages.iter().enumerate() {
            for frame in &page.frames {
                for object in &frame.objects {
                    objects += 1;
                    assert!(object.rect.y >= page.body.y, "page {pi}: {object:?}");
                    assert!(
                        object.rect.y + object.rect.h <= page.body.y + page.body.h,
                        "page {pi}: {object:?}"
                    );
                    for text in page
                        .frames
                        .iter()
                        .flat_map(|f| &f.lines)
                        .filter(|l| l.x == object.rect.x)
                    {
                        assert!(
                            text.y + 1000 <= object.rect.y
                                || text.y >= object.rect.y + object.rect.h,
                            "page {pi}: text {text:?} overlaps {object:?}"
                        );
                    }
                }
                for line in &frame.lines {
                    lines += 1;
                    assert!(
                        line.y + frame.paras[line.para as usize].lines[line.line as usize].height
                            < page.body.y + page.body.h,
                        "page {pi}: {line:?}"
                    );
                }
            }
        }
        assert_eq!(objects, heights.len());
        assert_eq!(lines, expected_lines);
    }
}

#[test]
fn cached_endnote_membership_survives_column_fragment_resets() {
    let fonts = hm_fonts::FontSystem::new();
    for ys in [[0, 0], [0, 1200], [7080, 8280]] {
        let note = Paragraph {
            text: "AB CD\r".encode_utf16().collect(),
            runs: vec![hm_model::CharRun { pos: 0, shape: 0 }],
            line_segs: [0, 3]
                .into_iter()
                .zip(ys)
                .map(|(text_start, vpos)| hm_model::LineSeg {
                    text_start,
                    vpos,
                    height: 1000,
                    text_height: 1000,
                    baseline: 850,
                    spacing: 0,
                    col_start: 0,
                    width: 5000,
                    flags: hm_model::LineSeg::FIRST_SEGMENT | hm_model::LineSeg::LAST_SEGMENT,
                })
                .collect(),
            ..Default::default()
        };
        let stored = note.line_segs.clone();
        let mut doc = note_document(vec![note]);
        doc.info.char_shapes.push(hm_model::CharShape {
            base_size: 1000,
            ratios: [100; 7],
            ..Default::default()
        });
        let ControlKind::Section(section) = &mut doc.sections[0].paras[0].controls[0].kind else {
            unreachable!()
        };
        let SectionChild::Page(page) = &mut section.children[0] else {
            unreachable!()
        };
        page.height = 2500;
        let doc = hm_hwp5::read(hm_model::SharedBytes::from(
            hm_hwp5::write_new(&doc).unwrap(),
        ))
        .unwrap();
        assert!(
            doc.sections[0].paras[0].controls[2]
                .as_note()
                .unwrap()
                .list
                .paras[0]
                .src
                .is_some()
        );
        let layout = hm_layout::layout(&doc, &fonts);
        let frames: Vec<_> = layout.pages[0]
            .frames
            .iter()
            .filter(|f| f.kind == FrameKind::Endnotes)
            .collect();
        assert_eq!(frames.len(), 2, "{ys:?}");
        assert_eq!((frames[0].rect.x, frames[0].rect.y), (0, 1000));
        assert_eq!((frames[1].rect.x, frames[1].rect.y), (5000, 0));
        for (i, frame) in frames.iter().enumerate() {
            assert_eq!(frame.lines.len(), 1);
            assert_eq!(frame.paras[0].lines[i].start, (3 * i) as u32);
            assert_eq!(frame.rect.h, 1000);
        }
        let note = doc.sections[0].paras[0].controls[2].as_note().unwrap();
        assert_eq!(note.list.paras[0].line_segs, stored);
        let fresh = hm_layout::layout_with(
            &doc,
            &fonts,
            hm_layout::Options {
                stored_lines: false,
            },
        );
        assert_eq!(fresh.pages[0].frames[0].lines.len(), 1);

        // The same source authority also governs the endnote path, whose
        // vertical fragment positions remain independent of membership.
        let mut xml_doc = doc.clone();
        xml_doc.convert_to(Format::Hwpx);
        let bytes = hm_hwpx::write_new(&xml_doc).unwrap();
        let archive = hm_hwpx::zip::Archive::open(bytes.into()).unwrap();
        for flags in [0, 8, 48, 56] {
            let mut parts = Vec::new();
            for entry in &archive.entries {
                let raw = archive.read_name(&entry.name).unwrap().unwrap();
                let raw = if entry.name == "Contents/header.xml" {
                    String::from_utf8(raw.to_vec())
                        .unwrap()
                        .replace(
                            "</hh:head>",
                            &format!("<hh:trackchageConfig flags=\"{flags}\"/></hh:head>"),
                        )
                        .into_bytes()
                } else {
                    raw.to_vec()
                };
                parts.push((entry.name.clone(), raw));
            }
            let input = hm_hwpx::zip::write(
                None,
                &parts
                    .iter()
                    .map(|(name, data)| hm_hwpx::zip::Member::New {
                        name,
                        data,
                        stored: true,
                    })
                    .collect::<Vec<_>>(),
            );
            let imported = hm_hwpx::read(input.into()).unwrap();
            let actual = hm_layout::layout(&imported, &fonts);
            let expected = hm_layout::layout_with(
                &imported,
                &fonts,
                hm_layout::Options {
                    stored_lines: flags & 8 != 0,
                },
            );
            assert_eq!(
                format!("{:?}", actual.pages),
                format!("{:?}", expected.pages)
            );
            let note = imported.sections[0].paras[0].controls[2].as_note().unwrap();
            assert_eq!(note.list.paras[0].line_segs, stored);
            let starts: Vec<_> = actual
                .pages
                .iter()
                .flat_map(|p| &p.frames)
                .filter(|f| f.kind == FrameKind::Endnotes)
                .flat_map(|f| &f.lines)
                .map(|line| line.line)
                .collect();
            assert_eq!(starts, if flags & 8 != 0 { vec![0, 1] } else { vec![0] });
        }
    }
}

#[test]
fn carrying_a_drawing_preserves_all_anchor_lines_and_independent_drawings() {
    for reverse in [false, true] {
        let mut controls = vec![drawing(2000, 0, false), drawing(6000, 4000, true)];
        if reverse {
            controls.reverse();
        }
        let mut host = paragraph(controls);
        let end = host.text.len() - 1;
        host.text.splice(
            end..end,
            [10, b'A' as u16, 10, b'B' as u16, 10, b'C' as u16],
        );
        let doc = note_document(vec![
            host,
            paragraph(vec![drawing(3000, 0, false)]),
            paragraph(Vec::new()),
        ]);
        let layout = hm_layout::layout(&doc, &hm_fonts::FontSystem::new());
        let frame = layout.pages[0]
            .frames
            .iter()
            .find(|f| !f.lines.is_empty())
            .unwrap();
        assert_eq!(frame.lines.len(), 6);
        assert_eq!(frame.objects.len(), 2);
        for pair in frame.lines.windows(2) {
            let previous = &frame.paras[pair[0].para as usize].lines[pair[0].line as usize];
            assert!(pair[1].y >= pair[0].y + previous.height, "{pair:?}");
        }
        for object in &frame.objects {
            for next in frame.lines.iter().filter(|l| l.para > object.para) {
                assert!(
                    next.y >= object.rect.y + object.rect.h,
                    "{next:?} {object:?}"
                );
            }
        }
    }
}

/// A picture crossing the column foot flows on independently. Its anchor
/// and the following equation remain in the first column, while the next
/// note follows the picture in the second column.
#[test]
fn endnote_drawings_carry_without_moving_their_anchor_text() {
    let Some((corpus, fonts)) = common::setup() else {
        return;
    };
    for ext in ["hwp", "hwpx"] {
        let path = corpus.join(format!(
            "3-11월_실전_통합_2024-구분선위20미주사이0구분선아래20.{ext}"
        ));
        let Some(doc) = common::read(&path) else {
            continue;
        };
        let layout = hm_layout::layout(&doc, &fonts);
        assert_eq!(layout.pages.len(), 21, "{ext}");
        let page = &layout.pages[10];
        let note = ListRef::Control {
            section: 0,
            para: 99,
            control: 0,
        };
        let anchor = page
            .frames
            .iter()
            .find(|f| f.kind == FrameKind::Endnotes && f.list == note && !f.lines.is_empty())
            .expect("anchor text stays on page 11");
        assert!(anchor.lines.iter().any(|l| l.para == 12), "{ext}");
        assert!(anchor.objects.is_empty(), "{ext}");
        let picture = page
            .frames
            .iter()
            .find(|f| f.kind == FrameKind::Endnotes && f.list == note && !f.objects.is_empty())
            .expect("picture continues in the next column");
        assert!(picture.rect.x > anchor.rect.x, "{ext}");
        assert!((picture.objects[0].rect.y - 6803).abs() <= 12, "{ext}");
        let next = page
            .frames
            .iter()
            .find(|f| {
                f.list
                    == ListRef::Control {
                        section: 0,
                        para: 108,
                        control: 0,
                    }
            })
            .expect("note 13 follows the picture");
        assert_eq!(next.rect.x, picture.rect.x, "{ext}");
        assert!(
            next.rect.y >= picture.objects[0].rect.y + picture.objects[0].rect.h,
            "{ext}"
        );
        // Hancom's note-13 heading baseline is 19272 HU, with a 765 HU
        // text baseline in the heading's top-aligned 900 HU line.
        assert!(
            (next.rect.y + 765 - 19272).abs() <= 12,
            "{ext}: {}",
            next.rect.y
        );
    }
}

#[test]
fn endnote_tables_admit_visible_grids_above_footnotes() {
    let fonts = endnote_fonts();
    // The table's outside margin can share the footnote area; its visible
    // grid must move when it fits an empty column but not the reserved one.
    // Endnotes leave the body's page when it has footnotes, so the footnote
    // area they meet is the rest of a ten-line footnote carried to their
    // page: two lines with no gaps leave the table's grid in the first
    // column with its bottom margin over the notes; more lines, or the gaps,
    // push it to the second column.
    for (height, above, below, expected) in [
        (7500, 0, 0, (100, 300)),
        (6000, 850, 567, (5100, 300)),
        (6000, 2000, 2000, (5100, 300)),
        (7500, 2000, 2000, (5100, 300)),
    ] {
        let mut doc = note_document(vec![paragraph(vec![floating_endnote_table(height)])]);
        let ControlKind::Section(section) = &mut doc.sections[0].paras[0].controls[0].kind else {
            unreachable!()
        };
        section.children.extend([
            SectionChild::NoteShape(hm_model::NoteShape {
                space_above: above,
                space_below: below,
                ..Default::default()
            }),
            SectionChild::NoteShape(hm_model::NoteShape::default()),
        ]);
        let mut controls = doc.sections[0].paras[0].controls.clone();
        controls.push(Control {
            kind: ControlKind::Note(Box::new(Note {
                list: ParaList {
                    paras: vec![
                        Paragraph {
                            text: vec!['F' as u16, hm_model::PARA_END],
                            ..Default::default()
                        };
                        10
                    ],
                    ..Default::default()
                },
                ..Default::default()
            })),
            src: None,
        });
        doc.sections[0].paras[0] = paragraph(controls);
        doc.info.char_shapes.push(hm_model::CharShape {
            base_size: 1000,
            ratios: [100; 7],
            rel_sizes: [100; 7],
            ..Default::default()
        });
        let bytes = hm_hwpx::write_new(&doc).unwrap();
        let doc = hm_hwpx::read(hm_model::SharedBytes::from(bytes)).unwrap();
        let layout = hm_layout::layout(&doc, &fonts);
        let grids: Vec<_> = layout
            .pages
            .iter()
            .flat_map(|p| &p.frames)
            .flat_map(|f| &f.tables)
            .collect();
        assert_eq!(grids.len(), 1);
        assert_eq!(
            (grids[0].x, grids[0].y),
            expected,
            "height={height}, footnote gaps={above}/{below}"
        );
        assert_eq!(grids[0].height(), height);
    }
}
