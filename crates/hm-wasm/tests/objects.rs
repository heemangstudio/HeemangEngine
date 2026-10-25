//! Object boxes and geometry edits: a moved or resized object, and resized
//! table cells, keep their geometry through a save in either format, and
//! undo restores it (`HEEMANG_CORPUS`, skipped when unset).

use serde_json::{Value, json};

fn corpus(name: &str) -> Option<Vec<u8>> {
    let root = std::env::var_os("HEEMANG_CORPUS")?;
    Some(std::fs::read(std::path::Path::new(&root).join(name)).unwrap())
}

fn objects(doc: &mut hm_wasm::Doc, page: usize) -> Vec<Value> {
    serde_json::from_str::<Value>(&doc.page_objects(page).unwrap())
        .unwrap()
        .as_array()
        .unwrap()
        .clone()
}

fn address(o: &Value) -> Value {
    let mut a = json!({ "section": o["section"], "para": o["para"], "control": o["control"] });
    if let Some(steps) = o.get("steps") {
        a["steps"] = steps.clone();
    }
    a
}

fn find(doc: &mut hm_wasm::Doc, page: usize, at: &Value) -> Value {
    objects(doc, page)
        .into_iter()
        .find(|o| address(o) == *at)
        .unwrap_or_else(|| panic!("no object {at} on page {page}"))
}

fn transact(doc: &mut hm_wasm::Doc, ops: Value) -> Value {
    serde_json::from_str(&doc.transact(&json!({ "ops": ops }).to_string()).unwrap()).unwrap()
}

fn geometry(o: &Value) -> [i64; 4] {
    ["x", "y", "width", "height"].map(|k| o[k].as_i64().unwrap())
}

/// Saving in both formats and opening again keeps `want` for the object.
fn saved_keeps(doc: &mut hm_wasm::Doc, page: usize, at: &Value, want: [i64; 4]) {
    for format in ["hwp5", "hwpx"] {
        let bytes = doc.serialize_as(format, false).unwrap();
        let engine = hm_wasm::Engine::new();
        let mut again = engine.open(bytes).unwrap();
        assert_eq!(
            geometry(&find(&mut again, page, at)),
            want,
            "{format} save of {at}"
        );
    }
}

#[test]
fn moved_and_resized_pictures_survive_saves_and_undo() {
    for name in ["test-image.hwp", "test-image.hwpx"] {
        let Some(bytes) = corpus(name) else {
            eprintln!("HEEMANG_CORPUS is not set; skipping");
            return;
        };
        let engine = hm_wasm::Engine::new();
        let mut doc = engine.open(bytes).unwrap();
        let before = find(&mut doc, 0, &json!({"section":0,"para":0,"control":2}));
        assert_eq!(before["kind"], "picture");
        assert_eq!(before["inline"], false);
        let at = address(&before);
        let [x, y, w, h] = geometry(&before);
        transact(
            &mut doc,
            json!([
                { "op": "resizeObject", "object": at, "width": w + 1500, "height": h - 600 },
                { "op": "moveObject", "object": at, "dx": 2000, "dy": 1000 },
            ]),
        );
        let want = [x + 2000, y + 1000, w + 1500, h - 600];
        assert_eq!(geometry(&find(&mut doc, 0, &at)), want, "{name}");
        saved_keeps(&mut doc, 0, &at, want);
        doc.undo().unwrap();
        assert_eq!(
            geometry(&find(&mut doc, 0, &at)),
            [x, y, w, h],
            "{name} undo"
        );
    }
}

#[test]
fn resized_shapes_and_groups_keep_their_box() {
    let cases = [
        (
            "mix-shape-01.hwp",
            json!({"section":0,"para":0,"control":3}),
        ),
        (
            "mix-shape-01.hwp",
            json!({"section":0,"para":0,"control":2}),
        ),
        ("draw-group.hwp", json!({"section":0,"para":0,"control":2})),
        ("group-box.hwp", json!({"section":0,"para":0,"control":2})),
    ];
    for (name, at) in cases {
        let Some(bytes) = corpus(name) else {
            eprintln!("HEEMANG_CORPUS is not set; skipping");
            return;
        };
        let engine = hm_wasm::Engine::new();
        let mut doc = engine.open(bytes).unwrap();
        let [x, y, w, h] = geometry(&find(&mut doc, 0, &at));
        transact(
            &mut doc,
            json!([{ "op": "resizeObject", "object": at, "width": w * 3 / 2, "height": h / 2 + 300 }]),
        );
        let after = geometry(&find(&mut doc, 0, &at));
        assert_eq!(after[2..], [w * 3 / 2, h / 2 + 300], "{name} {at}");
        assert_eq!(after[..2], [x, y], "{name} {at} keeps its top left");
        saved_keeps(&mut doc, 0, &at, after);
    }
}

#[test]
fn table_borders_move_between_neighbours() {
    let Some(bytes) = corpus("hwp_table_test.hwp") else {
        eprintln!("HEEMANG_CORPUS is not set; skipping");
        return;
    };
    let engine = hm_wasm::Engine::new();
    let mut doc = engine.open(bytes).unwrap();
    let table = objects(&mut doc, 0)
        .into_iter()
        .find(|o| o["kind"] == "table" && o["furniture"] == false)
        .unwrap();
    let at = address(&table);
    let info: Value = serde_json::from_str(
        &doc.query(&json!({"kind":"table","table":at}).to_string())
            .unwrap(),
    )
    .unwrap();
    let cells = info["cells"].as_array().unwrap();
    let first = |row: u64, col: u64| {
        cells
            .iter()
            .position(|c| c["row"] == row && c["column"] == col)
            .unwrap()
    };
    let width = |i: usize| cells[i]["rects"][0]["width"].as_i64().unwrap();
    let (a, b) = (first(0, 0), first(0, 1));
    let (wa, wb) = (width(a), width(b));
    // The border between the first two columns, in every row.
    let mut updates = Vec::new();
    for (i, c) in cells.iter().enumerate() {
        let end = c["column"].as_u64().unwrap() + c["columnSpan"].as_u64().unwrap();
        if end == 1 {
            updates.push(json!({"index": i, "widthDelta": 1200}));
        } else if c["column"] == 1 {
            updates.push(json!({"index": i, "widthDelta": -1200}));
        }
    }
    transact(
        &mut doc,
        json!([{ "op": "resizeTableCells", "table": at, "cells": updates }]),
    );
    let info: Value = serde_json::from_str(
        &doc.query(&json!({"kind":"table","table":at}).to_string())
            .unwrap(),
    )
    .unwrap();
    let cells = info["cells"].as_array().unwrap();
    let width = |i: usize| cells[i]["rects"][0]["width"].as_i64().unwrap();
    assert_eq!((width(a), width(b)), (wa + 1200, wb - 1200));
    let after = find(&mut doc, 0, &at);
    assert_eq!(after["width"], table["width"], "the table keeps its width");
    saved_keeps(&mut doc, 0, &at, geometry(&after));

    // The bottom border of the first row grows that row and the table.
    transact(
        &mut doc,
        json!([{ "op": "resizeTableCells", "table": at, "cells": cells.iter().enumerate()
            .filter(|(_, c)| c["row"] == 0)
            .map(|(i, _)| json!({"index": i, "heightDelta": 900}))
            .collect::<Vec<_>>() }]),
    );
    let grown = find(&mut doc, 0, &at);
    assert_eq!(
        grown["height"].as_i64().unwrap(),
        after["height"].as_i64().unwrap() + 900
    );
    saved_keeps(&mut doc, 0, &at, geometry(&grown));
}
