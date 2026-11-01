//! Converting to the other format and back: every conversion writes a file,
//! the strict checks of `hm-check` pass, and the reopened document shows the
//! same pages as the one saved in its own format.

use serde_json::{Value, json};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn transact(doc: &mut hm_wasm::Doc, ops: Value) {
    doc.transact(&json!({ "ops": ops }).to_string()).unwrap();
}

fn query(doc: &mut hm_wasm::Doc, q: Value) -> Value {
    serde_json::from_str(&doc.query(&q.to_string()).unwrap()).unwrap()
}

/// Saves as `format` the way Studio's Save As does, checks the file
/// strictly and opens it again.
fn converted(doc: &mut hm_wasm::Doc, format: &str) -> hm_wasm::Doc {
    let bytes = if format == "hwpx" {
        doc.export_hwpx()
    } else {
        doc.serialize_as(format, false)
    }
    .unwrap_or_else(|e| panic!("{format} conversion refused: {e:?}"));
    let report = hm_check::check(&bytes);
    let errors: Vec<String> = report.errors().map(|i| i.to_string()).collect();
    assert!(errors.is_empty(), "{format} save: {errors:#?}");
    hm_wasm::Engine::new().open(bytes).unwrap()
}

/// Every page as SVG.
fn pages(doc: &mut hm_wasm::Doc) -> Vec<String> {
    let n = serde_json::from_str::<Value>(&doc.pages().unwrap())
        .unwrap()
        .as_array()
        .unwrap()
        .len();
    (0..n).map(|i| doc.page_svg(i, 96.0).unwrap()).collect()
}

/// An HWPX fixture with one part's text replaced.
fn patched(name: &str, part: &str, from: &str, to: &str) -> Vec<u8> {
    patched_bytes(fixture(name), part, from, to)
}

/// An HWPX package with every `from` in one part replaced by `to`.
fn patched_bytes(bytes: Vec<u8>, part: &str, from: &str, to: &str) -> Vec<u8> {
    use hm_hwpx::zip::{Archive, Member, write};
    let archive = Archive::open(bytes.into()).unwrap();
    let xml = archive.read_name(part).unwrap().unwrap();
    let xml = String::from_utf8(xml.to_vec()).unwrap();
    assert!(xml.contains(from), "{part} has no {from}");
    let xml = xml.replace(from, to);
    let members: Vec<Member<'_>> = archive
        .entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            if e.name == part {
                Member::New {
                    name: &e.name,
                    data: xml.as_bytes(),
                    stored: false,
                }
            } else {
                Member::Copy(i)
            }
        })
        .collect();
    write(Some(&archive), &members)
}

#[test]
fn a_master_page_no_page_shows_converts() {
    // An even-page master page in a one-page section.
    let bytes = patched(
        "master-page-both.hwpx",
        "Contents/masterpage0.xml",
        r#"type="BOTH""#,
        r#"type="EVEN""#,
    );
    let mut doc = hm_wasm::Engine::new().open(bytes).unwrap();
    let direct = pages(&mut doc);
    let mut hwp = converted(&mut doc, "hwp5");
    assert_eq!(pages(&mut hwp), direct);
    let mut back = converted(&mut hwp, "hwpx");
    assert_eq!(pages(&mut back), direct);
}

#[test]
fn a_header_shown_at_two_page_widths_converts() {
    let mut doc = hm_wasm::Engine::new()
        .open(fixture("hancom-notes-twosec.hwpx"))
        .unwrap();
    // The second section gets wider margins; a header made in the first
    // runs on into it.
    transact(
        &mut doc,
        json!([
            {"op": "setPageSettings", "section": 1, "patch": {"marginLeft": 9000, "marginRight": 9000}},
            {"op": "createHeaderFooter", "section": 0, "kind": "header", "side": "both"},
        ]),
    );
    let direct = {
        let bytes = doc.serialize(false).unwrap();
        pages(&mut hm_wasm::Engine::new().open(bytes).unwrap())
    };
    let mut hwp = converted(&mut doc, "hwp5");
    assert_eq!(pages(&mut hwp), direct);
}

