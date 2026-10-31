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
fn mixed_text_requests_retain_carets_controls_and_atomic_history() {
    for in_cell in [false, true] {
        let engine = Engine::new();
        let mut doc = engine.open(SOURCE.to_vec()).unwrap();
        let body = q(&mut doc, json!({"kind":"paragraphs","list":{"section":0}}));
        let mut list = json!({"section":0});
        if in_cell {
            let (p, c) = body
                .as_array()
                .unwrap()
                .iter()
                .find_map(|p| {
                    p["controls"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|c| c["kind"] == "table")
                        .map(|c| (p, c))
                })
                .unwrap();
            list["steps"] = json!([{"para":p["index"],"control":c["index"],"index":0}]);
        }
        let mut start = list.clone();
        start["para"] = json!(0);
        start["offset"] =
            q(&mut doc, json!({"kind":"paragraphs","list":list}))[0]["caret"][0].clone();
        tx(
            &mut doc,
            json!([{"op":"insertText","at":start,"text":"abc"}]),
        );
        let before = q(&mut doc, json!({"kind":"paragraphs","list":list}));
        let before_format = q(&mut doc, json!({"kind":"formatContext","at":start}));
        let mut from = start.clone();
        from["offset"] = json!(start["offset"].as_u64().unwrap() + 1);
        let mut to = from.clone();
        to["offset"] = json!(from["offset"].as_u64().unwrap() + 1);
        let selection = json!({"anchor":to,"focus":from,"anchorAffinity":"upstream","focusAffinity":"downstream"});
        let receipt: Value = serde_json::from_str(&doc.transact(&json!({"selection":selection,"ops":[
            {"op":"delete","from":from,"to":to},
            {"op":"insertText","at":from,"text":"가\t😀\n나","format":{"bold":true,"size":1740}},
            {"op":"formatParas","from":from,"to":from,"patch":{"spaceBefore":17}}
        ]}).to_string()).unwrap()).unwrap();
        let mut end = from.clone();
        end["para"] = json!(1);
        end["offset"] = json!(1);
        assert_eq!(
            receipt["caret"], end,
            "a following format must retain the insertion caret"
        );
        let after = q(&mut doc, json!({"kind":"paragraphs","list":list}));
        assert_eq!(
            q(&mut doc, json!({"kind":"formatContext","at":end}))["characters"]["size"],
            1740
        );
        let undone: Value = serde_json::from_str(&doc.undo().unwrap()).unwrap();
        assert_eq!(undone["selection"], selection);
        assert_eq!(
            q(&mut doc, json!({"kind":"paragraphs","list":list})),
            before
        );
        assert_eq!(
            q(&mut doc, json!({"kind":"formatContext","at":start})),
            before_format
        );
        let redone: Value = serde_json::from_str(&doc.redo().unwrap()).unwrap();
        assert_eq!(redone["selection"], receipt["selection"]);
        assert_eq!(q(&mut doc, json!({"kind":"paragraphs","list":list})), after);
        let mut reopened = engine.open(doc.serialize(false).unwrap()).unwrap();
        assert_eq!(
            q(&mut reopened, json!({"kind":"paragraphs","list":list})),
            after
        );

        let state = doc.state().unwrap();
        let bytes = doc.serialize(false).unwrap();
        for bad in [
            json!({"ops":[{"op":"insertText","at":end}]}),
            json!({"ops":[{"op":"insertText","at":end,"text":"x","unknown":true}]}),
            json!({"ops":[{"op":"unknown"}]}),
            json!({"expectedEpoch":0,"ops":[{"op":"insertText","at":end,"text":"x"}]}),
            json!({"ops":[{"op":"insertText","at":end,"text":"temporary"},
                {"op":"formatParas","from":end,"to":end,"patch":{"spaceBefore":23}},
                {"op":"insertText","at":{"section":0,"para":999999,"offset":0},"text":"x"}]}),
        ] {
            assert!(doc.transact(&bad.to_string()).is_err());
            assert_eq!(doc.state().unwrap(), state);
            assert_eq!(doc.serialize(false).unwrap(), bytes);
        }
        // Text-only clipboard paste shares the insertion translator, including paragraph breaks.
        let pasted: Value = serde_json::from_str(
            &doc.paste(&json!({"at":end,"text":"\t다\n끝"}).to_string())
                .unwrap(),
        )
        .unwrap();
        end["para"] = json!(2);
        assert_eq!(pasted["caret"], end);
        doc.undo().unwrap();
        assert_eq!(q(&mut doc, json!({"kind":"paragraphs","list":list})), after);
    }
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

#[test]
fn programmatic_fragment_import_preserves_user_clipboard_on_success_and_failure() {
    let engine = Engine::new();
    let mut source = engine.create(SOURCE.to_vec()).unwrap();
    let from = at(&mut source, 0);
    let to = tx(
        &mut source,
        json!([{"op":"insertText","at":from,"text":"TEMPLATE","format":{"italic":true}}]),
    )["caret"]
        .clone();
    let range = json!({"from":from,"to":to}).to_string();
    let mut target = engine.create(SOURCE.to_vec()).unwrap();
    let a = at(&mut target, 0);
    let end = tx(
        &mut target,
        json!([{"op":"insertText","at":a,"text":"A","format":{"bold":true}}]),
    )["caret"]
        .clone();
    target
        .copy(&json!({"from":a,"to":end}).to_string())
        .unwrap();
    let before: Value = serde_json::from_str(&target.state().unwrap()).unwrap();
    target
        .paste_from(&source, &range, &json!({"at":end}).to_string())
        .unwrap();
    let text = |d: &mut Doc| {
        q(
            d,
            json!({"kind":"paragraphs","list":{"section":0},"count":1}),
        )[0]["text"]
            .clone()
    };
    assert_eq!(text(&mut target), "ATEMPLATE");
    let after: Value = serde_json::from_str(&target.state().unwrap()).unwrap();
    assert_eq!(
        after["undo"].as_u64().unwrap(),
        before["undo"].as_u64().unwrap() + 1
    );
    target.undo().unwrap();
    assert_eq!(text(&mut target), "A");
    target.redo().unwrap();
    assert_eq!(text(&mut target), "ATEMPLATE");
    let saved = target.serialize(false).unwrap();
    let state = target.state().unwrap();
    for (bad_range, bad_target) in [
        (
            json!({"from":from,"to":{"section":0,"para":99,"offset":0}}).to_string(),
            json!({"at":end}).to_string(),
        ),
        (
            range.clone(),
            json!({"at":{"section":0,"para":99,"offset":0}}).to_string(),
        ),
    ] {
        assert!(target.paste_from(&source, &bad_range, &bad_target).is_err());
        assert_eq!(target.state().unwrap(), state);
        assert_eq!(target.serialize(false).unwrap(), saved);
    }
    source.close();
    assert!(
        target
            .paste_from(&source, &range, &json!({"at":end}).to_string())
            .is_err()
    );
    let mut tail = end.clone();
    tail["offset"] = json!(end["offset"].as_u64().unwrap() + 8);
    let pasted: Value =
        serde_json::from_str(&target.paste(&json!({"at":tail}).to_string()).unwrap()).unwrap();
    assert_eq!(text(&mut target), "ATEMPLATEA");
    let look = q(
        &mut target,
        json!({"kind":"formatContext","at":pasted["caret"]}),
    );
    assert_eq!(look["characters"]["bold"], true);
    assert_eq!(look["characters"]["italic"], false);

    let isolated = Engine::new();
    let mut empty = isolated.create(SOURCE.to_vec()).unwrap();
    let mut source = isolated.create(SOURCE.to_vec()).unwrap();
    let from = at(&mut source, 0);
    let to = tx(
        &mut source,
        json!([{"op":"insertText","at":from,"text":"TEMPLATE"}]),
    )["caret"]
        .clone();
    let a = at(&mut empty, 0);
    empty
        .paste_from(
            &source,
            &json!({"from":from,"to":to}).to_string(),
            &json!({"at":a}).to_string(),
        )
        .unwrap();
    assert!(
        empty.paste(&json!({"at":a}).to_string()).is_err(),
        "import must not populate an empty user clipboard"
    );
}

#[test]
fn pasted_paragraphs_take_the_copied_style_and_shape_where_hancom_does() {
    // P0 'AAAABBBB' centered, P1 'XXXX' right-aligned and indented from the left, P2 'YYYY'
    // with a first-line indent, P3 'HEAD' in 개요 1, P4 'end': Hancom clipboard cases c02, c04,
    // c08 and fixes/clipboard-06 ('HEAD' has a style of its own here, 'end' another). A paragraph
    // starting with pasted text takes the copied paragraph's style and shape; one starting with
    // the target's own text keeps the target's.
    let engine = Engine::new();
    let p = |para: u64, offset: u64| json!({"section":0,"para":para,"offset":offset});
    let caret = |d: &mut Doc, para: u64| {
        let ps = q(d, json!({"kind":"paragraphs","list":{"section":0}}));
        ps[para as usize]["caret"][0].as_u64().unwrap()
    };
    let look = |d: &mut Doc, para: u64| {
        let c = caret(d, para);
        let f = q(d, json!({"kind":"formatContext","at":p(para, c)}));
        (f["styleId"].clone(), f["paragraphs"].clone())
    };
    let texts = |d: &mut Doc| {
        q(d, json!({"kind":"paragraphs","list":{"section":0}}))
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["text"].as_str().unwrap().to_string())
            .collect::<Vec<_>>()
    };
    let named = |d: &mut Doc, name: &str| {
        q(d, json!({"kind":"styles"}))
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["name"] == name)
            .unwrap()["id"]
            .clone()
    };
    let styles = |d: &mut Doc| q(d, json!({"kind":"styles"})).as_array().unwrap().len();
    let make = || {
        let mut d = engine.create(SOURCE.to_vec()).unwrap();
        let a = at(&mut d, 0);
        tx(
            &mut d,
            json!([{"op":"insertText","at":a,"text":"AAAABBBB\nXXXX\nYYYY\nHEAD\nend"}]),
        );
        let lead = a["offset"].as_u64().unwrap();
        tx(
            &mut d,
            json!([
                {"op":"createStyle","style":{"name":"머리","paragraphs":{"align":"justify","left":1000}}},
                {"op":"createStyle","style":{"name":"끝","paragraphs":{"align":"distribute"}}},
            ]),
        );
        let (head, end) = (named(&mut d, "머리"), named(&mut d, "끝"));
        tx(
            &mut d,
            json!([
                {"op":"formatParas","from":p(0,lead),"to":p(0,lead),"patch":{"align":"center"}},
                {"op":"formatParas","from":p(1,0),"to":p(1,0),"patch":{"align":"right","left":2000}},
                {"op":"formatParas","from":p(2,0),"to":p(2,0),"patch":{"align":"left","indent":1500}},
                {"op":"applyStyle","from":p(3,0),"to":p(3,4),"styleId":head},
                {"op":"applyStyle","from":p(4,0),"to":p(4,3),"styleId":end},
            ]),
        );
        (d, lead)
    };
    let (mut d, lead) = make();
    let orig: Vec<_> = (0..5).map(|i| look(&mut d, i)).collect();
    // c02: 'XXXX<end>' pasted inside P0: the tail 'BBBB' keeps P0's shape.
    d.copy(&json!({"from":p(1,0),"to":p(2,0)}).to_string())
        .unwrap();
    d.paste(&json!({"at":p(0,lead+4)}).to_string()).unwrap();
    assert_eq!(&texts(&mut d)[..3], ["AAAAXXXX", "BBBB", "XXXX"]);
    assert_eq!(look(&mut d, 0), orig[0]);
    assert_eq!(look(&mut d, 1), orig[0]);
    d.undo().unwrap();
    // c04: 'XXXX<end>YYYY' pasted at the start of P0: P0 takes P1's shape.
    d.copy(&json!({"from":p(1,0),"to":p(2,4)}).to_string())
        .unwrap();
    d.paste(&json!({"at":p(0,lead)}).to_string()).unwrap();
    assert_eq!(&texts(&mut d)[..2], ["XXXX", "YYYYAAAABBBB"]);
    assert_eq!(look(&mut d, 0), orig[1]);
    assert_eq!(look(&mut d, 1), orig[2]);
    d.undo().unwrap();
    // c08: 'HEAD<end>' pasted at the start of P2: 'HEAD' arrives in its style, 'YYYY' keeps its own.
    d.copy(&json!({"from":p(3,0),"to":p(4,0)}).to_string())
        .unwrap();
    d.paste(&json!({"at":p(2,0)}).to_string()).unwrap();
    assert_eq!(&texts(&mut d)[2..4], ["HEAD", "YYYY"]);
    assert_eq!(look(&mut d, 2), orig[3]);
    assert_eq!(look(&mut d, 3), orig[2]);
    d.undo().unwrap();
    // A middle paragraph always takes its own style and shape, the last one when it has text.
    d.copy(&json!({"from":p(2,0),"to":p(4,3)}).to_string())
        .unwrap();
    d.paste(&json!({"at":p(0,lead+4)}).to_string()).unwrap();
    assert_eq!(&texts(&mut d)[..3], ["AAAAYYYY", "HEAD", "endBBBB"]);
    assert_eq!(look(&mut d, 0), orig[0]);
    assert_eq!(look(&mut d, 1), orig[3]);
    assert_eq!(look(&mut d, 2), orig[4]);
    d.undo().unwrap();
    assert_eq!((0..5).map(|i| look(&mut d, i)).collect::<Vec<_>>(), orig);

    // Into another document a style goes to the destination's style of the same name; one it
    // lacks is added once.
    let mut target = engine.create(SOURCE.to_vec()).unwrap();
    tx(
        &mut target,
        json!([{"op":"createStyle","style":{"name":"머리","paragraphs":{"left":4500}}}]),
    );
    let count = styles(&mut target);
    let dest = at(&mut target, 0);
    let range = json!({"from":p(3,0),"to":p(4,3)}).to_string();
    for _ in 0..2 {
        target
            .paste_from(&d, &range, &json!({"at":dest}).to_string())
            .unwrap();
    }
    assert_eq!(texts(&mut target)[..3], ["HEAD", "endHEAD", "end"]);
    let (head, end) = (named(&mut target, "머리"), named(&mut target, "끝"));
    assert_eq!(look(&mut target, 0).0, head);
    assert_eq!(look(&mut target, 1).0, end);
    assert_eq!(look(&mut target, 2).0, end);
    assert_eq!(styles(&mut target), count + 1);
    for format in ["hwp5", "hwpx"] {
        let mut reopened = engine
            .open(target.serialize_as(format, false).unwrap())
            .unwrap();
        assert_eq!(look(&mut reopened, 0).0, head);
        assert_eq!(look(&mut reopened, 2).0, end);
        assert_eq!(texts(&mut reopened)[..3], texts(&mut target)[..3]);
        assert_eq!(styles(&mut reopened), count + 1);
    }
    // Text pasted inside a paragraph keeps that paragraph's style and shape, so the copied
    // paragraph's style and shape are not added.
    let mut inside = engine.create(SOURCE.to_vec()).unwrap();
    let dest = at(&mut inside, 0);
    tx(
        &mut inside,
        json!([{"op":"insertText","at":dest,"text":"TT"}]),
    );
    let shapes = |d: &mut Doc| {
        (0..)
            .take_while(|id| !q(d, json!({"kind":"paraShape","id":id})).is_null())
            .count()
    };
    let before = (
        styles(&mut inside),
        shapes(&mut inside),
        look(&mut inside, 0),
    );
    let mut mid = dest.clone();
    mid["offset"] = json!(dest["offset"].as_u64().unwrap() + 1);
    inside
        .paste_from(
            &d,
            &json!({"from":p(4,1),"to":p(4,2)}).to_string(),
            &json!({"at":mid}).to_string(),
        )
        .unwrap();
    assert_eq!(texts(&mut inside)[0], "TnT");
    assert_eq!(
        (
            styles(&mut inside),
            shapes(&mut inside),
            look(&mut inside, 0)
        ),
        before
    );
}

