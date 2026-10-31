//! Paragraph edits that follow Hancom's rules: merging, breaking and splitting
//! paragraphs, and their styles and shapes.
use hm_wasm::{Doc, Engine};
use serde_json::{Value, json};

fn query(d: &mut Doc, q: Value) -> Value {
    serde_json::from_str(&d.query(&q.to_string()).unwrap()).unwrap()
}
fn tx(d: &mut Doc, ops: Value) -> Value {
    serde_json::from_str(&d.transact(&json!({"ops":ops}).to_string()).unwrap()).unwrap()
}
/// An empty document: one paragraph holding the section and column definitions.
fn blank() -> Doc {
    Engine::new()
        .create(include_bytes!("fixtures/hancom-table-4x4.hwpx").to_vec())
        .unwrap()
}
fn reopen(d: &mut Doc, format: &str) -> Doc {
    Engine::new()
        .open(d.serialize_as(format, false).unwrap())
        .unwrap()
}
fn at(para: u32, offset: u32) -> Value {
    json!({"section":0,"para":para,"offset":offset})
}
fn paras(d: &mut Doc) -> Vec<Value> {
    query(d, json!({"kind":"paragraphs","list":{"section":0}}))
        .as_array()
        .unwrap()
        .clone()
}
fn texts(d: &mut Doc) -> Vec<String> {
    paras(d)
        .iter()
        .map(|p| p["text"].as_str().unwrap().to_string())
        .collect()
}
/// Where text starts in paragraph `para` and where it ends.
fn ends(d: &mut Doc, para: usize) -> (u32, u32) {
    let p = &paras(d)[para];
    (
        p["caret"][0].as_u64().unwrap() as u32,
        p["length"].as_u64().unwrap() as u32 - 1,
    )
}
fn formats(d: &mut Doc) -> Vec<Value> {
    query(d, json!({"kind":"formatRuns","list":{"section":0}}))
        .as_array()
        .unwrap()
        .clone()
}
/// Alignment, page break and column break of every paragraph.
fn shapes(d: &mut Doc) -> Vec<(String, bool, bool)> {
    formats(d)
        .iter()
        .map(|f| {
            (
                f["paragraphs"]["align"].as_str().unwrap().to_string(),
                f["pageBreak"].as_bool().unwrap(),
                f["columnBreak"].as_bool().unwrap(),
            )
        })
        .collect()
}
fn shape(align: &str, page: bool, column: bool) -> (String, bool, bool) {
    (align.to_string(), page, column)
}

/// Paragraphs from `(text, align)` pairs, the first one after the definitions.
fn build(d: &mut Doc, rows: &[(&str, &str)]) {
    let (lead, _) = ends(d, 0);
    for (i, (text, align)) in rows.iter().enumerate() {
        let i = i as u32;
        let start = if i == 0 { lead } else { 0 };
        if i > 0 {
            let (_, end) = ends(d, i as usize - 1);
            tx(d, json!([{"op":"split","at":at(i - 1, end)}]));
        }
        tx(
            d,
            json!([{"op":"formatParas","from":at(i, start),"to":at(i, start),"patch":{"align":align}}]),
        );
        if !text.is_empty() {
            tx(
                d,
                json!([{"op":"insertText","at":at(i, start),"text":text}]),
            );
        }
    }
}

#[test]
fn merging_into_an_empty_paragraph_keeps_the_lower_paragraph_in_the_upper_ones_place() {
    // Hancom (typing cases backspace-merge-empty-prev, delete-merge-empty-cur,
    // delete-multi-para-whole-first; fixes/typing-01/hancom): an upper paragraph holding no
    // characters gives way to the lower one, which keeps its shape but takes the upper one's
    // breaks.
    let mut d = blank();
    build(
        &mut d,
        &[("AAA", "justify"), ("", "center"), ("BBB", "right")],
    );
    tx(&mut d, json!([{"op":"pageBreak","at":at(1, 0),"on":true}]));
    let before = shapes(&mut d);
    assert_eq!(before[1], shape("center", true, false));
    tx(
        &mut d,
        json!([{"op":"delete","from":at(1, 0),"to":at(2, 0)}]),
    );
    assert_eq!(texts(&mut d), ["AAA", "BBB"]);
    assert_eq!(shapes(&mut d)[1], shape("right", true, false));
    d.undo().unwrap();
    assert_eq!(texts(&mut d), ["AAA", "", "BBB"]);
    assert_eq!(shapes(&mut d), before);

    // The lower paragraph's own break goes with it: the result stands where the upper was.
    tx(&mut d, json!([{"op":"pageBreak","at":at(1, 0),"on":false}]));
    tx(&mut d, json!([{"op":"pageBreak","at":at(2, 0),"on":true}]));
    tx(
        &mut d,
        json!([{"op":"delete","from":at(1, 0),"to":at(2, 1)}]),
    );
    assert_eq!(texts(&mut d), ["AAA", "BB"]);
    assert_eq!(shapes(&mut d)[1], shape("right", false, false));

    // With characters left in the upper paragraph, it keeps its own shape.
    let mut d = blank();
    build(&mut d, &[("AAA", "center"), ("BBB", "right")]);
    let (_, end) = ends(&mut d, 0);
    tx(
        &mut d,
        json!([{"op":"delete","from":at(0, end),"to":at(1, 0)}]),
    );
    assert_eq!(texts(&mut d), ["AAABBB"]);
    assert_eq!(shapes(&mut d), [shape("center", false, false)]);
}

