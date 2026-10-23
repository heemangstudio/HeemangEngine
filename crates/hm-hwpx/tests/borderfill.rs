//! Native BorderFill fields must survive editing and cross-format saves.

use hm_edit::{Editor, ListPath, Op, Pos};
use hm_hwpx::xml::{Ns, Xml};
use hm_hwpx::zip::{Archive, Member};
use hm_model::*;

// Native HWP SaveAs pairs: slash Crooked carries both crooked directions;
// backSlash Crooked is ignored. Each split setting adds exactly 0x0400.
const CASES: &[(&str, u16)] = &[
    (
        r#"centerLine="NONE"><hh:slash type="NONE" Crooked="0"/><hh:backSlash type="CENTER" Crooked="0"/>"#,
        64,
    ),
    (
        r#"centerLine="NONE"><hh:slash type="NONE" Crooked="2"/><hh:backSlash type="CENTER" Crooked="0"/>"#,
        576,
    ),
    (
        r#"centerLine="NONE"><hh:slash type="NONE" Crooked="0"/><hh:backSlash type="CENTER" Crooked="1"/>"#,
        64,
    ),
    (
        r#"centerLine="NONE"><hh:slash type="CENTER" Crooked="1"/><hh:backSlash type="NONE" Crooked="0"/>"#,
        264,
    ),
    (
        r#"centerLine="NONE"><hh:slash type="CENTER" Crooked="3"/><hh:backSlash type="CENTER" Crooked="0"/>"#,
        840,
    ),
    (
        r#"centerLine="NONE"><hh:slash type="CENTER" Crooked="1"/><hh:backSlash type="CENTER" Crooked="1"/>"#,
        328,
    ),
    (
        r#"centerLine="VERTICAL"><hh:slash type="NONE" Crooked="1"/><hh:backSlash type="NONE" Crooked="0"/>"#,
        8448,
    ),
    (
        r#"centerLine="VERTICAL"><hh:slash type="NONE" Crooked="3"/><hh:backSlash type="NONE" Crooked="0"/>"#,
        8960,
    ),
    (
        r#"centerLine="NONE"><hh:slash type="ALL" Crooked="0" isCounter="1"/><hh:backSlash type="ALL" Crooked="0" isCounter="1"/>"#,
        6396,
    ),
];

fn package() -> Vec<u8> {
    let doc = Document {
        meta: DocMeta {
            format: Format::Hwpx,
            version: Version(0x0501_0100),
            flags: 0,
        },
        info: DocInfo::default(),
        sections: vec![Section {
            paras: vec![Paragraph {
                text: "control\r".encode_utf16().collect(),
                ..Default::default()
            }],
            src: None,
            source_index: None,
            last_page_master: None,
        }],
        source: None,
        bins: None,
        diagnostics: Vec::new(),
    };
    let archive = Archive::open(SharedBytes::from(hm_hwpx::write_new(&doc).unwrap())).unwrap();
    let mut fills = String::new();
    for (i, (fields, _)) in CASES.iter().enumerate() {
        for split in 0..=1 {
            fills.push_str(&format!(
                r##"<hh:borderFill id="{}" threeD="1" shadow="1" breakCellSeparateLine="{split}" future="keep" {fields}<hh:leftBorder type="SOLID" width="0.12 mm" color="#123456"/><hh:rightBorder type="DASH" width="0.2 mm" color="#456789"/><hh:topBorder type="DOT" width="0.3 mm" color="#ABCDEF"/><hh:bottomBorder type="DOUBLE_SLIM" width="0.5 mm" color="#FEDCBA"/><hh:diagonal type="SOLID" width="0.1 mm" color="#102030"/></hh:borderFill>"##,
                i * 2 + split + 1,
            ));
        }
    }
    let header = format!(
        r#"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head" version="1.5" secCnt="1"><hh:refList><hh:borderFills itemCnt="{}">{fills}</hh:borderFills></hh:refList></hh:head>"#,
        CASES.len() * 2
    );
    let members: Vec<_> = archive
        .entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            if e.name == "Contents/header.xml" {
                Member::New {
                    name: &e.name,
                    data: header.as_bytes(),
                    stored: false,
                }
            } else {
                Member::Copy(i)
            }
        })
        .collect();
    hm_hwpx::zip::write(Some(&archive), &members)
}

fn assert_fields(doc: &Document) {
    for (i, (_, attr)) in CASES.iter().enumerate() {
        for split in 0..=1 {
            let b = &doc.info.border_fills[i * 2 + split];
            assert_eq!(
                b.attr,
                attr | 3 | (split as u16 * 1024),
                "case {i}, split {split}"
            );
        }
    }
}

