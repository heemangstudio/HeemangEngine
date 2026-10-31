//! Imported membership authority leaves raw geometry and source markup intact.
//! Edits and changes in available width still invalidate replay.

use hm_edit::{Editor, ListPath, Op, Pos, Step, ops};
use hm_fonts::FontSystem;
use hm_layout::{
    para::{ParaEnv, format},
    style::Styles,
};
use hm_model::{Document, Paragraph, SharedBytes};

fn paragraph(text: &str) -> String {
    format!(
        r#"<hp:p paraPrIDRef="0"><hp:run charPrIDRef="0"><hp:t>{text}</hp:t></hp:run><hp:linesegarray><hp:lineseg textpos="0" vertpos="0" vertsize="1000" textheight="1000" baseline="850" spacing="600" horzpos="0" horzsize="4000" flags="393216"/><hp:lineseg textpos="3" vertpos="1600" vertsize="1000" textheight="1000" baseline="850" spacing="600" horzpos="0" horzsize="4000" flags="393216"/></hp:linesegarray></hp:p>"#
    )
}

fn table() -> String {
    format!(
        r#"<hp:tbl rowCnt="1" colCnt="1" pageBreak="CELL"><hp:sz width="4000" height="4000"/><hp:pos treatAsChar="1"/><hp:tr><hp:tc><hp:subList>{}{}</hp:subList><hp:cellAddr colAddr="0" rowAddr="0"/><hp:cellSpan colSpan="1" rowSpan="1"/><hp:cellSz width="4000" height="4000"/></hp:tc></hp:tr></hp:tbl>"#,
        paragraph("abcdefghijklmnop"),
        paragraph("qrstuvwxyzabcdef")
    )
}