#[test]
fn search_can_step_over_overlapping_matches_and_a_match_ending_in_a_tab_covers_it() {
    // Hancom's find next selects 'aa' in 'aaaa' three times (find case f06-overlap) while replace
    // all takes two; a match that ends in a tab ends after the tab's 8 units (find-15).
    let engine = Engine::new();
    let mut d = engine.create(SOURCE.to_vec()).unwrap();
    let a = at(&mut d, 0);
    let lead = a["offset"].as_u64().unwrap();
    tx(
        &mut d,
        json!([{"op":"insertText","at":a,"text":"aaaa x a\tb"}]),
    );
    let search = |d: &mut Doc, req: Value| -> Vec<(u64, u64)> {
        let r: Value = serde_json::from_str(&d.search(&req.to_string()).unwrap()).unwrap();
        r["matches"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| {
                (
                    m["from"]["offset"].as_u64().unwrap() - lead,
                    m["to"]["offset"].as_u64().unwrap() - lead,
                )
            })
            .collect()
    };
    assert_eq!(search(&mut d, json!({"text":"aa"})), [(0, 2), (2, 4)]);
    assert_eq!(
        search(&mut d, json!({"text":"aa","overlapping":true})),
        [(0, 2), (1, 3), (2, 4)]
    );
    let mut from = a.clone();
    from["offset"] = json!(lead + 1);
    assert_eq!(
        search(&mut d, json!({"text":"aa","overlapping":true,"from":from})),
        [(2, 4)]
    );
    for across in [false, true] {
        assert_eq!(
            search(&mut d, json!({"text":"a\t","acrossControls":across})),
            [(7, 16)]
        );
    }
    let mut to = a.clone();
    to["offset"] = json!(lead + 16);
    from["offset"] = json!(lead + 7);
    tx(
        &mut d,
        json!([{"op":"delete","from":from,"to":to},{"op":"insertText","at":from,"text":"X"}]),
    );
    let p = q(
        &mut d,
        json!({"kind":"paragraphs","list":{"section":0},"count":1}),
    );
    assert_eq!(p[0]["text"], "aaaa x Xb");
}

