use hm_wasm::{Doc, Engine};
use serde_json::{Value, json};
fn query(d: &mut Doc, q: Value) -> Value {
    serde_json::from_str(&d.query(&q.to_string()).unwrap()).unwrap()
}
fn tx(d: &mut Doc, ops: Value) -> Value {
    serde_json::from_str(&d.transact(&json!({"ops":ops}).to_string()).unwrap()).unwrap()
}
fn open() -> Doc {
    Engine::new()
        .create(include_bytes!("fixtures/query-equations.hwpx").to_vec())
        .unwrap()
}
fn transpose(d: &mut Doc, q: Value) -> Value {
    serde_json::from_str(&d.transpose_table(&q.to_string()).unwrap()).unwrap()
}
fn table(d: &mut Doc) -> Value {
    let p = query(
        d,
        json!({"kind":"paragraphs","list":{"section":0},"count":1}),
    );
    let c = p[0]["controls"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["kind"] == "table")
        .unwrap();
    json!({"section":0,"para":0,"control":c["index"]})
}
fn cell_text(d: &mut Doc, t: &Value, index: usize) -> String {
    query(d,json!({"kind":"paragraphs","list":{"section":0,"steps":[{"para":0,"control":t["control"],"index":index}]}}))[0]["text"].as_str().unwrap().into()
}
#[test]
fn transposed_cells_preserve_contents_history_and_both_saved_formats() {
    let mut d = open();
    let at = query(
        &mut d,
        json!({"kind":"paragraphs","list":{"section":0},"count":1}),
    )[0]["caret"][0]
        .clone();
    tx(
        &mut d,
        json!([{ "op":"createTable","at":{"section":0,"para":0,"offset":at},"rows":2,"columns":3 }]),
    );
    let t = table(&mut d);
    for i in 0..6 {
        tx(
            &mut d,
            json!([{ "op":"insertText","at":{"section":0,"steps":[{"para":0,"control":t["control"],"index":i}],"para":0,"offset":0},"text":format!("C{i}😀")} ]),
        );
    }
    let capture = d
        .capture_paragraph(&json!({"section":0,"para":0,"offset":0}).to_string())
        .unwrap();
    transpose(
        &mut d,
        json!({"action":"copy","table":t,"startRow":0,"startColumn":0,"endRow":1,"endColumn":2}),
    );
    assert!(d.has_table_transpose_clipboard().unwrap());
    let receipt = transpose(&mut d, json!({"action":"inPlace","table":t}));
    assert_eq!(receipt["targetRows"], 3);
    for (i, text) in ["C0😀", "C3😀", "C1😀", "C4😀", "C2😀", "C5😀"]
        .iter()
        .enumerate()
    {
        assert_eq!(cell_text(&mut d, &t, i), *text);
    }
    for format in ["hwp5", "hwpx"] {
        let mut r = Engine::new()
            .open(d.serialize_as(format, false).unwrap())
            .unwrap();
        assert_eq!(query(&mut r, json!({"kind":"table","table":t}))["rows"], 3);
        assert_eq!(cell_text(&mut r, &t, 1), "C3😀");
    }
    d.undo().unwrap();
    assert_eq!(cell_text(&mut d, &t, 1), "C1😀");
    d.redo().unwrap();
    let saved = d.serialize(false).unwrap();
    assert!(
        d.transpose_table(
            &json!({"action":"paste","table":t,"startRow":2,"startColumn":1}).to_string()
        )
        .is_err()
    );
    assert_eq!(d.serialize(false).unwrap(), saved);
    // Restore one paragraph after an unrelated body edit; nested table content travels whole.
    tx(
        &mut d,
        json!([{ "op":"split","at":{"section":0,"para":0,"offset":at} }]),
    );
    tx(
        &mut d,
        json!([{ "op":"insertText","at":{"section":0,"para":0,"offset":at},"text":"untouched"}]),
    );
    d.restore_paragraph(
        capture,
        &json!({"section":0,"para":1,"offset":0}).to_string(),
    )
    .unwrap();
    assert_eq!(
        query(
            &mut d,
            json!({"kind":"paragraphs","list":{"section":0},"count":1})
        )[0]["text"],
        "untouched"
    );
    let restored = json!({"section":0,"para":1,"control":t["control"]});
    assert_eq!(
        query(&mut d, json!({"kind":"table","table":restored}))["rows"],
        2
    );
    assert!(d.discard_paragraph_capture(capture).unwrap());
    assert!(
        d.restore_paragraph(
            capture,
            &json!({"section":0,"para":1,"offset":0}).to_string()
        )
        .is_err()
    );
}
#[test]
fn copied_transpose_pastes_into_cells_or_a_new_table_as_one_step() {
    let mut d = open();
    let p = query(
        &mut d,
        json!({"kind":"paragraphs","list":{"section":0},"count":1}),
    );
    let at = json!({"section":0,"para":0,"offset":p[0]["caret"][0]});
    tx(
        &mut d,
        json!([{ "op":"createTable","at":at,"rows":2,"columns":2 }]),
    );
    let t = table(&mut d);
    tx(
        &mut d,
        json!([{ "op":"insertText","at":{"section":0,"steps":[{"para":0,"control":t["control"],"index":1}],"para":0,"offset":0},"text":"styled"}]),
    );
    transpose(
        &mut d,
        json!({"action":"copy","table":t,"startRow":0,"startColumn":0,"endRow":1,"endColumn":1}),
    );
    transpose(
        &mut d,
        json!({"action":"paste","table":t,"startRow":0,"startColumn":0}),
    );
    assert_eq!(cell_text(&mut d, &t, 2), "styled");
    d.undo().unwrap();
    assert_eq!(cell_text(&mut d, &t, 1), "styled");
    let pasted = transpose(&mut d, json!({"action":"insert","at":at}));
    let new = json!({"section":0,"para":pasted["paraIdx"],"control":pasted["controlIdx"]});
    assert_eq!(cell_text(&mut d, &new, 2), "styled");
    d.undo().unwrap();
    assert_eq!(
        query(
            &mut d,
            json!({"kind":"paragraphs","list":{"section":0},"count":1})
        )[0]["controls"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["kind"] == "table")
            .count(),
        1
    );
}
#[test]
fn connector_subjects_follow_moved_shapes_and_survive_both_formats() {
    let mut d = open();
    let mut shapes = Vec::new();
    for x in [1000, 10000] {
        let p = query(
            &mut d,
            json!({"kind":"paragraphs","list":{"section":0},"count":1}),
        );
        let at = json!({"section":0,"para":0,"offset":p[0]["offsets"].as_array().unwrap().last().unwrap()});
        tx(
            &mut d,
            json!([{ "op":"createShape","at":at,"kind":"rect","width":3000,"height":2000,"horzOffset":x,"vertOffset":2000 }]),
        );
        let p = query(
            &mut d,
            json!({"kind":"paragraphs","list":{"section":0},"count":1}),
        );
        let c = p[0]["controls"]
            .as_array()
            .unwrap()
            .iter()
            .rev()
            .find(|c| c["kind"] == "object")
            .unwrap();
        let a = json!({"section":0,"para":0,"control":c["index"]});
        let prop = query(&mut d, json!({"kind":"objectProperties","object":a}));
        shapes.push((a, prop["instanceId"].clone()));
    }
    assert_ne!(shapes[0].1, shapes[1].1);
    let p = query(
        &mut d,
        json!({"kind":"paragraphs","list":{"section":0},"count":1}),
    );
    let at =
        json!({"section":0,"para":0,"offset":p[0]["offsets"].as_array().unwrap().last().unwrap()});
    tx(
        &mut d,
        json!([{ "op":"createShape","at":at,"kind":"line","width":6000,"height":1000,"horzOffset":4000,"vertOffset":3000,"connector":{"kind":3,"start":[shapes[0].1,1],"end":[shapes[1].1,3]}}]),
    );
    let p = query(
        &mut d,
        json!({"kind":"paragraphs","list":{"section":0},"count":1}),
    );
    let connector = json!({"section":0,"para":0,"control":p[0]["controls"].as_array().unwrap().iter().rev().find(|c|c["kind"]=="object").unwrap()["index"]});
    d.update_connectors(0).unwrap();
    tx(
        &mut d,
        json!([{ "op":"moveObject","object":shapes[1].0,"dx":4000,"dy":2000 }]),
    );
    let before = d.serialize(false).unwrap();
    assert_ne!(d.update_connectors(0).unwrap(), "null");
    let moved = d.serialize(false).unwrap();
    assert_ne!(moved, before);
    assert_eq!(d.update_connectors(0).unwrap(), "null");
    d.undo().unwrap();
    assert_eq!(d.serialize(false).unwrap(), before);
    d.redo().unwrap();
    for format in ["hwp5", "hwpx"] {
        let bytes = d.serialize_as(format, false).unwrap();
        let mut r = Engine::new().open(bytes).unwrap();
        assert_eq!(
            query(
                &mut r,
                json!({"kind":"objectProperties","object":connector})
            )["border"]["width"],
            28
        );
        let unchanged = r.serialize(false).unwrap();
        assert_eq!(r.update_connectors(0).unwrap(), "null");
        assert_eq!(r.serialize(false).unwrap(), unchanged);
        tx(
            &mut r,
            json!([{ "op":"moveObject","object":shapes[1].0,"dx":1000,"dy":0 }]),
        );
        assert_ne!(r.update_connectors(0).unwrap(), "null");
    }
}