fn package(config: &str, body: &str) -> Vec<u8> {
    let header = format!(
        r#"<hh:head version="1.5" xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head" xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph" xmlns:hc="http://www.hancom.co.kr/hwpml/2011/core"><hh:refList><hh:charProperties itemCnt="1"><hh:charPr id="0" height="1000"/></hh:charProperties><hh:paraProperties itemCnt="1"><hh:paraPr id="0"><hh:lineSpacing type="PERCENT" value="160" unit="HWPUNIT"/></hh:paraPr></hh:paraProperties></hh:refList>{config}</hh:head>"#
    );
    let section = format!(
        r#"<hs:sec xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section" xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph">{body}</hs:sec>"#
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

fn read(bytes: &[u8]) -> Document {
    hm_hwpx::read(SharedBytes::from(bytes.to_vec())).unwrap()
}

fn starts(doc: &Document, p: &Paragraph, width: i32, stored: bool) -> Vec<u32> {
    let fonts = FontSystem::new();
    let mut styles = Styles::new(&doc.info, &fonts);
    styles.stored_lines = stored;
    format(
        p,
        &doc.info,
        &mut styles,
        ParaEnv {
            column_width: width,
            default_tab: 4000,
        },
        None,
    )
    .lines
    .iter()
    .map(|l| l.start)
    .collect()
}

fn lists(doc: &Document) -> Vec<ListPath> {
    fn walk(paras: &[Paragraph], path: ListPath, out: &mut Vec<ListPath>) {
        out.push(path.clone());
        for (pi, p) in paras.iter().enumerate() {
            for (ci, c) in p.controls.iter().enumerate() {
                for (i, child) in ops::control_lists(c).iter().enumerate() {
                    let mut sub = path.clone();
                    sub.steps.push(Step {
                        para: pi as u32,
                        control: ci as u32,
                        index: i as u32,
                    });
                    walk(child, sub, out);
                }
            }
        }
    }
    let mut out = Vec::new();
    for (i, s) in doc.sections.iter().enumerate() {
        walk(
            &s.paras,
            ListPath {
                section: i as u32,
                steps: vec![],
            },
            &mut out,
        );
    }
    out
}

#[test]
fn all_imported_lists_keep_sources_and_survive_edit_undo_redo() {
    let p = paragraph("abcdefghijklmnop");
    let body = format!(
        r#"{p}<hp:p><hp:run charPrIDRef="0">{}<hp:ctrl><hp:header applyPageType="BOTH"><hp:subList>{p}{p}</hp:subList></hp:header><hp:footer applyPageType="BOTH"><hp:subList>{p}{p}</hp:subList></hp:footer><hp:footNote number="1"><hp:subList>{p}{p}</hp:subList></hp:footNote><hp:endNote number="1"><hp:subList>{p}{p}</hp:subList></hp:endNote></hp:ctrl></hp:run></hp:p>"#,
        table()
    );
    for flags in [0, 8, 48, 56] {
        let config = format!(
            r#"<hh:trackchageConfig hc:flags="8" flags="{flags}" hh:flags="0" future="keep"><hh:unknown value="tail"/></hh:trackchageConfig>"#
        );
        let bytes = package(&config, &body);
        let doc = read(&bytes);
        let paths = lists(&doc);
        assert_eq!(
            paths.len(),
            6,
            "body, cell, header, footer and both note lists"
        );
        assert!(doc.sections[0].src.is_some());
        for path in paths {
            let p = &ops::list(&doc, &path).unwrap()[0];
            assert!(p.src.is_some());
            let raw = p.line_segs.clone();
            let legacy = read(&package("", &body));
            assert_eq!(raw, ops::list(&legacy, &path).unwrap()[0].line_segs);
            assert_eq!(p.src, ops::list(&legacy, &path).unwrap()[0].src);
            let initial = starts(&doc, p, 4000, true);
            assert_eq!(
                initial,
                if flags & 8 != 0 {
                    vec![0, 3]
                } else {
                    starts(&doc, p, 4000, false)
                }
            );
            assert_eq!(starts(&doc, p, 2000, true), starts(&doc, p, 2000, false));
            let mut ed = Editor::new(doc.clone());
            ed.layout(&FontSystem::new());
            ed.apply(Op::InsertText {
                at: Pos {
                    list: path.clone(),
                    para: 0,
                    offset: 1,
                },
                text: "z".into(),
            })
            .unwrap();
            let edited = &ops::list(&ed.doc, &path).unwrap()[0];
            assert_eq!(
                starts(&ed.doc, edited, 4000, true),
                starts(&ed.doc, edited, 4000, false)
            );
            assert!(edited.src.is_none());
            let geometry = starts(&ed.doc, edited, 4000, true);
            let saved = hm_hwpx::write(&ed.doc).unwrap();
            let back = read(&saved);
            assert_eq!(
                starts(&back, &ops::list(&back, &path).unwrap()[0], 4000, true),
                geometry
            );
            assert_eq!(
                ops::list(&back, &path).unwrap()[0].plain_text(),
                "azbcdefghijklmnop"
            );
            assert!(ed.undo().unwrap());
            assert_eq!(ops::list(&ed.doc, &path).unwrap()[0].line_segs, raw);
            assert_eq!(hm_hwpx::write(&ed.doc).unwrap(), bytes);
            assert_eq!(
                starts(&ed.doc, &ops::list(&ed.doc, &path).unwrap()[0], 4000, true),
                initial
            );
            assert!(ed.redo().unwrap());
            assert_eq!(hm_hwpx::write(&ed.doc).unwrap(), saved);
            // A new style forces header encoding. The policy field and its
            // unknown attributes/children must survive that path too.
            hm_edit::script::run(
                &mut ed,
                r#"{"ops":[{"op":"para","at":{"section":0,"para":0},"indent":-700}]}"#,
            )
            .unwrap();
            let saved = hm_hwpx::write(&ed.doc).unwrap();
            let archive = hm_hwpx::zip::Archive::open(SharedBytes::from(saved)).unwrap();
            let header = archive.read_name("Contents/header.xml").unwrap().unwrap();
            assert!(String::from_utf8_lossy(&header).contains(&config));
        }
    }
}

#[test]
fn unknown_and_malformed_configs_keep_replay_and_raw_markup_when_header_changes() {
    for config in [
        "",
        r#"<hh:trackchageConfig/>"#,
        r#"<hh:trackchageConfig flags="unknown" future="x"/>"#,
        r#"<hh:trackchageConfig flags="256"/>"#,
        r#"<hh:trackchageConfig flags="1"/>"#,
        r#"<hh:trackchageConfig flags="64"/>"#,
        r#"<hc:trackchageConfig flags="0"/>"#,
        r#"<hh:trackchageConfig flags="-1"/>"#,
        r#"<hh:trackchageConfig flags="4294967296"/>"#,
        r#"<hh:trackchageConfig flags="0" flags="8" future="keep"><hh:unknown value="tail"/></hh:trackchageConfig>"#,
        r#"<hh:trackchageConfig flags="8" flags="0"/>"#,
        r#"<hh:trackchageConfig flags="0" flags="0"/>"#,
        r#"<hh:trackchageConfig flags="8" flags="8"/>"#,
        r#"<hh:trackchageConfig flags="0" flags="unknown"/>"#,
        r#"<hh:trackchageConfig flags="unknown" flags="0"/>"#,
        r#"<hh:trackchageConfig hc:flags="0" hh:flags="8"/>"#,
    ] {
        let bytes = package(config, &paragraph("abcdefghijklmnop"));
        let mut ed = Editor::new(read(&bytes));
        assert_eq!(
            ed.doc.info.imported_line_membership,
            hm_model::ImportedLineMembership::Unknown,
            "{config}"
        );
        let legacy = read(&package("", &paragraph("abcdefghijklmnop")));
        assert_eq!(
            ed.doc.sections[0].paras[0].src,
            legacy.sections[0].paras[0].src
        );
        assert_eq!(
            ed.doc.sections[0].paras[0].line_segs,
            legacy.sections[0].paras[0].line_segs
        );
        assert_eq!(
            starts(&ed.doc, &ed.doc.sections[0].paras[0], 4000, true),
            vec![0, 3]
        );
        assert_eq!(hm_hwpx::write(&ed.doc).unwrap(), bytes);
        hm_edit::script::run(&mut ed,r#"{"ops":[{"op":"format","from":{"section":0,"para":0,"offset":0},"to":{"section":0,"para":0,"offset":1},"size":1100}]}"#).unwrap();
        let saved = hm_hwpx::write(&ed.doc).unwrap();
        let archive = hm_hwpx::zip::Archive::open(SharedBytes::from(saved.clone())).unwrap();
        let header = archive.read_name("Contents/header.xml").unwrap().unwrap();
        assert!(String::from_utf8_lossy(&header).contains(config));
        let back = read(&saved);
        assert_eq!(
            back.info.imported_line_membership,
            hm_model::ImportedLineMembership::Unknown,
            "{config} after dirty-header save"
        );
        assert_eq!(
            starts(&back, &back.sections[0].paras[0], 4000, true),
            starts(&ed.doc, &ed.doc.sections[0].paras[0], 4000, true)
        );
    }
}

#[test]
fn same_height_line_groups_are_recomposed_in_both_policy_routes() {
    // 같은 높이에 나란히 놓인 줄 영역은 개체를 감싼 줄이라 새로 배치한다. 같은
    // 영역이 같은 높이에 다시 오면 쪽을 넘겨 이어진 줄이라 저장된 나눔을 쓴다.
    let grouped = paragraph("abcdefghijklmnop").replace(
        "textpos=\"3\" vertpos=\"1600\" vertsize=\"1000\" textheight=\"1000\" baseline=\"850\" spacing=\"600\" horzpos=\"0\"",
        "textpos=\"3\" vertpos=\"0\" vertsize=\"1000\" textheight=\"1000\" baseline=\"850\" spacing=\"600\" horzpos=\"2000\"",
    );
    for flags in [0, 8] {
        let doc = read(&package(
            &format!(r#"<hh:trackchageConfig flags="{flags}"/>"#),
            &grouped,
        ));
        let p = &doc.sections[0].paras[0];
        assert_eq!(starts(&doc, p, 4000, true), starts(&doc, p, 4000, false));
        assert_ne!(starts(&doc, p, 4000, true), vec![0, 3]);
        assert_eq!(
            hm_hwpx::write(&doc).unwrap(),
            package(
                &format!(r#"<hh:trackchageConfig flags="{flags}"/>"#),
                &grouped
            )
        );
    }
}

#[test]
fn cell_resize_rejects_narrow_cache_and_restores_it_after_undo() {
    let body = format!(
        r#"<hp:p><hp:run charPrIDRef="0">{}</hp:run></hp:p>"#,
        table()
    );
    for flags in [0, 8] {
        let bytes = package(&format!(r#"<hh:trackchageConfig flags="{flags}"/>"#), &body);
        let mut ed = Editor::new(read(&bytes));
        let fonts = FontSystem::new();
        let before = ed.layout(&fonts);
        let initial: Vec<_> = before.para(0, 0).table(0).unwrap().cells[0].paras[0]
            .lines
            .iter()
            .map(|l| l.start)
            .collect();
        for width in [2000, 8000] {
            let mut anchor = ed.doc.sections[0].paras[0].clone();
            anchor.src = None;
            anchor.controls[0].src = None;
            let hm_model::ControlKind::Table(t) = &mut anchor.controls[0].kind else {
                panic!("table")
            };
            t.header.common.width = width;
            t.cells[0].width = width;
            ed.apply(Op::Replace {
                list: ListPath::default(),
                start: 0,
                count: 1,
                paras: vec![anchor],
            })
            .unwrap();
            let l = ed.layout(&fonts);
            let cell = &l.para(0, 0).table(0).unwrap().cells[0];
            let expected = starts(
                &ed.doc,
                &ed.doc.sections[0].paras[0].controls[0]
                    .as_table()
                    .unwrap()
                    .cells[0]
                    .paras[0],
                width,
                width >= 4000 && flags & 8 != 0,
            );
            assert_eq!(
                cell.paras[0]
                    .lines
                    .iter()
                    .map(|l| l.start)
                    .collect::<Vec<_>>(),
                expected
            );
            if width < 4000 {
                assert_ne!(
                    expected, initial,
                    "narrowing must exercise different line membership"
                );
            }
            let back = read(&hm_hwpx::write(&ed.doc).unwrap());
            let reopened = hm_layout::layout(&back, &fonts);
            assert_eq!(format!("{:?}", reopened.pages), format!("{:?}", l.pages));
            assert!(ed.undo().unwrap());
            assert_eq!(hm_hwpx::write(&ed.doc).unwrap(), bytes);
            assert_eq!(
                format!("{:?}", ed.layout(&fonts).pages),
                format!("{:?}", before.pages)
            );
        }
    }
}

#[test]
fn square_objects_reflow_text_in_their_actual_exclusion_segments() {
    let object = r#"<hp:rect id="1" textWrap="SQUARE" textFlow="BOTH_SIDES"><hp:sz width="2000" height="2000"/><hp:pos treatAsChar="0" vertRelTo="PARA" horzRelTo="PARA" vertAlign="TOP" horzAlign="LEFT" vertOffset="0" horzOffset="0"/><hp:orgSz width="2000" height="2000"/><hp:curSz width="2000" height="2000"/></hp:rect>"#;
    let body = format!(
        r#"<hp:p><hp:run charPrIDRef="0"><hp:secPr><hp:pagePr width="4000" height="20000"><hp:margin left="0" right="0" top="0" bottom="0" header="0" footer="0" gutter="0"/></hp:pagePr></hp:secPr></hp:run></hp:p>{}"#,
        paragraph("abcdefghijklmnop")
            .replace("<hp:t>", &format!("{object}<hp:t>"))
            .replace("textpos=\"3\"", "textpos=\"11\"")
            .replace(
                "horzpos=\"0\" horzsize=\"4000\"",
                "horzpos=\"2000\" horzsize=\"2000\""
            )
    );
    for flags in [0, 8] {
        let bytes = package(&format!(r#"<hh:trackchageConfig flags="{flags}"/>"#), &body);
        let mut ed = Editor::new(read(&bytes));
        assert_eq!(ed.doc.sections[0].paras[1].controls.len(), 1);
        let layout = ed.layout(&FontSystem::new());
        let p = layout.para(0, 1);
        assert_ne!(
            p.lines.iter().map(|l| l.start).collect::<Vec<_>>(),
            vec![0, 11]
        );
        assert!(
            p.lines
                .iter()
                .any(|l| l.seg_x >= 2000 && l.seg_width <= 2000)
        );
        let mut placed = Vec::new();
        hm_layout::place(p, &p.lines[0], &mut placed);
        assert!(
            placed
                .iter()
                .filter(|g| matches!(g.kind, hm_layout::Kind::Char(_)))
                .all(|g| g.x >= 2000)
        );
        assert_eq!(hm_hwpx::write(&ed.doc).unwrap(), bytes);
    }
}

#[test]
fn page_width_changes_reformat_without_losing_undo_or_source_identity() {
    let body = format!(
        r#"<hp:p><hp:run charPrIDRef="0"><hp:secPr><hp:pagePr width="4000" height="20000"><hp:margin left="0" right="0" top="0" bottom="0" header="0" footer="0" gutter="0"/></hp:pagePr></hp:secPr></hp:run></hp:p>{}"#,
        paragraph("abcdefghijklmnop")
    );
    for flags in [0, 8] {
        let bytes = package(&format!(r#"<hh:trackchageConfig flags="{flags}"/>"#), &body);
        let mut ed = Editor::new(read(&bytes));
        let fonts = FontSystem::new();
        let original = ed.layout(&fonts);
        for width in [2000, 8000] {
            let mut anchor = ed.doc.sections[0].paras[0].clone();
            anchor.src = None;
            anchor.controls[0].src = None;
            let hm_model::ControlKind::Section(s) = &mut anchor.controls[0].kind else {
                panic!("section")
            };
            let hm_model::SectionChild::Page(p) = &mut s.children[0] else {
                panic!("page")
            };
            p.width = width as u32;
            ed.apply(Op::Replace {
                list: ListPath::default(),
                start: 0,
                count: 1,
                paras: vec![anchor],
            })
            .unwrap();
            let layout = ed.layout(&fonts);
            assert_eq!(layout.pages[0].body.w, width);
            assert_eq!(
                layout
                    .para(0, 1)
                    .lines
                    .iter()
                    .map(|l| l.start)
                    .collect::<Vec<_>>(),
                starts(
                    &ed.doc,
                    &ed.doc.sections[0].paras[1],
                    width,
                    width >= 4000 && flags & 8 != 0
                )
            );
            let saved = hm_hwpx::write(&ed.doc).unwrap();
            assert_eq!(
                format!("{:?}", hm_layout::layout(&read(&saved), &fonts).pages),
                format!("{:?}", layout.pages)
            );
            ed.undo().unwrap();
            assert_eq!(hm_hwpx::write(&ed.doc).unwrap(), bytes);
            assert_eq!(
                format!("{:?}", ed.layout(&fonts).pages),
                format!("{:?}", original.pages)
            );
        }
    }
}

#[test]
fn memo_keys_use_effective_authority_when_source_spans_are_shared() {
    use hm_model::ImportedLineMembership::{Preserve, Recompose, Unknown};
    let bytes = package(
        r#"<hh:trackchageConfig flags="8"/>"#,
        &paragraph("abcdefghijklmnop"),
    );
    let mut doc = read(&bytes);
    let fonts = FontSystem::new();
    let mut cache = hm_layout::LayoutCache::new();
    for policy in [Preserve, Recompose, Unknown, Recompose, Preserve] {
        doc.info.imported_line_membership = policy;
        for enabled in [true, false, true] {
            let options = hm_layout::Options {
                stored_lines: enabled,
                ..Default::default()
            };
            let expected = hm_layout::layout_with(&doc, &fonts, options);
            let cold = hm_layout::layout_cached(&doc, &fonts, options, &mut cache);
            let warm = hm_layout::layout_cached(&doc, &fonts, options, &mut cache);
            assert_eq!(format!("{:?}", expected.pages), format!("{:?}", cold.pages));
            assert_eq!(format!("{:?}", expected.pages), format!("{:?}", warm.pages));
            let fresh = starts(&doc, &doc.sections[0].paras[0], 4000, false);
            let actual = starts(&doc, &doc.sections[0].paras[0], 4000, enabled);
            if policy == Recompose || !enabled {
                assert_eq!(actual, fresh);
            } else {
                assert_eq!(actual, vec![0, 3]);
                assert_ne!(actual, fresh);
            }
            assert!(cache.stats().0 > 0);
        }
    }
    for flags in ["0", "8", "48", "56"] {
        let back = read(&package(
            &format!(r#"<hh:trackchageConfig flags="{flags}"/>"#),
            &paragraph("abcdefghijklmnop"),
        ));
        let expected = hm_layout::layout(&back, &fonts);
        let actual = hm_layout::layout_cached(&back, &fonts, Default::default(), &mut cache);
        assert_eq!(
            format!("{:?}", expected.pages),
            format!("{:?}", actual.pages)
        );
        assert_eq!(
            back.info.imported_line_membership,
            if flags == "0" || flags == "48" {
                Recompose
            } else {
                Preserve
            }
        );
    }
}
