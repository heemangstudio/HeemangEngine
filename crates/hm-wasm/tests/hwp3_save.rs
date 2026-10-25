//! Legacy HWP 3 opens editable and saves through the existing modern writers.
use hm_wasm::{Doc, Engine};
use serde_json::{Value, json};

fn state(doc: &mut Doc) -> Value {
    serde_json::from_str(&doc.state().unwrap()).unwrap()
}
fn paragraphs(doc: &mut Doc) -> Value {
    fn list(doc: &mut Doc, steps: Vec<Value>) -> Value {
        let mut paras: Value = serde_json::from_str(
            &doc.query(
                &json!({"kind":"paragraphs","list":{"section":0,"steps":steps},"count":100000})
                    .to_string(),
            )
            .unwrap(),
        )
        .unwrap();
        for p in paras.as_array_mut().unwrap() {
            let para = p["index"].clone();
            for c in p["controls"].as_array_mut().unwrap() {
                let mut children = Vec::new();
                for index in 0..c["lists"].as_u64().unwrap() {
                    let mut path = steps.clone();
                    path.push(json!({"para":para,"control":c["index"],"index":index}));
                    children.push(list(doc, path));
                }
                c["children"] = json!(children);
            }
        }
        paras
    }
    list(doc, Vec::new())
}

#[test]
fn hwp3_edits_save_as_hwp5_and_hwpx_without_marking_unsaved_output_clean() {
    for bytes in [
        include_bytes!("fixtures/hwp3-tabs.hwp").as_slice(),
        include_bytes!("fixtures/hwp3-objects.hwp").as_slice(),
        include_bytes!("fixtures/hwp3-furniture.hwp").as_slice(),
    ] {
        let engine = Engine::new();
        let mut doc = engine.open(bytes.to_vec()).unwrap();
        assert_eq!(state(&mut doc)["format"], "hwp3");
        let original = paragraphs(&mut doc);
        let at = json!({"section":0,"para":0,"offset":original[0]["caret"][0]});
        doc.transact(
            &json!({"ops":[{"op":"insertText","at":at,"text":"Saved 한글 😀 "}]}).to_string(),
        )
        .unwrap();
        let edited = paragraphs(&mut doc);
        assert_ne!(edited, original);
        let before = state(&mut doc);
        assert_eq!(before["dirty"], true);
        let hwp = doc.serialize(false).unwrap();
        assert!(hm_hwp5::is_hwp5(&hwp));
        assert_eq!(doc.serialize_as("hwp5", false).unwrap(), hwp);
        let hwpx = doc.serialize_as("hwpx", false).unwrap();
        assert!(hm_hwpx::is_hwpx(&hwpx));
        assert_eq!(
            state(&mut doc),
            before,
            "export is not a completed file write"
        );
        assert!(doc.serialize_as("hwp3", true).is_err());
        assert_eq!(
            state(&mut doc),
            before,
            "failed serialization preserves dirty/history"
        );
        for output in [hwp, hwpx] {
            let mut reopened = engine.open(output).unwrap();
            let back = paragraphs(&mut reopened);
            assert_eq!(state(&mut reopened)["pageCount"], before["pageCount"]);
            assert_eq!(
                back.as_array().unwrap().len(),
                edited.as_array().unwrap().len()
            );
            for (was, now) in edited
                .as_array()
                .unwrap()
                .iter()
                .zip(back.as_array().unwrap())
            {
                assert_eq!(now["text"], was["text"]);
                assert_eq!(now["controls"], was["controls"]);
            }
        }
        doc.undo().unwrap();
        assert_eq!(paragraphs(&mut doc)[0]["text"], original[0]["text"]);
        doc.redo().unwrap();
        assert_eq!(paragraphs(&mut doc)[0]["text"], edited[0]["text"]);
        doc.serialize_as("hwpx", true).unwrap();
        assert_eq!(state(&mut doc)["dirty"], false);
        assert_eq!(
            state(&mut doc)["format"],
            "hwp3",
            "conversion leaves the open source intact"
        );
    }
}