#[test]
fn a_copied_cell_subset_pastes_as_a_valid_table_in_both_formats() {
    let mut d = open();
    let at = query(
        &mut d,
        json!({"kind":"paragraphs","list":{"section":0},"count":1}),
    )[0]["caret"][0]
        .clone();
    tx(
        &mut d,
        json!([{ "op":"createTable","at":{"section":0,"para":0,"offset":at},"rows":2,"columns":3 }]),
    );
    let t = table(&mut d);
    tx(
        &mut d,
        json!([{ "op":"insertText","at":{"section":0,"steps":[{"para":0,"control":t["control"],"index":1}],"para":0,"offset":0},"text":"subset"}]),
    );
    let copied = transpose(
        &mut d,
        json!({"action":"copy","transpose":false,"table":t,"startRow":0,"startColumn":1,"endRow":0,"endColumn":1}),
    );
    assert_eq!(copied["text"], "subset");
    transpose(
        &mut d,
        json!({"action":"paste","transpose":false,"table":t,"startRow":1,"startColumn":2}),
    );
    assert_eq!(cell_text(&mut d, &t, 5), "subset");
    d.paste(&json!({"at":{"section":0,"para":0,"offset":at}}).to_string())
        .unwrap();
    for format in ["hwp5", "hwpx"] {
        let mut reopened = Engine::new()
            .open(d.serialize_as(format, false).unwrap())
            .unwrap();
        let t = table(&mut reopened);
        let info = query(&mut reopened, json!({"kind":"table","table":t}));
        assert_eq!(info["rows"], 1);
        assert_eq!(info["columns"], 1);
        assert_eq!(cell_text(&mut reopened, &t, 0), "subset");
    }
}

