//! Edits whose saves Hancom rejected or misread: each save here passes the
//! strict structural checks of `hm-check` and opens again.

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

fn at(para: u32, offset: u32) -> Value {
    json!({ "section": 0, "para": para, "offset": offset })
}

/// Where text may start in body paragraph `para`, after any section and
/// column definitions.
fn lead(doc: &mut hm_wasm::Doc, para: u32) -> u32 {
    let p = query(
        doc,
        json!({ "kind": "paragraphs", "list": { "section": 0 }, "from": para, "count": 1 }),
    );
    p[0]["caret"][0].as_u64().unwrap() as u32
}

/// Saves as `format`, checks the file strictly and opens it again.
fn saved(doc: &mut hm_wasm::Doc, format: &str) -> hm_wasm::Doc {
    let bytes = doc.serialize_as(format, false).unwrap();
    let report = hm_check::check(&bytes);
    let errors: Vec<String> = report.errors().map(|i| i.to_string()).collect();
    assert!(errors.is_empty(), "{format} save: {errors:#?}");
    hm_wasm::Engine::new().open(bytes).unwrap()
}

fn styles(doc: &mut hm_wasm::Doc) -> u64 {
    query(doc, json!({ "kind": "styles" }))
        .as_array()
        .unwrap()
        .len() as u64
}

#[test]
fn deleted_styles_leave_counts_and_ids_that_match_the_styles() {
    for (name, format) in [
        ("hancom-notes.hwp", "hwp5"),
        ("hancom-table-4x4.hwpx", "hwpx"),
    ] {
        let engine = hm_wasm::Engine::new();
        let mut doc = engine.open(fixture(name)).unwrap();
        let n = styles(&mut doc);
        // 개요 10 is unused. 쪽 번호 after it names 바탕글 as its next
        // style, so it moves up unchanged while the styles after it, which
        // name themselves, are renumbered.
        transact(&mut doc, json!([{ "op": "deleteStyle", "id": 11 }]));
        transact(
            &mut doc,
            json!([{ "op": "createStyle", "style": { "name": "added", "kind": "paragraph" } }]),
        );
        let mut again = saved(&mut doc, format);
        assert_eq!(styles(&mut again), n, "{name}");
    }
}

#[test]
fn shapes_pasted_from_another_document_are_counted() {
    let engine = hm_wasm::Engine::new();
    let mut target = engine.open(fixture("hancom-notes.hwp")).unwrap();
    // Its faces, shapes and border fills are new to the target.
    let source = engine.open(fixture("last-page-master.hwpx")).unwrap();
    let mut probe = engine.open(fixture("last-page-master.hwpx")).unwrap();
    let paras = query(
        &mut probe,
        json!({ "kind": "paragraphs", "list": { "section": 0 } }),
    );
    let last = paras.as_array().unwrap().last().unwrap().clone();
    let range = json!({
        "from": at(0, paras[0]["caret"][0].as_u64().unwrap() as u32),
        "to": at(last["index"].as_u64().unwrap() as u32, last["caret"][1].as_u64().unwrap() as u32),
    });
    let o = lead(&mut target, 0);
    target
        .paste_from(
            &source,
            &range.to_string(),
            &json!({ "at": at(0, o) }).to_string(),
        )
        .unwrap();
    saved(&mut target, "hwp5");
}

