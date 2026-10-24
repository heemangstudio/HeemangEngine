//! Package XML versions govern paragraph distances independently of header metadata.
use hm_edit::{Editor, script};
use hm_hwpx::zip::Member;
use hm_model::LineSpacingKind;

const UNIT: &str = "http://www.hancom.co.kr/hwpml/2016/HwpUnitChar";

fn package(version: Option<&str>, header_version: &str, values: &str) -> Vec<u8> {
    let header = format!(
        r#"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head" xmlns:hc="http://www.hancom.co.kr/hwpml/2011/core" xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph" version="{header_version}" secCnt="1"><hh:refList><hh:charProperties itemCnt="1"><hh:charPr id="0" height="1151"/></hh:charProperties><hh:paraProperties itemCnt="1"><hh:paraPr id="0">{values}</hh:paraPr></hh:paraProperties></hh:refList></hh:head>"#
    );
    let mut files = vec![
        ("mimetype", "application/hwp+zip".to_string()),
        ("Contents/header.xml", header),
        ("Contents/content.hpf", r#"<opf:package xmlns:opf="http://www.idpf.org/2007/opf"><opf:manifest><opf:item id="header" href="Contents/header.xml"/><opf:item id="section0" href="Contents/section0.xml"/></opf:manifest><opf:spine><opf:itemref idref="section0"/></opf:spine></opf:package>"#.to_string()),
        ("Contents/section0.xml", format!(r#"<hs:sec xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section" xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph"><hp:p id="0" paraPrIDRef="0" styleIDRef="0"><hp:run charPrIDRef="0"><hp:secPr><hp:pagePr width="60000" height="80000"><hp:margin left="1000" right="1000" top="1000" bottom="1000" header="0" footer="0" gutter="0"/></hp:pagePr></hp:secPr><hp:ctrl><hp:colPr colCount="1" sameSz="1" sameGap="0"/></hp:ctrl><hp:t>{}</hp:t></hp:run></hp:p></hs:sec>"#,["x";8].join("<hp:lineBreak/>"))),
    ];
    if let Some(version) = version {
        files.push(("version.xml",format!(r#"<hv:HCFVersion xmlns:hv="http://www.hancom.co.kr/hwpml/2011/version" major="5" minor="1" micro="bad" buildNumber="7"{version}/>"#)));
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

fn read(bytes: &[u8]) -> hm_model::Document {
    hm_hwpx::read(bytes.to_vec().into()).unwrap()
}

fn switched(values: &str, supported: bool) -> String {
    if supported {
        format!(
            r#"<hp:switch><hp:case hp:required-namespace="{UNIT}">{values}</hp:case><hp:default><hh:lineSpacing type="FIXED" value="6000" unit="HWPUNIT"/></hp:default></hp:switch>"#
        )
    } else {
        format!(
            r#"<hp:switch><hp:case hp:required-namespace="urn:unsupported"><hh:lineSpacing type="FIXED" value="5000" unit="HWPUNIT"/></hp:case><hp:default>{values}</hp:default></hp:switch>"#
        )
    }
}

#[test]
fn package_version_controls_selected_default_and_outside_values() {
    let values = r#"<hh:margin><hc:prev value="126" unit="HWPUNIT"/></hh:margin><hh:lineSpacing type="FIXED" value="2400" unit="HWPUNIT"/>"#;
    for (version, literal) in [
        (Some(r#" xmlVersion="1.2""#), false),
        (Some(r#" xmlVersion="1.3""#), false),
        (Some(r#" xmlVersion="1.31""#), true),
        (Some(r#" xmlVersion="1.4""#), true),
        (Some(r#" xmlVersion="1.5""#), true),
        (Some(r#" xmlVersion="1.bad""#), false),
        (Some(""), true),
        (None, true),
    ] {
        for header in ["1.2", "1.5"] {
            for source in [
                values.to_string(),
                switched(values, true),
                switched(values, false),
            ] {
                let doc = read(&package(version, header, &source));
                let p = &doc.info.para_shapes[0];
                assert_eq!(
                    p.before(500),
                    if literal { 126 } else { 63 },
                    "{version:?} {header} {source}"
                );
                assert_eq!(
                    p.line_spacing(),
                    (LineSpacingKind::Fixed, if literal { 4800 } else { 2400 })
                );
                if version.is_some() {
                    assert_eq!(doc.meta.version.0, 0x0501_0007);
                }
            }
        }
    }
}

#[test]
fn unreadable_version_xml_keeps_legacy_units_instead_of_claiming_modern() {
    let values = r#"<hh:margin><hc:prev value="126" unit="HWPUNIT"/></hh:margin>"#;
    let doc = read(&package(
        Some(" xmlVersion=\"1.5\" broken=\""),
        "1.5",
        values,
    ));
    assert_eq!(doc.info.para_shapes[0].before(500), 63);
    assert_eq!(doc.meta.version.0, 0x0501_0000);
}

#[test]
fn missing_spacing_keeps_percent_remainder_through_text_edit_and_reopen() {
    for version in [
        Some(r#" xmlVersion="1.2""#),
        Some(r#" xmlVersion="1.4""#),
        Some(r#" xmlVersion="1.bad""#),
        None,
    ] {
        for source in [String::new(), switched("", true), switched("", false)] {
            let bytes = package(version, "1.5", &source);
            let doc = read(&bytes);
            let check = |doc: &hm_model::Document| {
                assert_eq!(
                    doc.info.para_shapes[0].line_spacing(),
                    (LineSpacingKind::Percent, 0)
                );
                let layout = Editor::new(doc.clone()).layout(&Default::default());
                let p = &layout.sections[0].paras[0];
                assert_eq!(p.lines.len(), 8);
                assert!(p.lines.iter().all(|l| l.advance() == 3));
            };
            check(&doc);
            assert_eq!(hm_hwpx::write(&doc).unwrap(), bytes);
            let mut editor = Editor::new(doc);
            let at = hm_edit::ops::lead(&editor.doc.sections[0].paras[0]);
            script::run(&mut editor,&format!(r#"{{"ops":[{{"op":"insert","at":{{"section":0,"para":0,"offset":{at}}},"text":"edited "}}]}}"#)).unwrap();
            let saved = hm_hwpx::write(&editor.doc).unwrap();
            let reopened = read(&saved);
            assert!(
                reopened.sections[0].paras[0]
                    .plain_text()
                    .starts_with("edited ")
            );
            check(&reopened);
            assert!(editor.undo().unwrap());
            assert_eq!(hm_hwpx::write(&editor.doc).unwrap(), bytes);
        }
    }
}