fn emitted(bytes: &[u8], expected: &Document) {
    let archive = Archive::open(SharedBytes::from(bytes.to_vec())).unwrap();
    let xml = Xml::parse(archive.read_name("Contents/header.xml").unwrap().unwrap()).unwrap();
    let fills = xml
        .root()
        .child(Ns::Hh, "refList")
        .unwrap()
        .child(Ns::Hh, "borderFills")
        .unwrap();
    for (el, b) in fills.children().zip(&expected.info.border_fills) {
        assert_eq!(
            el.int("breakCellSeparateLine"),
            Some(((b.attr >> 10) & 1).into())
        );
        assert_eq!(
            el.attrs()
                .filter(|(_, n, _)| *n == b"breakCellSeparateLine")
                .count(),
            1
        );
        assert_eq!(el.attr("future").as_deref(), Some("keep"));
        assert_eq!(
            el.child(Ns::Hh, "slash").unwrap().int("Crooked"),
            Some(((b.attr >> 8) & 3).into())
        );
        assert_eq!(
            el.child(Ns::Hh, "backSlash").unwrap().int("Crooked"),
            Some(0)
        );
    }
}

#[test]
fn native_borderfill_flags_survive_editor_touch_and_cross_format_saves() {
    let bytes = package();
    let doc = hm_hwpx::read(SharedBytes::from(bytes.clone())).unwrap();
    assert_fields(&doc);
    assert_eq!(hm_hwpx::write(&doc).unwrap(), bytes);
    let before: Vec<_> = doc
        .info
        .border_fills
        .iter()
        .map(|b| (b.attr, b.sides, b.diagonal))
        .collect();
    let mut editor = Editor::new(doc);
    editor
        .apply(Op::InsertText {
            at: Pos {
                list: ListPath::default(),
                para: 0,
                offset: 0,
            },
            text: "edited ".into(),
        })
        .unwrap();
    let clean_styles = hm_hwpx::write(&editor.doc).unwrap();
    let original = Archive::open(SharedBytes::from(bytes)).unwrap();
    let edited = Archive::open(SharedBytes::from(clean_styles.clone())).unwrap();
    assert_eq!(
        original.read_name("Contents/header.xml").unwrap(),
        edited.read_name("Contents/header.xml").unwrap()
    );
    let clean_doc = hm_hwpx::read(SharedBytes::from(clean_styles)).unwrap();
    assert_fields(&clean_doc);
    assert_eq!(
        clean_doc.sections[0].paras[0].plain_text(),
        "edited control"
    );
    editor.doc.mark_all_dirty();
    let touched = hm_hwpx::write(&editor.doc).unwrap();
    emitted(&touched, &editor.doc);
    let mut reopened = hm_hwpx::read(SharedBytes::from(touched)).unwrap();
    assert_fields(&reopened);
    assert_eq!(
        reopened
            .info
            .border_fills
            .iter()
            .map(|b| (b.attr, b.sides, b.diagonal))
            .collect::<Vec<_>>(),
        before
    );
    // Changing a newly known field must replace its stored value, including
    // an explicit zero, without an old unknown attribute overriding it.
    for b in &mut reopened.info.border_fills {
        b.attr ^= 1024;
        b.src = None;
    }
    let changed = hm_hwpx::write(&reopened).unwrap();
    emitted(&changed, &reopened);
    let changed = hm_hwpx::read(SharedBytes::from(changed)).unwrap();
    for (a, b) in changed
        .info
        .border_fills
        .iter()
        .zip(&reopened.info.border_fills)
    {
        assert_eq!(a.attr, b.attr);
    }
    reopened.convert_to(Format::Hwp5);
    let hwp = hm_hwp5::write_new(&reopened).unwrap();
    let mut hwp = hm_hwp5::read(SharedBytes::from(hwp)).unwrap();
    for (a, b) in hwp
        .info
        .border_fills
        .iter()
        .zip(&reopened.info.border_fills)
    {
        assert_eq!((a.attr, a.sides, a.diagonal), (b.attr, b.sides, b.diagonal));
    }
    hwp.convert_to(Format::Hwpx);
    let converted = hm_hwpx::read(SharedBytes::from(hm_hwpx::write_new(&hwp).unwrap())).unwrap();
    for (a, b) in converted
        .info
        .border_fills
        .iter()
        .zip(&reopened.info.border_fills)
    {
        assert_eq!((a.attr, a.sides, a.diagonal), (b.attr, b.sides, b.diagonal));
    }
}
