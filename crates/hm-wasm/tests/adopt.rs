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

#[test]
fn a_history_limit_keeps_the_newest_steps_and_zero_turns_history_off() {
    let engine = Engine::new();
    let mut doc = engine
        .open(include_bytes!("fixtures/query-equations.hwpx").to_vec())
        .unwrap();
    let typed = |doc: &mut Doc, n: usize| {
        for k in 0..n {
            let p = query(
                doc,
                json!({"kind":"paragraphs","list":{"section":0},"count":1}),
            );
            let at = json!({"section":0,"para":0,"offset":p[0]["caret"][1]});
            doc.transact(
                &json!({"ops":[{"op":"insertText","at":at,"text":k.to_string()}]}).to_string(),
            )
            .unwrap();
        }
    };
    let saved = |doc: &mut Doc| {
        let epoch = state(doc)["contentEpoch"].as_u64().unwrap() as u32;
        doc.mark_saved(epoch).unwrap();
    };
    saved(&mut doc);
    doc.set_history_limit(Some(2)).unwrap();
    typed(&mut doc, 3);
    let three = text(&mut doc);
    typed(&mut doc, 2);
    assert_eq!(state(&mut doc)["undo"], 2);
    assert_ne!(doc.undo().unwrap(), "null");
    assert_ne!(doc.undo().unwrap(), "null");
    assert_eq!(doc.undo().unwrap(), "null", "older steps are gone");
    assert_eq!(text(&mut doc), three);
    // What the dropped steps typed is still unsaved.
    assert_eq!(state(&mut doc)["dirty"], true);
    assert_ne!(doc.redo().unwrap(), "null");
    // Off: edits, undo and checkpoints keep no history, and a restore
    // brings back the document.
    doc.set_history_limit(Some(0)).unwrap();
    typed(&mut doc, 3);
    let s = state(&mut doc);
    assert_eq!((s["undo"].clone(), s["redo"].clone()), (json!(0), json!(0)));
    let kept = text(&mut doc);
    let checkpoint = doc.checkpoint().unwrap();
    saved(&mut doc);
    assert_eq!(doc.undo().unwrap(), "null");
    typed(&mut doc, 1);
    assert_eq!(state(&mut doc)["dirty"], true);
    doc.restore(checkpoint).unwrap();
    assert_eq!(text(&mut doc), kept);
    // Back where it was saved.
    assert_eq!(state(&mut doc)["dirty"], false);
    // Back on: new edits undo again.
    doc.set_history_limit(None).unwrap();
    typed(&mut doc, 1);
    assert_ne!(doc.undo().unwrap(), "null");
    assert_eq!(text(&mut doc), kept);
}
