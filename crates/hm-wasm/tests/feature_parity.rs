//! Conditional Studio features retain their document state and undo contract.
use hm_wasm::{Doc, Engine};
use serde_json::{Value, json};
const SOURCE: &[u8] = include_bytes!("fixtures/query-equations.hwpx");
fn q(doc: &mut Doc, request: Value) -> Value {
    serde_json::from_str(&doc.query(&request.to_string()).unwrap()).unwrap()
}
fn tx(doc: &mut Doc, ops: Value) -> Value {
    serde_json::from_str(&doc.transact(&json!({"ops":ops}).to_string()).unwrap()).unwrap()
}
fn at(doc: &mut Doc, section: u32) -> Value {
    let p = q(
        doc,
        json!({"kind":"paragraphs","list":{"section":section},"count":1}),
    );
    json!({"section":section,"para":0,"offset":p[0]["caret"][0]})
}
#[test]
fn character_borders_outline_and_caption_variants_roundtrip() {
    for format in ["hwp5", "hwpx"] {
        let engine = Engine::new();
        let mut doc = engine.create(SOURCE.to_vec()).unwrap();
        let a = at(&mut doc, 0);
        let end = tx(&mut doc, json!([{"op":"insertText","at":a,"text":"Border","format":{"borderFill":{"left":{"style":1,"width":3,"color":255},"fill":{"kind":"solid","background":65535,"pattern":-1}}}}]))["caret"].clone();
        tx(
            &mut doc,
            json!([{"op":"setList","from":a,"to":end,"list":{"kind":"outline","id":0,"level":2}}]),
        );
        let expected = q(&mut doc, json!({"kind":"formatContext","at":end}));
        assert_eq!(expected["characters"]["borderFill"]["left"]["color"], 255);
        assert_eq!(expected["list"]["kind"], "outline");
        let mut reopened = engine
            .open(doc.serialize_as(format, false).unwrap())
            .unwrap();
        let value = q(&mut reopened, json!({"kind":"formatContext","at":end}));
        assert_eq!(
            value["characters"]["borderFill"],
            expected["characters"]["borderFill"]
        );
        assert_eq!(value["list"], expected["list"]);
        tx(
            &mut doc,
            json!([{"op":"createShape","at":end,"kind":"rect","width":3000,"height":4000}]),
        );
        let p = q(
            &mut doc,
            json!({"kind":"paragraphs","list":{"section":0},"count":1}),
        );
        let c = p[0]["controls"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["offset"] == end["offset"])
            .unwrap()["index"]
            .clone();
        let object = json!({"section":0,"para":0,"control":c});
        tx(
            &mut doc,
            json!([{"op":"setObjectCaption","object":object,"text":" Caption","withNumber":true},{"op":"setObjectProperties","object":object,"patch":{"caption":{"enabled":true,"side":"left","verticalAlign":"bottom","width":1800,"gap":200,"includeMargin":true}}}]),
        );
        let mut reopened = engine
            .open(doc.serialize_as(format, false).unwrap())
            .unwrap();
        let value = q(
            &mut reopened,
            json!({"kind":"objectProperties","object":object}),
        );
        assert_eq!(value["caption"]["side"], "left");
        assert_eq!(value["caption"]["verticalAlign"], "bottom");
        assert_eq!(value["caption"]["gap"], 200);
        tx(
            &mut doc,
            json!([{"op":"setObjectProperties","object":object,"patch":{"caption":{"enabled":false}}}]),
        );
        assert!(
            q(&mut doc, json!({"kind":"objectProperties","object":object}))["caption"].is_null()
        );
        doc.undo().unwrap();
        assert_eq!(
            q(&mut doc, json!({"kind":"objectProperties","object":object}))["caption"]["side"],
            "left"
        );
    }
}
#[test]
fn cross_section_copy_and_delete_keep_sections_and_undo_atomically() {
    let mut model = hm_hwpx::read(SOURCE.to_vec().into()).unwrap();
    model.sections.push(model.sections[0].clone());
    model.source = None;
    model.info.properties.section_count = 2;
    model.info.properties.src = None;
    let engine = Engine::new();
    let mut doc = engine.open(hm_hwpx::write_new(&model).unwrap()).unwrap();
    let a = at(&mut doc, 0);
    let b = at(&mut doc, 1);
    tx(
        &mut doc,
        json!([{"op":"insertText","at":a,"text":"First"},{"op":"insertText","at":b,"text":"Last"}]),
    );
    let mut from = a.clone();
    from["offset"] = json!(a["offset"].as_u64().unwrap() + 2);
    let mut to = b.clone();
    to["offset"] = json!(b["offset"].as_u64().unwrap() + 2);
    let copied: Value =
        serde_json::from_str(&doc.copy(&json!({"from":from,"to":to}).to_string()).unwrap())
            .unwrap();
    assert!(copied["text"].as_str().unwrap().starts_with("rst"));
    assert!(copied["text"].as_str().unwrap().ends_with("La"));
    tx(&mut doc, json!([{"op":"delete","from":from,"to":to}]));
    assert_eq!(
        q(&mut doc, json!({"kind":"documentProperties"}))["sectionCount"],
        2
    );
    assert_eq!(
        q(
            &mut doc,
            json!({"kind":"paragraphs","list":{"section":0},"count":1})
        )[0]["text"],
        "Fi"
    );
    assert!(
        q(
            &mut doc,
            json!({"kind":"paragraphs","list":{"section":1},"count":1})
        )[0]["text"]
            .as_str()
            .unwrap()
            .starts_with("st")
    );
    doc.undo().unwrap();
    assert!(
        q(
            &mut doc,
            json!({"kind":"paragraphs","list":{"section":0},"count":1})
        )[0]["text"]
            .as_str()
            .unwrap()
            .starts_with("First")
    );
}

