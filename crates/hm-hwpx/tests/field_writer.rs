//! Field ends written for an edited paragraph keep their reference to the copied field begin.
use hm_edit::{Editor, script};
use hm_hwpx::xml::{Ns, Xml};
use hm_hwpx::zip::{Archive, Member};

const BEGIN: &str = r#"<hp:ctrl><hp:fieldBegin id="1234914821" type="HYPERLINK" name="" editable="0" dirty="0" zorder="-1" fieldid="627272811"><hp:parameters cnt="1" name=""><hp:stringParam name="Command">http\://example.com;1;0;0;</hp:stringParam></hp:parameters></hp:fieldBegin></hp:ctrl>"#;
const END: &str =
    r#"<hp:ctrl><hp:fieldEnd beginIDRef="1234914821" fieldid="627272811"/></hp:ctrl>"#;

fn package(body: &str) -> Vec<u8> {
    let header = r#"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head" xmlns:hc="http://www.hancom.co.kr/hwpml/2011/core" xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph" version="1.2" secCnt="1"><hh:refList><hh:paraProperties itemCnt="1"><hh:paraPr id="0"/></hh:paraProperties></hh:refList></hh:head>"#;
    let section = format!(
        r#"<hs:sec xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section" xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph">{body}</hs:sec>"#
    );
    let files = [
        ("mimetype", "application/hwp+zip".to_string()),
        ("Contents/header.xml", header.to_string()),
        ("Contents/content.hpf", r#"<opf:package xmlns:opf="http://www.idpf.org/2007/opf"><opf:manifest><opf:item id="header" href="Contents/header.xml"/><opf:item id="section0" href="Contents/section0.xml"/></opf:manifest><opf:spine><opf:itemref idref="section0"/></opf:spine></opf:package>"#.into()),
        ("Contents/section0.xml", section),
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

fn field_end_refs(bytes: &[u8]) -> Vec<String> {
    let section = Archive::open(bytes.to_vec().into())
        .unwrap()
        .read_name("Contents/section0.xml")
        .unwrap()
        .unwrap();
    let xml = Xml::parse(section).unwrap();
    let mut out = Vec::new();
    fn walk(e: hm_hwpx::xml::El<'_>, out: &mut Vec<String>) {
        if e.is(Ns::Hp, "fieldEnd") {
            out.push(e.attr("beginIDRef").unwrap_or_default().into_owned());
        }
        for c in e.children() {
            walk(c, out);
        }
    }
    walk(xml.root(), &mut out);
    out
}

fn edited(body: &str, at: (u32, u32)) -> Vec<u8> {
    let doc = hm_hwpx::read(package(body).into()).unwrap();
    let mut editor = Editor::new(doc);
    let ops = format!(
        r#"{{"ops":[{{"op":"insert","at":{{"section":0,"para":{},"offset":{}}},"text":"X"}}]}}"#,
        at.0, at.1
    );
    script::run(&mut editor, &ops).unwrap();
    hm_hwpx::write(&editor.doc).unwrap()
}

#[test]
fn edited_paragraph_keeps_the_begin_id_of_a_copied_field() {
    // Typing after the link re-encodes the paragraph; the field begin itself stays a copied control.
    let body = format!(
        r#"<hp:p id="0" paraPrIDRef="0" styleIDRef="0"><hp:run charPrIDRef="0">{BEGIN}<hp:t>link</hp:t>{END}<hp:t> tail</hp:t></hp:run></hp:p>"#
    );
    let saved = edited(&body, (0, 8 + 4 + 8 + 5));
    assert_eq!(field_end_refs(&saved), ["1234914821"]);
}

#[test]
fn field_end_in_an_edited_paragraph_after_a_copied_begin_keeps_its_reference() {
    // The begin sits in an untouched paragraph that is copied whole; the end's paragraph is edited.
    let body = format!(
        r#"<hp:p id="0" paraPrIDRef="0" styleIDRef="0"><hp:run charPrIDRef="0">{BEGIN}<hp:t>one</hp:t></hp:run></hp:p><hp:p id="0" paraPrIDRef="0" styleIDRef="0"><hp:run charPrIDRef="0"><hp:t>two</hp:t>{END}</hp:run></hp:p>"#
    );
    let saved = edited(&body, (1, 0));
    assert_eq!(field_end_refs(&saved), ["1234914821"]);
}

#[test]
fn a_summary_field_reads_and_writes_as_hancom_spells_it() {
    // Hancom writes 문서 요약 fields with type="SUMMERY" (fixes/clipboard-03/hancom-date): such a
    // field stays a summary field when its markup is written anew, as a pasted copy is.
    let body = r#"<hp:p id="0" paraPrIDRef="0" styleIDRef="0"><hp:run charPrIDRef="0"><hp:ctrl><hp:fieldBegin id="7" type="SUMMERY" name="" editable="1" dirty="0" zorder="-1" fieldid="628321650"><hp:parameters cnt="1" name=""><hp:stringParam name="Command">$createtime</hp:stringParam></hp:parameters></hp:fieldBegin></hp:ctrl><hp:t>x</hp:t><hp:ctrl><hp:fieldEnd beginIDRef="7" fieldid="628321650"/></hp:ctrl></hp:run></hp:p>"#;
    let kind = |doc: &hm_model::Document| match &doc.sections[0].paras[0].controls[0].kind {
        hm_model::ControlKind::Field(f) => f.kind(),
        _ => panic!("no field"),
    };
    let mut doc = hm_hwpx::read(package(body).into()).unwrap();
    assert_eq!(kind(&doc), hm_model::FieldKind::Summary);
    let p = &mut doc.sections[0].paras[0];
    p.src = None;
    p.controls[0].src = None;
    let saved = hm_hwpx::write(&doc).unwrap();
    let section = Archive::open(saved.clone().into())
        .unwrap()
        .read_name("Contents/section0.xml")
        .unwrap()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&section).contains(r#"type="SUMMERY""#),
        "{}",
        String::from_utf8_lossy(&section)
    );
    assert_eq!(
        kind(&hm_hwpx::read(saved.into()).unwrap()),
        hm_model::FieldKind::Summary
    );
}
