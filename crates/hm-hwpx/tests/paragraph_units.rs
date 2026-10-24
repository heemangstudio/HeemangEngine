//! Paragraph unit branches keep their authority through editing and saving.
use hm_edit::{Editor, script};
use hm_hwpx::zip::{Archive, Member};
use hm_model::{Document, LineSpacingKind, ParaShape};

const UNIT_NS: &str = "http://www.hancom.co.kr/hwpml/2016/HwpUnitChar";

fn package(values: &str) -> Vec<u8> {
    let header = format!(
        r#"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head" xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph" xmlns:hc="http://www.hancom.co.kr/hwpml/2011/core" version="1.4" secCnt="1"><hh:refList><hh:paraProperties itemCnt="1"><hh:paraPr id="0">{values}</hh:paraPr></hh:paraProperties></hh:refList></hh:head>"#
    );
    let files = [
        ("mimetype", "application/hwp+zip"),
        (
            "version.xml",
            r#"<version major="5" minor="1" xmlVersion="1.4"/>"#,
        ),
        ("Contents/header.xml", header.as_str()),
        (
            "Contents/content.hpf",
            r#"<opf:package xmlns:opf="http://www.idpf.org/2007/opf"><opf:manifest><opf:item id="header" href="Contents/header.xml"/><opf:item id="section0" href="Contents/section0.xml"/></opf:manifest><opf:spine><opf:itemref idref="section0"/></opf:spine></opf:package>"#,
        ),
        (
            "Contents/section0.xml",
            r#"<hs:sec xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section" xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph"><hp:p id="0" paraPrIDRef="0" styleIDRef="0"><hp:run charPrIDRef="0"><hp:t>host</hp:t></hp:run></hp:p></hs:sec>"#,
        ),
    ];
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

fn values(before: i32, unit: &str, spacing: &str) -> String {
    format!(r#"<hh:margin><hc:prev value="{before}" unit="{unit}"/></hh:margin>{spacing}"#)
}

fn case(namespace: &str, values: &str) -> String {
    format!(r#"<hp:case hp:required-namespace="{namespace}">{values}</hp:case>"#)
}

fn switched(cases: &str, default: &str) -> String {
    format!("<hp:switch>{cases}<hp:default>{default}</hp:default></hp:switch>")
}

fn read(bytes: &[u8]) -> Document {
    hm_hwpx::read(bytes.to_vec().into()).unwrap()
}

fn metrics(p: &ParaShape) -> (i32, LineSpacingKind, i32) {
    let (kind, raw) = p.line_spacing();
    let distance = if kind == LineSpacingKind::Percent {
        raw
    } else {
        hm_model::units::unpack_distance(raw, || 575)
    };
    (p.before(575), kind, distance)
}

#[test]
fn paragraph_values_select_one_supported_case_in_source_order() {
    let fixed = r#"<hh:lineSpacing type="FIXED" value="250" unit="CHAR"/>"#;
    let fallback = values(
        62,
        "HWPUNIT",
        r#"<hh:lineSpacing type="FIXED" value="2800" unit="HWPUNIT"/>"#,
    );
    let supported = case(UNIT_NS, &values(50, "CHAR", fixed));
    let cases = case("urn:unsupported", &values(9000, "HWPUNIT", fixed))
        + &supported
        + &case(UNIT_NS, &values(200, "HWPUNIT", fixed));
    let doc = read(&package(&switched(&cases, &fallback)));
    assert_eq!(
        metrics(&doc.info.para_shapes[0]),
        (288, LineSpacingKind::Fixed, 1438)
    );

    // The first supported branch is the whole value set. Missing spacing
    // must not come from another case or the fallback.
    let sparse = case(UNIT_NS, &values(17, "HWPUNIT", "")) + &supported;
    let doc = read(&package(&switched(&sparse, &fallback)));
    let p = &doc.info.para_shapes[0];
    assert_eq!(p.before(575), 17);
    assert_ne!(p.line_spacing(), (LineSpacingKind::Fixed, 2800));
    assert_ne!(p.line_spacing(), (LineSpacingKind::Fixed, 501));

    for cases in [
        case("urn:unsupported", &values(9000, "HWPUNIT", fixed)),
        String::new(),
    ] {
        let doc = read(&package(&switched(&cases, &fallback)));
        assert_eq!(
            metrics(&doc.info.para_shapes[0]),
            (62, LineSpacingKind::Fixed, 2800)
        );
    }
    let doc = read(&package(&fallback));
    assert_eq!(
        metrics(&doc.info.para_shapes[0]),
        (62, LineSpacingKind::Fixed, 2800)
    );
    // A switch without a default keeps the pre-existing first-case fallback.
    let doc = read(&package(&format!(
        "<hp:switch>{}</hp:switch>",
        case("urn:unsupported", &values(37, "HWPUNIT", ""))
    )));
    assert_eq!(
        metrics(&doc.info.para_shapes[0]),
        (37, LineSpacingKind::Percent, 0)
    );
    assert_eq!(
        metrics(&read(&package("<hp:switch/>")).info.para_shapes[0]),
        (0, LineSpacingKind::Percent, 0)
    );
}

#[test]
fn unit_branch_survives_noop_edit_generated_header_and_undo() {
    let selected = case(
        UNIT_NS,
        &values(
            50,
            "CHAR",
            r#"<hh:lineSpacing type="FIXED" value="250" unit="CHAR"/>"#,
        ),
    );
    let bytes = package(&switched(&selected, &values(62, "HWPUNIT", "")));
    let doc = read(&bytes);
    assert_eq!(
        metrics(&doc.info.para_shapes[0]),
        (288, LineSpacingKind::Fixed, 1438)
    );
    assert_eq!(hm_hwpx::write(&doc).unwrap(), bytes);
    let original_header = Archive::open(bytes.clone().into())
        .unwrap()
        .read_name("Contents/header.xml")
        .unwrap()
        .unwrap();
    let mut editor = Editor::new(doc);
    script::run(
        &mut editor,
        r#"{"ops":[{"op":"insert","at":{"section":0,"para":0,"offset":0},"text":"edited "}]}"#,
    )
    .unwrap();
    let saved = hm_hwpx::write(&editor.doc).unwrap();
    let saved_header = Archive::open(saved.clone().into())
        .unwrap()
        .read_name("Contents/header.xml")
        .unwrap()
        .unwrap();
    assert_eq!(&saved_header[..], &original_header[..]);
    let mut reopened = read(&saved);
    assert_eq!(reopened.sections[0].paras[0].plain_text(), "edited host");
    assert_eq!(
        metrics(&reopened.info.para_shapes[0]),
        (288, LineSpacingKind::Fixed, 1438)
    );
    reopened.mark_all_dirty();
    let generated = hm_hwpx::write(&reopened).unwrap();
    assert_eq!(
        metrics(&read(&generated).info.para_shapes[0]),
        (288, LineSpacingKind::Fixed, 1438)
    );
    assert!(editor.undo().unwrap());
    assert_eq!(hm_hwpx::write(&editor.doc).unwrap(), bytes);
}

#[test]
fn omitted_spacing_in_a_supported_value_set_keeps_percent_zero_through_edits() {
    let check = |doc: Document, advance: i32| {
        let mut editor = Editor::new(doc);
        let layout = editor.layout(&Default::default());
        let lines = &layout.para(0, 0).lines;
        assert_eq!(lines.len(), 8);
        assert!(lines.iter().all(|line| line.advance() == advance));
    };
    for (size, advance) in [(1150, 2), (1151, 3)] {
        let source = package(&switched(
            &case(UNIT_NS, &values(19, "HWPUNIT", "")),
            &values(
                100,
                "HWPUNIT",
                r#"<hh:lineSpacing type="FIXED" value="2800" unit="HWPUNIT"/>"#,
            ),
        ));
        let archive = Archive::open(source.into()).unwrap();
        let mut files = Vec::new();
        for name in [
            "mimetype",
            "Contents/header.xml",
            "Contents/content.hpf",
            "Contents/section0.xml",
        ] {
            let data = archive.read_name(name).unwrap().unwrap();
            let text = String::from_utf8(data.to_vec()).unwrap();
            let text = if name == "Contents/header.xml" {
                text.replace("version=\"1.4\"", "version=\"1.5\"")
                    .replace(
                        "<hh:refList>",
                        &format!(
                            r#"<hh:refList><hh:charProperties itemCnt="1"><hh:charPr id="0" height="{size}"/></hh:charProperties>"#
                        ),
                    )
            } else if name == "Contents/section0.xml" {
                text.replace(
                    "<hp:t>host</hp:t>",
                    &format!(
                        r#"<hp:secPr><hp:pagePr width="60000" height="80000"><hp:margin left="1000" right="1000" top="1000" bottom="1000" header="0" footer="0" gutter="0"/></hp:pagePr></hp:secPr><hp:ctrl><hp:colPr colCount="1" sameSz="1" sameGap="0"/></hp:ctrl><hp:t>{}</hp:t>"#,
                        ["x"; 8].join("<hp:lineBreak/>")
                    ),
                )
            } else {
                text
            };
            files.push((name, text));
        }
        files.push((
            "version.xml",
            r#"<hv:HCFVersion xmlns:hv="http://www.hancom.co.kr/hwpml/2011/version" major="5" minor="1" micro="1" buildNumber="0" xmlVersion="1.5"/>"#.into(),
        ));
        let bytes = hm_hwpx::zip::write(
            None,
            &files
                .iter()
                .map(|(name, text)| Member::New {
                    name,
                    data: text.as_bytes(),
                    stored: *name == "mimetype",
                })
                .collect::<Vec<_>>(),
        );
        let doc = read(&bytes);
        assert_eq!(
            doc.info.para_shapes[0].line_spacing(),
            (LineSpacingKind::Percent, 0)
        );
        check(doc.clone(), advance);
        assert_eq!(hm_hwpx::write(&doc).unwrap(), bytes);
        let mut editor = Editor::new(doc);
        let at = hm_edit::ops::lead(&editor.doc.sections[0].paras[0]);
        script::run(
            &mut editor,
            &format!(
                r#"{{"ops":[{{"op":"insert","at":{{"section":0,"para":0,"offset":{at}}},"text":"edited "}}]}}"#
            ),
        )
        .unwrap();
        let saved = hm_hwpx::write(&editor.doc).unwrap();
        let mut reopened = read(&saved);
        assert!(
            reopened.sections[0].paras[0]
                .plain_text()
                .starts_with("edited ")
        );
        check(reopened.clone(), advance);
        reopened.mark_all_dirty();
        check(read(&hm_hwpx::write(&reopened).unwrap()), advance);
        assert!(editor.undo().unwrap());
        assert_eq!(hm_hwpx::write(&editor.doc).unwrap(), bytes);
    }
}
