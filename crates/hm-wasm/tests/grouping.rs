//! Line end points, grouping and ungrouping and objects carried by the
//! clipboard, each kept through undo and through saving in both formats.
use serde_json::{Value, json};

const TEMPLATE: &[u8] = include_bytes!("fixtures/query-equations.hwpx");

fn query(d: &mut hm_wasm::Doc, q: Value) -> Value {
    serde_json::from_str(&d.query(&q.to_string()).unwrap()).unwrap()
}
fn tx(d: &mut hm_wasm::Doc, ops: Value) -> Value {
    serde_json::from_str(&d.transact(&json!({ "ops": ops }).to_string()).unwrap()).unwrap()
}
fn blank() -> hm_wasm::Doc {
    hm_wasm::Engine::new().create(TEMPLATE.to_vec()).unwrap()
}
fn reopen(d: &mut hm_wasm::Doc, format: &str) -> hm_wasm::Doc {
    hm_wasm::Engine::new()
        .open(d.serialize_as(format, false).unwrap())
        .unwrap()
}
fn objects(d: &mut hm_wasm::Doc) -> Vec<Value> {
    let all: Value = serde_json::from_str(&d.page_objects(0).unwrap()).unwrap();
    all.as_array()
        .unwrap()
        .iter()
        .filter(|o| o["furniture"] == false)
        .cloned()
        .collect()
}
fn caret(d: &mut hm_wasm::Doc) -> Value {
    let p = query(
        d,
        json!({"kind":"paragraphs","list":{"section":0},"count":1}),
    );
    json!({"section":0,"para":0,"offset":p[0]["caret"][0]})
}
fn shape(d: &mut hm_wasm::Doc, kind: &str, x: i32, y: i32) {
    let at = caret(d);
    tx(
        d,
        json!([{"op":"createShape","at":at,"kind":kind,"width":6000,"height":3000,"horzOffset":x,"vertOffset":y}]),
    );
}
fn address(o: &Value) -> Value {
    json!({"section":o["section"],"para":o["para"],"control":o["control"]})
}
fn boxes(d: &mut hm_wasm::Doc) -> Vec<[i64; 4]> {
    let mut out: Vec<[i64; 4]> = objects(d)
        .iter()
        .map(|o| ["x", "y", "width", "height"].map(|k| o[k].as_i64().unwrap()))
        .collect();
    out.sort();
    out
}

#[test]
fn line_ends_set_the_line_and_its_box_and_survive_saving() {
    let mut d = blank();
    shape(&mut d, "line", 1000, 1000);
    let line = objects(&mut d)[0].clone();
    let (x, y) = (line["x"].as_i64().unwrap(), line["y"].as_i64().unwrap());
    // Drag the end point right and up past the start: the line turns.
    tx(
        &mut d,
        json!([{"op":"setLineEnds","object":address(&line),"start":[0,3000],"end":[9000,-1000]}]),
    );
    let moved = objects(&mut d)[0].clone();
    assert_eq!(moved["line"], json!([x, y + 3000, x + 9000, y - 1000]));
    assert_eq!(
        (moved["width"].as_i64(), moved["height"].as_i64()),
        (Some(9000), Some(4000))
    );
    for format in ["hwp5", "hwpx"] {
        let mut r = reopen(&mut d, format);
        assert_eq!(objects(&mut r)[0]["line"], moved["line"], "{format}");
    }
    d.undo().unwrap();
    assert_eq!(objects(&mut d)[0]["line"], line["line"]);
    let err = d
        .transact(
            &json!({"ops":[{"op":"setLineEnds","object":{"section":0,"para":0,"control":99},"start":[0,0],"end":[1,1]}]})
                .to_string(),
        )
        .unwrap_err();
    assert!(format!("{err:?}").contains("control"));
}

#[test]
fn grouping_keeps_members_in_place_and_ungrouping_restores_them() {
    let mut d = blank();
    shape(&mut d, "rect", 1000, 1000);
    shape(&mut d, "ellipse", 5000, 4000);
    let before = boxes(&mut d);
    let members: Vec<Value> = objects(&mut d).iter().map(address).collect();
    tx(&mut d, json!([{"op":"groupObjects","objects":members}]));
    let grouped = objects(&mut d);
    assert_eq!(grouped.len(), 1);
    assert_eq!(grouped[0]["kind"], "group");
    let union = [
        before.iter().map(|b| b[0]).min().unwrap(),
        before.iter().map(|b| b[1]).min().unwrap(),
        before.iter().map(|b| b[0] + b[2]).max().unwrap(),
        before.iter().map(|b| b[1] + b[3]).max().unwrap(),
    ];
    let g = &grouped[0];
    let got = ["x", "y", "width", "height"].map(|k| g[k].as_i64().unwrap());
    assert_eq!(
        got,
        [union[0], union[1], union[2] - union[0], union[3] - union[1]]
    );
    let painted = d.page_svg(0, 96.0).unwrap();
    assert!(
        painted.contains("<path") || painted.contains("<ellipse"),
        "the members still draw"
    );
    for format in ["hwp5", "hwpx"] {
        let mut r = reopen(&mut d, format);
        let o = objects(&mut r);
        assert_eq!(o.len(), 1, "{format}");
        assert_eq!(o[0]["kind"], "group", "{format}");
        tx(
            &mut r,
            json!([{"op":"ungroupObject","object":address(&o[0])}]),
        );
        assert_eq!(
            boxes(&mut r),
            before,
            "{format}: members back where they were"
        );
        let mut again = reopen(&mut r, format);
        assert_eq!(
            boxes(&mut again),
            before,
            "{format}: ungrouped members saved"
        );
    }
    d.undo().unwrap();
    assert_eq!(boxes(&mut d), before);
    d.redo().unwrap();
    assert_eq!(objects(&mut d).len(), 1);
}

#[test]
fn copied_objects_paste_back_into_their_document() {
    let mut d = blank();
    shape(&mut d, "rect", 1000, 1000);
    let o = objects(&mut d)[0].clone();
    let p = query(
        &mut d,
        json!({"kind":"paragraphs","list":{"section":0},"count":1}),
    );
    let control = p[0]["controls"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["index"] == o["control"])
        .unwrap()
        .clone();
    let offset = control["offset"].as_u64().unwrap();
    let copied: Value = serde_json::from_str(
        &d.copy(
            &json!({"from":{"section":0,"para":0,"offset":offset},"to":{"section":0,"para":0,"offset":offset + 8}})
                .to_string(),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(copied["objects"], 1);
    d.paste(&json!({"at":{"section":0,"para":0,"offset":offset + 8}}).to_string())
        .unwrap();
    assert_eq!(objects(&mut d).len(), 2);
    for format in ["hwp5", "hwpx"] {
        let mut r = reopen(&mut d, format);
        assert_eq!(objects(&mut r).len(), 2, "{format}");
    }
    d.undo().unwrap();
    assert_eq!(objects(&mut d).len(), 1);
    // Another document gets the text only.
    let mut other = blank();
    let at = caret(&mut other);
    other.paste(&json!({ "at": at }).to_string()).ok();
    assert_eq!(objects(&mut other).len(), 0);
}
