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
    // An endnote in that section sits where it did, under no separator:
    // the added shapes are the ones the layout assumed without them.
    let p = query(
        &mut doc,
        json!({"kind": "paragraphs", "list": {"section": 0}, "count": 1}),
    );
    let end = p[0]["length"].as_u64().unwrap() - 1;
    let at = json!({"section": 0, "para": 0, "offset": end});
    transact(
        &mut doc,
        json!([{"op": "insertNote", "at": at, "kind": "endnote"}]),
    );
    let direct = pages(&mut doc);
    assert_eq!(pages(&mut converted(&mut doc, "hwp5")), direct);
}

#[test]
fn a_char_shape_naming_a_missing_face_converts_to_a_valid_hwpx() {
    // Another writer's HWP: a char shape names a Latin face the table lacks.
    let mut model = hm_hwp5::read(fixture("hancom-notes.hwp").into()).unwrap();
    let cs = &mut model.info.char_shapes[0];
    cs.faces[hm_model::Script::Latin.index()] = 99;
    cs.src = None;
    let bytes = hm_hwp5::write(&model).unwrap();
    let mut doc = hm_wasm::Engine::new().open(bytes).unwrap();
    let direct = pages(&mut doc);
    let mut back = converted(&mut doc, "hwpx");
    assert_eq!(pages(&mut back), direct);
}

