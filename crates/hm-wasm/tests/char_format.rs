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