#[test]
fn characters_xml_cannot_hold_save_as_replacement_characters() {
    let engine = hm_wasm::Engine::new();
    let mut doc = engine.open(fixture("hancom-table-4x4.hwpx")).unwrap();
    let o = lead(&mut doc, 0);
    transact(
        &mut doc,
        json!([
            { "op": "insertText", "at": at(0, o), "text": "a\u{FFFE}b\u{FFFF}c" },
            { "op": "insertEquation", "at": at(0, o), "script": "x\u{1}y\u{1F}" },
        ]),
    );
    let mut again = saved(&mut doc, "hwpx");
    let text = query(
        &mut again,
        json!({ "kind": "paragraphs", "list": { "section": 0 } }),
    )[0]["text"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(text.contains("a\u{FFFD}b\u{FFFD}c"), "{text:?}");
    // The same text converted from HWP 5, where it is legal.
    let mut hwp = engine.open(fixture("hancom-notes.hwp")).unwrap();
    let o = lead(&mut hwp, 0);
    transact(
        &mut hwp,
        json!([{ "op": "insertText", "at": at(0, o), "text": "\u{FFFF}" }]),
    );
    saved(&mut hwp, "hwpx");
}

#[test]
fn editing_beside_a_saved_field_keeps_its_end_linked_to_its_begin() {
    let engine = hm_wasm::Engine::new();
    let mut doc = engine.open(fixture("hancom-table-4x4.hwpx")).unwrap();
    let o = lead(&mut doc, 0);
    transact(
        &mut doc,
        json!([
            { "op": "insertText", "at": at(0, o), "text": "name: " },
            { "op": "insertField", "at": at(0, o + 6), "kind": "clickHere", "name": "name", "text": "type here" },
        ]),
    );
    // Reopened, the field's begin is source markup; typing after it
    // rewrites the paragraph around that copy.
    let mut doc = saved(&mut doc, "hwpx");
    let p = &query(
        &mut doc,
        json!({ "kind": "paragraphs", "list": { "section": 0 } }),
    )[0];
    let end = p["caret"][1].as_u64().unwrap() as u32;
    transact(
        &mut doc,
        json!([{ "op": "insertText", "at": at(0, end), "text": " typed" }]),
    );
    saved(&mut doc, "hwpx");
}

#[test]
fn a_field_end_whose_begin_was_deleted_is_left_out_of_hwpx() {
    let engine = hm_wasm::Engine::new();
    let mut doc = engine.open(fixture("hancom-table-4x4.hwpx")).unwrap();
    let o = lead(&mut doc, 0);
    transact(
        &mut doc,
        json!([{ "op": "insertField", "at": at(0, o), "kind": "clickHere", "name": "name", "text": "type here" }]),
    );
    // The range takes the begin and two characters, not the end.
    transact(
        &mut doc,
        json!([{ "op": "delete", "from": at(0, o), "to": at(0, o + 10) }]),
    );
    saved(&mut doc, "hwpx");
}

#[test]
fn edits_in_a_master_page_reach_its_part() {
    let engine = hm_wasm::Engine::new();
    let mut hwpx = engine.open(fixture("master-page-both.hwpx")).unwrap();
    let hwp = saved(&mut hwpx, "hwp5");
    for (mut doc, format) in [(hwpx, "hwpx"), (hwp, "hwp5")] {
        master_edit_survives(&mut doc, format);
    }
}

fn master_edit_survives(doc: &mut hm_wasm::Doc, format: &str) {
    let p = &query(
        doc,
        json!({ "kind": "paragraphs", "list": { "section": 0 }, "count": 1 }),
    )[0];
    let secd = p["controls"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["lists"] == 1)
        .unwrap()["index"]
        .clone();
    let steps = json!([{ "para": 0, "control": secd, "index": 0 }]);
    let list = json!({ "section": 0, "steps": steps });
    let before = query(doc, json!({ "kind": "paragraphs", "list": list }))[0]["text"].clone();
    assert_eq!(before, "master", "{format}");
    transact(
        doc,
        json!([{ "op": "insertText", "at": { "section": 0, "steps": steps, "para": 0, "offset": 6 }, "text": " edited" }]),
    );
    let mut again = saved(doc, format);
    let after = query(&mut again, json!({ "kind": "paragraphs", "list": list }))[0]["text"].clone();
    assert_eq!(after, "master edited", "{format}");
}

#[test]
fn a_linked_text_box_is_not_pasted_into_another_format() {
    let engine = hm_wasm::Engine::new();
    let mut target = engine.open(fixture("hancom-notes.hwp")).unwrap();
    // The chain lives in HWPX markup that an HWP file cannot carry, so the
    // paste is refused as conversion refuses it, and the file stays valid.
    let source = engine.open(fixture("conversion-linked.hwpx")).unwrap();
    let mut probe = engine.open(fixture("conversion-linked.hwpx")).unwrap();
    let paras = query(
        &mut probe,
        json!({ "kind": "paragraphs", "list": { "section": 0 } }),
    );
    let last = paras.as_array().unwrap().last().unwrap().clone();
    let range = json!({
        "from": at(0, paras[0]["caret"][0].as_u64().unwrap() as u32),
        "to": at(last["index"].as_u64().unwrap() as u32, last["caret"][1].as_u64().unwrap() as u32),
    });
    let o = lead(&mut target, 0);
    let pasted = target.paste_from(
        &source,
        &range.to_string(),
        &json!({ "at": at(0, o) }).to_string(),
    );
    assert!(pasted.is_err());
    saved(&mut target, "hwp5");
}

#[test]
fn drawings_made_in_the_editor_read_back_with_their_style() {
    for kind in ["rect", "ellipse", "line", "textbox"] {
        let engine = hm_wasm::Engine::new();
        let mut doc = engine.open(fixture("hancom-notes.hwp")).unwrap();
        let o = lead(&mut doc, 0);
        transact(
            &mut doc,
            json!([{ "op": "createShape", "at": at(0, o), "kind": kind, "width": 5000, "height": 3000 }]),
        );
        saved(&mut doc, "hwp5");
        // The fill and shadow lay out the fields after them; a misplaced
        // field leaves the reread outline, fill and shadow in the tail.
        // The written defaults (an empty extra block, no shadow, the text
        // box mark) read back.
        let v: Value = serde_json::from_str(&doc.verify_hwp().unwrap()).unwrap();
        let lost: Vec<&Value> = v["serializationLosses"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| {
                let p = p.as_str().unwrap();
                p.contains("/component/")
                    && !p.ends_with("/fill/extra")
                    && !p.ends_with("/shadow")
                    && !p.ends_with("/component/flip")
            })
            .collect();
        assert!(lost.is_empty(), "{kind}: {lost:?}");
    }
}

#[test]
fn a_field_end_left_in_an_untouched_paragraph_is_dropped_with_its_begin() {
    let engine = hm_wasm::Engine::new();
    let mut doc = engine.open(fixture("hancom-table-4x4.hwpx")).unwrap();
    let o = lead(&mut doc, 0);
    transact(
        &mut doc,
        json!([{ "op": "insertText", "at": at(0, o), "text": "first second" }]),
    );
    transact(
        &mut doc,
        json!([{ "op": "insertField", "at": at(0, o), "to": at(0, o + 12), "kind": "clickHere", "name": "name" }]),
    );
    // Splitting inside the field leaves its begin in one paragraph and
    // its end in the next.
    transact(&mut doc, json!([{ "op": "split", "at": at(0, o + 8 + 5) }]));
    // Reopened, both paragraphs are source markup; deleting the begin
    // rewrites the first and leaves the second, with the end, untouched.
    let mut doc = saved(&mut doc, "hwpx");
    transact(
        &mut doc,
        json!([{ "op": "delete", "from": at(0, o), "to": at(0, o + 10) }]),
    );
    saved(&mut doc, "hwpx");
}

#[test]
fn a_paragraph_with_more_lines_than_its_header_counts_saves_none() {
    let mut doc = hm_hwp5::read(fixture("hancom-notes.hwp").into()).unwrap();
    let p = &mut doc.sections[0].paras[0];
    let seg = p.line_segs[0].clone();
    p.line_segs = vec![seg; u16::MAX as usize + 1];
    p.src = None;
    doc.sections[0].src = None;
    let bytes = hm_hwp5::write(&doc).unwrap();
    let errors: Vec<String> = hm_check::check(&bytes)
        .errors()
        .map(|i| i.to_string())
        .collect();
    assert!(errors.is_empty(), "{errors:#?}");
}

/// HWP 3 documents save as new HWP 5 files (`HEEMANG_CORPUS`, skipped when
/// unset).
#[test]
fn hwp3_documents_save_as_valid_hwp5() {
    let Some(root) = std::env::var_os("HEEMANG_CORPUS") else {
        eprintln!("HEEMANG_CORPUS is not set; skipping");
        return;
    };
    let mut stack = vec![std::path::PathBuf::from(root)];
    let mut checked = 0;
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = e.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            if !bytes.starts_with(b"HWP Document File V") {
                continue;
            }
            let mut doc = hm_wasm::Engine::new().open(bytes).unwrap();
            let saved = doc.serialize(false).unwrap();
            let errors: Vec<String> = hm_check::check(&saved)
                .errors()
                .map(|i| i.to_string())
                .collect();
            assert!(errors.is_empty(), "{}: {errors:#?}", path.display());
            checked += 1;
        }
    }
    eprintln!("{checked} HWP 3 documents saved as HWP 5");
}

