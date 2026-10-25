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
/// The picture's drawn box on page 0, from the page's object list.
fn drawn(d: &mut hm_wasm::Doc, a: &Value) -> (i64, i64, i64, i64) {
    let objects: Value = serde_json::from_str(&d.page_objects(0).unwrap()).unwrap();
    let o = objects
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["para"] == a["para"] && o["control"] == a["control"])
        .unwrap()
        .clone();
    let n = |k: &str| o[k].as_i64().unwrap();
    (n("x"), n("y"), n("width"), n("height"))
}
#[test]
fn turned_pictures_frame_their_bounds_and_swap_transforms_back_exactly() {
    let mut d = blank();
    let at = caret(&mut d);
    tx(
        &mut d,
        json!([{ "op":"insertPicture","at":at,"bytes":PNG,"extension":"png","width":6000,"height":3000,"inline":false,"horzOffset":9000,"vertOffset":9000}]),
    );
    let a = object_at(&mut d, &at);
    let (x0, y0, w0, h0) = drawn(&mut d, &a);
    let center = (x0 * 2 + w0, y0 * 2 + h0);
    let kept = d.capture_object_transform(&a.to_string()).unwrap();
    tx(
        &mut d,
        json!([{ "op":"setObjectProperties","object":a,"patch":{"rotation":90,"flipHorizontal":true}}]),
    );
    let p = props(&mut d, &a);
    // The box turns with the drawing; the drawing keeps its size and center.
    assert_eq!(
        (p["width"].as_i64(), p["height"].as_i64()),
        (Some(3000), Some(6000))
    );
    assert_eq!(p["size"], json!([6000, 3000]));
    assert_eq!(
        (p["rotation"].as_i64(), p["flipHorizontal"].as_bool()),
        (Some(90), Some(true))
    );
    let (x, y, w, h) = drawn(&mut d, &a);
    assert!((x * 2 + w - center.0).abs() <= 2 && (y * 2 + h - center.1).abs() <= 2);
    // A new frame sizes the drawing so its turned bounds fill it.
    tx(
        &mut d,
        json!([{ "op":"setObjectProperties","object":a,"patch":{"width":4000,"height":8000}}]),
    );
    let p = props(&mut d, &a);
    assert_eq!(
        (p["width"].as_i64(), p["height"].as_i64()),
        (Some(4000), Some(8000))
    );
    assert_eq!(p["size"], json!([8000, 4000]));
    for format in ["hwp5", "hwpx"] {
        let mut r = reopen(&mut d, format);
        let q = props(&mut r, &a);
        for k in [
            "width",
            "height",
            "horzOffset",
            "vertOffset",
            "rotation",
            "flipHorizontal",
            "size",
        ] {
            assert_eq!(q[k], p[k], "{format} {k}");
        }
        assert_eq!(drawn(&mut r, &a), drawn(&mut d, &a), "{format}");
    }
    // The kept transform comes back exactly, as one undo step, and swaps again.
    let undo = d.state().unwrap();
    d.swap_object_transform(kept).unwrap();
    let back = props(&mut d, &a);
    assert_eq!(
        (
            back["width"].as_i64(),
            back["height"].as_i64(),
            back["rotation"].as_i64()
        ),
        (Some(6000), Some(3000), Some(0))
    );
    assert_eq!(back["flipHorizontal"], false);
    assert_eq!(drawn(&mut d, &a), (x0, y0, w0, h0));
    let undo_after: Value = serde_json::from_str(&d.state().unwrap()).unwrap();
    let undo_before: Value = serde_json::from_str(&undo).unwrap();
    assert_eq!(
        undo_after["undo"].as_i64().unwrap(),
        undo_before["undo"].as_i64().unwrap() + 1
    );
    d.swap_object_transform(kept).unwrap();
    assert_eq!(props(&mut d, &a)["size"], json!([8000, 4000]));
    assert!(d.discard_object_transform(kept).unwrap());
    assert!(d.swap_object_transform(kept).is_err());
}
#[test]
fn a_point_in_an_empty_cell_of_an_inline_table_finds_that_cell() {
    let mut d = blank();
    let at = caret(&mut d);
    tx(
        &mut d,
        json!([{ "op":"createTable","at":at,"rows":2,"columns":3,"inline":true}]),
    );
    let a = object_at(&mut d, &at);
    let info = query(&mut d, json!({"kind":"table","table":a}));
    for (i, cell) in info["cells"].as_array().unwrap().iter().enumerate() {
        let r = &cell["rects"][0];
        let n = |k: &str| r[k].as_i64().unwrap();
        // Toward the cell's right edge, away from its empty line's caret.
        let (x, y) = (n("x") + n("width") * 4 / 5, n("y") + n("height") / 2);
        let hit: Value = serde_json::from_str(
            &d.locate(&json!({"kind":"point","page":0,"x":x,"y":y,"body":true}).to_string())
                .unwrap(),
        )
        .unwrap();
        assert_eq!(hit["position"]["steps"][0]["index"], i, "cell {i}: {hit}");
    }
}
#[test]
fn cell_borders_fills_and_zones_share_edges_and_save_in_both_formats() {
    let mut d = blank();
    let at = caret(&mut d);
    tx(
        &mut d,
        json!([{ "op":"createTable","at":at,"rows":3,"columns":3}]),
    );
    let a = object_at(&mut d, &at);
    let thick = json!({"style":1,"width":7,"color":255});
    let none = json!({"style":0,"width":0,"color":0});
    tx(
        &mut d,
        json!([
            {"op":"setCellProperties","table":a,"row":1,"column":1,"patch":{"textDirection":1,"editableInForm":true,
                "borderFill":{"left":thick,"top":none,"fill":{"kind":"solid","background":0x00FFEE,"pattern":2,"patternColor":0x112233},"slash":2}}},
            {"op":"setCellZone","table":a,"startRow":2,"startColumn":0,"endRow":2,"endColumn":2,
                "borderFill":{"left":thick,"right":thick,"top":thick,"bottom":thick,"fill":{"kind":"solid","background":0x0000FF}}},
            {"op":"setTableProperties","table":a,"patch":{"borderFill":{"fill":{"kind":"solid","background":0xABCDEF}}}},
            {"op":"setObjectProperties","object":a,"patch":{"restrictInPage":true,"allowOverlap":true,"keepWithAnchor":true}}
        ]),
    );
    let check = |r: &mut hm_wasm::Doc, label: &str| {
        let cell = |r: &mut hm_wasm::Doc, row: u16, column: u16| {
            query(
                r,
                json!({"kind":"cellProperties","table":a,"row":row,"column":column}),
            )
        };
        let c = cell(r, 1, 1);
        let b = &c["borderFill"];
        assert_eq!(
            (
                b["left"]["style"].as_i64(),
                b["left"]["widthIndex"].as_i64(),
                b["left"]["color"].as_i64()
            ),
            (Some(1), Some(7), Some(255)),
            "{label}"
        );
        assert_eq!(b["top"]["style"], 0, "{label}");
        assert_eq!(
            b["fill"]["solid"],
            json!({"background":0x00FFEE,"patternColor":0x112233,"pattern":2}),
            "{label}"
        );
        assert_eq!(b["slash"], 2, "{label}");
        assert_eq!(
            (c["textDirection"].as_i64(), c["editableInForm"].as_bool()),
            (Some(1), Some(true)),
            "{label}"
        );
        // The neighbors across the changed sides show the same edge.
        assert_eq!(
            cell(r, 1, 0)["borderFill"]["right"]["widthIndex"],
            7,
            "{label}"
        );
        assert_eq!(cell(r, 0, 1)["borderFill"]["bottom"]["style"], 0, "{label}");
        assert_eq!(cell(r, 1, 2)["borderFill"]["left"]["style"], 1, "{label}");
        assert_eq!(
            cell(r, 1, 2)["borderFill"]["left"]["widthIndex"],
            0,
            "{label}"
        );
        let t = query(r, json!({"kind":"tableProperties","table":a}));
        assert_eq!(t["borderFill"]["background"], 0xABCDEF, "{label}");
        let p = props(r, &a);
        assert_eq!(
            (
                p["restrictInPage"].as_bool(),
                p["allowOverlap"].as_bool(),
                p["keepWithAnchor"].as_bool()
            ),
            (Some(true), Some(true), Some(true)),
            "{label}"
        );
    };
    check(&mut d, "live");
    for format in ["hwp5", "hwpx"] {
        let mut r = reopen(&mut d, format);
        check(&mut r, format);
        let svg = r.page_svg(0, 96.0).unwrap().to_lowercase();
        assert!(
            svg.contains("#ff0000") || svg.contains("rgb(255,0,0)"),
            "{format}: the zone fill paints"
        );
    }
    // One undo takes the whole batch back.
    d.undo().unwrap();
    let c = query(
        &mut d,
        json!({"kind":"cellProperties","table":a,"row":1,"column":0}),
    );
    assert_eq!(c["borderFill"]["right"]["widthIndex"], 0);
}
