//! Where a click or an arrow key puts the caret around short lines and tables.
use serde_json::{Value, json};

/// An A4 page with ordinary margins.
const TEMPLATE: &[u8] = include_bytes!("fixtures/last-page-master.hwpx");
const LONG: &str = "The quick brown fox jumps over the lazy dog and keeps running through the \
long field until the line wraps twice across the page width here and again.";

fn query(d: &mut hm_wasm::Doc, q: Value) -> Value {
    serde_json::from_str(&d.query(&q.to_string()).unwrap()).unwrap()
}
fn tx(d: &mut hm_wasm::Doc, ops: Value) -> Value {
    serde_json::from_str(&d.transact(&json!({ "ops": ops }).to_string()).unwrap()).unwrap()
}
fn para(d: &mut hm_wasm::Doc, list: Value, index: usize) -> Value {
    query(
        d,
        json!({"kind":"paragraphs","list":list,"from":index,"count":1}),
    )[0]
    .clone()
}
fn caret(d: &mut hm_wasm::Doc, at: Value) -> Value {
    serde_json::from_str(
        &d.locate(&json!({"kind":"caret","position":at}).to_string())
            .unwrap(),
    )
    .unwrap()
}
fn point(d: &mut hm_wasm::Doc, x: i64, y: i64) -> Value {
    serde_json::from_str(
        &d.locate(&json!({"kind":"point","page":0,"x":x,"y":y,"body":true}).to_string())
            .unwrap(),
    )
    .unwrap()
}
fn navigate(d: &mut hm_wasm::Doc, at: &Value, motion: &str) -> Value {
    serde_json::from_str(
        &d.navigate(&json!({"position":at,"motion":motion}).to_string())
            .unwrap(),
    )
    .unwrap()
}
/// A blank document with `lines` as its paragraphs; their text addresses.
fn document(lines: &[&str]) -> (hm_wasm::Doc, Vec<Value>) {
    let mut d = hm_wasm::Engine::new().create(TEMPLATE.to_vec()).unwrap();
    let lead = para(&mut d, json!({"section":0}), 0)["caret"][0].clone();
    tx(
        &mut d,
        json!([{ "op":"insertText","at":{"section":0,"para":0,"offset":lead},"text":lines.join("\n")}]),
    );
    let starts = (0..lines.len())
        .map(|i| {
            let p = para(&mut d, json!({"section":0}), i);
            json!({"section":0,"para":i,"offset":p["caret"][0]})
        })
        .collect();
    (d, starts)
}

#[test]
fn a_point_right_of_a_short_line_finds_the_end_of_that_line() {
    let (mut d, starts) = document(&["short", LONG]);
    let short_end = para(&mut d, json!({"section":0}), 0)["caret"][1].clone();
    let short = caret(&mut d, starts[0].clone());
    let wrapped = caret(&mut d, json!({"section":0,"para":1,"offset":60}));
    let (x, y) = (
        wrapped["x"].as_i64().unwrap(),
        short["y"].as_i64().unwrap() + short["height"].as_i64().unwrap() * 3 / 4,
    );
    let hit = point(&mut d, x, y);
    assert_eq!(hit["position"]["para"], 0, "{hit}");
    assert_eq!(hit["position"]["offset"], short_end, "{hit}");
    // Below the short line's text but above the next line, still that line.
    let below = caret(&mut d, starts[1].clone())["y"].as_i64().unwrap() - 1;
    assert_eq!(point(&mut d, x, below)["position"]["para"], 0);
}

#[test]
fn up_and_down_onto_a_table_go_into_the_cell_under_the_caret() {
    // A table set in a line of text, and one in its own paragraph below the
    // line it is anchored to (Studio's default).
    for inline in [true, false] {
        onto_a_table(inline);
    }
}

fn onto_a_table(inline: bool) {
    let (mut d, starts) = document(&["above", "", "below"]);
    let at = if inline {
        let end = para(&mut d, json!({"section":0}), 0)["caret"][1].clone();
        json!({"section":0,"para":0,"offset":end})
    } else {
        starts[1].clone()
    };
    tx(
        &mut d,
        json!([{ "op":"createTable","at":at,"rows":2,"columns":2,"inline":inline }]),
    );
    let body = query(&mut d, json!({"kind":"paragraphs","list":{"section":0}}));
    let (host, control) = body
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .find_map(|(i, p)| {
            p["controls"]
                .as_array()?
                .iter()
                .find(|c| c["kind"] == "table")
                .map(|c| (i, c["index"].clone()))
        })
        .unwrap();
    let cell = |d: &mut hm_wasm::Doc, i: u32| {
        let list = json!({"section":0,"steps":[{"para":host,"control":control,"index":i}]});
        let lead = para(d, list.clone(), 0)["caret"][0].clone();
        json!({"section":0,"steps":list["steps"],"para":0,"offset":lead})
    };
    let (c1, c3) = (cell(&mut d, 1), cell(&mut d, 3));
    tx(
        &mut d,
        json!([
            { "op":"insertText","at":c3,"text":"right bottom" },
            { "op":"insertText","at":c1,"text":"right top" },
        ]),
    );
    // Caret under the right column: x of the text in cell 1.
    let right = caret(&mut d, c1)["x"].as_i64().unwrap() + 300;
    let above = navigate(&mut d, &starts[0], "down");
    assert!(
        above["position"]["steps"]
            .as_array()
            .is_some_and(|s| s.len() == 1),
        "inline {inline}: down from above: {above}"
    );
    let from_above: Value = serde_json::from_str(
        &d.navigate(&json!({"position":starts[0],"motion":"down","preferredX":right}).to_string())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        from_above["position"]["steps"][0]["index"], 1,
        "inline {inline}: {from_above}"
    );
    let last = body.as_array().unwrap().len() - 1;
    let below = json!({"section":0,"para":last,"offset":para(&mut d, json!({"section":0}), last)["caret"][0]});
    let from_below: Value = serde_json::from_str(
        &d.navigate(&json!({"position":below,"motion":"up","preferredX":right}).to_string())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        from_below["position"]["steps"][0]["index"], 3,
        "inline {inline}: {from_below}"
    );
}
