//! Taking another document's content keeps checkpoints of the one before.
use hm_wasm::{Doc, Engine};
use serde_json::{Value, json};

fn query(doc: &mut Doc, value: Value) -> Value {
    serde_json::from_str(&doc.query(&value.to_string()).unwrap()).unwrap()
}
fn text(doc: &mut Doc) -> String {
    query(doc, json!({"kind":"paragraphs","list":{"section":0}}))
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["text"].as_str().unwrap().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}
fn state(doc: &mut Doc) -> Value {
    serde_json::from_str(&doc.state().unwrap()).unwrap()
}
fn written(engine: &Engine, source: &[u8], words: &str) -> Vec<u8> {
    let mut doc = engine.open(source.to_vec()).unwrap();
    let p = query(
        &mut doc,
        json!({"kind":"paragraphs","list":{"section":0},"count":1}),
    );
    let at = json!({"section":0,"para":0,"offset":p[0]["caret"][0]});
    doc.transact(&json!({"ops":[{"op":"insertText","at":at,"text":words}]}).to_string())
        .unwrap();
    doc.serialize(false).unwrap()
}

#[test]
fn adopted_content_replaces_the_document_and_checkpoints_restore_the_old_one() {
    for source in [
        &include_bytes!("fixtures/query-equations.hwp")[..],
        &include_bytes!("fixtures/query-equations.hwpx")[..],
    ] {
        let engine = Engine::new();
        let mut doc = engine.open(written(&engine, source, "지금 문서")).unwrap();
        let mut earlier = engine.open(written(&engine, source, "지난 버전")).unwrap();
        let (now, then) = (text(&mut doc), text(&mut earlier));
        assert_ne!(now, then);
        let before = doc.checkpoint().unwrap();
        let epoch = state(&mut doc)["contentEpoch"].as_u64().unwrap();
        doc.adopt(engine.open(written(&engine, source, "지난 버전")).unwrap())
            .unwrap();
        assert_eq!(text(&mut doc), then);
        let after = state(&mut doc);
        assert!(after["contentEpoch"].as_u64().unwrap() > epoch);
        assert_eq!(
            after["dirty"], true,
            "adopted content is not what was saved"
        );
        assert_eq!(after["undo"], 0);
        let checkpoint = doc.checkpoint().unwrap();
        doc.restore(before).unwrap();
        assert_eq!(text(&mut doc), now, "a checkpoint from before comes back");
        doc.restore(checkpoint).unwrap();
        assert_eq!(text(&mut doc), then);
        let mut reopened = engine.open(doc.serialize(false).unwrap()).unwrap();
        assert_eq!(text(&mut reopened), then);
    }
}