#[test]
fn clipboard_objects_import_referenced_resources_after_source_closure() {
    let engine = Engine::new();
    let mut source = engine.create(SOURCE.to_vec()).unwrap();
    let a = at(&mut source, 0);
    tx(
        &mut source,
        json!([{"op":"createTable","at":a,"rows":1,"columns":1}]),
    );
    let controls = q(
        &mut source,
        json!({"kind":"paragraphs","list":{"section":0},"count":1}),
    );
    let c = controls[0]["controls"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["kind"] == "table")
        .unwrap()["index"]
        .clone();
    let cell = json!({"section":0,"steps":[{"para":0,"control":c,"index":0}],"para":0,"offset":0});
    tx(
        &mut source,
        json!([{"op":"insertText","at":cell,"text":"Portable table","format":{"bold":true,"borderFill":{"fill":{"kind":"solid","background":255}}}}]),
    );
    let mut end = a.clone();
    end["offset"] = json!(a["offset"].as_u64().unwrap() + 8);
    source
        .copy(&json!({"from":a,"to":end}).to_string())
        .unwrap();
    source.close();
    let mut target = engine.create(SOURCE.to_vec()).unwrap();
    let dest = at(&mut target, 0);
    target.paste(&json!({"at":dest}).to_string()).unwrap();
    for format in ["hwp5", "hwpx"] {
        let mut reopened = engine
            .open(target.serialize_as(format, false).unwrap())
            .unwrap();
        let p = q(
            &mut reopened,
            json!({"kind":"paragraphs","list":{"section":0,"steps":[{"para":0,"control":c,"index":0}]},"count":1}),
        );
        assert_eq!(p[0]["text"], "Portable table");
        let props = q(&mut reopened, json!({"kind":"formatContext","at":cell}));
        assert_eq!(props["characters"]["bold"], true);
        assert_eq!(props["characters"]["borderFill"]["fill"]["background"], 255);
    }
    target.undo().unwrap();
    assert!(
        q(
            &mut target,
            json!({"kind":"paragraphs","list":{"section":0},"count":1})
        )[0]["controls"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["kind"] != "table")
    );
}