#[test]
fn an_emptied_first_paragraph_hands_its_definitions_to_the_lower_one() {
    let mut d = blank();
    build(&mut d, &[("", "center"), ("BBB", "right")]);
    tx(
        &mut d,
        json!([{"op":"formatChars","from":at(1, 0),"to":at(1, 3),"patch":{"bold":true}}]),
    );
    let bold = formats(&mut d)[1]["runs"][0]["charShapeId"].clone();
    let (lead, _) = ends(&mut d, 0);
    tx(
        &mut d,
        json!([{"op":"delete","from":at(0, lead),"to":at(1, 0)}]),
    );
    assert_eq!(texts(&mut d), ["BBB"]);
    assert_eq!(shapes(&mut d), [shape("right", false, false)]);
    // The section and column definitions stay first and take the text's shape, as Hancom
    // saves them.
    assert!(lead > 0);
    assert_eq!(ends(&mut d, 0).0, lead);
    let runs = formats(&mut d)[0]["runs"].clone();
    assert_eq!(runs.as_array().unwrap().len(), 1, "{runs}");
    assert_eq!(runs[0]["charShapeId"], bold);
    for format in ["hwp5", "hwpx", "hml"] {
        let mut r = reopen(&mut d, format);
        assert_eq!(texts(&mut r), ["BBB"], "{format}");
        assert_eq!(shapes(&mut r), [shape("right", false, false)], "{format}");
    }
}

#[test]
fn column_breaks_split_only_inside_a_paragraph_and_clear_like_page_breaks() {
    // Hancom's BreakColumn (page cases col06-column-break, col08-colbreak-mid-empty).
    let mut d = blank();
    build(
        &mut d,
        &[
            ("Col A", "justify"),
            ("Col B", "justify"),
            ("", "justify"),
            ("Col D", "justify"),
        ],
    );
    let flags = |d: &mut Doc| -> Vec<bool> { shapes(d).iter().map(|s| s.2).collect() };
    let column = |d: &mut Doc, para: u32, offset: u32| {
        tx(d, json!([{"op":"columnBreak","at":at(para, offset)}]))["caret"].clone()
    };
    // At the start the paragraph itself starts the column.
    assert_eq!(column(&mut d, 1, 0), at(1, 0));
    assert_eq!(texts(&mut d), ["Col A", "Col B", "", "Col D"]);
    assert_eq!(flags(&mut d), [false, true, false, false]);
    d.undo().unwrap();
    // At the end the next one does, and the caret goes there.
    assert_eq!(column(&mut d, 1, 5), at(2, 0));
    assert_eq!(flags(&mut d), [false, false, true, false]);
    assert_eq!(texts(&mut d).len(), 4);
    d.undo().unwrap();
    // An empty paragraph is marked, not split.
    assert_eq!(column(&mut d, 2, 0), at(2, 0));
    assert_eq!(flags(&mut d), [false, false, true, false]);
    assert_eq!(texts(&mut d).len(), 4);
    d.undo().unwrap();
    // Only the middle of a paragraph, or the end of the last one, splits.
    assert_eq!(column(&mut d, 1, 2), at(2, 0));
    assert_eq!(texts(&mut d), ["Col A", "Co", "l B", "", "Col D"]);
    assert_eq!(flags(&mut d), [false, false, true, false, false]);
    d.undo().unwrap();
    assert_eq!(column(&mut d, 3, 5), at(4, 0));
    assert_eq!(texts(&mut d), ["Col A", "Col B", "", "Col D", ""]);
    assert_eq!(flags(&mut d), [false, false, false, false, true]);
    d.undo().unwrap();

    // A column break replaces a page break and both clear with `on:false`.
    tx(&mut d, json!([{"op":"pageBreak","at":at(1, 0),"on":true}]));
    column(&mut d, 1, 0);
    assert_eq!(shapes(&mut d)[1], shape("justify", false, true));
    for format in ["hwp5", "hwpx", "hml"] {
        let mut r = reopen(&mut d, format);
        assert_eq!(shapes(&mut r)[1], shape("justify", false, true), "{format}");
    }
    tx(
        &mut d,
        json!([{"op":"columnBreak","at":at(1, 0),"on":false}]),
    );
    assert_eq!(shapes(&mut d)[1], shape("justify", false, false));
    for format in ["hwp5", "hwpx", "hml"] {
        let mut r = reopen(&mut d, format);
        assert_eq!(
            shapes(&mut r)[1],
            shape("justify", false, false),
            "{format}"
        );
    }
    d.undo().unwrap();
    tx(&mut d, json!([{"op":"pageBreak","at":at(1, 0),"on":false}]));
    assert_eq!(shapes(&mut d)[1], shape("justify", false, false));
    d.undo().unwrap();
    assert_eq!(shapes(&mut d)[1], shape("justify", false, true));
}
