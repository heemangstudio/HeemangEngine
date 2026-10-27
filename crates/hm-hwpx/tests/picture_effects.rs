//! Picture effects feed the shared rendering model and survive host edits.
use hm_edit::{Editor, script};
use hm_hwpx::zip::Member;
use hm_model::{Document, Picture, ShapePart};

fn package(shadow: &str) -> Vec<u8> {
    let section = format!(
        r#"<hs:sec xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section" xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph" xmlns:hc="http://www.hancom.co.kr/hwpml/2011/core"><hp:p id="0" paraPrIDRef="0" styleIDRef="0"><hp:run charPrIDRef="0"><hp:pic id="19"><hp:orgSz width="10000" height="12000"/><hp:curSz width="10000" height="12000"/><hp:imgRect><hc:pt0 x="0" y="0"/><hc:pt1 x="10000" y="0"/><hc:pt2 x="10000" y="12000"/><hc:pt3 x="0" y="12000"/></hp:imgRect><hp:imgDim dimwidth="30000" dimheight="36000"/><hc:img alpha="73"/><hp:effects>{shadow}</hp:effects><hp:sz width="10000" height="12000"/><hp:pos treatAsChar="0"/></hp:pic><hp:t>host</hp:t></hp:run></hp:p></hs:sec>"#
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
fn picture(doc: &Document) -> &Picture {
    let s = doc.sections[0].paras[0].controls[0]
        .as_object()
        .unwrap()
        .shape()
        .unwrap();
    let Some(ShapePart::Picture(p)) = s.kind() else {
        panic!("expected picture")
    };
    p
}
#[test]
fn outside_shadow_populates_shared_model_after_read_edit_save_and_undo() {
    let raw = r#"<hp:shadow style="OUTSIDE" alpha="0.8" radius="400" direction="45" distance="1000" alignStyle="TOP_LEFT" rotationStyle="0" future="preserve"><hp:skew x="0" y="0"/><hp:scale x="0.75" y="1.25"/><hp:effectsColor type="RGB"><hp:rgb r="18" g="52" b="86"/></hp:effectsColor></hp:shadow>"#;
    let bytes = package(raw);
    let doc = hm_hwpx::read(bytes.clone().into()).unwrap();
    let expected = hm_model::PictureShadow {
        transparency: 0.8,
        blur: 400.0,
        direction: 45.0,
        distance: 1000.0,
        scale: (0.75, 1.25),
        color: 0x0056_3412,
    };
    assert_eq!(picture(&doc).effect_list().shadow, Some(expected));
    assert_eq!(picture(&doc).original_size(), Some((30000, 36000)));
    assert_eq!(picture(&doc).alpha(), 73);
    assert_eq!(hm_hwpx::write(&doc).unwrap(), bytes);
    let mut editor = Editor::new(doc);
    script::run(
        &mut editor,
        r#"{"ops":[{"op":"insert","at":{"section":0,"para":0,"offset":8},"text":"edited "}]}"#,
    )
    .unwrap();
    let saved = hm_hwpx::write(&editor.doc).unwrap();
    let reopened = hm_hwpx::read(saved.clone().into()).unwrap();
    assert_eq!(picture(&reopened).effect_list().shadow, Some(expected));
    let archive = hm_hwpx::zip::Archive::open(saved.into()).unwrap();
    let section = archive.read_name("Contents/section0.xml").unwrap().unwrap();
    assert!(
        std::str::from_utf8(&section)
            .unwrap()
            .contains("future=\"preserve\"")
    );
    assert!(editor.undo().unwrap());
    assert_eq!(hm_hwpx::write(&editor.doc).unwrap(), bytes);
}
#[test]
fn no_effect_preserves_additional_picture_properties() {
    let doc = hm_hwpx::read(package("").into()).unwrap();
    assert_eq!(picture(&doc).effects(), 0);
    assert!(picture(&doc).effect_list().shadow.is_none());
    assert_eq!(picture(&doc).original_size(), Some((30000, 36000)));
    assert_eq!(picture(&doc).alpha(), 73);
}
