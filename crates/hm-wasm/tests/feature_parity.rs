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
    let png = vec![
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 4,
        0, 0, 0, 181, 28, 12, 2, 0, 0, 0, 11, 73, 68, 65, 84, 120, 218, 99, 252, 255, 31, 0, 3, 3,
        2, 0, 239, 191, 107, 153, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ];
    let mut image_at = cell.clone();
    image_at["offset"] = json!(14);
    tx(
        &mut source,
        json!([{"op":"insertPicture","at":image_at,"bytes":png,"extension":"png","width":3000,"height":3000}]),
    );
    let mut end = a.clone();
    end["offset"] = json!(a["offset"].as_u64().unwrap() + 8);
    let copied: Value = serde_json::from_str(
        &source
            .copy(&json!({"from":a,"to":end}).to_string())
            .unwrap(),
    )
    .unwrap();
    assert!(copied["html"].as_str().unwrap().contains("<table>"));
    assert!(copied["html"].as_str().unwrap().contains("Portable"));
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
        let picture =
            json!({"section":0,"steps":[{"para":0,"control":c,"index":0}],"para":0,"control":0});
        assert_eq!(
            q(
                &mut reopened,
                json!({"kind":"pictureData","object":picture})
            )["bytes"],
            json!(png)
        );
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

#[test]
fn shape_variants_cell_names_and_native_furniture_fields_roundtrip() {
    let engine = Engine::new();
    let mut doc = engine.create(SOURCE.to_vec()).unwrap();
    let a = at(&mut doc, 0);
    tx(
        &mut doc,
        json!([{"op":"createShape","at":a,"kind":"textbox","width":6000,"height":4000,"text":"Shape"}]),
    );
    let object = json!({"section":0,"para":0,"control":2});
    let p = q(
        &mut doc,
        json!({"kind":"paragraphs","list":{"section":0},"count":1}),
    );
    let c = p[0]["controls"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["offset"] == a["offset"])
        .unwrap()["index"]
        .clone();
    let mut object = object;
    object["control"] = c;
    tx(
        &mut doc,
        json!([{"op":"setObjectProperties","object":object,"patch":{"fill":{"kind":"gradient","gradientType":2,"angle":45,"centerX":35,"centerY":65,"step":20},"textMargins":[100,200,300,400],"textVerticalAlign":"bottom","round":20,"border":{"style":2,"head":1,"tail":2,"headSize":4,"tailSize":5},"shadow":{"kind":1,"color":255,"x":100,"y":200}}}]),
    );
    for format in ["hwp5", "hwpx"] {
        let mut r = engine
            .open(doc.serialize_as(format, false).unwrap())
            .unwrap();
        let props = q(&mut r, json!({"kind":"objectProperties","object":object}));
        assert_eq!(props["fill"]["gradient"]["angle"], 45);
        assert_eq!(props["textMargins"], json!([100, 200, 300, 400]));
        assert_eq!(props["textVerticalAlign"], "bottom");
        assert_eq!(props["round"], 20);
        assert_eq!(props["border"]["head"], 1);
        assert_eq!(props["border"]["tailSize"], 5);
        assert_eq!(props["shadow"]["color"], 255);
    }
    doc.undo().unwrap();
    doc.undo().unwrap();
    tx(
        &mut doc,
        json!([{"op":"createTable","at":a,"rows":1,"columns":1}]),
    );
    tx(
        &mut doc,
        json!([{"op":"setCellProperties","table":object,"row":0,"column":0,"patch":{"fieldName":"수신인","borderFill":{"left":{"style":1,"width":2,"color":255}}}}]),
    );
    for format in ["hwp5", "hwpx"] {
        let mut r = engine
            .open(doc.serialize_as(format, false).unwrap())
            .unwrap();
        assert_eq!(
            q(
                &mut r,
                json!({"kind":"cellProperties","table":object,"row":0,"column":0})
            )["fieldName"],
            "수신인"
        );
    }
    tx(
        &mut doc,
        json!([{"op":"createHeaderFooter","section":0,"kind":"header","side":"even"}]),
    );
    let controls = q(&mut doc, json!({"kind":"controls"}));
    let header = controls
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["kind"] == "header")
        .unwrap()["target"]
        .clone();
    let nested = json!({"section":0,"steps":[{"para":header["para"],"control":header["control"],"index":0}],"para":0,"offset":0});
    tx(
        &mut doc,
        json!([{"op":"insertField","at":nested,"kind":"pageCount"}]),
    );
    let mut filename = nested.clone();
    filename["offset"] = json!(8);
    tx(
        &mut doc,
        json!([{"op":"insertField","at":filename,"kind":"fileName","text":"sample.hwp"},{"op":"formatParas","from":nested,"to":nested,"patch":{"tabStops":[{"position":30000,"type":1,"fill":0}]}}]),
    );
    let before = doc.serialize(false).unwrap();
    let mut preview = doc.furniture_preview(&header.to_string()).unwrap();
    assert!(preview.page(0).unwrap().starts_with(b"HMPK"));
    assert_eq!(doc.serialize(false).unwrap(), before);
    for format in ["hwp5", "hwpx"] {
        let mut r = engine
            .open(doc.serialize_as(format, false).unwrap())
            .unwrap();
        let records = q(&mut r, json!({"kind":"controls"}));
        assert!(
            records
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["fieldKind"] == "pageCount")
        );
        assert!(
            records
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["fieldKind"] == "fileName" && r["value"] == "sample.hwp")
        );
    }
}