#[test]
fn search_across_controls_spans_an_inline_note_and_stops_after_its_last_character() {
    let mut d = open();
    let at = query(
        &mut d,
        json!({"kind":"paragraphs","list":{"section":0},"count":1}),
    )[0]["caret"][0]
        .clone();
    tx(
        &mut d,
        json!([{"op":"insertText","at":{"section":0,"para":0,"offset":at},"text":"각주가 "}]),
    );
    let start = at.as_u64().unwrap();
    tx(
        &mut d,
        json!([{"op":"insertNote","at":{"section":0,"para":0,"offset":start + 2},"kind":"footnote"}]),
    );
    let search = |d: &mut Doc, text: &str, across: bool| -> Value {
        serde_json::from_str(
            &d.search(
                &json!({"text":text,"caseSensitive":true,"acrossControls":across}).to_string(),
            )
            .unwrap(),
        )
        .unwrap()
    };
    let body = |m: &Value| {
        m["matches"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["from"]["steps"].is_null())
            .cloned()
            .collect::<Vec<_>>()
    };
    assert!(body(&search(&mut d, "각주가", false)).is_empty());
    let spanning = body(&search(&mut d, "각주가", true));
    assert_eq!(spanning.len(), 1);
    assert_eq!(spanning[0]["from"]["offset"], start);
    // Two characters, the note's eight units, then the last character.
    assert_eq!(spanning[0]["to"]["offset"], start + 11);
    let before = body(&search(&mut d, "각주", true));
    assert_eq!(
        before[0]["to"]["offset"],
        start + 2,
        "a match must not swallow the note after it"
    );
}
#[test]
fn a_table_pasted_into_a_narrower_cell_fits_it_and_cells_take_absolute_widths() {
    let mut d = open();
    let at = query(
        &mut d,
        json!({"kind":"paragraphs","list":{"section":0},"count":1}),
    )[0]["caret"][0]
        .clone();
    tx(
        &mut d,
        json!([{ "op":"createTable","at":{"section":0,"para":0,"offset":at},"rows":2,"columns":2 }]),
    );
    let outer = table(&mut d);
    let end = at.as_u64().unwrap() + 8;
    tx(
        &mut d,
        json!([{ "op":"split","at":{"section":0,"para":0,"offset":end} }]),
    );
    // A body-wide table, twice as wide as one outer cell.
    tx(
        &mut d,
        json!([{ "op":"createTable","at":{"section":0,"para":1,"offset":0},"rows":2,"columns":2 }]),
    );
    d.copy(
        &json!({"from":{"section":0,"para":1,"offset":0},"to":{"section":0,"para":1,"offset":8}})
            .to_string(),
    )
    .unwrap();
    let cell = json!({"section":0,"steps":[{"para":0,"control":outer["control"],"index":0}]});
    // 셀 안에 표로 넣기: the table goes into the cell instead of over the cells.
    d.paste(
        &json!({"at":{"section":0,"steps":cell["steps"],"para":0,"offset":0},"cells":"nest"})
            .to_string(),
    )
    .unwrap();
    let host = query(&mut d, json!({"kind":"table","table":outer}));
    let room = host["cells"][0]["rects"][0]["width"].as_i64().unwrap();
    let inner_control =
        query(&mut d, json!({"kind":"paragraphs","list":cell,"count":1}))[0]["controls"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["kind"] == "table")
            .unwrap()["index"]
            .clone();
    let inner = json!({"section":0,"steps":cell["steps"],"para":0,"control":inner_control});
    let widths = |d: &mut Doc| -> Vec<i64> {
        query(d, json!({"kind":"table","table":inner}))["cells"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["rects"][0]["width"].as_i64().unwrap_or(0))
            .collect()
    };
    let w = widths(&mut d);
    assert!(
        w.iter().all(|w| *w > 0),
        "every pasted cell is drawn: {w:?}"
    );
    assert!(
        w[0] + w[1] < room,
        "the pasted table fits its cell: {w:?} in {room}"
    );
    // An absolute size sets the laid-out width, whatever the drawn width.
    tx(
        &mut d,
        json!([{ "op":"resizeTableCells","table":inner,"cells":[
            {"index":0,"width":4000},{"index":1,"width":4000},
            {"index":2,"width":4000},{"index":3,"width":4000}]}]),
    );
    let w = widths(&mut d);
    assert!(w.iter().all(|w| (w - 4000).abs() <= 1), "{w:?}");
}

