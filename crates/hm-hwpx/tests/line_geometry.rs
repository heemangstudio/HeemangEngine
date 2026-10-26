//! Endpoint namespaces determine line geometry, including after editing a host.

use hm_edit::{Editor, script};
use hm_hwpx::zip::Member;
use hm_model::{Document, LineGeom, ShapePart};

fn package(kind: &str, prefix: &str, original_height: u32, points: &str) -> Vec<u8> {
    let section = format!(
        r#"<hs:sec xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section" xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph" xmlns:point="http://www.hancom.co.kr/hwpml/2011/core"><hp:p id="0" paraPrIDRef="0" styleIDRef="0"><hp:run charPrIDRef="0"><hp:{kind} id="19" isReverseHV="0"><hp:orgSz width="100" height="{original_height}"/><hp:curSz width="8000" height="4000"/><hp:renderingInfo><point:transMatrix e1="1" e2="0" e3="17" e4="0" e5="1" e6="29"/><point:scaMatrix e1="80" e2="0" e3="0" e4="0" e5="40" e6="0"/><point:rotMatrix e1="1" e2="0" e3="0" e4="0" e5="1" e6="0"/></hp:renderingInfo><{prefix}:startPt x="7" y="11"/>{points}<hp:sz width="8000" height="4000"/><hp:pos treatAsChar="0"/></hp:{kind}><hp:t>host</hp:t></hp:run></hp:p></hs:sec>"#,
    );
    let files = [
        ("mimetype", "application/hwp+zip"),
        (
            "Contents/header.xml",
            r#"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head" version="1.4" secCnt="1"/>"#,
        ),
        (
            "Contents/content.hpf",
            r#"<opf:package xmlns:opf="http://www.idpf.org/2007/opf"><opf:manifest><opf:item id="header" href="Contents/header.xml" media-type="application/xml"/><opf:item id="section0" href="Contents/section0.xml" media-type="application/xml"/></opf:manifest><opf:spine><opf:itemref idref="section0"/></opf:spine></opf:package>"#,
        ),
        ("Contents/section0.xml", &section),
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

fn line(doc: &Document) -> &LineGeom {
    let shape = doc.sections[0].paras[0].controls[0]
        .as_object()
        .unwrap()
        .shape()
        .unwrap();
    let Some(ShapePart::Line(line)) = shape.kind() else {
        panic!("expected a line")
    };
    line
}

#[test]
fn line_endpoints_use_core_namespace_without_a_size_visibility_filter() {
    for height in [0, 100] {
        for (prefix, expected) in [("point", ((7, 11), (83, 97))), ("hp", ((0, 0), (0, 0)))] {
            let bytes = package(
                "line",
                prefix,
                height,
                &format!(r#"<{prefix}:endPt x="83" y="97"/>"#),
            );
            let doc = read(&bytes);
            assert_eq!((line(&doc).start, line(&doc).end), expected);
            assert_eq!(hm_hwpx::write(&doc).unwrap(), bytes);
        }
    }
    let bytes = package(
        "line",
        "point",
        100,
        r#"<hp:endPt x="91" y="99"/><point:endPt x="83" y="97"/>"#,
    );
    assert_eq!(line(&read(&bytes)).end, (83, 97));
}

#[test]
fn host_edit_save_reopen_preserves_the_imported_line_geometry_and_undo_bytes() {
    for (prefix, expected) in [("point", ((7, 11), (83, 97))), ("hp", ((0, 0), (0, 0)))] {
        let bytes = package(
            "line",
            prefix,
            100,
            &format!(r#"<{prefix}:endPt x="83" y="97"/>"#),
        );
        let mut editor = Editor::new(read(&bytes));
        script::run(
            &mut editor,
            r#"{"ops":[{"op":"insert","at":{"section":0,"para":0,"offset":8},"text":"edited "}]}"#,
        )
        .unwrap();
        let saved = hm_hwpx::write(&editor.doc).unwrap();
        let reopened = read(&saved);
        assert_eq!((line(&reopened).start, line(&reopened).end), expected);
        assert_eq!(reopened.sections[0].paras[0].plain_text(), "edited host");
        assert_eq!(hm_hwpx::write(&reopened).unwrap(), saved);
        assert!(editor.undo().unwrap());
        assert_eq!(hm_hwpx::write(&editor.doc).unwrap(), bytes);
    }
}

#[test]
fn connector_control_points_survive_native_and_legacy_markup_edit_and_reopen() {
    for wrapped in [true, false] {
        let points = r#"<hp:point x="7" y="11" type="3" stamp="first"/><hp:point x="83" y="97" type="26" stamp="second"/>"#;
        let points = if wrapped {
            format!(
                r#"<hp:controlPoints marker="native">{points}<hp:future value="retained"/></hp:controlPoints>"#
            )
        } else {
            points.to_string()
        };
        let bytes = package(
            "connectLine",
            "hp",
            100,
            &format!(r#"<hp:endPt x="83" y="97"/>{points}"#),
        );
        let source = read(&bytes);
        let expected = vec![(7, 11, 3), (83, 97, 26)];
        assert_eq!(line(&source).connector.as_ref().unwrap().points, expected);
        assert_eq!(hm_hwpx::write(&source).unwrap(), bytes);
        let mut editor = Editor::new(source);
        script::run(
            &mut editor,
            r#"{"ops":[{"op":"insert","at":{"section":0,"para":0,"offset":8},"text":"edited "}]}"#,
        )
        .unwrap();
        let mut materialized = editor.doc.clone();
        materialized.mark_all_dirty();
        let saved = hm_hwpx::write_new(&materialized).unwrap();
        let reopened = read(&saved);
        assert_eq!(line(&reopened).connector.as_ref().unwrap().points, expected);
        assert_eq!(reopened.sections[0].paras[0].plain_text(), "edited host");
        let archive = hm_hwpx::zip::Archive::open(saved.into()).unwrap();
        let section = archive.read_name("Contents/section0.xml").unwrap().unwrap();
        let xml = hm_hwpx::xml::Xml::parse(section).unwrap();
        let mut nodes = vec![xml.root()];
        let mut stamps = Vec::new();
        let mut containers = 0;
        while let Some(node) = nodes.pop() {
            if node.local() == b"controlPoints" {
                containers += 1;
                if wrapped {
                    assert_eq!(node.attr("marker").as_deref(), Some("native"));
                }
            }
            if node.local() == b"point" {
                stamps.push((
                    node.i32("x").unwrap(),
                    node.attr("stamp").unwrap().to_string(),
                ));
            }
            nodes.extend(node.children());
        }
        stamps.sort();
        assert_eq!(
            containers, 1,
            "writers must emit the native controlPoints container once"
        );
        assert_eq!(
            stamps,
            [(7, "first".to_string()), (83, "second".to_string())]
        );
        if wrapped {
            assert!(
                String::from_utf8_lossy(
                    &archive.read_name("Contents/section0.xml").unwrap().unwrap()
                )
                .contains("retained")
            );
        }
        assert!(editor.undo().unwrap());
        assert_eq!(hm_hwpx::write(&editor.doc).unwrap(), bytes);
    }
}
