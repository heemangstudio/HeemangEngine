//! Paragraph shape patches as Hancom stores them (heemang-cleanroom
//! parity/notes/formatting-20261104/P).
use hm_wasm::{Doc, Engine};
use serde_json::{Value, json};

fn query(d: &mut Doc, q: Value) -> Value {
    serde_json::from_str(&d.query(&q.to_string()).unwrap()).unwrap()
}
fn tx(d: &mut Doc, ops: Value) {
    d.transact(&json!({ "ops": ops }).to_string()).unwrap();
}
fn document() -> Doc {
    let mut d = Engine::new()
        .create(include_bytes!("fixtures/hancom-table-4x4.hwpx").to_vec())
        .unwrap();
    let end = paragraph(&mut d, 0)["caret"][0].clone();
    tx(
        &mut d,
        json!([{"op":"insertText","at":{"section":0,"para":0,"offset":end},"text":"가나다"},
               {"op":"split","at":{"section":0,"para":0,"offset":end.as_u64().unwrap() + 3}}]),
    );
    d
}
fn paragraph(d: &mut Doc, para: u32) -> Value {
    query(
        d,
        json!({"kind":"paragraphs","list":{"section":0},"from":para,"count":1}),
    )[0]
    .clone()
}
/// The whole of paragraph `para` as a range.
fn whole(d: &mut Doc, para: u32) -> (Value, Value) {
    let p = paragraph(d, para);
    let offsets = p["offsets"].as_array().unwrap();
    let a = |o: &Value| json!({"section":0,"para":para,"offset":o});
    (a(&p["caret"][0]), a(offsets.last().unwrap()))
}
fn format(d: &mut Doc, para: u32, patch: Value) {
    let (from, to) = whole(d, para);
    tx(
        d,
        json!([{"op":"formatParas","from":from,"to":to,"patch":patch}]),
    );
}
fn paragraphs(d: &mut Doc, para: u32) -> Value {
    query(
        d,
        json!({"kind":"formatRuns","list":{"section":0},"from":para,"count":1}),
    )[0]["paragraphs"]
        .clone()
}
fn line(style: u32, width: u32, color: u32) -> Value {
    json!({"style":style,"width":width,"color":color})
}

/// Hancom's 문단 테두리/배경 (captures b8-para-border): each patch changes the paragraph's own
/// border fill, so a background added after a left line keeps the line, and two paragraphs given
/// the same border share one border fill.
#[test]
fn paragraph_border_patch_follows_each_paragraphs_own_border_fill() {
    let mut d = document();
    format(
        &mut d,
        0,
        json!({"borderFill":{"left":line(3, 4, 0xFF0000)}}),
    );
    format(
        &mut d,
        0,
        json!({"borderFill":{"fill":{"kind":"solid","background":0x00FFFF,"patternColor":0,"pattern":-1}}}),
    );
    let p = paragraphs(&mut d, 0);
    assert_eq!(p["borderFill"]["left"], line(3, 4, 0xFF0000));
    assert_eq!(p["borderFill"]["right"]["style"], 0);
    assert_eq!(p["borderFill"]["fill"]["background"], 0x00FFFF);

    format(
        &mut d,
        1,
        json!({"borderFill":{"left":line(3, 4, 0xFF0000)}}),
    );
    format(
        &mut d,
        1,
        json!({"borderFill":{"fill":{"kind":"solid","background":0x00FFFF,"patternColor":0,"pattern":-1}}}),
    );
    assert_eq!(
        paragraphs(&mut d, 1)["borderFillId"],
        p["borderFillId"],
        "equal borders share one border fill"
    );

    let mut again = Engine::new()
        .open(d.serialize_as("hwpx", false).unwrap())
        .unwrap();
    assert_eq!(paragraphs(&mut again, 0)["borderFill"], p["borderFill"]);
}

/// Hancom stores a paragraph taken out of its numbering or bullet list with no definition
/// (heading NONE, idRef 0), whether through the toolbar toggle or Backspace at its start
/// (captures b4-lists nb-*-off, nb-bs-start-*).
#[test]
fn a_paragraph_taken_out_of_its_list_names_no_definition() {
    let mut d = document();
    tx(
        &mut d,
        json!([{"op":"ensureList","definition":{"kind":"bullet","bullet":"\u{f06c}"}}]),
    );
    let (from, to) = whole(&mut d, 0);
    tx(
        &mut d,
        json!([{"op":"setList","from":from,"to":to,"list":{"kind":"bullet","id":1,"level":0}}]),
    );
    assert_eq!(paragraphs(&mut d, 0)["heading"]["id"], 1);
    tx(
        &mut d,
        json!([{"op":"setList","from":from,"to":to,"list":null}]),
    );
    assert_eq!(
        paragraphs(&mut d, 0)["heading"],
        json!({"kind":"none","id":0,"level":0})
    );
    assert_eq!(
        paragraphs(&mut d, 0)["borderFillId"],
        paragraphs(&mut d, 1)["borderFillId"]
    );
    assert_eq!(
        query(&mut d, json!({"kind":"formatRuns","list":{"section":0}}))[0]["paraShapeId"],
        query(&mut d, json!({"kind":"formatRuns","list":{"section":0}}))[1]["paraShapeId"],
        "the plain paragraph's shape again"
    );
}

/// Hancom's 문단 번호 on a document whose only numbering is the section's outline numbering
/// makes a second, separate definition even though the outline one has the same levels
/// (captures b4-lists nb-number: idRef 2).
#[test]
fn a_paragraph_numbering_is_never_the_outline_numbering() {
    let mut d = document();
    let outline = query(&mut d, json!({"kind":"sectionSettings","section":0}))["settings"]
        ["outlineNumbering"]
        .as_u64()
        .unwrap();
    let count = |d: &mut Doc| {
        query(d, json!({"kind":"lists"}))
            .as_array()
            .unwrap()
            .iter()
            .filter(|l| l["kind"] == "numbering")
            .count()
    };
    let before = count(&mut d);
    let levels: Vec<Value> = [
        ("^1.", 0),
        ("^2.", 8),
        ("^3)", 0),
        ("^4)", 8),
        ("(^5)", 0),
        ("(^6)", 8),
        ("^7", 1),
    ]
    .iter()
    .map(|(f, k)| json!({"format":f,"numberKind":k,"start":1}))
    .collect();
    let ensure =
        json!([{"op":"ensureList","definition":{"kind":"numbering","start":0,"levels":levels}}]);
    tx(&mut d, ensure.clone());
    assert_eq!(count(&mut d), before + 1);
    assert_ne!(before as u64 + 1, outline);
    tx(&mut d, ensure);
    assert_eq!(
        count(&mut d),
        before + 1,
        "the paragraph numbering is reused"
    );
}
