//! Character decorations that HWP 5 and OWPML store differently keep Hancom's OWPML form.
use hm_hwpx::xml::{Ns, Xml};
use hm_hwpx::zip::{Archive, Member};

fn package(char_prs: &[&str]) -> Vec<u8> {
    let shapes: String = char_prs
        .iter()
        .enumerate()
        .map(|(i, body)| format!(r#"<hh:charPr id="{i}" height="1000" {body}</hh:charPr>"#))
        .collect();
    let header = format!(
        r#"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head" xmlns:hc="http://www.hancom.co.kr/hwpml/2011/core" xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph" version="1.2" secCnt="1"><hh:refList><hh:charProperties itemCnt="{}">{shapes}</hh:charProperties><hh:paraProperties itemCnt="1"><hh:paraPr id="0"/></hh:paraProperties></hh:refList></hh:head>"#,
        char_prs.len()
    );
    let files = [
        ("mimetype", "application/hwp+zip".to_string()),
        ("Contents/header.xml", header),
        ("Contents/content.hpf", r#"<opf:package xmlns:opf="http://www.idpf.org/2007/opf"><opf:manifest><opf:item id="header" href="Contents/header.xml"/><opf:item id="section0" href="Contents/section0.xml"/></opf:manifest><opf:spine><opf:itemref idref="section0"/></opf:spine></opf:package>"#.into()),
        ("Contents/section0.xml", r#"<hs:sec xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section" xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph"><hp:p id="0" paraPrIDRef="0" styleIDRef="0"><hp:run charPrIDRef="0"><hp:t>a</hp:t></hp:run></hp:p></hs:sec>"#.into()),
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

/// Each charPr's symMark, underline type and strikeout shape after a full re-encode.
fn rewritten(char_prs: &[&str]) -> (Vec<u32>, Vec<(String, String, String)>) {
    let mut doc = hm_hwpx::read(package(char_prs).into()).unwrap();
    let attrs = doc.info.char_shapes.iter().map(|c| c.attr).collect();
    doc.info.mark_all_dirty();
    let saved = hm_hwpx::write(&doc).unwrap();
    let header = Archive::open(saved.into())
        .unwrap()
        .read_name("Contents/header.xml")
        .unwrap()
        .unwrap();
    let xml = Xml::parse(header).unwrap();
    let list = xml
        .root()
        .child(Ns::Hh, "refList")
        .unwrap()
        .child(Ns::Hh, "charProperties")
        .unwrap();
    let out = list
        .children()
        .filter(|c| c.is(Ns::Hh, "charPr"))
        .map(|c| {
            let get = |el: &str, a: &str| {
                c.child(Ns::Hh, el)
                    .and_then(|e| e.attr(a).map(|v| v.into_owned()))
                    .unwrap_or_default()
            };
            (
                c.attr("symMark").unwrap_or_default().into_owned(),
                get("underline", "type"),
                get("strikeout", "shape"),
            )
        })
        .collect();
    (attrs, out)
}

#[test]
fn strikeout_is_written_without_the_centered_underline_hwp5_keeps_for_it() {
    let (attrs, out) = rewritten(&[
        r##"><hh:underline type="NONE" shape="SOLID" color="#000000"/><hh:strikeout shape="SOLID" color="#000000"/>"##,
        r##"><hh:underline type="CENTER" shape="SOLID" color="#000000"/><hh:strikeout shape="NONE" color="#000000"/>"##,
        r##"><hh:underline type="BOTTOM" shape="SOLID" color="#000000"/><hh:strikeout shape="DASH" color="#000000"/>"##,
    ]);
    // The model keeps HWP 5's form: a strikeout also sets the centered underline kind.
    assert_eq!((attrs[0] >> 2) & 3, 2);
    assert_eq!((attrs[0] >> 18) & 7, 1);
    assert_eq!((attrs[1] >> 2) & 3, 2);
    assert_eq!((attrs[1] >> 18) & 7, 0);
    let lines: Vec<_> = out
        .iter()
        .map(|(_, u, s)| (u.as_str(), s.as_str()))
        .collect();
    assert_eq!(
        lines,
        [("NONE", "SOLID"), ("CENTER", "NONE"), ("BOTTOM", "DASH")]
    );
}

#[test]
fn a_centered_underline_beside_hancoms_3d_strikeout_stays_an_underline() {
    // Hancom writes strikeout shape 3D for text that is not struck; a centered underline of
    // its own shape beside it is a line of its own (fixtures/probes/decorations.hwpx).
    let (_, out) = rewritten(&[
        r##"><hh:underline type="CENTER" shape="SOLID" color="#FF0000"/><hh:strikeout shape="3D" color="#0000FF"/>"##,
        r##"><hh:underline type="NONE" shape="SOLID" color="#000000"/><hh:strikeout shape="3D" color="#000000"/>"##,
    ]);
    let lines: Vec<_> = out
        .iter()
        .map(|(_, u, s)| (u.as_str(), s.as_str()))
        .collect();
    assert_eq!(lines, [("CENTER", "3D"), ("NONE", "3D")]);
}

#[test]
fn emphasis_marks_use_hancom_names_and_codes() {
    let names = [
        "DOT_ABOVE",
        "RING_ABOVE",
        "TILDE",
        "CARON",
        "SIDE",
        "COLON",
        "GRAVE_ACCENT",
        "ACUTE_ACCENT",
        "CIRCUMFLEX",
        "MACRON",
        "HOOK_ABOVE",
        "DOT_BELOW",
    ];
    let bodies: Vec<String> = names.iter().map(|n| format!(r#"symMark="{n}">"#)).collect();
    let (attrs, out) = rewritten(&bodies.iter().map(String::as_str).collect::<Vec<_>>());
    let codes: Vec<u32> = attrs.iter().map(|a| (a >> 21) & 15).collect();
    assert_eq!(codes, (1..=12).collect::<Vec<_>>());
    let written: Vec<&str> = out.iter().map(|(m, _, _)| m.as_str()).collect();
    assert_eq!(written, names);
    // Earlier engine builds wrote code 5 as DOT_MIDDLE.
    let (attrs, out) = rewritten(&[r#"symMark="DOT_MIDDLE">"#]);
    assert_eq!((attrs[0] >> 21) & 15, 5);
    assert_eq!(out[0].0, "SIDE");
}
