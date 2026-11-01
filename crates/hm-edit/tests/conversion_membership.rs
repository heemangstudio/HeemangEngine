//! Cross-format output uses the current layout without editing the open file.
use hm_edit::{Editor, lines::prepare_conversion};
use hm_fonts::FontSystem;
use hm_model::{Document, Format, Paragraph};

fn paragraph(text: &str) -> String {
    format!(
        r#"<hp:p><hp:run charPrIDRef="0"><hp:t>{text}</hp:t></hp:run><hp:linesegarray><hp:lineseg textpos="0" vertpos="0" vertsize="1000" textheight="1000" baseline="850" spacing="600" horzpos="0" horzsize="4000" flags="393216"/><hp:lineseg textpos="3" vertpos="1600" vertsize="1000" textheight="1000" baseline="850" spacing="600" horzpos="0" horzsize="4000" flags="393216"/></hp:linesegarray></hp:p>"#
    )
}

fn package(body: &str, flags: u32) -> Vec<u8> {
    let header = format!(
        r#"<hh:head version="1.5" xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head"><hh:refList><hh:charProperties itemCnt="1"><hh:charPr id="0" height="1000"/></hh:charProperties><hh:paraProperties itemCnt="1"><hh:paraPr id="0"><hh:lineSpacing type="PERCENT" value="160" unit="HWPUNIT"/></hh:paraPr></hh:paraProperties></hh:refList><hh:trackchageConfig flags="{flags}"/></hh:head>"#
    );
    let section = format!(
        r#"<hs:sec xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section" xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph"><hp:p><hp:run charPrIDRef="0"><hp:secPr><hp:pagePr width="4000" height="12000"><hp:margin left="0" right="0" top="0" bottom="0" header="1000" footer="1000" gutter="0"/></hp:pagePr></hp:secPr></hp:run></hp:p>{body}</hs:sec>"#
    );
    let parts = [
        ("mimetype", b"application/hwp+zip".as_slice()),
        ("Contents/content.hpf", br#"<opf:package xmlns:opf="http://www.idpf.org/2007/opf"><opf:manifest><opf:item id="header" href="Contents/header.xml"/><opf:item id="section0" href="Contents/section0.xml"/></opf:manifest><opf:spine><opf:itemref idref="section0"/></opf:spine></opf:package>"#.as_slice()),
        ("Contents/header.xml", header.as_bytes()),
        ("Contents/section0.xml", section.as_bytes()),
    ];
    hm_hwpx::zip::write(
        None,
        &parts
            .iter()
            .map(|(name, data)| hm_hwpx::zip::Member::New {
                name,
                data,
                stored: true,
            })
            .collect::<Vec<_>>(),
    )
}

fn read(bytes: Vec<u8>) -> Document {
    hm_hwpx::read(bytes.into()).unwrap()
}
fn starts(p: &Paragraph) -> Vec<u32> {
    p.line_segs.iter().map(|l| l.text_start).collect()
}

#[test]
fn conversion_materializes_selected_membership_and_keeps_source_history() {
    for flags in [0, 8, 48, 56] {
        let bytes = package(
            &paragraph("abcdefghijklmnopqrstuvwxyzabcdefghijklmno"),
            flags,
        );
        let mut editor = Editor::new(read(bytes.clone()));
        let before = format!("{:?}", editor.doc);
        let history = editor.history_id();
        let fonts = FontSystem::new();
        let layout = editor.layout(&fonts);
        let expected: Vec<_> = layout.para(0, 1).lines.iter().map(|l| l.start).collect();
        if flags & 8 == 0 {
            assert_ne!(expected, vec![0, 3]);
        } else {
            assert_eq!(expected, vec![0, 3]);
        }
        let output = prepare_conversion(&editor.doc, &layout, Format::Hwp5).unwrap();
        assert!(output.source.is_none());
        assert!(output.sections[0].paras[1].src.is_none());
        assert_eq!(starts(&output.sections[0].paras[1]), expected);
        let back = hm_hwp5::read(hm_hwp5::write_new(&output).unwrap().into()).unwrap();
        assert_eq!(
            back.info.imported_line_membership,
            hm_model::ImportedLineMembership::Unknown
        );
        let reopened = hm_layout::layout(&back, &fonts);
        assert_eq!(
            reopened
                .para(0, 1)
                .lines
                .iter()
                .map(|l| l.start)
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(
            back.sections[0].paras[1].text,
            editor.doc.sections[0].paras[1].text
        );
        assert_eq!(
            back.sections[0].paras[1].runs,
            editor.doc.sections[0].paras[1].runs
        );
        assert_eq!(editor.history_id(), history);
        assert_eq!(format!("{:?}", editor.doc), before);
        assert_eq!(hm_hwpx::write(&editor.doc).unwrap(), bytes);
        assert_eq!(starts(&editor.doc.sections[0].paras[1]), vec![0, 3]);
        let same = prepare_conversion(&editor.doc, &Default::default(), Format::Hwpx).unwrap();
        assert_eq!(hm_hwpx::write(&same).unwrap(), bytes);
    }
}

#[test]
fn missing_list_layout_refuses_conversion_without_changing_the_source() {
    let source = read(package(&paragraph("abcdefghijklmnop"), 0));
    let before = format!("{source:?}");
    let error = prepare_conversion(&source, &Default::default(), Format::Hwp5).unwrap_err();
    assert!(error.contains("section count"));
    assert_eq!(format!("{source:?}"), before);
}

fn host(controls: &str) -> String {
    format!(r#"<hp:p><hp:run charPrIDRef="0">{controls}<hp:t>host text</hp:t></hp:run></hp:p>"#)
}

fn caption() -> String {
    format!(
        r#"<hp:caption side="BOTTOM" width="4000" lastWidth="4000" gap="100"><hp:subList>{}</hp:subList></hp:caption>"#,
        paragraph("caption text abcdefghijklmnopqrstuvwxyz")
    )
}

fn table(contents: &str, cap: bool) -> String {
    format!(
        r#"<hp:tbl rowCnt="1" colCnt="1" pageBreak="CELL"><hp:sz width="4000" height="2000"/><hp:pos treatAsChar="1"/>{}<hp:tr><hp:tc><hp:subList>{contents}</hp:subList><hp:cellAddr colAddr="0" rowAddr="0"/><hp:cellSpan colSpan="1" rowSpan="1"/><hp:cellSz width="4000" height="2000"/></hp:tc></hp:tr></hp:tbl>"#,
        if cap { caption() } else { String::new() }
    )
}

fn shape(contents: &str, links: (u32, u32), cap: bool) -> String {
    format!(
        r#"<hp:rect id="1"><hp:sz width="4000" height="2000"/><hp:pos treatAsChar="1"/><hp:orgSz width="4000" height="2000"/><hp:curSz width="4000" height="2000"/><hp:drawText lastWidth="4000"><hp:subList linkListIDRef="{}" linkListNextIDRef="{}">{contents}</hp:subList><hp:textMargin left="0" right="0" top="0" bottom="0"/></hp:drawText>{}</hp:rect>"#,
        links.0,
        links.1,
        if cap { caption() } else { String::new() }
    )
}

fn inventory(doc: &Document) -> Vec<(Vec<u16>, Vec<hm_model::CharRun>, Vec<u32>)> {
    fn walk(ps: &[Paragraph], out: &mut Vec<(Vec<u16>, Vec<hm_model::CharRun>, Vec<u32>)>) {
        for p in ps {
            out.push((p.text.clone(), p.runs.clone(), starts(p)));
            for c in &p.controls {
                for list in hm_edit::ops::control_lists(c) {
                    walk(list, out);
                }
            }
        }
    }
    let mut out = vec![];
    for s in &doc.sections {
        walk(&s.paras, &mut out);
    }
    out
}

fn check_context(source: &Document, minimum_lists: usize) {
    let fonts = FontSystem::new();
    let layout = hm_layout::layout(source, &fonts);
    let output = prepare_conversion(source, &layout, Format::Hwp5).unwrap();
    let expected = inventory(&output);
    assert!(
        expected.len() >= minimum_lists,
        "fixture must exercise its nested lists"
    );
    let back = hm_hwp5::read(hm_hwp5::write_new(&output).unwrap().into()).unwrap();
    assert_eq!(inventory(&back), expected);
    let reopened = hm_layout::layout(&back, &fonts);
    let selected = prepare_conversion(&back, &reopened, Format::Hwpx).unwrap();
    assert_eq!(
        inventory(&selected),
        expected,
        "reopened layout must select the written membership"
    );
    let roundtrip = read(hm_hwpx::write_new(&selected).unwrap());
    assert_eq!(inventory(&roundtrip), expected);
    assert_eq!(layout.pages.len(), reopened.pages.len());
}

#[test]
fn tables_nested_cells_object_frames_and_both_caption_kinds_reopen() {
    let text = paragraph("abcdefghijklmnopqrstuvwxyz abcdefghijklmnopqrstuvwxyz");
    let body = host(&table(
        &format!("{text}{}", host(&table(&text, true))),
        true,
    ));
    check_context(&read(package(&body, 0)), 7);
    let body = host(&shape(
        &format!("{text}{}", host(&table(&text, true))),
        (0, 0),
        true,
    ));
    check_context(&read(package(&body, 0)), 7);
}

#[test]
fn header_footer_footnote_and_split_endnote_lists_reopen() {
    for kind in ["header", "footer", "footNote", "endNote"] {
        let text = if kind.ends_with("Note") {
            "abcdefghijklmnopqrstuvwxyz ".repeat(12)
        } else {
            "abcdefghijklmnopqrstuvwxyz ".repeat(2)
        };
        let body = host(&format!(
            r#"<hp:ctrl><hp:{kind} applyPageType="BOTH" number="1"><hp:subList textWidth="4000" textHeight="1000">{}</hp:subList></hp:{kind}></hp:ctrl>"#,
            paragraph(&text)
        ));
        let source = read(package(&body, 0));
        let layout = hm_layout::layout(&source, &FontSystem::new());
        assert!(
            layout
                .pages
                .iter()
                .flat_map(|p| &p.frames)
                .any(|f| !f.paras.is_empty()),
            "{kind} fixture has a real frame"
        );
        if kind.ends_with("Note") {
            let frame_kind = if kind == "endNote" {
                hm_layout::FrameKind::Endnotes
            } else {
                hm_layout::FrameKind::Footnotes
            };
            assert!(
                layout
                    .pages
                    .iter()
                    .flat_map(|p| &p.frames)
                    .filter(|f| f.kind == frame_kind)
                    .count()
                    > 1
            );
        }
        check_context(&source, 3);
    }
}

#[test]
fn footnotes_reached_through_actual_cell_and_shape_references_reopen() {
    let note = format!(
        "<hp:ctrl><hp:footNote><hp:subList>{}</hp:subList></hp:footNote></hp:ctrl>",
        paragraph("note text abcdefghijklmnopqrstuvwxyz")
    );
    for body in [
        host(&table(&host(&note), false)),
        host(&shape(&host(&note), (0, 0), false)),
    ] {
        let source = read(package(&body, 0));
        let layout = hm_layout::layout(&source, &FontSystem::new());
        assert!(
            layout
                .pages
                .iter()
                .flat_map(|p| &p.frames)
                .any(|f| matches!(f.list, hm_layout::ListRef::Cell { .. })),
            "fixture must resolve its note through the actual cell/text-frame source path"
        );
        check_context(&source, 4);
    }
}

#[test]
fn selected_fresh_mode_overrides_a_positive_imported_cache_only_on_output() {
    let source = read(package(&paragraph("abcdefghijklmnopqrstuvwxyzabcdef"), 8));
    let fonts = FontSystem::new();
    let layout = hm_layout::layout_with(
        &source,
        &fonts,
        hm_layout::Options {
            stored_lines: false,
            ..Default::default()
        },
    );
    let output = prepare_conversion(&source, &layout, Format::Hwp5).unwrap();
    assert_ne!(
        starts(&output.sections[0].paras[1]),
        starts(&source.sections[0].paras[1])
    );
    let back = hm_hwp5::read(hm_hwp5::write_new(&output).unwrap().into()).unwrap();
    assert_eq!(
        hm_layout::layout(&back, &fonts)
            .para(0, 1)
            .lines
            .iter()
            .map(|l| l.start)
            .collect::<Vec<_>>(),
        starts(&output.sections[0].paras[1])
    );
}

#[test]
fn linked_boxes_refuse_output_until_writers_preserve_their_ids() {
    let body = host(&format!(
        "{}{}",
        shape(
            &paragraph("abcdefghijklmnopqrstuvwxyz ".repeat(6).as_str()),
            (1, 2),
            false
        ),
        shape("", (2, 0), false)
    ));
    let source = read(package(&body, 0));
    let before = format!("{source:?}");
    let layout = hm_layout::layout(&source, &FontSystem::new());
    let objects = &layout.para(0, 1).measured.objects;
    assert!(
        objects
            .iter()
            .flat_map(|(_, o)| &o.frames)
            .any(|f| f.borrowed.is_some()),
        "fixture must actually flow into the linked box"
    );
    let error = prepare_conversion(&source, &layout, Format::Hwp5).unwrap_err();
    assert!(error.contains("linked text-box IDs"), "{error}");
    assert_eq!(format!("{source:?}"), before);
}

#[test]
fn last_page_master_membership_survives_both_formats() {
    let mut source = read(package(&paragraph("abcdefghijklmnop"), 0));
    source.sections[0].last_page_master = Some(hm_model::ParaList {
        paras: vec![source.sections[0].paras[1].clone()],
        ..Default::default()
    });
    check_context(&source, 2);
}

#[test]
fn unplaced_furniture_keeps_its_stored_lines() {
    let body = host(&format!(
        r#"<hp:ctrl><hp:header applyPageType="EVEN"><hp:subList textWidth="4000" textHeight="1000">{}</hp:subList></hp:header></hp:ctrl>"#,
        paragraph("abcdefghijklmnop")
    ));
    // Stored lines are authoritative here, so an unchanged list no page
    // shows needs no layout.
    let source = read(package(&body, 8));
    let layout = hm_layout::layout(&source, &FontSystem::new());
    let output = prepare_conversion(&source, &layout, Format::Hwp5).unwrap();
    assert_eq!(inventory(&output), inventory(&source));
    let header = |d: &Document| {
        d.sections[0].paras[1].controls[0]
            .as_header_footer()
            .unwrap()
            .list
            .paras[0]
            .line_segs
            .clone()
    };
    assert_eq!(header(&output), header(&source));
    assert!(!header(&source).is_empty());
}

#[test]
fn ordinary_master_page_membership_survives_both_formats() {
    let mut source = read(package(
        &paragraph("abcdefghijklmnopqrstuvwxyzabcdefghijklmnop"),
        0,
    ));
    let master = hm_model::ParaList {
        text_size: Some((4000, 1000)),
        paras: vec![source.sections[0].paras[1].clone()],
        ..Default::default()
    };
    let hm_model::ControlKind::Section(def) = &mut source.sections[0].paras[0].controls[0].kind
    else {
        panic!("section")
    };
    def.attr |= 1 << 29;
    def.children
        .push(hm_model::SectionChild::MasterPage(master));
    let layout = hm_layout::layout(&source, &FontSystem::new());
    assert!(
        layout
            .pages
            .iter()
            .flat_map(|p| &p.frames)
            .any(|f| f.kind == hm_layout::FrameKind::MasterPage)
    );
    check_context(&source, 3);
}

#[test]
fn unknown_hwp5_historical_cache_and_endnote_final_y_remain_positive() {
    for endnote in [false, true] {
        let text = paragraph("abcdefghijklmnopqrstuvwxyzabcdefghijklmnop");
        let body = if endnote {
            host(&format!(
                "<hp:ctrl><hp:endNote><hp:subList>{text}</hp:subList></hp:endNote></hp:ctrl>"
            ))
        } else {
            text
        };
        let source = read(package(&body, 8));
        let fonts = FontSystem::new();
        let layout = hm_layout::layout(&source, &fonts);
        let output = prepare_conversion(&source, &layout, Format::Hwp5).unwrap();
        let mut hwp5 = hm_hwp5::read(hm_hwp5::write_new(&output).unwrap().into()).unwrap();
        assert_eq!(
            hwp5.info.imported_line_membership,
            hm_model::ImportedLineMembership::Unknown
        );
        if endnote {
            let hm_model::ControlKind::Note(n) = &mut hwp5.sections[0].paras[1].controls[0].kind
            else {
                panic!("note")
            };
            n.list.paras[0].line_segs[1].vpos = n.list.paras[0].line_segs[0].vpos;
        }
        let current = hm_layout::layout(&hwp5, &fonts);
        let converted = prepare_conversion(&hwp5, &current, Format::Hwpx).unwrap();
        let back = read(hm_hwpx::write_new(&converted).unwrap());
        let reopened = hm_layout::layout(&back, &fonts);
        let again = prepare_conversion(&back, &reopened, Format::Hwp5).unwrap();
        assert_eq!(inventory(&again), inventory(&converted));
        let row = inventory(&converted).pop().unwrap();
        assert_eq!(
            row.2,
            vec![0, 3],
            "historical cache must survive even when fresh fit differs"
        );
        let fresh = hm_layout::layout_with(
            &hwp5,
            &fonts,
            hm_layout::Options {
                stored_lines: false,
                ..Default::default()
            },
        );
        assert_ne!(
            inventory(&prepare_conversion(&hwp5, &fresh, Format::Hwpx).unwrap()),
            inventory(&converted)
        );
    }
}

#[test]
fn contentless_float_host_keeps_the_segment_that_advances_past_its_table() {
    // A table that may overlap leaves its host line below it; a lone one
    // that may not takes no room whatever the segment.
    let table = table(&paragraph("cell text"), false)
        .replace("treatAsChar=\"1\"", "treatAsChar=\"0\" allowOverlap=\"1\"")
        .replace("<hp:tbl ", "<hp:tbl textWrap=\"TOP_AND_BOTTOM\" ");
    let body = format!(
        r#"<hp:p><hp:run charPrIDRef="0">{table}</hp:run><hp:linesegarray><hp:lineseg textpos="0" vertpos="1600" vertsize="1000" textheight="1000" baseline="850" spacing="600" horzpos="0" horzsize="43000" flags="393216"/></hp:linesegarray></hp:p>{}"#,
        paragraph("following text")
    );
    let bytes = package(&body, 0);
    let source = read(bytes);
    let fonts = FontSystem::new();
    let layout = hm_layout::layout(&source, &fonts);
    let prepared = prepare_conversion(&source, &layout, Format::Hwp5).unwrap();
    assert_eq!(prepared.sections[0].paras[1].line_segs[0].width, 43000);
    assert_eq!(source.sections[0].paras[1].line_segs[0].width, 43000);
    let back = hm_hwp5::read(hm_hwp5::write_new(&prepared).unwrap().into()).unwrap();
    let positions = |l: &hm_layout::DocLayout| {
        l.pages
            .iter()
            .map(|p| {
                p.lines
                    .iter()
                    .map(|l| (l.para, l.line, l.vpos))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        positions(&hm_layout::layout(&back, &fonts)),
        positions(&layout)
    );
    // Lines written fresh for that host keep a span too, or it would reopen
    // beside the table.
    let mut fresh = source.clone();
    fresh.convert_to(Format::Hwp5);
    hm_edit::lines::fill(&mut fresh, &layout);
    assert!(fresh.sections[0].paras[1].line_segs[0].width > 0);
    let fresh = hm_hwp5::read(hm_hwp5::write_new(&fresh).unwrap().into()).unwrap();
    assert_eq!(
        positions(&hm_layout::layout(&fresh, &fonts)),
        positions(&layout)
    );
}

#[test]
fn one_furniture_source_shown_at_two_section_widths_converts_with_both_layouts() {
    let body = host(&format!(
        "<hp:ctrl><hp:header applyPageType=\"BOTH\"><hp:subList textWidth=\"0\" textHeight=\"1000\">{}</hp:subList></hp:header></hp:ctrl>",
        paragraph("abcdefghijklmnopqrstuvwxyz ".repeat(3).as_str())
    ));
    let mut source = read(package(&body, 0));
    let mut next = read(package(&paragraph("next section"), 0))
        .sections
        .remove(0);
    let hm_model::ControlKind::Section(def) = &mut next.paras[0].controls[0].kind else {
        panic!("section")
    };
    let hm_model::SectionChild::Page(page) = &mut def.children[0] else {
        panic!("page")
    };
    page.width = 2000;
    source.sections.push(next);
    let before = format!("{source:?}");
    let fonts = FontSystem::new();
    let layout = hm_layout::layout(&source, &fonts);
    // The header runs on into the narrower section and breaks differently
    // there.
    let header_starts = |l: &hm_layout::DocLayout| -> Vec<Vec<u32>> {
        l.pages
            .iter()
            .flat_map(|p| &p.frames)
            .filter(|f| f.kind == hm_layout::FrameKind::Header)
            .map(|f| f.paras[0].lines.iter().map(|l| l.start).collect())
            .collect()
    };
    let starts = header_starts(&layout);
    assert!(starts.len() >= 2 && starts.iter().any(|s| *s != starts[0]));
    let output = prepare_conversion(&source, &layout, Format::Hwp5).unwrap();
    assert_eq!(format!("{source:?}"), before);
    // The file keeps the first page's lines; the narrower pages do not fit
    // them and break the header again as before.
    let back = hm_hwp5::read(hm_hwp5::write_new(&output).unwrap().into()).unwrap();
    assert_eq!(header_starts(&hm_layout::layout(&back, &fonts)), starts);
}

#[test]
fn repeated_page_number_text_box_converts_across_digit_counts() {
    let number = r#"<hp:p><hp:run charPrIDRef="0"><hp:ctrl><hp:autoNum num="1" numType="PAGE"><hp:autoNumFormat type="DIGIT"/></hp:autoNum></hp:ctrl></hp:run></hp:p>"#;
    let body = format!(
        "{}{}",
        host(&shape(number, (0, 0), false)),
        (0..12)
            .map(|_| paragraph("body").replace("<hp:p>", "<hp:p pageBreak=\"1\">"))
            .collect::<String>()
    );
    let mut source = read(package(&body, 0));
    let master = hm_model::ParaList {
        text_size: Some((4000, 2000)),
        paras: vec![source.sections[0].paras.remove(1)],
        ..Default::default()
    };
    let hm_model::ControlKind::Section(def) = &mut source.sections[0].paras[0].controls[0].kind
    else {
        panic!("section")
    };
    def.attr |= 1 << 29;
    def.children
        .push(hm_model::SectionChild::MasterPage(master));
    let fonts = FontSystem::new();
    let layout = hm_layout::layout(&source, &fonts);
    assert!(layout.pages.len() >= 10);
    let variants: Vec<_> = layout
        .pages
        .iter()
        .flat_map(|p| &p.frames)
        .filter(|f| f.kind == hm_layout::FrameKind::MasterPage)
        .map(|f| f.paras[0].measured.objects[0].1.frames[0].paras[0].lines[0])
        .collect();
    assert!(
        variants
            .iter()
            .any(|l| l.end_item != variants[0].end_item && l.used != variants[0].used)
    );
    let before = format!("{source:?}");
    check_context(&source, 15);
    assert_eq!(format!("{source:?}"), before);
}