/// A valid 1x1 PNG.
const PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0,
    0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 156, 99, 248, 207, 192, 240, 31, 0,
    5, 0, 1, 255, 137, 153, 61, 29, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];

#[test]
fn first_pictures_bullets_and_numberings_follow_hancoms_docinfo_order() {
    // hancom-notes.hwp has no BinData, bullets or numberings; their first
    // records go after the id mappings, not after the trailing records.
    for (name, ops) in [
        (
            "picture",
            json!([{ "op": "insertPicture", "at": "AT", "bytes": PNG, "extension": "png", "width": 5000, "height": 5000 }]),
        ),
        (
            "bullet",
            json!([{ "op": "ensureList", "definition": { "kind": "bullet", "bullet": "●" } }]),
        ),
        (
            "numbering",
            json!([{ "op": "ensureList", "definition": { "kind": "numbering", "levels": [{ "format": "^1." }] } }]),
        ),
    ] {
        let engine = hm_wasm::Engine::new();
        let mut doc = engine.open(fixture("hancom-notes.hwp")).unwrap();
        let o = lead(&mut doc, 0);
        let ops: Value =
            serde_json::from_str(&ops.to_string().replace("\"AT\"", &at(0, o).to_string()))
                .unwrap();
        transact(&mut doc, ops);
        let bytes = doc.serialize_as("hwp5", false).unwrap();
        let errors: Vec<String> = hm_check::check(&bytes)
            .errors()
            .map(|i| i.to_string())
            .collect();
        assert!(errors.is_empty(), "{name}: {errors:#?}");
    }
}

