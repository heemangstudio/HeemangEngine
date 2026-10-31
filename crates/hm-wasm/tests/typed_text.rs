//! Text typed, pasted or sent by a tool saves as text Hancom can open.
use serde_json::{Value, json};

/// An A4 page with ordinary margins.
const TEMPLATE: &[u8] = include_bytes!("fixtures/last-page-master.hwpx");

fn query(d: &mut hm_wasm::Doc, q: Value) -> Value {
    serde_json::from_str(&d.query(&q.to_string()).unwrap()).unwrap()
}
fn first(d: &mut hm_wasm::Doc) -> Value {
    query(
        d,
        json!({"kind":"paragraphs","list":{"section":0},"from":0,"count":1}),
    )[0]
    .clone()
}
/// A blank document and the address its text starts at.
fn blank(engine: &hm_wasm::Engine) -> (hm_wasm::Doc, Value) {
    let mut d = engine.create(TEMPLATE.to_vec()).unwrap();
    let lead = first(&mut d)["caret"][0].clone();
    (d, json!({"section":0,"para":0,"offset":lead}))
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

/// U+FFFE and U+FFFF are not XML characters: Hancom refuses a section
/// holding them. Typed or pasted, they become U+FFFD; a bookmark name
/// holding one is written with U+FFFD in its place.
#[test]
fn characters_xml_cannot_hold_do_not_reach_the_file() {
    let engine = hm_wasm::Engine::new();
    let (mut d, at) = blank(&engine);
    let request = json!({"ops":[
        {"op":"insertText","at":at,"text":"a\u{fffe}b\u{ffff}c"},
        {"op":"insertBookmark","at":at,"name":"m\u{ffff}k"}
    ]});
    d.transact(&request.to_string()).unwrap();
    assert_eq!(first(&mut d)["text"], "a\u{fffd}b\u{fffd}c");
    let saved = d.serialize_as("hwpx", false).unwrap();
    let section = unzip(&saved, "Contents/section0.xml");
    let xml = String::from_utf8(section).unwrap();
    assert!(!xml.contains(['\u{fffe}', '\u{ffff}']));
    assert!(xml.contains("name=\"m\u{fffd}k\""), "{xml}");
    let mut reopened = engine.open(saved).unwrap();
    assert_eq!(first(&mut reopened)["text"], "a\u{fffd}b\u{fffd}c");
}

/// `JSON.stringify` writes half of a surrogate pair as an escape; the text
/// goes in with U+FFFD for it instead of the whole paste failing.
#[test]
fn a_lone_surrogate_escape_inserts_a_replacement_character() {
    let engine = hm_wasm::Engine::new();
    let (mut d, at) = blank(&engine);
    let at = at.to_string();
    for (escaped, text) in [
        (r#""a\ud800b""#, "a\u{fffd}b"),
        (r#""\udc00😀""#, "\u{fffd}\u{1f600}"),
        (r#""x\\ud800""#, "x\\ud800"),
    ] {
        let request = format!(r#"{{"ops":[{{"op":"insertText","at":{at},"text":{escaped}}}]}}"#);
        d.transact(&request).unwrap();
        assert!(
            first(&mut d)["text"].as_str().unwrap().starts_with(text),
            "{escaped}"
        );
        d.undo().unwrap();
    }
}

/// U+00AD goes in as the soft hyphen Hancom's 하이픈 inserts (`hp:hypen`),
/// reads back as U+00AD and saves in every format.
#[test]
fn a_soft_hyphen_goes_in_as_hancoms_hyphen() {
    let engine = hm_wasm::Engine::new();
    let (mut d, at) = blank(&engine);
    let request = json!({"ops":[{"op":"insertText","at":at,"text":"co\u{ad}op"}]});
    d.transact(&request.to_string()).unwrap();
    let p = first(&mut d);
    assert_eq!(p["text"], "co\u{ad}op");
    assert_eq!(p["offsets"].as_array().unwrap().len(), 6, "{p}");
    let hwpx = d.serialize_as("hwpx", false).unwrap();
    let xml = String::from_utf8(unzip(&hwpx, "Contents/section0.xml")).unwrap();
    assert!(xml.contains("<hp:hypen/>"), "{xml}");
    for format in ["hwpx", "hwp5", "hml"] {
        let mut reopened = engine.open(d.serialize_as(format, false).unwrap()).unwrap();
        assert_eq!(first(&mut reopened)["text"], "co\u{ad}op", "{format}");
    }
}