#[test]
fn repeated_pastes_from_another_document_reuse_equal_definitions_and_pictures() {
    // clipboard-22: each paste from another document added its character and paragraph shapes,
    // tab definitions and picture data again, so the destination grew on every paste.
    let engine = Engine::new();
    let png = vec![
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 4,
        0, 0, 0, 181, 28, 12, 2, 0, 0, 0, 11, 73, 68, 65, 84, 120, 218, 99, 252, 255, 31, 0, 3, 3,
        2, 0, 239, 191, 107, 153, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ];
    let mut source = engine.create(SOURCE.to_vec()).unwrap();
    let a = at(&mut source, 0);
    tx(
        &mut source,
        json!([{"op":"insertText","at":a,"text":"Bold red\nplain","format":{"bold":true,"color":255}}]),
    );
    let p1 = json!({"section":0,"para":1,"offset":5});
    tx(
        &mut source,
        json!([
            {"op":"formatParas","from":a,"to":a,"patch":{"align":"right","left":2000}},
            {"op":"insertPicture","at":p1,"bytes":png,"extension":"png","width":3000,"height":3000},
        ]),
    );
    let range = json!({"from":a,"to":{"section":0,"para":1,"offset":13}}).to_string();
    let count = |d: &mut Doc, kind: &str| {
        (0..)
            .take_while(|id| !q(d, json!({"kind":kind,"id":id})).is_null())
            .count()
    };
    let pictures = |d: &mut Doc| {
        let ps = q(d, json!({"kind":"paragraphs","list":{"section":0}}));
        let mut out = Vec::new();
        for p in ps.as_array().unwrap() {
            for c in p["controls"].as_array().unwrap() {
                if c["kind"] == "object" {
                    let object = json!({"section":0,"para":p["index"],"control":c["index"]});
                    let data = q(d, json!({"kind":"pictureData","object":object}));
                    assert_eq!(data["bytes"], json!(png));
                    out.push(data["binDataId"].clone());
                }
            }
        }
        out
    };
    let mut target = engine.create(SOURCE.to_vec()).unwrap();
    let dest = at(&mut target, 0);
    let shapes = (
        count(&mut target, "charShape"),
        count(&mut target, "paraShape"),
    );
    target
        .paste_from(&source, &range, &json!({"at":dest}).to_string())
        .unwrap();
    let once = (
        count(&mut target, "charShape"),
        count(&mut target, "paraShape"),
    );
    assert!(
        once.0 > shapes.0 && once.1 > shapes.1,
        "{shapes:?} {once:?}"
    );
    for _ in 0..3 {
        target
            .paste_from(&source, &range, &json!({"at":dest}).to_string())
            .unwrap();
    }
    assert_eq!(
        (
            count(&mut target, "charShape"),
            count(&mut target, "paraShape")
        ),
        once
    );
    let ids = pictures(&mut target);
    assert_eq!(ids.len(), 4);
    assert!(ids.iter().all(|id| *id == ids[0]), "{ids:?}");
    for format in ["hwp5", "hwpx"] {
        let mut reopened = engine
            .open(target.serialize_as(format, false).unwrap())
            .unwrap();
        let ids = pictures(&mut reopened);
        assert_eq!(ids.len(), 4);
        assert!(ids.iter().all(|id| *id == ids[0]), "{format} {ids:?}");
        assert_eq!(
            (
                count(&mut reopened, "charShape"),
                count(&mut reopened, "paraShape")
            ),
            once
        );
    }
}

