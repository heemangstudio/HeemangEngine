//! Regenerated paragraph properties keep the preserved package's unit contract.
use hm_edit::{Editor, script};
use hm_hwpx::xml::{Ns, Xml};
use hm_hwpx::zip::{Archive, Member};
use hm_model::{Document, LineSpacingKind};

const UNIT: &str = "http://www.hancom.co.kr/hwpml/2016/HwpUnitChar";
const MARGINS: [i32; 5] = [-7, 252, -151, 101, 66];
const UNRELATED: &str = r#"<hh:retained value="unchanged &amp; exact"/>"#;

fn distance(raw: i32, packed: bool) -> (i32, &'static str) {
    if packed {
        (raw, "HWPUNIT")
    } else {
        (raw >> 1, if raw & 1 == 0 { "HWPUNIT" } else { "CHAR" })
    }
}

fn package(version: Option<&str>, packed: bool, kind: &str, spacing: i32) -> Vec<u8> {
    let margin: String = ["intent", "left", "right", "prev", "next"]
        .into_iter()
        .zip(MARGINS)
        .map(|(tag, raw)| {
            let (v, unit) = distance(raw, packed);
            format!(r#"<hc:{tag} value="{v}" unit="{unit}"/>"#)
        })
        .collect();
    let (value, unit) = distance(spacing, packed || kind == "PERCENT");
    let header = format!(
        r#"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head" xmlns:hc="http://www.hancom.co.kr/hwpml/2011/core" xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph" version="1.2" secCnt="1">{UNRELATED}<hh:refList><hh:paraProperties itemCnt="1"><hh:paraPr id="0"><hp:switch><hp:case hp:required-namespace="{UNIT}"><hh:margin>{margin}</hh:margin><hh:lineSpacing type="{kind}" value="{value}" unit="{unit}"/></hp:case><hp:default><hh:margin><hc:prev value="9000" unit="HWPUNIT"/></hh:margin></hp:default></hp:switch></hh:paraPr></hh:paraProperties></hh:refList></hh:head>"#
    );
    let mut files = vec![
        ("mimetype", "application/hwp+zip".to_string()),
        ("Contents/header.xml", header),
        ("Contents/content.hpf", r#"<opf:package xmlns:opf="http://www.idpf.org/2007/opf"><opf:manifest><opf:item id="header" href="Contents/header.xml"/><opf:item id="section0" href="Contents/section0.xml"/></opf:manifest><opf:spine><opf:itemref idref="section0"/></opf:spine></opf:package>"#.into()),
        ("Contents/section0.xml", r#"<hs:sec xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section" xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph"><hp:p id="0" paraPrIDRef="0" styleIDRef="0"><hp:run charPrIDRef="0"><hp:t>host</hp:t></hp:run></hp:p></hs:sec>"#.into()),
    ];
    if let Some(version) = version {
        files.push(("version.xml", version.to_string()));
    }
    hm_hwpx::zip::write(
        None,
        &files
            .iter()
            .map(|(name, data)| Member::New {
                name,
                data: data.as_bytes(),
                stored: *name == "mimetype",
            })
            .collect::<Vec<_>>(),
    )
}

fn read(bytes: &[u8]) -> Document {
    hm_hwpx::read(bytes.to_vec().into()).unwrap()
}

fn values(doc: &Document) -> ([i32; 5], (LineSpacingKind, i32)) {
    let p = &doc.info.para_shapes[0];
    (
        [
            p.indent,
            p.margin_left,
            p.margin_right,
            p.space_before,
            p.space_after,
        ],
        p.line_spacing(),
    )
}

fn member(bytes: &[u8], name: &str) -> Option<hm_model::SharedBytes> {
    Archive::open(bytes.to_vec().into())
        .unwrap()
        .read_name(name)
        .unwrap()
}

fn check_generated(bytes: &[u8], packed: bool, kind: LineSpacingKind, spacing: i32) {
    assert_eq!(values(&read(bytes)), (MARGINS, (kind, spacing)));
    let header = member(bytes, "Contents/header.xml").unwrap();
    assert!(std::str::from_utf8(&header).unwrap().contains(UNRELATED));
    let xml = Xml::parse(header).unwrap();
    let p = xml
        .root()
        .child(Ns::Hh, "refList")
        .unwrap()
        .child(Ns::Hh, "paraProperties")
        .unwrap()
        .child(Ns::Hh, "paraPr")
        .unwrap();
    let switch = p.child(Ns::Hp, "switch").unwrap();
    for (branch, packed) in [("case", packed), ("default", true)] {
        let branch = switch.child(Ns::Hp, branch).unwrap();
        let margin = branch.child(Ns::Hh, "margin").unwrap();
        for (element, raw) in margin.children().zip(MARGINS) {
            let (value, unit) = distance(raw, packed);
            assert_eq!(element.i32("value"), Some(value));
            assert_eq!(element.attr("unit").as_deref(), Some(unit));
        }
        let line = branch.child(Ns::Hh, "lineSpacing").unwrap();
        let (value, unit) = distance(spacing, packed || kind == LineSpacingKind::Percent);
        assert_eq!(line.i32("value"), Some(value));
        assert_eq!(line.attr("unit").as_deref(), Some(unit));
    }
}

#[test]
fn generated_header_reopens_with_exact_typed_values_and_preserved_package_context() {
    for (version, packed) in [
        (
            Some(r#"<version major="5" minor="1" xmlVersion="1.2"/>"#),
            true,
        ),
        (Some(r#"<version xmlVersion="1.3"/>"#), true),
        (Some(r#"<version xmlVersion="1.bad"/>"#), true),
        (Some("<version xmlVersion=\"1.5\" broken=\""), true),
        (Some(r#"<version xmlVersion="NaN"/>"#), true),
        (Some(r#"<version xmlVersion="1.31"/>"#), false),
        (Some(r#"<version xmlVersion="1.4"/>"#), false),
        (Some(r#"<version xmlVersion="1.5"/>"#), false),
        (Some("<version/>"), false),
        (None, false),
    ] {
        for (name, kind, spacing) in [
            ("PERCENT", LineSpacingKind::Percent, 163),
            ("FIXED", LineSpacingKind::Fixed, 501),
            ("BETWEEN_LINES", LineSpacingKind::Added, -249),
            ("AT_LEAST", LineSpacingKind::AtLeast, 302),
        ] {
            let original = package(version, packed, name, spacing);
            let doc = read(&original);
            assert_eq!(values(&doc), (MARGINS, (kind, spacing)));
            assert_eq!(hm_hwpx::write(&doc).unwrap(), original);
            let mut editor = Editor::new(doc);
            script::run(&mut editor, r#"{"ops":[{"op":"insert","at":{"section":0,"para":0,"offset":0},"text":"edited "}]}"#).unwrap();
            let saved = hm_hwpx::write(&editor.doc).unwrap();
            assert_eq!(
                member(&saved, "Contents/header.xml"),
                member(&original, "Contents/header.xml")
            );
            let mut reopened = read(&saved);
            for _ in 0..2 {
                reopened.info.para_shapes[0].src = None;
                let generated = hm_hwpx::write(&reopened).unwrap();
                assert_eq!(
                    member(&generated, "version.xml"),
                    member(&original, "version.xml")
                );
                check_generated(&generated, packed, kind, spacing);
                reopened = read(&generated);
                assert_eq!(reopened.sections[0].paras[0].plain_text(), "edited host");
            }
            assert!(editor.undo().unwrap());
            assert_eq!(hm_hwpx::write(&editor.doc).unwrap(), original);
        }
    }
}

#[test]
fn new_package_uses_literal_units_without_inheriting_the_imported_version() {
    let mut doc = read(&package(
        Some(r#"<version xmlVersion="1.2"/>"#),
        true,
        "FIXED",
        501,
    ));
    doc.convert_to(hm_model::Format::Hwp5);
    doc.convert_to(hm_model::Format::Hwpx);
    let bytes = hm_hwpx::write_new(&doc).unwrap();
    assert_eq!(
        values(&read(&bytes)),
        (MARGINS, (LineSpacingKind::Fixed, 501))
    );
    let version = member(&bytes, "version.xml").unwrap();
    assert_eq!(
        Xml::parse(version)
            .unwrap()
            .root()
            .attr("xmlVersion")
            .as_deref(),
        Some("1.4")
    );
}

#[test]
fn paragraph_format_edit_saves_in_the_source_context_and_undo_restores_the_shape() {
    for (version, packed) in [
        (Some(r#"<version xmlVersion="1.2"/>"#), true),
        (Some(r#"<version xmlVersion="1.bad"/>"#), true),
        (Some(r#"<version xmlVersion="1.4"/>"#), false),
        (Some("<version/>"), false),
        (None, false),
    ] {
        let original = package(version, packed, "FIXED", 501);
        let mut editor = Editor::new(read(&original));
        script::run(&mut editor, r#"{"ops":[{"op":"para","at":{"section":0,"para":0},"left":127,"indent":-17,"space_before":63,"space_after":85}]}"#).unwrap();
        let saved = hm_hwpx::write(&editor.doc).unwrap();
        assert_eq!(
            member(&saved, "version.xml"),
            member(&original, "version.xml")
        );
        let back = read(&saved);
        let p = &back.info.para_shapes[back.sections[0].paras[0].header.para_shape as usize];
        assert_eq!(
            [
                p.indent,
                p.margin_left,
                p.margin_right,
                p.space_before,
                p.space_after
            ],
            [-34, 254, -151, 126, 170]
        );
        assert_eq!(p.line_spacing(), (LineSpacingKind::Fixed, 501));
        assert_eq!(values(&back), (MARGINS, (LineSpacingKind::Fixed, 501)));
        assert!(editor.undo().unwrap());
        let undone = read(&hm_hwpx::write(&editor.doc).unwrap());
        let p = &undone.info.para_shapes[undone.sections[0].paras[0].header.para_shape as usize];
        assert_eq!(
            [
                p.indent,
                p.margin_left,
                p.margin_right,
                p.space_before,
                p.space_after
            ],
            MARGINS
        );
        assert_eq!(p.line_spacing(), (LineSpacingKind::Fixed, 501));
    }
}
