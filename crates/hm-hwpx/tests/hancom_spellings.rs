//! Line names as Hancom 2022 saves them (COM captures f1-enums borders/chars, f2-enums under):
//! Hancom reads no line for any other spelling, so a re-encode writes these.
use hm_hwpx::xml::{Ns, Xml};
use hm_hwpx::zip::{Archive, Member};

fn package(border_fills: &str, char_prs: &str) -> Vec<u8> {
    let header = format!(
        r#"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head" xmlns:hc="http://www.hancom.co.kr/hwpml/2011/core" xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph" version="1.2" secCnt="1"><hh:refList><hh:borderFills>{border_fills}</hh:borderFills><hh:charProperties>{char_prs}</hh:charProperties><hh:paraProperties itemCnt="1"><hh:paraPr id="0"/></hh:paraProperties></hh:refList></hh:head>"#
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

/// Reads the package, re-encodes every definition and returns the saved header.
fn resaved(border_fills: &str, char_prs: &str) -> (hm_model::Document, String) {
    let mut doc = hm_hwpx::read(package(border_fills, char_prs).into()).unwrap();
    let read = doc.clone();
    doc.info.mark_all_dirty();
    let saved = hm_hwpx::write(&doc).unwrap();
    let header = Archive::open(saved.into())
        .unwrap()
        .read_name("Contents/header.xml")
        .unwrap()
        .unwrap();
    (read, String::from_utf8(header.to_vec()).unwrap())
}

fn attrs(header: &str, el: &str, attr: &str) -> Vec<String> {
    let xml = Xml::parse(header.as_bytes().to_vec().into()).unwrap();
    let mut out = Vec::new();
    let mut stack = vec![xml.root()];
    while let Some(e) = stack.pop() {
        if e.is(Ns::Hh, el)
            && let Some(v) = e.attr(attr)
        {
            out.push(v.into_owned());
        }
        stack.extend(e.children().collect::<Vec<_>>().into_iter().rev());
    }
    out
}

#[test]
fn border_line_kinds_are_saved_in_hancoms_spelling() {
    // Kinds 12-17, then the spellings earlier engine builds wrote for 13, 14, 15 and 17.
    let kinds = [
        "WAVE",
        "DOUBLEWAVE",
        "THICK3D",
        "THICKREV3D",
        "3D",
        "REV3D",
        "DOUBLE_WAVE",
        "THICK_3D",
        "THICK_3D_REVERSE_LIGHTING",
        "3D_REVERSE_LIGHTING",
    ];
    let fills: String = kinds
        .iter()
        .enumerate()
        .map(|(i, k)| {
            format!(
                r##"<hh:borderFill id="{}" threeD="0" shadow="0" centerLine="NONE" breakCellSeparateLine="0"><hh:leftBorder type="{k}" width="0.4 mm" color="#000000"/><hh:rightBorder type="NONE" width="0.1 mm" color="#000000"/><hh:topBorder type="NONE" width="0.1 mm" color="#000000"/><hh:bottomBorder type="NONE" width="0.1 mm" color="#000000"/></hh:borderFill>"##,
                i + 1
            )
        })
        .collect();
    let (doc, header) = resaved(&fills, r#"<hh:charPr id="0" height="1000"/>"#);
    let read: Vec<u8> = doc
        .info
        .border_fills
        .iter()
        .map(|b| b.sides[0].kind)
        .collect();
    assert_eq!(read, [12, 13, 14, 15, 16, 17, 13, 14, 15, 17]);
    assert_eq!(
        attrs(&header, "leftBorder", "type"),
        [
            "WAVE",
            "DOUBLEWAVE",
            "THICK3D",
            "THICKREV3D",
            "3D",
            "REV3D",
            "DOUBLEWAVE",
            "THICK3D",
            "THICKREV3D",
            "REV3D"
        ]
    );
}

#[test]
fn character_lines_are_saved_in_hancoms_spelling_and_rev3d_is_solid() {
    let shapes = [
        "WAVE",
        "DOUBLEWAVE",
        "THICK3D",
        "THICKREV3D",
        "3D",
        "REV3D",
        "DOUBLE_WAVE",
    ];
    let char_prs: String = shapes
        .iter()
        .enumerate()
        .map(|(i, s)| {
            format!(
                r##"<hh:charPr id="{i}" height="1000"><hh:underline type="BOTTOM" shape="{s}" color="#000000"/><hh:strikeout shape="{s}" color="#000000"/></hh:charPr>"##
            )
        })
        .collect();
    let (doc, header) = resaved("", &char_prs);
    // Hancom has no character line 16: it reads REV3D as a solid line, which leaves the
    // kerning bit (30) above the strikeout shape alone.
    for (c, want) in doc.info.char_shapes.iter().zip([11, 12, 13, 14, 15, 0, 12]) {
        assert_eq!(((c.attr >> 4) & 15, (c.attr >> 26) & 15), (want, want));
        assert_eq!(c.attr & 1 << 30, 0);
    }
    let want = [
        "WAVE",
        "DOUBLEWAVE",
        "THICK3D",
        "THICKREV3D",
        "3D",
        "SOLID",
        "DOUBLEWAVE",
    ];
    assert_eq!(attrs(&header, "underline", "shape"), want);
    assert_eq!(attrs(&header, "strikeout", "shape"), want);
}
