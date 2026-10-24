//! Objects created in a live document retain their bytes, geometry and cell
//! content in both file formats and participate in one atomic undo history.
use serde_json::{Value, json};
const TEMPLATE: &[u8] = include_bytes!("fixtures/query-equations.hwpx");
const PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 4, 0,
    0, 0, 181, 28, 12, 2, 0, 0, 0, 11, 73, 68, 65, 84, 120, 218, 99, 252, 255, 31, 0, 3, 3, 2, 0,
    239, 191, 107, 153, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];
fn query(d: &mut hm_wasm::Doc, q: Value) -> Value {
    serde_json::from_str(&d.query(&q.to_string()).unwrap()).unwrap()
}
fn tx(d: &mut hm_wasm::Doc, ops: Value) -> Value {
    serde_json::from_str(&d.transact(&json!({"ops":ops}).to_string()).unwrap()).unwrap()
}
fn blank() -> hm_wasm::Doc {
    hm_wasm::Engine::new().create(TEMPLATE.to_vec()).unwrap()
}
fn caret(d: &mut hm_wasm::Doc) -> Value {
    let p = query(
        d,
        json!({"kind":"paragraphs","list":{"section":0},"count":1}),
    );
    json!({"section":0,"para":0,"offset":p[0]["caret"][0]})
}
fn object_at(d: &mut hm_wasm::Doc, at: &Value) -> Value {
    let p = query(
        d,
        json!({"kind":"paragraphs","list":{"section":0},"count":1}),
    );
    let c = p[0]["controls"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["offset"] == at["offset"])
        .unwrap();
    json!({"section":0,"para":0,"control":c["index"]})
}
fn props(d: &mut hm_wasm::Doc, a: &Value) -> Value {
    query(d, json!({"kind":"objectProperties","object":a}))
}
fn reopen(d: &mut hm_wasm::Doc, format: &str) -> hm_wasm::Doc {
    hm_wasm::Engine::new()
        .open(d.serialize_as(format, false).unwrap())
        .unwrap()
}
#[test]
fn created_tables_keep_cell_text_properties_and_history() {
    let mut d = blank();
    let at = caret(&mut d);
    tx(
        &mut d,
        json!([{ "op":"createTable","at":at,"rows":2,"columns":3,"columnWidths":[3000,4000,5000],"rowHeights":[2000,2500]}]),
    );
    let a = object_at(&mut d, &at);
    let info = query(&mut d, json!({"kind":"table","table":a}));
    assert_eq!(info["cells"].as_array().unwrap().len(), 6);
    let cell = json!({"section":0,"steps":[{"para":0,"control":a["control"],"index":4}],"para":0,"offset":0});
    tx(
        &mut d,
        json!([{ "op":"insertText","at":cell,"text":"표 내용"},{"op":"setTableProperties","table":a,"patch":{"padding":[100,200,300,400],"repeatHeader":true,"split":"cell"}},{"op":"setCellProperties","table":a,"row":1,"column":1,"patch":{"padding":[40,50,60,70],"verticalAlign":"bottom","header":true}}]),
    );
    for format in ["hwp5", "hwpx"] {
        let mut r = reopen(&mut d, format);
        let t = query(&mut r, json!({"kind":"tableProperties","table":a}));
        assert_eq!(t["padding"], json!([100, 200, 300, 400]));
        assert_eq!(t["repeatHeader"], true);
        assert_eq!(t["split"], "cell");
        let c = query(
            &mut r,
            json!({"kind":"cellProperties","table":a,"row":1,"column":1}),
        );
        assert_eq!(c["padding"], json!([40, 50, 60, 70]));
        assert_eq!(c["ownPadding"], true);
        assert_eq!(c["verticalAlign"], "bottom");
        let p = query(
            &mut r,
            json!({"kind":"paragraphs","list":{"section":0,"steps":cell["steps"]}}),
        );
        assert_eq!(p[0]["text"], "표 내용");
    }
    d.undo().unwrap();
    assert_eq!(
        query(
            &mut d,
            json!({"kind":"cellProperties","table":a,"row":1,"column":1})
        )["ownPadding"],
        false
    );
    d.undo().unwrap();
    assert!(
        d.query(&json!({"kind":"table","table":a}).to_string())
            .is_err()
    );
    d.redo().unwrap();
    assert_eq!(
        query(&mut d, json!({"kind":"tableProperties","table":a}))["columns"],
        3
    );
    tx(
        &mut d,
        json!([{ "op":"setCellProperties","table":a,"row":1,"column":1,"patch":{"padding":[40,50,60,70]} }]),
    );
    tx(
        &mut d,
        json!([{ "op":"setCellProperties","table":a,"row":1,"column":1,"patch":{"ownPadding":false} }]),
    );
    let cell = query(
        &mut d,
        json!({"kind":"cellProperties","table":a,"row":1,"column":1}),
    );
    assert_eq!(cell["ownPadding"], false);
    assert_eq!(cell["padding"], cell["tablePadding"]);
    d.undo().unwrap();
    assert_eq!(
        query(
            &mut d,
            json!({"kind":"cellProperties","table":a,"row":1,"column":1})
        )["padding"],
        json!([40, 50, 60, 70])
    );
}
#[test]
fn embedded_pictures_keep_bytes_and_delete_undo_in_both_formats() {
    for template_format in ["hwp5", "hwpx"] {
        let mut d = blank();
        let mut d = reopen(&mut d, template_format);
        let at = caret(&mut d);
        tx(
            &mut d,
            json!([{ "op":"insertPicture","at":at,"bytes":PNG,"extension":"png","description":"그림 설명","width":3000,"height":4500,"naturalWidth":75,"naturalHeight":75,"inline":false,"horzOffset":123,"vertOffset":456}]),
        );
        let a = object_at(&mut d, &at);
        tx(
            &mut d,
            json!([{ "op":"setObjectProperties","object":a,"patch":{"width":6000,"height":9000,"horzRelTo":"page","vertRelTo":"para","textWrap":"inFront","zOrder":9,"border":{"color":255,"width":56,"style":1}}}]),
        );
        for format in ["hwp5", "hwpx"] {
            let mut r = reopen(&mut d, format);
            let p = props(&mut r, &a);
            assert_eq!(p["kind"], "picture");
            assert_eq!(p["width"], 6000);
            assert_eq!(p["height"], 9000);
            assert_eq!(p["description"], "그림 설명");
            assert_eq!(p["horzOffset"], 123);
            assert_eq!(p["vertOffset"], 456);
            assert_eq!(p["textWrap"], "inFront");
            assert_eq!(p["zOrder"], 9);
            assert_eq!(p["border"]["width"], 56);
            let data = query(&mut r, json!({"kind":"pictureData","object":a}));
            assert_eq!(data["bytes"], json!(PNG));
            assert_eq!(data["naturalWidth"], 75);
            assert_eq!(data["naturalHeight"], 75);
            assert!(r.page_objects(0).unwrap().contains("picture"));
        }
        tx(&mut d, json!([{ "op":"deleteObject","object":a }]));
        assert!(
            d.query(&json!({"kind":"pictureData","object":a}).to_string())
                .is_err()
        );
        d.undo().unwrap();
        assert_eq!(
            query(&mut d, json!({"kind":"pictureData","object":a}))["bytes"],
            json!(PNG)
        );
    }
}
#[test]
fn shapes_and_relocation_keep_nested_addresses_and_undo() {
    for kind in ["rect", "ellipse", "line", "textbox"] {
        let mut d = blank();
        let at = caret(&mut d);
        let mut create = json!({"op":"createShape","at":at,"kind":kind,"width":5000,"height":3000});
        if kind == "textbox" {
            create["text"] = json!("텍스트\n둘째 줄");
        }
        tx(&mut d, json!([create]));
        let a = object_at(&mut d, &at);
        for format in ["hwp5", "hwpx"] {
            let mut r = reopen(&mut d, format);
            let p = props(&mut r, &a);
            assert_eq!(p["width"], 5000);
            if kind == "textbox" {
                assert_eq!(p["text"], "텍스트\n둘째 줄");
            }
        }
        let next = json!({"section":0,"para":0,"offset":at["offset"].as_u64().unwrap()+8});
        tx(
            &mut d,
            json!([{ "op":"createTable","at":next,"rows":1,"columns":1 }]),
        );
        let t = object_at(&mut d, &next);
        let nested = json!({"section":0,"steps":[{"para":0,"control":t["control"],"index":0}],"para":0,"offset":0});
        tx(
            &mut d,
            json!([{ "op":"relocateObject","object":a,"at":nested }]),
        );
        let moved = json!({"section":0,"steps":[{"para":0,"control":t["control"].as_u64().unwrap()-1,"index":0}],"para":0,"control":0});
        assert_eq!(props(&mut d, &moved)["width"], 5000);
        d.undo().unwrap();
        assert_eq!(props(&mut d, &a)["width"], 5000);
    }
}
#[test]
fn invalid_object_batches_restore_images_and_document_state() {
    let mut d = blank();
    let at = caret(&mut d);
    let state = d.state().unwrap();
    let original = d.serialize(false).unwrap();
    assert!(d.transact(&json!({"ops":[{"op":"insertPicture","at":at,"bytes":PNG,"extension":"png","width":3000,"height":3000},{"op":"createTable","at":at,"rows":0,"columns":1}]}).to_string()).is_err());
    assert_eq!(d.state().unwrap(), state);
    assert_eq!(d.serialize(false).unwrap(), original);
    assert!(d.transact(&json!({"ops":[{"op":"setCellProperties","table":{"section":0,"para":0,"control":0},"row":0,"column":0,"patch":{"unknown":true}}]}).to_string()).is_err());
    assert_eq!(d.state().unwrap(), state);
}
