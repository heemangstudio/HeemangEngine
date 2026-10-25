use hm_wasm::{Doc, Engine};
use serde_json::{Value, json};
const FIXTURE: &[u8] = include_bytes!("fixtures/legacy-equation-body.hwpx");
const SCRIPT: &str = "N``=`  {(0.75  TIMES  V  TIMES  1000)-(M+F)} over {75}  ";
fn query(d: &mut Doc, q: Value) -> Value {
    serde_json::from_str(&d.query(&q.to_string()).unwrap()).unwrap()
}
fn promote(d: &mut Doc) -> Result<String, hm_wasm::types::EngineError> {
    d.transact(
        &json!({"ops":[{"op":"promoteOleEquation","object":{"section":0,"para":4,"control":0}}]})
            .to_string(),
    )
}
#[test]
fn promotion_keeps_the_frame_identity_history_and_both_saved_formats() {
    let mut d = Engine::new().open(FIXTURE.to_vec()).unwrap();
    let object = json!({"section":0,"para":4,"control":0});
    d.transact(
        &json!({"ops":[{"op":"setObjectCaption","object":object,"text":"Preserved caption"}]})
            .to_string(),
    )
    .unwrap();
    let before = d.serialize(false).unwrap();
    let frame = query(&mut d, json!({"kind":"objectProperties","object":object}));
    assert_eq!(frame["objectId"], 1701116216u32);
    assert_eq!(frame["instanceId"], 627374393u32);
    let state: Value = serde_json::from_str(&d.state().unwrap()).unwrap();
    let receipt: Value = serde_json::from_str(&promote(&mut d).unwrap()).unwrap();
    assert_eq!(
        receipt["undo"].as_u64().unwrap(),
        state["undo"].as_u64().unwrap() + 1
    );
    let equation = query(&mut d, json!({"kind":"equation","object":object}));
    assert_eq!(equation["script"], SCRIPT);
    assert_eq!(equation["size"], 950);
    assert_eq!(equation["baseline"], 60);
    assert_eq!(equation["font"], "HYhwpEQ");
    for format in ["hwp5", "hwpx"] {
        let mut r = Engine::new()
            .open(d.serialize_as(format, false).unwrap())
            .unwrap();
        assert_eq!(
            query(&mut r, json!({"kind":"equation","object":object})),
            equation
        );
        let saved = query(&mut r, json!({"kind":"objectProperties","object":object}));
        for key in [
            "width",
            "height",
            "horzOffset",
            "vertOffset",
            "treatAsChar",
            "horzRelTo",
            "vertRelTo",
            "textWrap",
            "margins",
            "zOrder",
            "instanceId",
            "caption",
        ] {
            assert_eq!(saved[key], frame[key], "{format}:{key}");
        }
        assert_eq!(
            query(
                &mut r,
                json!({"kind":"paragraphs","list":{"section":0},"from":4,"count":1})
            )[0]["controls"][0]["kind"],
            "equation"
        );
    }
    d.undo().unwrap();
    assert_eq!(d.serialize(false).unwrap(), before);
    d.redo().unwrap();
    assert_eq!(
        query(&mut d, json!({"kind":"equation","object":object}))["script"],
        SCRIPT
    );
}
#[test]
fn unknown_or_missing_ole_storage_fails_without_changing_content_or_history() {
    for missing in [false, true] {
        let mut model = hm_hwpx::read(FIXTURE.to_vec().into()).unwrap();
        // Locate the actual source storage id rather than assuming its package order.
        let id = model.sections[0].paras[4].controls[0]
            .as_object()
            .unwrap()
            .shape()
            .and_then(|s| match s.kind() {
                Some(hm_model::ShapePart::Ole(o)) => Some(o.bin_id),
                _ => None,
            })
            .unwrap();
        let original = model.bins.clone().unwrap();
        model.bins = Some(hm_model::BinStore::new(move |which| {
            if which == id {
                if missing {
                    None
                } else {
                    Some(b"not a supported OLE storage".to_vec().into())
                }
            } else {
                original.get(which)
            }
        }));
        model.convert_to(hm_model::Format::Hwp5);
        model.convert_to(hm_model::Format::Hwpx);
        let bytes = hm_hwpx::write_new(&model).unwrap();
        let mut d = Engine::new().open(bytes).unwrap();
        let before = d.serialize(false).unwrap();
        let state = d.state().unwrap();
        assert!(promote(&mut d).is_err());
        assert_eq!(d.serialize(false).unwrap(), before);
        assert_eq!(d.state().unwrap(), state);
    }
}
#[test]
fn nested_or_non_ole_controls_are_rejected_atomically() {
    let mut d = Engine::new().open(FIXTURE.to_vec()).unwrap();
    let before = d.serialize(false).unwrap();
    let state = d.state().unwrap();
    for object in [
        json!({"section":0,"para":2,"control":0}),
        json!({"section":0,"steps":[{"para":2,"control":0,"index":3}],"para":1,"control":0}),
    ] {
        assert!(
            d.transact(&json!({"ops":[{"op":"promoteOleEquation","object":object}]}).to_string())
                .is_err()
        );
        assert_eq!(d.serialize(false).unwrap(), before);
        assert_eq!(d.state().unwrap(), state);
    }
}

#[test]
fn ole_xml_object_id_and_logical_identity_roundtrip_where_the_format_supports_them() {
    let mut d = Engine::new().open(FIXTURE.to_vec()).unwrap();
    let object = json!({"section":0,"para":4,"control":0});
    d.transact(&json!({"ops":[{"op":"setObjectProperties","object":object,"patch":{"description":"Identity preservation"}}]}).to_string()).unwrap();
    for format in ["hwpx", "hwp5"] {
        let mut r = Engine::new()
            .open(d.serialize_as(format, false).unwrap())
            .unwrap();
        let props = query(&mut r, json!({"kind":"objectProperties","object":object}));
        assert_eq!(props["instanceId"], 627374393u32, "{format}:logical");
        assert_eq!(
            props["objectId"],
            if format == "hwpx" {
                1701116216u32
            } else {
                627374393u32
            },
            "{format}:object"
        );
        promote(&mut r).unwrap();
        assert_eq!(
            query(&mut r, json!({"kind":"objectProperties","object":object}))["instanceId"],
            627374393u32
        );
    }
}
