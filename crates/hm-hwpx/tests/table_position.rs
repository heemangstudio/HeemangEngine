//! Negative literals and unsigned coordinate words are distinct in HWPX.

use hm_edit::{Editor, script};
use hm_hwpx::zip::{Archive, Member};
use hm_model::Document;

fn package(x: &str, y: &str, attrs: &str) -> Vec<u8> {
    let section = format!(
        r#"<hs:sec xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section" xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph"><hp:p id="0" paraPrIDRef="0" styleIDRef="0"><hp:run charPrIDRef="0"><hp:tbl id="19" rowCnt="1" colCnt="1" textWrap="TOP_AND_BOTTOM"><hp:sz width="5000" height="2400"/><hp:pos horzOffset="{x}" vertOffset="{y}" {attrs} future="keep"/><hp:tr><hp:tc><hp:subList><hp:p id="1" paraPrIDRef="0" styleIDRef="0"><hp:run charPrIDRef="0"><hp:t>cell</hp:t></hp:run></hp:p></hp:subList><hp:cellAddr colAddr="0" rowAddr="0"/><hp:cellSpan colSpan="1" rowSpan="1"/><hp:cellSz width="5000" height="2400"/></hp:tc></hp:tr></hp:tbl><hp:t>host</hp:t></hp:run></hp:p></hs:sec>"#
    );
    let files = [
        ("mimetype", "application/hwp+zip"),
        (
            "version.xml",
            r#"<hv:HCFVersion xmlns:hv="http://www.hancom.co.kr/hwpml/2011/version" major="5" minor="1" micro="0" buildNumber="1" xmlVersion="1.4"/>"#,
        ),
        (
            "Contents/header.xml",
            r#"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head" version="1.4" secCnt="1"/>"#,
        ),
        (
            "Contents/content.hpf",
            r#"<opf:package xmlns:opf="http://www.idpf.org/2007/opf"><opf:manifest><opf:item id="header" href="Contents/header.xml" media-type="application/xml"/><opf:item id="section0" href="Contents/section0.xml" media-type="application/xml"/></opf:manifest><opf:spine><opf:itemref idref="section0"/></opf:spine></opf:package>"#,
        ),
        ("Contents/section0.xml", section.as_str()),
    ];
    let members: Vec<_> = files
        .iter()
        .map(|(name, data)| Member::New {
            name,
            data: data.as_bytes(),
            stored: *name == "mimetype",
        })
        .collect();
    hm_hwpx::zip::write(None, &members)
}

fn read(bytes: &[u8]) -> Document {
    hm_hwpx::read(bytes.to_vec().into()).unwrap()
}

fn position(doc: &Document) -> (i32, i32) {
    let t = doc.sections[0].paras[0].controls[0].as_table().unwrap();
    (t.header.common.x_offset, t.header.common.y_offset)
}

#[test]
fn table_offset_literals_normalize_without_clamping_unsigned_words() {
    for attrs in [
        r#"treatAsChar="0" flowWithText="0" allowOverlap="1" vertRelTo="PARA" vertAlign="TOP""#,
        r#"treatAsChar="0" flowWithText="1" allowOverlap="0" vertRelTo="PAGE" vertAlign="CENTER""#,
        r#"treatAsChar="1" flowWithText="0" allowOverlap="1" vertRelTo="PAPER" vertAlign="BOTTOM""#,
    ] {
        for (x, y, expected) in [
            ("-1201", "-2", (0, 0)),
            ("4294966095", "4294967294", (-1201, -2)),
            ("0", "1201", (0, 1201)),
        ] {
            let bytes = package(x, y, attrs);
            let doc = read(&bytes);
            assert_eq!(position(&doc), expected);
            assert_eq!(hm_hwpx::write(&doc).unwrap(), bytes);
            assert!(doc.sections[0].paras[0].controls[0].src.is_some());
        }
    }
}

#[test]
fn editing_a_host_keeps_authored_table_positions_and_undo_bytes() {
    let bytes = package(
        "4294966095",
        "-2",
        r#"treatAsChar="0" vertRelTo="PARA" vertAlign="TOP""#,
    );
    let mut ed = Editor::new(read(&bytes));
    script::run(
        &mut ed,
        r#"{"ops":[{"op":"insert","at":{"section":0,"para":0,"offset":8},"text":"edited "}]}"#,
    )
    .unwrap();
    let saved = hm_hwpx::write(&ed.doc).unwrap();
    let reopened = read(&saved);
    assert_eq!(position(&reopened), (-1201, 0));
    assert!(
        reopened.sections[0].paras[0]
            .plain_text()
            .contains("edited host")
    );
    let archive = Archive::open(saved.clone().into()).unwrap();
    let xml = archive.read_name("Contents/section0.xml").unwrap().unwrap();
    let xml = String::from_utf8_lossy(&xml);
    assert!(xml.contains("horzOffset=\"4294966095\""));
    assert!(xml.contains("vertOffset=\"-2\""));
    assert!(xml.contains("future=\"keep\""));
    assert_eq!(hm_hwpx::write(&reopened).unwrap(), saved);
    assert!(ed.undo().unwrap());
    assert_eq!(hm_hwpx::write(&ed.doc).unwrap(), bytes);
    assert!(ed.redo().unwrap());
    assert_eq!(
        position(&read(&hm_hwpx::write(&ed.doc).unwrap())),
        (-1201, 0)
    );
}

#[test]
fn editing_a_cell_writes_signed_table_coordinates_as_unsigned_words() {
    let bytes = package(
        "4294966095",
        "-2",
        r#"treatAsChar="0" vertRelTo="PARA" vertAlign="TOP""#,
    );
    let mut ed = Editor::new(read(&bytes));
    script::run(&mut ed, r#"{"ops":[{"op":"insert","at":{"section":0,"para":0,"offset":0,"cells":[[0,0,0]]},"text":"edited "}]}"#).unwrap();
    let saved = hm_hwpx::write(&ed.doc).unwrap();
    let reopened = read(&saved);
    assert_eq!(position(&reopened), (-1201, 0));
    let t = reopened.sections[0].paras[0].controls[0]
        .as_table()
        .unwrap();
    assert_eq!(t.cells[0].paras[0].plain_text(), "edited cell");
    let archive = Archive::open(saved.into()).unwrap();
    let xml = archive.read_name("Contents/section0.xml").unwrap().unwrap();
    let xml = String::from_utf8_lossy(&xml);
    assert!(xml.contains("horzOffset=\"4294966095\""));
    assert!(xml.contains("vertOffset=\"0\""));
    assert!(ed.undo().unwrap());
    assert_eq!(hm_hwpx::write(&ed.doc).unwrap(), bytes);
}
