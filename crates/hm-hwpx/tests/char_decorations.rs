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

/// Underline and strikeout of every charPr: (type, shape, color) and (shape, color).
fn lines_of(hwpx: Vec<u8>) -> Vec<String> {
    let header = Archive::open(hwpx.into())
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
    list.children()
        .filter(|c| c.is(Ns::Hh, "charPr"))
        .map(|c| {
            let get = |el: &str, a: &str| {
                c.child(Ns::Hh, el)
                    .and_then(|e| e.attr(a).map(|v| v.into_owned()))
                    .unwrap_or_default()
            };
            format!(
                "{}/{}/{} {}/{}",
                get("underline", "type"),
                get("underline", "shape"),
                get("underline", "color"),
                get("strikeout", "shape"),
                get("strikeout", "color")
            )
        })
        .collect()
}

/// Hancom 2022 (COM capture formatting-20261104/K k1, paragraphs K09, K14, K03, K10 and the
/// corpus's 3D no-strike line): OWPML keeps a centered underline and a strikeout apart and
/// writes them back as read. HWP 5 has one centered line: a centered underline keeps its own
/// shape and color there, a strikeout without an underline lends it its shape and color; and
/// Hancom reads that line back as a strikeout with a none, solid, black underline.
#[test]
fn hwp5_keeps_a_centered_underline_and_a_strikeout_in_one_line() {
    let cases = [
        r##"borderFillIDRef="1"><hh:underline type="CENTER" shape="LONG_DASH" color="#00FF00"/><hh:strikeout shape="DASH_DOT" color="#FF0000"/>"##,
        r##"borderFillIDRef="1"><hh:underline type="NONE" shape="LONG_DASH" color="#00FF00"/><hh:strikeout shape="DASH_DOT" color="#FF0000"/>"##,
        r##"borderFillIDRef="1"><hh:underline type="CENTER" shape="SOLID" color="#000000"/><hh:strikeout shape="NONE" color="#000000"/>"##,
        r##"borderFillIDRef="1"><hh:underline type="BOTTOM" shape="SOLID" color="#0000FF"/><hh:strikeout shape="DASH_DOT" color="#FF0000"/>"##,
        r##"borderFillIDRef="1"><hh:underline type="NONE" shape="SOLID" color="#000000"/><hh:strikeout shape="3D" color="#000000"/>"##,
    ];
    let mut doc = hm_hwpx::read(package(&cases).into()).unwrap();
    doc.info.mark_all_dirty();
    let read: Vec<String> = cases
        .iter()
        .map(|c| {
            c.split_once('>')
                .unwrap()
                .1
                .replace(r#"<hh:underline type=""#, "")
                .replace(r#"" shape=""#, "/")
                .replace(r#"" color=""#, "/")
                .replace(r#""/><hh:strikeout shape=""#, " ")
                .replace(r#""/>"#, "")
        })
        .collect();
    assert_eq!(lines_of(hm_hwpx::write(&doc).unwrap()), read);

    doc.convert_to(hm_model::Format::Hwp5);
    let hwp = hm_hwp5::write_new(&doc).unwrap();
    let back = hm_hwp5::read(hwp.into()).unwrap();
    // (kind, shape, underline color, strike bits, strike shape, strike color) as stored.
    let stored: Vec<_> = back
        .info
        .char_shapes
        .iter()
        .map(|c| {
            let r = c.src.as_ref().unwrap();
            let u32_at = |o: usize| u32::from_le_bytes(r[4 + o..8 + o].try_into().unwrap());
            let a = u32_at(46);
            let (kind, shape, strike, sshape) =
                ((a >> 2) & 3, (a >> 4) & 15, (a >> 18) & 7, a >> 26);
            (kind, shape, u32_at(56), strike, sshape, u32_at(70))
        })
        .collect();
    let (g, r, b) = (0x00FF00, 0x0000FF, 0xFF0000);
    assert_eq!(
        stored,
        [
            (2, 5, g, 1, 3, r),
            (2, 3, r, 1, 3, r),
            (2, 0, 0, 0, 0, 0),
            (1, 0, b, 1, 3, r),
            (2, 15, 0, 1, 15, 0),
        ]
    );
    let mut back = back;
    back.convert_to(hm_model::Format::Hwpx);
    assert_eq!(
        lines_of(hm_hwpx::write_new(&back).unwrap()),
        [
            "NONE/SOLID/#000000 LONG_DASH/#00FF00",
            "NONE/SOLID/#000000 DASH_DOT/#FF0000",
            "NONE/SOLID/#000000 SOLID/#000000",
            "BOTTOM/SOLID/#0000FF DASH_DOT/#FF0000",
            "NONE/SOLID/#000000 3D/#000000",
        ]
    );
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