/// An HWPX fixture with every stored line removed from its sections.
fn without_lines(name: &str) -> Vec<u8> {
    use hm_hwpx::zip::{Archive, Member, write};
    let archive = Archive::open(fixture(name).into()).unwrap();
    let parts: Vec<(usize, String)> = archive
        .entries
        .iter()
        .enumerate()
        .filter(|(_, e)| e.name.starts_with("Contents/section"))
        .map(|(i, e)| {
            let xml = archive.read_name(&e.name).unwrap().unwrap();
            let mut xml = String::from_utf8(xml.to_vec()).unwrap();
            while let Some(a) = xml.find("<hp:linesegarray>") {
                let b = xml[a..].find("</hp:linesegarray>").unwrap() + a;
                xml.replace_range(a..b + "</hp:linesegarray>".len(), "");
            }
            (i, xml)
        })
        .collect();
    let members: Vec<Member<'_>> = archive
        .entries
        .iter()
        .enumerate()
        .map(|(i, e)| match parts.iter().find(|(k, _)| *k == i) {
            Some((_, xml)) => Member::New {
                name: &e.name,
                data: xml.as_bytes(),
                stored: false,
            },
            None => Member::Copy(i),
        })
        .collect();
    write(Some(&archive), &members)
}

/// Every paragraph's stored lines, lists included, in document order.
fn stored_lines(doc: &hm_model::Document) -> Vec<Vec<hm_model::LineSeg>> {
    fn walk(paras: &[hm_model::Paragraph], out: &mut Vec<Vec<hm_model::LineSeg>>) {
        for p in paras {
            out.push(p.line_segs.clone());
            for c in &p.controls {
                for list in hm_edit::ops::control_lists(c) {
                    walk(list, out);
                }
            }
        }
    }
    let mut out = Vec::new();
    for s in &doc.sections {
        walk(&s.paras, &mut out);
    }
    out
}

#[test]
fn unchanged_paragraphs_keep_their_stored_lines_or_their_absence() {
    for bytes in [
        fixture("hancom-notes.hwpx"),
        without_lines("hancom-notes.hwpx"),
    ] {
        let source = hm_hwpx::read(bytes.clone().into()).unwrap();
        let mut doc = hm_wasm::Engine::new().open(bytes).unwrap();
        let direct = pages(&mut doc);
        let hwp = doc.serialize_as("hwp5", false).unwrap();
        // Reopened, a paragraph replays or refits its lines as before.
        let back = hm_hwp5::read(hwp.clone().into()).unwrap();
        assert_eq!(stored_lines(&back), stored_lines(&source));
        let mut hwp = converted(&mut doc, "hwp5");
        assert_eq!(pages(&mut hwp), direct);
        let hwpx = hwp.export_hwpx().unwrap();
        let again = hm_hwpx::read(hwpx.into()).unwrap();
        assert_eq!(stored_lines(&again), stored_lines(&source));
    }
}

