//! Character formatting that Hancom's dialog offers reads, applies and saves as Hancom does.
use hm_wasm::{Doc, Engine};
use serde_json::{Value, json};

fn q(doc: &mut Doc, value: Value) -> Value {
    serde_json::from_str(&doc.query(&value.to_string()).unwrap()).unwrap()
}
fn tx(doc: &mut Doc, ops: Value) -> Value {
    serde_json::from_str(&doc.transact(&json!({"ops":ops}).to_string()).unwrap()).unwrap()
}
fn at(offset: u64) -> Value {
    json!({"section":0,"para":0,"offset":offset})
}

#[test]
fn centered_underline_is_its_own_kind_and_a_strikeout_does_not_add_one() {
    let engine = Engine::new();
    let mut doc = engine
        .open(include_bytes!("fixtures/query-equations.hwpx").to_vec())
        .unwrap();
    let p = q(
        &mut doc,
        json!({"kind":"paragraphs","list":{"section":0},"count":1}),
    );
    let s = p[0]["length"].as_u64().unwrap() - 1;
    tx(
        &mut doc,
        json!([{"op":"insertText","at":at(s),"text":"abc def"}]),
    );
    tx(
        &mut doc,
        json!([
            {"op":"formatChars","from":at(s),"to":at(s+3),"patch":{"underlineKind":"center"}},
            {"op":"formatChars","from":at(s+4),"to":at(s+7),"patch":{"strike":true}}
        ]),
    );
    let kind = |doc: &mut Doc, o: u64| {
        q(doc, json!({"kind":"formatContext","at":at(o),"to":at(o+1)}))["characters"]
            ["underlineKind"]
            .clone()
    };
    assert_eq!(kind(&mut doc, s), "center");
    assert_eq!(kind(&mut doc, s + 4), "none");
    let saved = doc.serialize(false).unwrap();
    let header = String::from_utf8_lossy(&unzip(&saved, "Contents/header.xml")).into_owned();
    assert_eq!(header.matches(r#"<hh:underline type="CENTER""#).count(), 1);
    assert_eq!(header.matches(r#"<hh:strikeout shape="SOLID""#).count(), 1);
    let mut reopened = engine.open(saved).unwrap();
    assert_eq!(kind(&mut reopened, s), "center");
    assert_eq!(kind(&mut reopened, s + 4), "none");
    // Turning the underline off removes a centered one; it no longer belongs to a strikeout.
    tx(
        &mut doc,
        json!([{"op":"formatChars","from":at(s),"to":at(s+3),"patch":{"underline":false}}]),
    );
    assert_eq!(kind(&mut doc, s), "none");
}

/// One member of a saved HWPX package.
fn unzip(zip: &[u8], name: &str) -> Vec<u8> {
    hm_hwpx::zip::Archive::open(zip.to_vec().into())
        .unwrap()
        .read_name(name)
        .unwrap()
        .unwrap()
        .to_vec()
}

fn blank() -> (Doc, u64) {
    let engine = Engine::new();
    let mut doc = engine
        .open(include_bytes!("fixtures/query-equations.hwpx").to_vec())
        .unwrap();
    let p = q(
        &mut doc,
        json!({"kind":"paragraphs","list":{"section":0},"count":1}),
    );
    let s = p[0]["length"].as_u64().unwrap() - 1;
    tx(
        &mut doc,
        json!([{"op":"insertText","at":at(s),"text":"abc def"}]),
    );
    (doc, s)
}

#[test]
fn a_border_or_background_change_keeps_the_other_half() {
    let (mut doc, s) = blank();
    let line = json!({"style":1,"width":1,"color":255});
    tx(
        &mut doc,
        json!([{"op":"formatChars","from":at(s),"to":at(s+3),"patch":{"borderFill":{"left":line,"right":line,"top":line,"bottom":line}}}]),
    );
    tx(
        &mut doc,
        json!([{"op":"formatChars","from":at(s),"to":at(s+3),"patch":{"borderFill":{"fill":{"kind":"solid","background":65280}}}}]),
    );
    let c = q(
        &mut doc,
        json!({"kind":"formatContext","at":at(s),"to":at(s+3)}),
    )["characters"]["borderFill"]
        .clone();
    assert_eq!(c["left"]["color"], 255, "{c}");
    assert_eq!(c["fill"]["background"], 65280, "{c}");
}

#[test]
fn faces_sent_back_unchanged_need_no_registered_family() {
    // No fonts are registered here: the document's own faces stay valid, a new name does not.
    let (mut doc, s) = blank();
    let mut faces =
        q(&mut doc, json!({"kind":"formatContext","at":at(s)}))["characters"]["fontFaces"].clone();
    faces[1] = faces[0].clone();
    let ok = doc.transact(
        &json!({"ops":[{"op":"formatChars","from":at(s),"to":at(s+3),"patch":{"fontFaces":faces}}]})
            .to_string(),
    );
    assert!(ok.is_ok(), "{ok:?}");
    let bad = doc.transact(
        &json!({"ops":[{"op":"formatChars","from":at(s),"to":at(s+3),"patch":{"fontFaces":[null,"No Such Font"]}}]})
            .to_string(),
    );
    assert!(bad.is_err());
}

#[test]
fn font_spacing_mixed_state_and_size_bounds() {
    let (mut doc, s) = blank();
    tx(
        &mut doc,
        json!([{"op":"formatChars","from":at(s),"to":at(s+3),"patch":{"useFontSpace":true,"emboss":true}}]),
    );
    let one = q(
        &mut doc,
        json!({"kind":"formatContext","at":at(s),"to":at(s+3)}),
    );
    assert_eq!(one["characters"]["useFontSpace"], true);
    let both = q(
        &mut doc,
        json!({"kind":"formatContext","at":at(s),"to":at(s+7)}),
    );
    let mixed: Vec<&str> = both["mixed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(mixed.contains(&"characters.useFontSpace"), "{mixed:?}");
    assert!(mixed.contains(&"characters.emboss"), "{mixed:?}");
    let header = String::from_utf8_lossy(&unzip(
        &doc.serialize(false).unwrap(),
        "Contents/header.xml",
    ))
    .into_owned();
    assert!(header.contains(r#"useFontSpace="1""#));
    for size in [0, -100, 409_700] {
        let r = doc.transact(
            &json!({"ops":[{"op":"formatChars","from":at(s),"to":at(s+3),"patch":{"size":size}}]})
                .to_string(),
        );
        assert!(r.is_err(), "size {size}");
    }
}

#[test]
fn formatting_to_the_last_character_takes_the_paragraph_end_mark() {
    // Hancom's selection to a paragraph's end formats its end mark too, which sizes an
    // empty last line and the paragraph head.
    let (mut doc, s) = blank();
    tx(
        &mut doc,
        json!([{"op":"formatChars","from":at(s+4),"to":at(s+7),"patch":{"size":2400}}]),
    );
    let p = q(
        &mut doc,
        json!({"kind":"formatRuns","list":{"section":0},"count":1}),
    )[0]
    .clone();
    let runs = p["runs"].as_array().unwrap();
    let last = runs.last().unwrap();
    assert_eq!(last["offset"], s + 4, "{p}");
    // A range that stops short of the last character leaves the end mark alone.
    tx(
        &mut doc,
        json!([{"op":"formatChars","from":at(s),"to":at(s+2),"patch":{"bold":true}}]),
    );
    let p = q(
        &mut doc,
        json!({"kind":"formatRuns","list":{"section":0},"count":1}),
    )[0]
    .clone();
    assert_eq!(
        p["runs"].as_array().unwrap().last().unwrap()["offset"],
        s + 4
    );
}