#[test]
fn text_boxes_made_in_the_editor_save_as_hancom_reads_them() {
    for kind in ["textbox", "ellipse"] {
        let engine = hm_wasm::Engine::new();
        let mut doc = engine.open(fixture("hancom-notes.hwp")).unwrap();
        let o = lead(&mut doc, 0);
        transact(
            &mut doc,
            json!([{ "op": "createShape", "at": at(0, o), "kind": kind, "width": 8000, "height": 5000, "text": "글" }]),
        );
        let mut again = saved(&mut doc, "hwp5");
        let objects = query(
            &mut again,
            json!({ "kind": "paragraphs", "list": { "section": 0 }, "count": 1 }),
        );
        assert!(
            objects[0]["controls"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c["lists"] == 1),
            "{kind}"
        );
    }
}

/// A memo read from HWPX converts to HWP like the rest of the document
/// (`HEEMANG_CORPUS`, skipped when unset).
#[test]
fn memos_read_from_hwpx_save_as_hwp_records() {
    let Some(root) = std::env::var_os("HEEMANG_CORPUS") else {
        eprintln!("HEEMANG_CORPUS is not set; skipping");
        return;
    };
    let bytes = std::fs::read(std::path::Path::new(&root).join("basic/english.hwp")).unwrap();
    let engine = hm_wasm::Engine::new();
    let mut hwp = engine.open(bytes).unwrap();
    let mut hwpx = saved(&mut hwp, "hwpx");
    saved(&mut hwpx, "hwp5");
}

#[test]
fn merging_whole_rows_saves_hwpx_without_empty_rows() {
    let engine = hm_wasm::Engine::new();
    let mut probe = engine.open(fixture("hancom-table-4x4.hwpx")).unwrap();
    let paras = query(
        &mut probe,
        json!({ "kind": "paragraphs", "list": { "section": 0 } }),
    );
    let table = paras
        .as_array()
        .unwrap()
        .iter()
        .find_map(|p| {
            let c = p["controls"]
                .as_array()?
                .iter()
                .find(|c| c["kind"] == "table")?;
            Some(json!({ "section": 0, "para": p["index"], "control": c["index"] }))
        })
        .unwrap();
    for (end_row, end_col) in [(1, 3), (3, 3)] {
        let mut doc = engine.open(fixture("hancom-table-4x4.hwpx")).unwrap();
        transact(
            &mut doc,
            json!([{ "op": "mergeTableCells", "table": table, "startRow": 0, "startColumn": 0, "endRow": end_row, "endColumn": end_col }]),
        );
        saved(&mut doc, "hwpx");
    }
}

/// Linked pictures keep their file path in an HWPX conversion
/// (`HEEMANG_CORPUS`, skipped when unset).
#[test]
fn linked_pictures_are_listed_when_converted_to_hwpx() {
    let Some(root) = std::env::var_os("HEEMANG_CORPUS") else {
        eprintln!("HEEMANG_CORPUS is not set; skipping");
        return;
    };
    let bytes = std::fs::read(std::path::Path::new(&root).join("hwp3-sample10-hwp5.hwp")).unwrap();
    let mut doc = hm_wasm::Engine::new().open(bytes).unwrap();
    let saved = doc.serialize_as("hwpx", false).unwrap();
    let archive = hm_hwpx::zip::Archive::open(saved.into()).unwrap();
    let hpf = archive.read_name("Contents/content.hpf").unwrap().unwrap();
    let hpf = String::from_utf8_lossy(&hpf);
    for id in ["image1", "image2", "image3"] {
        assert!(
            hpf.contains(&format!(r#"<opf:item id="{id}" href="D:\Work"#)),
            "{id} is not listed as a linked item"
        );
    }
}