#[test]
fn copied_text_carries_paragraph_numbers_and_html_the_paragraph_shapes_and_links() {
    // Hancom's clipboard text starts a numbered paragraph with its number ('1. HEAD',
    // clipboard dump 1178) and its HTML gives each paragraph its alignment and indents
    // (dump 1154: margin-left:20.0pt;text-align:right and text-indent:15.0pt;text-align:left),
    // a numbered one's left margin left to its list.
    let engine = Engine::new();
    let mut d = engine.create(SOURCE.to_vec()).unwrap();
    let a = at(&mut d, 0);
    let lead = a["offset"].as_u64().unwrap();
    let p = |para: u64, offset: u64| json!({"section":0,"para":para,"offset":offset});
    tx(
        &mut d,
        json!([
            {"op":"insertText","at":a,"text":"first
second
plain
indented"},
            {"op":"ensureList","definition":{"kind":"numbering","levels":[{"format":"^1."}]}},
        ]),
    );
    let id = q(&mut d, json!({"kind":"lists"}))
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["kind"] == "numbering")
        .unwrap()["id"]
        .clone();
    tx(
        &mut d,
        json!([
            {"op":"setList","from":p(0,lead),"to":p(1,0),"list":{"kind":"numbering","id":id}},
            {"op":"formatParas","from":p(0,lead),"to":p(0,lead),"patch":{"left":2000}},
            {"op":"formatParas","from":p(2,0),"to":p(2,0),"patch":{"align":"right","left":2000}},
            {"op":"formatParas","from":p(3,0),"to":p(3,0),"patch":{"align":"left","indent":1500}},
        ]),
    );
    let copy = |d: &mut Doc, from: Value, to: Value| -> Value {
        serde_json::from_str(&d.copy(&json!({"from":from,"to":to}).to_string()).unwrap()).unwrap()
    };
    let all = copy(&mut d, p(0, lead), p(3, 8));
    assert_eq!(
        all["text"],
        "1. first
2. second
plain
indented"
    );
    let html = all["html"].as_str().unwrap();
    let opens: Vec<&str> = html
        .match_indices("<p")
        .map(|(i, _)| &html[i..i + html[i..].find('>').unwrap() + 1])
        .collect();
    assert_eq!(
        opens,
        [
            "<p>",
            "<p>",
            "<p style=\"margin-left:20.0pt;text-align:right\">",
            "<p style=\"text-indent:15.0pt;text-align:left\">"
        ],
        "{html}"
    );
    assert!(!html.contains("1."), "{html}");
    // From inside the first paragraph only the second gets its number; pasting back inserts
    // no number text.
    let part = copy(&mut d, p(0, lead + 2), p(1, 6));
    assert_eq!(
        part["text"],
        "rst
2. second"
    );
    d.paste(&json!({"at":p(3,8)}).to_string()).unwrap();
    let texts = q(&mut d, json!({"kind":"paragraphs","list":{"section":0}}));
    assert_eq!(texts[3]["text"], "indentedrst");
    assert_eq!(texts[4]["text"], "second");

    // A copied table writes a hyperlink in a cell as a link.
    tx(
        &mut d,
        json!([{"op":"createTable","at":p(4,6),"rows":1,"columns":1}]),
    );
    let cell = json!({"section":0,"steps":[{"para":4,"control":0,"index":0}],"para":0,"offset":0});
    let mut to = cell.clone();
    to["offset"] = json!(4);
    tx(
        &mut d,
        json!([
            {"op":"insertText","at":cell,"text":"link"},
            {"op":"insertField","at":cell,"to":to,"kind":"hyperlink","command":"https://example.com"},
        ]),
    );
    let table = copy(&mut d, p(4, 6), p(4, 14));
    let html = table["html"].as_str().unwrap();
    assert!(
        html.contains("<a href=\"https://example.com\"><span"),
        "{html}"
    );
    assert!(html.contains("link</span></a></p></td>"), "{html}");
}

