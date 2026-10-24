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
