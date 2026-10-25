//! Opening and saving HWPML uses the same edit/history contract as binary HWP.
use hm_wasm::{Doc, Engine};
use serde_json::{Value, json};
fn query(doc: &mut Doc) -> Value {
    serde_json::from_str(
        &doc.query(&json!({"kind":"paragraphs","list":{"section":0}}).to_string())
            .unwrap(),
    )
    .unwrap()
}
#[test]
fn hml_edits_undo_and_save_without_changing_the_open_format() {
    let engine = Engine::new();
    let mut doc = engine
        .open(include_bytes!("../../hm-hml/tests/fixtures/formatting_table.hml").to_vec())
        .unwrap();
    let before = query(&mut doc);
    let wire = before[0]["caret"][0].as_u64().unwrap();
    doc.transact(&json!({"ops":[{"op":"insertText","at":{"section":0,"para":0,"offset":wire},"text":"수정😀"}]}).to_string()).unwrap();
    let expected = query(&mut doc);
    assert_ne!(expected[0]["text"], before[0]["text"]);
    doc.undo().unwrap();
    assert_eq!(query(&mut doc)[0]["text"], before[0]["text"]);
    doc.redo().unwrap();
    for format in ["hml", "hwp5", "hwpx"] {
        let bytes = doc.serialize_as(format, false).unwrap();
        let mut reopened = engine.open(bytes).unwrap();
        assert_eq!(query(&mut reopened)[0]["text"], expected[0]["text"]);
        let state: Value = serde_json::from_str(&doc.state().unwrap()).unwrap();
        assert_eq!(state["format"], "hml");
        assert_eq!(state["dirty"], true);
    }
    let saved = doc.serialize(true).unwrap();
    let mut reopened = engine.open(saved).unwrap();
    assert_eq!(query(&mut reopened)[0]["text"], expected[0]["text"]);
    let state: Value = serde_json::from_str(&doc.state().unwrap()).unwrap();
    assert_eq!(state["dirty"], false);
}
#[test]
fn both_paired_formats_export_hml_with_their_equations() {
    fn scripts(paras: &Value) -> Vec<String> {
        paras
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|p| p["controls"].as_array().unwrap())
            .filter_map(|c| c["script"].as_str().map(str::to_owned))
            .collect()
    }
    let engine = Engine::new();
    for bytes in [
        include_bytes!("fixtures/query-equations.hwp").as_slice(),
        include_bytes!("fixtures/query-equations.hwpx"),
    ] {
        let mut doc = engine.open(bytes.to_vec()).unwrap();
        let expected = query(&mut doc);
        let expected_scripts = scripts(&expected);
        assert!(!expected_scripts.is_empty());
        let mut reopened = engine
            .open(doc.serialize_as("hml", false).unwrap())
            .unwrap();
        let actual = query(&mut reopened);
        assert_eq!(actual[0]["text"], expected[0]["text"]);
        assert_eq!(scripts(&actual), expected_scripts);
    }
}