#[test]
fn a_header_lacking_tables_it_names_converts_to_a_complete_hwp() {
    // Another writer's HWPX: no Latin faces, tab definitions or styles,
    // and an outline numbering past the numbering table. `converted`
    // checks the HWP strictly.
    let header = |x: String| {
        let x = without(&x, r#"<hh:fontface lang="LATIN""#, "</hh:fontface>");
        let x = without(&x, "<hh:tabProperties", "</hh:tabProperties>");
        without(&x, "<hh:styles", "</hh:styles>")
    };
    let section = |x: String| x.replace(r#"outlineShapeIDRef="1""#, r#"outlineShapeIDRef="2""#);
    let bytes = rewritten(
        fixture("hancom-table-4x4.hwpx"),
        &[
            ("Contents/header.xml", &header),
            ("Contents/section0.xml", &section),
        ],
    );
    let source = hm_hwpx::read(bytes.clone().into()).unwrap();
    assert!(source.info.tab_defs.is_empty() && source.info.styles.is_empty());
    let mut doc = hm_wasm::Engine::new().open(bytes).unwrap();
    let direct = pages(&mut doc);
    let mut back = converted(&mut doc, "hwp5");
    assert_eq!(pages(&mut back), direct);
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

#[test]
fn line_end_caps_owpml_has_no_name_for_keep_their_look() {
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
    // An HWP 5 file whose rectangle outline has cap code 15, as some
    // Hancom files store it.
    let mut hwp = hm_hwp5::read(doc.serialize_as("hwp5", false).unwrap().into()).unwrap();
    let mut capped = 0;
    for c in &mut hwp.sections[0].paras[0].controls {
        if let Some(hm_model::ControlKind::Object(o)) = Some(&mut c.kind)
            && let Some(hm_model::ObjectPart::Shape(s)) = o
                .parts
                .iter_mut()
                .find(|p| matches!(p, hm_model::ObjectPart::Shape(_)))
            && let Some(st) = s.component.style.as_mut()
        {
            st.line.attr = st.line.attr & !(0xF << 6) | 15 << 6;
            capped += 1;
        }
        c.src = None;
    }
    assert_eq!(capped, 1);
    hwp.sections[0].paras[0].src = None;
    hwp.sections[0].src = None;
    let mut doc = hm_wasm::Engine::new()
        .open(hm_hwp5::write(&hwp).unwrap())
        .unwrap();
    let direct = pages(&mut doc);
    assert!(direct[0].contains("stroke"));
    let mut hwpx = converted(&mut doc, "hwpx");
    assert_eq!(pages(&mut hwpx), direct);
    assert_eq!(pages(&mut converted(&mut hwpx, "hwp5")), direct);
}

#[test]
fn equation_scripts_with_control_characters_convert_as_hancom_saves_them() {
    let mut doc = hm_wasm::Engine::new()
        .open(fixture("hancom-notes.hwp"))
        .unwrap();
    let p = query(
        &mut doc,
        json!({"kind": "paragraphs", "list": {"section": 0}, "count": 1}),
    );
    let at = json!({"section": 0, "para": 0, "offset": p[0]["caret"][0]});
    transact(
        &mut doc,
        json!([{"op": "insertEquation", "at": at, "script": "x\u{1}y\u{3}z over 2"}]),
    );
    let hwp = doc.serialize(false).unwrap();
    let mut doc = hm_wasm::Engine::new().open(hwp).unwrap();
    let direct = pages(&mut doc);
    let hwpx = doc.export_hwpx().unwrap();
    let back = hm_hwpx::read(hwpx.clone().into()).unwrap();
    let scripts: Vec<String> = back.sections[0].paras[0]
        .controls
        .iter()
        .filter_map(|c| {
            c.as_object()?
                .equation()
                .map(|e| e.script.to_string_lossy())
        })
        .collect();
    assert!(scripts.contains(&"x y z over 2".to_string()), "{scripts:?}");
    assert_eq!(pages(&mut converted(&mut doc, "hwpx")), direct);
}

#[test]
fn default_face_names_survive_hwpx_as_substitutes() {
    // The names a face falls back to when it is missing: its substitute,
    // then its default name.
    let fallbacks = |d: &hm_model::Document| -> Vec<Vec<String>> {
        d.info
            .faces
            .iter()
            .flatten()
            .map(|f| {
                f.substitute
                    .iter()
                    .map(|(_, n)| n.to_string_lossy())
                    .chain(f.default_name.iter().map(|n| n.to_string_lossy()))
                    .collect()
            })
            .collect()
    };
    let bytes = fixture("hancom-notes.hwp");
    let source = hm_hwp5::read(bytes.clone().into()).unwrap();
    assert!(
        source
            .info
            .faces
            .iter()
            .flatten()
            .any(|f| f.default_name.is_some())
    );
    let mut doc = hm_wasm::Engine::new().open(bytes).unwrap();
    let hwpx = hm_hwpx::read(doc.export_hwpx().unwrap().into()).unwrap();
    assert_eq!(fallbacks(&hwpx), fallbacks(&source));
    let mut back = converted(&mut converted(&mut doc, "hwpx"), "hwp5");
    let hwp = hm_hwp5::read(back.serialize(false).unwrap().into()).unwrap();
    assert_eq!(fallbacks(&hwp), fallbacks(&source));
}

#[test]
fn a_new_number_hancom_ignores_stays_ignored_in_hwpx() {
    let mut doc = hm_wasm::Engine::new()
        .open(fixture("hancom-notes-twopage.hwpx"))
        .unwrap();
    let paras = query(
        &mut doc,
        json!({"kind": "paragraphs", "list": {"section": 0}}),
    );
    let last = paras.as_array().unwrap().len() as u32 - 1;
    let at = json!({"section": 0, "para": last, "offset": paras[last as usize]["caret"][0]});
    transact(
        &mut doc,
        json!([
            {"op": "setPageNumberPos", "section": 0, "position": 5},
            {"op": "insertNewNumber", "at": at, "number": 7},
        ]),
    );
    // As another writer stores it: under code 18, which Hancom ignores.
    let mut hwp = hm_hwp5::read(doc.serialize_as("hwp5", false).unwrap().into()).unwrap();
    let mut moved = 0;
    for s in &mut hwp.sections {
        for p in &mut s.paras {
            let at: Vec<usize> = p
                .units()
                .filter_map(|(pos, u)| match u {
                    hm_model::Unit::Extended { code: 21, index }
                        if matches!(
                            p.controls[index as usize].kind,
                            hm_model::ControlKind::NewNumber(_)
                        ) =>
                    {
                        Some(pos as usize)
                    }
                    _ => None,
                })
                .collect();
            for pos in at {
                p.text[pos] = 18;
                p.text[pos + 7] = 18;
                p.src = None;
                moved += 1;
            }
        }
        s.src = None;
    }
    assert_eq!(moved, 1);
    let mut doc = hm_wasm::Engine::new()
        .open(hm_hwp5::write(&hwp).unwrap())
        .unwrap();
    // The page numbers each page shows.
    let numbers = |d: &mut hm_wasm::Doc| -> Vec<u64> {
        serde_json::from_str::<Value>(&d.pages().unwrap())
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["number"].as_u64().unwrap())
            .collect()
    };
    let direct = numbers(&mut doc);
    assert!(direct.len() >= 2 && !direct.contains(&7), "{direct:?}");
    let mut hwpx = converted(&mut doc, "hwpx");
    assert_eq!(numbers(&mut hwpx), direct);
    assert_eq!(numbers(&mut converted(&mut hwpx, "hwp5")), direct);
}

#[test]
fn a_combo_box_showing_nothing_keeps_showing_nothing() {
    // Hancom's own combo box markup (windows1 capture), entries added.
    let combo = r##"<hp:comboBox listBoxRows="10" listBoxWidth="0" editEnable="1" selectedValue="" name="ComboBox" foreColor="#000000" backColor="#E7E1CB" groupName="" tabStop="1" editable="1" tabOrder="3" enabled="1" borderTypeIDRef="1" drawFrame="1" printable="1" command=""><hp:formCharPr charPrIDRef="0" followContext="0" autoSz="0" wordWrap="0"/><hp:listItem displayText="봄" value="봄"/><hp:listItem displayText="여름" value="여름"/><hp:sz width="9921" widthRelTo="ABSOLUTE" height="1984" heightRelTo="ABSOLUTE" protect="0"/><hp:pos treatAsChar="1" affectLSpacing="0" flowWithText="1" allowOverlap="1" holdAnchorAndSO="0" vertRelTo="PARA" horzRelTo="COLUMN" vertAlign="TOP" horzAlign="LEFT" vertOffset="0" horzOffset="0"/><hp:outMargin left="0" right="0" top="0" bottom="0"/></hp:comboBox>"##;
    let bytes = patched(
        "hancom-table-4x4.hwpx",
        "Contents/section0.xml",
        "<hp:t>표 편집 캡처</hp:t>",
        &format!("{combo}<hp:t>표 편집 캡처</hp:t>"),
    );
    // As an HWP 5 combo box showing nothing reads: an empty text, its
    // entries from the document script.
    let mut doc = hm_hwpx::read(bytes.into()).unwrap();
    let mut emptied = 0;
    for c in &mut doc.sections[0].paras[0].controls {
        if let hm_model::ControlKind::Form(f) = &mut c.kind
            && let Some(hm_model::ParamValue::Set(set)) = f
                .props
                .items
                .iter_mut()
                .find(|(k, _)| k == "ComboBoxSet")
                .map(|(_, v)| v)
        {
            set.set("Text", hm_model::ParamValue::Str(String::new()));
            emptied += 1;
        }
    }
    assert_eq!(emptied, 1);
    doc.mark_all_dirty();
    doc.source = None;
    let shown = |d: &hm_model::Document| -> Vec<(String, usize)> {
        d.sections[0].paras[0]
            .controls
            .iter()
            .filter_map(|c| match &c.kind {
                hm_model::ControlKind::Form(f) => Some((
                    f.props
                        .find_set("ComboBoxSet")?
                        .find("Text")?
                        .as_str()?
                        .to_string(),
                    f.items.len(),
                )),
                _ => None,
            })
            .collect()
    };
    assert_eq!(shown(&doc), vec![(String::new(), 2)]);
    let again = hm_hwpx::read(hm_hwpx::write_new(&doc).unwrap().into()).unwrap();
    assert_eq!(shown(&again), vec![(String::new(), 2)]);
}

#[test]
fn merging_whole_rows_stores_the_same_grid_in_both_formats() {
    let mut doc = hm_wasm::Engine::new()
        .open(fixture("hancom-table-4x4.hwpx"))
        .unwrap();
    let p = query(
        &mut doc,
        json!({"kind": "paragraphs", "list": {"section": 0}}),
    );
    let (para, control) = p
        .as_array()
        .unwrap()
        .iter()
        .find_map(|p| {
            let c = p["controls"]
                .as_array()?
                .iter()
                .find(|c| c["kind"] == "table")?;
            Some((p["index"].clone(), c["index"].clone()))
        })
        .unwrap();
    let table = json!({"section": 0, "para": para, "control": control});
    // The first two rows merged whole: Hancom keeps one row for them.
    transact(
        &mut doc,
        json!([{"op": "mergeTableCells", "table": table, "startRow": 0, "startColumn": 0, "endRow": 1, "endColumn": 3}]),
    );
    let rows = |bytes: Vec<u8>| -> Vec<u16> {
        let d = if bytes.starts_with(b"PK") {
            hm_hwpx::read(bytes.into()).unwrap()
        } else {
            hm_hwp5::read(bytes.into()).unwrap()
        };
        d.sections[0]
            .paras
            .iter()
            .flat_map(|p| &p.controls)
            .filter_map(|c| c.as_table().map(|t| t.rows))
            .collect()
    };
    let hwpx = rows(doc.serialize(false).unwrap());
    assert_eq!(hwpx, vec![3]);
    assert_eq!(rows(doc.serialize_as("hwp5", false).unwrap()), hwpx);
    let direct = {
        let b = doc.serialize(false).unwrap();
        pages(&mut hm_wasm::Engine::new().open(b).unwrap())
    };
    assert_eq!(pages(&mut converted(&mut doc, "hwp5")), direct);
}

#[test]
fn a_table_pasted_into_a_narrower_cell_saves_its_fitted_size_and_order() {
    let mut doc = hm_wasm::Engine::new()
        .open(fixture("hancom-table-4x4.hwpx"))
        .unwrap();
    let p = query(
        &mut doc,
        json!({"kind": "paragraphs", "list": {"section": 0}}),
    );
    let (para, control, offset) = p
        .as_array()
        .unwrap()
        .iter()
        .find_map(|p| {
            let c = p["controls"]
                .as_array()?
                .iter()
                .find(|c| c["kind"] == "table")?;
            Some((
                p["index"].clone(),
                c["index"].clone(),
                c["offset"].as_u64()?,
            ))
        })
        .unwrap();
    // The whole table copied into its own first cell, which is narrower.
    // With text around it, so it pastes as a table in the text.
    let from = json!({"section": 0, "para": 0, "offset": p[0]["caret"][0]});
    let to = json!({"section": 0, "para": para, "offset": offset + 8});
    doc.copy(&json!({"from": from, "to": to}).to_string())
        .unwrap();
    // Into a cell of a new two-column table, half as wide.
    let end = query(
        &mut doc,
        json!({"kind": "paragraphs", "list": {"section": 0}}),
    );
    let last = end.as_array().unwrap().len() - 1;
    let at = json!({"section": 0, "para": last, "offset": end[last]["caret"][1]});
    transact(
        &mut doc,
        json!([{"op": "createTable", "at": at, "rows": 2, "columns": 2}]),
    );
    let end = query(
        &mut doc,
        json!({"kind": "paragraphs", "list": {"section": 0}}),
    );
    let (host, hc) = end
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find_map(|p| {
            let c = p["controls"]
                .as_array()?
                .iter()
                .find(|c| c["kind"] == "table")?;
            Some((p["index"].clone(), c["index"].clone()))
        })
        .unwrap();
    let cell = json!({"section": 0, "steps": [{"para": host, "control": hc, "index": 0}], "para": 0, "offset": 0});
    doc.paste(&json!({"at": cell}).to_string()).unwrap();
    // The nested table's width and z-order, as each save stores them.
    let nested = |bytes: Vec<u8>| -> Vec<(i32, i32)> {
        let d = if bytes.starts_with(b"PK") {
            hm_hwpx::read(bytes.into()).unwrap()
        } else {
            hm_hwp5::read(bytes.into()).unwrap()
        };
        let mut out = Vec::new();
        for p in &d.sections[0].paras {
            for c in &p.controls {
                if let Some(t) = c.as_table() {
                    for cell in &t.cells {
                        for q in &cell.paras {
                            for k in &q.controls {
                                if let Some(n) = k.as_table() {
                                    out.push((n.header.common.width, n.header.common.z_order));
                                }
                            }
                        }
                    }
                }
            }
        }
        out
    };
    let own = nested(doc.serialize(false).unwrap());
    assert_eq!(own.len(), 1);
    assert_eq!(nested(doc.serialize_as("hwp5", false).unwrap()), own);
}

#[test]
fn edited_note_paragraphs_save_the_lines_they_show() {
    let mut doc = hm_wasm::Engine::new()
        .open(fixture("hancom-notes.hwpx"))
        .unwrap();
    let p = query(
        &mut doc,
        json!({"kind": "paragraphs", "list": {"section": 0}}),
    );
    let (para, control) = p
        .as_array()
        .unwrap()
        .iter()
        .find_map(|p| {
            let c = p["controls"]
                .as_array()?
                .iter()
                .find(|c| c["kind"] == "footnote" || c["kind"] == "note")?;
            Some((p["index"].clone(), c["index"].clone()))
        })
        .unwrap();
    let note = json!({"section": 0, "steps": [{"para": para, "control": control, "index": 0}], "para": 0, "offset": 0});
    let n = query(
        &mut doc,
        json!({"kind": "paragraphs", "list": {"section": 0, "steps": [{"para": para, "control": control, "index": 0}]}}),
    );
    let at = json!({"section": 0, "steps": [{"para": para, "control": control, "index": 0}], "para": 0, "offset": n[0]["caret"][1]});
    let _ = note;
    // Text first, saved and reopened so the note stores its lines; then a
    // wide indent and an outline number break them differently.
    transact(
        &mut doc,
        json!([{"op": "insertText", "at": at, "text": "각주에 덧붙인 긴 문장이 여러 줄로 이어진다. ".repeat(6)}]),
    );
    let mut doc = hm_wasm::Engine::new()
        .open(doc.serialize(false).unwrap())
        .unwrap();
    let start = json!({"section": 0, "steps": [{"para": para, "control": control, "index": 0}], "para": 0, "offset": 0});
    let lead = p
        .as_array()
        .unwrap()
        .iter()
        .find(|q| q["index"] == para)
        .unwrap()["caret"][0]
        .clone();
    let host = json!({"section": 0, "para": para, "offset": lead});
    transact(
        &mut doc,
        json!([
            {"op": "formatParas", "from": start, "to": start, "patch": {"left": 6000}},
            {"op": "setPageSettings", "section": 0, "patch": {"marginLeft": 3839, "marginTop": 7660}},
            {"op": "insertNewNumber", "at": host, "number": 5},
        ]),
    );
    // The note paragraph's lines in each save: the ones laid out now.
    let note_lines = |d: hm_model::Document| -> Vec<(u32, i32)> {
        let host = &d.sections[0].paras[para.as_u64().unwrap() as usize];
        host.controls
            .iter()
            .find_map(|c| match &c.kind {
                hm_model::ControlKind::Note(n) => Some(
                    n.list.paras[0]
                        .line_segs
                        .iter()
                        .map(|l| (l.text_start, l.width))
                        .collect(),
                ),
                _ => None,
            })
            .unwrap()
    };
    let own = note_lines(hm_hwpx::read(doc.serialize(false).unwrap().into()).unwrap());
    let other = note_lines(hm_hwp5::read(doc.serialize_as("hwp5", false).unwrap().into()).unwrap());
    assert!(!own.is_empty());
    assert_eq!(own, other);
}

#[test]
fn a_paragraph_given_columns_is_marked_as_hancom_marks_it() {
    for name in ["hancom-notes.hwp", "hancom-notes.hwpx"] {
        let mut doc = hm_wasm::Engine::new().open(fixture(name)).unwrap();
        let p = query(
            &mut doc,
            json!({"kind": "paragraphs", "list": {"section": 0}}),
        );
        let last = p.as_array().unwrap().len() - 1;
        let at = json!({"section": 0, "para": last, "offset": p[last]["caret"][0]});
        transact(
            &mut doc,
            json!([{"op": "setColumns", "at": at, "patch": {"count": 2, "gap": 1000}}]),
        );
        // Every HWP save marks the paragraph holding the column definition.
        let marked = |bytes: Vec<u8>| -> Vec<bool> {
            let d = hm_hwp5::read(bytes.into()).unwrap();
            d.sections[0]
                .paras
                .iter()
                .filter(|p| {
                    p.controls
                        .iter()
                        .any(|c| matches!(c.kind, hm_model::ControlKind::Columns(_)))
                })
                .map(|p| p.header.break_kind & hm_model::break_kind::MULTI_COLUMN != 0)
                .collect()
        };
        let hwp = marked(doc.serialize_as("hwp5", false).unwrap());
        assert!(hwp.len() >= 2 && hwp.iter().all(|m| *m), "{name}: {hwp:?}");
    }
}

#[test]
fn typed_tabs_keep_their_width_when_reopened() {
    // A reopened paragraph whose stored lines replay takes its tabs'
    // advances from the tab controls, as Hancom stores them.
    for name in ["hancom-notes.hwp", "hancom-notes.hwpx"] {
        let mut doc = hm_wasm::Engine::new().open(fixture(name)).unwrap();
        let p = query(
            &mut doc,
            json!({"kind": "paragraphs", "list": {"section": 0}}),
        );
        let last = p.as_array().unwrap().len() - 1;
        let at = |offset: u64| json!({"section": 0, "para": last, "offset": offset});
        transact(
            &mut doc,
            json!([{"op": "insertText", "at": at(0), "text": "a\tb\tc"}]),
        );
        let caret = |doc: &mut hm_wasm::Doc| -> Vec<Value> {
            [0, 1, 9, 10, 18]
                .iter()
                .map(|&o| {
                    let v: Value = serde_json::from_str(
                        &doc.locate(&json!({"kind": "caret", "position": at(o)}).to_string())
                            .unwrap(),
                    )
                    .unwrap();
                    v["x"].clone()
                })
                .collect()
        };
        let typed = caret(&mut doc);
        assert!(typed[1] != typed[2], "{name}: {typed:?}");
        for format in ["hwp5", "hwpx"] {
            let mut back = converted(&mut doc, format);
            assert_eq!(caret(&mut back), typed, "{name} as {format}");
        }
    }
}

#[test]
fn a_host_line_keeps_its_place_below_its_tables_when_saved() {
    // A content-less line holding two flowing tables goes below them, one
    // holding a lone table takes no room; a save must reopen in that place.
    for name in ["hancom-notes.hwp", "hancom-notes.hwpx"] {
        let mut below = Vec::new();
        for tables in [1, 2] {
            let mut doc = hm_wasm::Engine::new().open(fixture(name)).unwrap();
            let p = query(
                &mut doc,
                json!({"kind": "paragraphs", "list": {"section": 0}}),
            );
            let last = p.as_array().unwrap().len() - 1;
            let end = p[last]["caret"].as_array().unwrap().last().unwrap().clone();
            let at =
                |para: usize, offset: Value| json!({"section": 0, "para": para, "offset": offset});
            transact(
                &mut doc,
                json!([
                    {"op": "split", "at": at(last, end), "pageBreak": false},
                    {"op": "insertText", "at": at(last + 1, json!(0)), "text": "after"},
                    {"op": "split", "at": at(last + 1, json!(0)), "pageBreak": false},
                ]),
            );
            for _ in 0..tables {
                transact(
                    &mut doc,
                    json!([{"op": "createTable", "at": at(last + 1, json!(0)), "rows": 2,
                        "columns": 2, "columnWidths": [9000, 9000], "inline": false}]),
                );
            }
            let top = |doc: &mut hm_wasm::Doc| -> i64 {
                let request = json!({"kind": "caret", "position": at(last + 2, json!(0))});
                let caret: Value =
                    serde_json::from_str(&doc.locate(&request.to_string()).unwrap()).unwrap();
                caret["y"].as_i64().unwrap()
            };
            let live = top(&mut doc);
            for format in ["hwp5", "hwpx"] {
                assert_eq!(
                    top(&mut converted(&mut doc, format)),
                    live,
                    "{name} with {tables} tables as {format}"
                );
            }
            below.push(live);
        }
        assert!(below[1] > below[0], "{name}: {below:?}");
    }
}