#[test]
fn session_list_restart_supports_plain_and_outline_history_but_never_files() {
    let engine = Engine::new();
    for kind in [None, Some("outline"), Some("numbering")] {
        let mut doc = engine.create(SOURCE.to_vec()).unwrap();
        let a = at(&mut doc, 0);
        tx(
            &mut doc,
            json!([{"op":"ensureList","definition":{"kind":"numbering","levels":[{"format":"^1."}]}}]),
        );
        let lists = q(&mut doc, json!({"kind":"lists"}));
        let id = lists
            .as_array()
            .unwrap()
            .iter()
            .find(|l| l["kind"] == "numbering")
            .unwrap()["id"]
            .clone();
        if let Some(kind) = kind {
            tx(
                &mut doc,
                json!([{"op":"setList","from":a,"to":a,"list":{"kind":kind,"id":id}}]),
            );
        }
        tx(
            &mut doc,
            json!([{"op":"setListRestart","at":a,"restart":{"mode":"restart","start":0}}]),
        );
        let read =
            |doc: &mut Doc| q(doc, json!({"kind":"formatContext","at":a}))["listRestart"].clone();
        assert_eq!(read(&mut doc), json!({"mode":"restart","start":1}));
        doc.undo().unwrap();
        assert!(read(&mut doc).is_null());
        doc.redo().unwrap();
        let checkpoint = doc.checkpoint().unwrap();
        tx(
            &mut doc,
            json!([{"op":"setListRestart","at":a,"restart":{"mode":"continue"}}]),
        );
        assert_eq!(read(&mut doc), json!({"mode":"continue"}));
        doc.restore(checkpoint).unwrap();
        assert_eq!(read(&mut doc), json!({"mode":"restart","start":1}));
        if kind.is_none() {
            tx(
                &mut doc,
                json!([{"op":"setList","from":a,"to":a,"list":{"kind":"numbering","id":id}}]),
            );
            assert_eq!(read(&mut doc), json!({"mode":"restart","start":1}));
        }
        let before = doc.serialize(false).unwrap();
        let mut invalid_at = a.clone();
        invalid_at["offset"] = json!(u32::MAX);
        assert!(
            doc.transact(
                &json!({"ops":[{"op":"setListRestart","at":invalid_at,"restart":null}]})
                    .to_string()
            )
            .is_err()
        );
        assert_eq!(doc.serialize(false).unwrap(), before);
        for bad in [
            json!({"mode":"restart","start":-1}),
            json!({"mode":"unknown"}),
        ] {
            assert!(
                doc.transact(
                    &json!({"ops":[{"op":"setListRestart","at":a,"restart":bad}]}).to_string()
                )
                .is_err()
            );
            assert_eq!(doc.serialize(false).unwrap(), before);
        }
        for format in ["hwp5", "hwpx"] {
            let mut reopened = engine
                .open(doc.serialize_as(format, false).unwrap())
                .unwrap();
            assert!(read(&mut reopened).is_null(), "{kind:?} {format}");
            assert_eq!(read(&mut doc), json!({"mode":"restart","start":1}));
        }
    }
}

#[test]
fn last_page_masters_survive_both_format_conversions_and_dirty_resaves_once() {
    let engine = Engine::new();
    let mut doc = engine
        .open(include_bytes!("fixtures/last-page-master.hwpx").to_vec())
        .unwrap();
    for format in ["hwp5", "hwpx"] {
        let saved = doc.serialize_as(format, false).unwrap();
        let model = if format == "hwp5" {
            hm_hwp5::read(saved.clone().into()).unwrap()
        } else {
            hm_hwpx::read(saved.clone().into()).unwrap()
        };
        let list = model.sections[0].last_page_master.as_ref().unwrap();
        assert_eq!(list.paras[0].plain_text(), "LAST_MASTER_PROBE");
        if format == "hwp5" {
            assert!(!list.paras[0].line_segs.is_empty());
        }
        let layout = hm_layout::layout(&model, &hm_fonts::FontSystem::new());
        assert_eq!(layout.pages.len(), 2);
        let counts: Vec<_> = layout
            .pages
            .iter()
            .map(|p| {
                p.frames
                    .iter()
                    .filter(|f| {
                        matches!(
                            f.list,
                            hm_layout::ListRef::MasterPage {
                                index: hm_layout::furniture::LAST_PAGE_MASTER,
                                ..
                            }
                        )
                    })
                    .count()
            })
            .collect();
        assert_eq!(counts, [0, 1], "{format}");
        let mut reopened = engine.open(saved).unwrap();
        let a = at(&mut reopened, 0);
        tx(
            &mut reopened,
            json!([{"op":"insertText","at":a,"text":"Edited "}]),
        );
        let own = reopened.serialize(false).unwrap();
        if format == "hwp5" {
            let saved = hm_hwp5::read(own.clone().into()).unwrap();
            let marker: Vec<u8> = "LAST_MASTER_PROBE"
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect();
            assert_eq!(
                saved.sections[0]
                    .src
                    .as_ref()
                    .unwrap()
                    .windows(marker.len())
                    .filter(|w| *w == marker)
                    .count(),
                1
            );
        }
        let mut reopened = engine.open(own).unwrap();
        let other = reopened
            .serialize_as(if format == "hwpx" { "hwp5" } else { "hwpx" }, false)
            .unwrap();
        let model = if format == "hwpx" {
            hm_hwp5::read(other.into()).unwrap()
        } else {
            hm_hwpx::read(other.into()).unwrap()
        };
        assert_eq!(
            model.sections[0].last_page_master.as_ref().unwrap().paras[0].plain_text(),
            "LAST_MASTER_PROBE"
        );
        assert_eq!(model.sections[0].paras.len(), 2);
    }
}