#[test]
fn a_cell_block_or_a_table_pasted_into_a_table_overwrites_the_cells_that_fit() {
    // Hancom's 셀 붙이기 default, 덮어쓰기: A1:B2 of a 3x3 table pasted at C2 fills C2 and C3 with
    // the block's first column and leaves out the second (clipboard case c13); a copied table
    // pasted with the caret in a cell overwrites the cells from there instead of nesting (c16).
    let mut d = open();
    let at = query(
        &mut d,
        json!({"kind":"paragraphs","list":{"section":0},"count":1}),
    )[0]["caret"][0]
        .clone();
    tx(
        &mut d,
        json!([{ "op":"createTable","at":{"section":0,"para":0,"offset":at},"rows":3,"columns":3 }]),
    );
    let t = table(&mut d);
    for i in 0..9 {
        let name = format!("{}{}", ["a", "b", "c"][i % 3], i / 3 + 1);
        tx(
            &mut d,
            json!([{ "op":"insertText","at":{"section":0,"steps":[{"para":0,"control":t["control"],"index":i}],"para":0,"offset":0},"text":name}]),
        );
    }
    let cells = |d: &mut Doc| (0..9).map(|i| cell_text(d, &t, i)).collect::<Vec<_>>();
    let before = cells(&mut d);
    transpose(
        &mut d,
        json!({"action":"copy","transpose":false,"table":t,"startRow":0,"startColumn":0,"endRow":1,"endColumn":1}),
    );
    transpose(
        &mut d,
        json!({"action":"paste","transpose":false,"table":t,"startRow":1,"startColumn":2}),
    );
    let mut expected = before.clone();
    expected[5] = "a1".into();
    expected[8] = "a2".into();
    assert_eq!(cells(&mut d), expected);
    d.undo().unwrap();
    assert_eq!(cells(&mut d), before);
    // A transposed block still has to fit.
    transpose(
        &mut d,
        json!({"action":"copy","table":t,"startRow":0,"startColumn":0,"endRow":1,"endColumn":1}),
    );
    let failed = d.transpose_table(
        &json!({"action":"paste","table":t,"startRow":1,"startColumn":2}).to_string(),
    );
    assert!(failed.is_err());
    assert_eq!(cells(&mut d), before);

    // The whole table copied as an object and pasted into B2.
    let offset = query(
        &mut d,
        json!({"kind":"paragraphs","list":{"section":0},"count":1}),
    )[0]["controls"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["kind"] == "table")
        .unwrap()["offset"]
        .as_u64()
        .unwrap();
    d.copy(
        &json!({"from":{"section":0,"para":0,"offset":offset},"to":{"section":0,"para":0,"offset":offset+8}})
            .to_string(),
    )
    .unwrap();
    let b2 = json!({"section":0,"steps":[{"para":0,"control":t["control"],"index":4}],"para":0,"offset":0});
    d.paste(&json!({"at":b2}).to_string()).unwrap();
    let mut expected = before.clone();
    expected[4] = "a1".into();
    expected[5] = "b1".into();
    expected[7] = "a2".into();
    expected[8] = "b2".into();
    assert_eq!(cells(&mut d), expected);
    let nested = |d: &mut Doc| {
        query(d,json!({"kind":"paragraphs","list":{"section":0,"steps":[{"para":0,"control":t["control"],"index":4}]}}))[0]["controls"]
            .as_array()
            .unwrap()
            .len()
    };
    assert_eq!(nested(&mut d), 0);
    for format in ["hwp5", "hwpx"] {
        let mut reopened = Engine::new()
            .open(d.serialize_as(format, false).unwrap())
            .unwrap();
        assert_eq!(cells(&mut reopened), expected);
    }
    d.undo().unwrap();
    assert_eq!(cells(&mut d), before);
    // 셀 안에 표로 넣기 nests it.
    d.paste(&json!({"at":b2,"cells":"nest"}).to_string())
        .unwrap();
    assert_eq!(nested(&mut d), 1);
}