/// Picture effects and gradient step centers, lists included.
fn effects(doc: &hm_model::Document) -> Vec<String> {
    fn shape(s: &hm_model::Shape, out: &mut Vec<String>) {
        if let Some(g) = s
            .component
            .style
            .as_ref()
            .and_then(|st| st.fill.gradient.as_ref())
        {
            out.push(format!("gradient center {:?}", g.step_center));
        }
        for p in &s.parts {
            match p {
                hm_model::ShapePart::Picture(p) => out.push(format!("{:?}", p.effect_list())),
                hm_model::ShapePart::Child(c) => shape(c, out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    for p in &doc.sections[0].paras {
        for c in &p.controls {
            if let Some(s) = c.as_object().and_then(|o| o.shape()) {
                shape(s, &mut out);
            }
        }
    }
    out
}

#[test]
fn picture_effects_and_gradient_centers_survive_conversion() {
    const PNG: &[u8] = &[
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 4,
        0, 0, 0, 181, 28, 12, 2, 0, 0, 0, 11, 73, 68, 65, 84, 120, 218, 99, 252, 255, 31, 0, 3, 3,
        2, 0, 239, 191, 107, 153, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ];
    let mut doc = hm_wasm::Engine::new()
        .open(fixture("hancom-table-4x4.hwpx"))
        .unwrap();
    let p = query(
        &mut doc,
        json!({"kind": "paragraphs", "list": {"section": 0}, "count": 1}),
    );
    let at = json!({"section": 0, "para": 0, "offset": p[0]["caret"][0]});
    transact(
        &mut doc,
        json!([{"op": "createShape", "at": at, "kind": "rect", "width": 6000, "height": 4000}]),
    );
    let p = query(
        &mut doc,
        json!({"kind": "paragraphs", "list": {"section": 0}, "count": 1}),
    );
    let rect = p[0]["controls"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["kind"] == "object")
        .unwrap()["index"]
        .clone();
    transact(
        &mut doc,
        json!([
            {"op": "setObjectProperties", "object": {"section": 0, "para": 0, "control": rect},
                "patch": {"fill": {"kind": "gradient", "gradientType": 1, "angle": 90, "step": 20}}},
            {"op": "insertPicture", "at": at, "bytes": PNG, "extension": "png", "width": 6000, "height": 6000},
        ]),
    );
    // As Hancom writes them: a shadow centered under the picture and a soft
    // edge; a gradient centered off the middle.
    let bytes = patched_bytes(
        doc.export_hwpx().unwrap(),
        "Contents/section0.xml",
        "<hp:effects/>",
        r#"<hp:effects><hp:shadow style="OUTSIDE" alpha="0.5" radius="600" direction="45" distance="300" alignStyle="CENTER" rotationStyle="0"><hp:skew x="0" y="0"/><hp:scale x="1" y="1"/><hp:effectsColor type="RGB" schemeIdx="-1" systemIdx="-1" presetIdx="-1"><hp:rgb r="0" g="0" b="0"/></hp:effectsColor></hp:shadow><hp:softEdge radius="1000"/></hp:effects>"#,
    );
    let bytes = patched_bytes(
        bytes,
        "Contents/section0.xml",
        r#"stepCenter="50""#,
        r#"stepCenter="44""#,
    );
    let source = hm_hwpx::read(bytes.clone().into()).unwrap();
    let expected = effects(&source);
    assert!(
        expected
            .iter()
            .any(|e| e.contains("soft_edge: Some(1000.0)") && e.contains("shadow: Some"))
    );
    assert!(expected.contains(&"gradient center Some(44)".to_string()));
    let mut doc = hm_wasm::Engine::new().open(bytes).unwrap();
    let direct = pages(&mut doc);
    let hwp = doc.serialize_as("hwp5", false).unwrap();
    assert_eq!(effects(&hm_hwp5::read(hwp.into()).unwrap()), expected);
    let mut hwp = converted(&mut doc, "hwp5");
    assert_eq!(pages(&mut hwp), direct);
    let hwpx = hwp.export_hwpx().unwrap();
    assert_eq!(effects(&hm_hwpx::read(hwpx.into()).unwrap()), expected);
    assert_eq!(pages(&mut converted(&mut hwp, "hwpx")), direct);
}

#[test]
fn verify_hwp_ignores_what_the_writer_recomputes() {
    for name in ["query-equations.hwp", "hancom-notes.hwp"] {
        let mut doc = hm_wasm::Engine::new().open(fixture(name)).unwrap();
        let p = query(
            &mut doc,
            json!({"kind": "paragraphs", "list": {"section": 0}, "count": 1}),
        );
        let at = json!({"section": 0, "para": 0, "offset": p[0]["caret"][0]});
        // A text box (its geometry and text records go in Hancom's order,
        // its flip word gains Hancom's marks), a field and a bookmark (their
        // records gain the writer's data), a table that grows and a style.
        transact(
            &mut doc,
            json!([
                {"op": "createShape", "at": at, "kind": "textbox", "width": 3000, "height": 3000, "text": "글상자"},
                {"op": "insertField", "at": at, "kind": "hyperlink", "command": "https://example.com/", "name": "f", "text": "링크"},
                {"op": "insertBookmark", "at": at, "name": "책갈피"},
                {"op": "createTable", "at": at, "rows": 2, "columns": 2},
                {"op": "createStyle", "style": {"name": "검증", "kind": "paragraph"}},
            ]),
        );
        let v: Value = serde_json::from_str(&doc.verify_hwp().unwrap()).unwrap();
        assert_eq!(v["ok"], true, "{name}: {v}");
    }
}

/// `xml` without the first element starting with `start`.
fn without(xml: &str, start: &str, end: &str) -> String {
    let a = xml.find(start).unwrap_or_else(|| panic!("no {start}"));
    let b = xml[a..].find(end).unwrap() + a + end.len();
    format!("{}{}", &xml[..a], &xml[b..])
}

/// An HWPX package with parts rewritten.
fn rewritten(bytes: Vec<u8>, parts: &[(&str, &dyn Fn(String) -> String)]) -> Vec<u8> {
    use hm_hwpx::zip::{Archive, Member, write};
    let archive = Archive::open(bytes.into()).unwrap();
    let new: Vec<(usize, String)> = archive
        .entries
        .iter()
        .enumerate()
        .filter_map(|(i, e)| {
            let (_, f) = parts.iter().find(|(name, _)| *name == e.name)?;
            let xml = archive.read_name(&e.name).unwrap().unwrap();
            Some((i, f(String::from_utf8(xml.to_vec()).unwrap())))
        })
        .collect();
    let members: Vec<Member<'_>> = archive
        .entries
        .iter()
        .enumerate()
        .map(|(i, e)| match new.iter().find(|(k, _)| *k == i) {
            Some((_, xml)) => Member::New {
                name: &e.name,
                data: xml.as_bytes(),
                stored: false,
            },
            None => Member::Copy(i),
        })
        .collect();
    write(Some(&archive), &members)
}

#[test]
fn hwp_conversions_carry_the_records_hancom_requires() {
    // A header without the layout compatibility record and with a style
    // naming a missing paragraph shape; a section without note shapes and
    // with a page border fill for both pages only (another writer's HWPX).
    let header = |x: String| {
        x.replace("<hh:layoutCompatibility/>", "").replacen(
            r#"paraPrIDRef="1" charPrIDRef="0" nextStyleIDRef="1""#,
            r#"paraPrIDRef="9999" charPrIDRef="0" nextStyleIDRef="1""#,
            1,
        )
    };
    let section = |x: String| {
        let x = without(&x, "<hp:footNotePr>", "</hp:footNotePr>");
        let x = without(&x, "<hp:endNotePr>", "</hp:endNotePr>");
        let x = without(
            &x,
            r#"<hp:pageBorderFill type="EVEN""#,
            "</hp:pageBorderFill>",
        );
        without(
            &x,
            r#"<hp:pageBorderFill type="ODD""#,
            "</hp:pageBorderFill>",
        )
    };
    let bytes = rewritten(
        fixture("hancom-table-4x4.hwpx"),
        &[
            ("Contents/header.xml", &header),
            ("Contents/section0.xml", &section),
        ],
    );
    let source = hm_hwpx::read(bytes.clone().into()).unwrap();
    assert!(source.info.layout_compat.is_none());
    let mut doc = hm_wasm::Engine::new().open(bytes).unwrap();
    let direct = pages(&mut doc);
    let hwp = doc.serialize_as("hwp5", false).unwrap();
    let back = hm_hwp5::read(hwp.into()).unwrap();
    assert!(back.info.layout_compat.is_some());
    assert_eq!(back.info.styles[1].para_shape, 0);
    let def = back.sections[0].definition().unwrap();
    let count =
        |f: fn(&hm_model::SectionChild) -> bool| def.children.iter().filter(|k| f(k)).count();
    assert_eq!(
        count(|k| matches!(k, hm_model::SectionChild::NoteShape(_))),
        2
    );
    assert_eq!(
        count(|k| matches!(k, hm_model::SectionChild::PageBorder(_))),
        3
    );
    assert_eq!(pages(&mut converted(&mut doc, "hwp5")), direct);
}

#[test]
fn only_drawings_holding_text_get_the_text_mark_in_hwp() {
    let mut doc = hm_wasm::Engine::new()
        .open(fixture("hancom-table-4x4.hwpx"))
        .unwrap();
    let p = query(
        &mut doc,
        json!({"kind": "paragraphs", "list": {"section": 0}, "count": 1}),
    );
    let at = json!({"section": 0, "para": 0, "offset": p[0]["caret"][0]});
    transact(
        &mut doc,
        json!([
            {"op": "createShape", "at": at, "kind": "rect", "width": 4000, "height": 3000},
            {"op": "createShape", "at": at, "kind": "textbox", "width": 4000, "height": 3000, "text": "글상자"},
        ]),
    );
    // Through HWPX, which keeps no such mark, back to HWP.
    let mut hwpx = converted(&mut converted(&mut doc, "hwpx"), "hwpx");
    let back = hm_hwp5::read(hwpx.serialize_as("hwp5", false).unwrap().into()).unwrap();
    let marks: Vec<(bool, u32)> = back.sections[0].paras[0]
        .controls
        .iter()
        .filter_map(|c| c.as_object()?.shape())
        .map(|s| {
            let text = s
                .parts
                .iter()
                .any(|p| matches!(p, hm_model::ShapePart::Text(_)));
            (text, s.component.flip >> 24 & 1)
        })
        .collect();
    assert_eq!(marks.len(), 2, "{marks:?}");
    for (text, mark) in marks {
        assert_eq!(mark, text as u32);
    }
}