#[test]
fn search_takes_hancoms_special_codes_and_initial_consonants() {
    // Hancom find cases f10-special-codes (^t ^l ^s ^n are codes, ^b is text; ^n selects a
    // paragraph end up to the next paragraph) and f08-jaso ('ㅎ' with 자소 단위 finds 한 and 하
    // as well as ㅎ).
    let engine = Engine::new();
    let mut d = engine.create(SOURCE.to_vec()).unwrap();
    let a = at(&mut d, 0);
    let lead = a["offset"].as_u64().unwrap();
    tx(
        &mut d,
        json!([{"op":"insertText","at":a,"text":"a\tb\u{2028}c\u{2007}d\ndog.\n한글 하나 ㅎ"}]),
    );
    let search = |d: &mut Doc, req: Value| -> Vec<[(u64, u64); 2]> {
        let r: Value = serde_json::from_str(&d.search(&req.to_string()).unwrap()).unwrap();
        let end = |a: &Value| (a["para"].as_u64().unwrap(), a["offset"].as_u64().unwrap());
        r["matches"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| [end(&m["from"]), end(&m["to"])])
            .collect()
    };
    for (code, at) in [("^t", (1, 9)), ("^l", (10, 11)), ("^s", (12, 13))] {
        assert_eq!(
            search(&mut d, json!({"text":code,"codes":true})),
            [[(0, lead + at.0), (0, lead + at.1)]],
            "{code}"
        );
        assert!(search(&mut d, json!({"text":code})).is_empty());
    }
    assert_eq!(
        search(&mut d, json!({"text":"dog.^n","codes":true})),
        [[(1, 0), (2, 0)]]
    );
    // The last paragraph has no end to find; '^b' is no code.
    assert_eq!(
        search(&mut d, json!({"text":"^n","codes":true})),
        [[(0, lead + 14), (1, 0)], [(1, 4), (2, 0)]]
    );
    assert!(search(&mut d, json!({"text":"^b","codes":true})).is_empty());
    assert_eq!(
        search(&mut d, json!({"text":"ㅎ","jamo":true})),
        [[(2, 0), (2, 1)], [(2, 3), (2, 4)], [(2, 6), (2, 7)]]
    );
    assert_eq!(search(&mut d, json!({"text":"ㅎ"})), [[(2, 6), (2, 7)]]);
}
