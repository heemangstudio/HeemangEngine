//! Agent edits for equations and table captions undo as one step and
//! survive saving to either format.
use hm_model::{ControlKind, NumberKind, SharedBytes};
use hm_wasm::{Doc, Engine};
use serde_json::{Value, json};

fn sources() -> [&'static [u8]; 2] {
    [
        include_bytes!("fixtures/query-equations.hwp"),
        include_bytes!("fixtures/query-equations.hwpx"),
    ]
}
fn query(d: &mut Doc, q: Value) -> Value {
    serde_json::from_str(&d.query(&q.to_string()).unwrap()).unwrap()
}
fn tx(d: &mut Doc, ops: Value) -> Value {
    serde_json::from_str(&d.transact(&json!({"ops":ops}).to_string()).unwrap()).unwrap()
}
fn reopen(d: &mut Doc, format: &str) -> Doc {
    Engine::new()
        .open(d.serialize_as(format, false).unwrap())
        .unwrap()
}
/// The end of the first body paragraph, past its section and column controls.
fn end(d: &mut Doc) -> Value {
    let p = query(
        d,
        json!({"kind":"paragraphs","list":{"section":0},"count":1}),
    );
    json!({"section":0,"para":0,"offset":p[0]["length"].as_u64().unwrap() - 1})
}
fn object_at(d: &mut Doc, at: &Value) -> Value {
    let p = query(
        d,
        json!({"kind":"paragraphs","list":{"section":0},"count":1}),
    );
    let c = p[0]["controls"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["offset"] == at["offset"])
        .unwrap()
        .clone();
    json!({"section":0,"para":0,"control":c["index"]})
}
/// The fixture's one-cell table.
fn table() -> Value {
    json!({"section":0,"para":0,"control":2})
}
/// The caption list follows every cell list of the table.
fn caption(d: &mut Doc, table: &Value) -> Value {
    let cells = query(d, json!({"kind":"table","table":table}))["cells"]
        .as_array()
        .unwrap()
        .len();
    let steps = json!([{"para":table["para"],"control":table["control"],"index":cells}]);
    query(
        d,
        json!({"kind":"paragraphs","list":{"section":0,"steps":steps}}),
    )
}
/// The table numbers saved in the fixture table's caption, read back from
/// `format` bytes by the file readers.
fn caption_numbers(d: &mut Doc, format: &str) -> Vec<(NumberKind, u16)> {
    let bytes = SharedBytes::from(d.serialize_as(format, false).unwrap());
    let doc = match format {
        "hwpx" => hm_hwpx::read(bytes).unwrap(),
        _ => hm_hwp5::read(bytes).unwrap(),
    };
    let t = doc.sections[0].paras[0].controls[2].as_table().unwrap();
    t.caption
        .iter()
        .flat_map(|c| &c.paras)
        .flat_map(|p| &p.controls)
        .filter_map(|c| match &c.kind {
            ControlKind::AutoNumber(a) => Some((a.kind(), a.number)),
            _ => None,
        })
        .collect()
}

#[test]
fn equations_insert_edit_undo_and_roundtrip() {
    for bytes in sources() {
        let mut d = Engine::new().open(bytes.to_vec()).unwrap();
        let at = end(&mut d);
        tx(
            &mut d,
            json!([{"op":"insertEquation","at":at,"script":"a over b","size":1200,"color":255}]),
        );
        let eq = object_at(&mut d, &at);
        let first = query(&mut d, json!({"kind":"equation","object":eq}));
        assert_eq!(first["script"], "a over b");
        assert_eq!(first["size"], 1200);
        assert_eq!(first["color"], 255);
        assert_eq!(first["font"], "HYhwpEQ");
        assert!(first["width"].as_i64().unwrap() > 0 && first["height"].as_i64().unwrap() > 0);

        tx(
            &mut d,
            json!([{"op":"setEquation","object":eq,"patch":{"script":"a over b + c over d + e over f","size":2000}}]),
        );
        let grown = query(&mut d, json!({"kind":"equation","object":eq}));
        assert!(grown["width"].as_i64() > first["width"].as_i64(), "{grown}");
        assert!(
            grown["height"].as_i64() > first["height"].as_i64(),
            "{grown}"
        );
        tx(
            &mut d,
            json!([{"op":"setEquation","object":eq,"patch":{"baseline":40}}]),
        );
        let moved = query(&mut d, json!({"kind":"equation","object":eq}));
        assert_eq!(moved["baseline"], 40);
        assert_eq!(
            (&moved["width"], &moved["height"]),
            (&grown["width"], &grown["height"])
        );

        for format in ["hwp5", "hwpx"] {
            let mut r = reopen(&mut d, format);
            assert_eq!(
                query(&mut r, json!({"kind":"equation","object":eq})),
                moved,
                "{format}"
            );
        }
        d.undo().unwrap();
        assert_eq!(query(&mut d, json!({"kind":"equation","object":eq})), grown);
        d.undo().unwrap();
        d.undo().unwrap();
        assert!(
            d.query(&json!({"kind":"equation","object":eq}).to_string())
                .is_err()
        );
    }
}

#[test]
fn equation_requests_reject_bad_input_and_preview_reads_nothing() {
    let mut d = Engine::new().open(sources()[1].to_vec()).unwrap();
    let at = end(&mut d);
    let state = d.state().unwrap();
    for op in [
        json!({"op":"insertEquation","at":at,"script":"  "}),
        json!({"op":"insertEquation","at":at,"script":"x","size":5}),
        json!({"op":"insertEquation","at":at,"script":"x","color":0x0100_0000}),
    ] {
        assert!(d.transact(&json!({"ops":[op]}).to_string()).is_err());
    }
    let table = json!({"section":0,"para":0,"control":0});
    assert!(
        d.transact(
            &json!({"ops":[{"op":"setEquation","object":table,"patch":{"size":1000}}]}).to_string()
        )
        .is_err()
    );
    assert_eq!(d.state().unwrap(), state);
    let p = query(
        &mut d,
        json!({"kind":"equationPreview","script":"sqrt {x^2 + 1}","size":1000}),
    );
    assert!(p["svg"].as_str().unwrap().starts_with("<svg"));
    assert!(p["widthPx"].as_f64().unwrap() > 0.0);
    assert_eq!(d.state().unwrap(), state);
}

#[test]
fn table_captions_number_replace_undo_and_roundtrip() {
    for bytes in sources() {
        let mut d = Engine::new().open(bytes.to_vec()).unwrap();
        let t = table();
        assert!(d.query(&json!({"kind":"paragraphs","list":{"section":0,"steps":[{"para":0,"control":t["control"],"index":1}]}}).to_string()).is_err());
        tx(
            &mut d,
            json!([{"op":"setTableCaption","table":t,"text":"분기별 결과","withNumber":true}]),
        );
        let numbered = caption(&mut d, &t);
        assert_eq!(numbered.as_array().unwrap().len(), 1);
        let props = query(&mut d, json!({"kind":"tableProperties","table":t}));
        assert_eq!(props["caption"]["side"], "bottom");
        assert_eq!(props["caption"]["paragraphs"], 1);
        let text = numbered[0]["text"].as_str().unwrap();
        assert!(
            text.starts_with("표 ") && text.ends_with(" 분기별 결과"),
            "{text}"
        );
        assert_eq!(numbered[0]["controls"][0]["kind"], "autoNumber");
        assert_eq!(numbered[0]["controls"][0]["number"], 1);

        tx(
            &mut d,
            json!([{"op":"setTableCaption","table":t,"text":"새 제목\n둘째 줄","withNumber":true}]),
        );
        let replaced = caption(&mut d, &t);
        assert_eq!(replaced.as_array().unwrap().len(), 2);
        assert!(replaced[0]["text"].as_str().unwrap().ends_with(" 새 제목"));
        assert_eq!(replaced[1]["text"], "둘째 줄");

        for format in ["hwp5", "hwpx"] {
            let mut r = reopen(&mut d, format);
            let got = caption(&mut r, &t);
            assert_eq!(got[0]["text"], replaced[0]["text"], "{format}");
            assert_eq!(got[1]["text"], "둘째 줄", "{format}");
            assert_eq!(
                caption_numbers(&mut d, format),
                [(NumberKind::Table, 1)],
                "{format}"
            );
        }

        tx(
            &mut d,
            json!([{"op":"setTableCaption","table":t,"text":"번호 없음"}]),
        );
        let plain = caption(&mut d, &t);
        assert_eq!(plain[0]["text"], "번호 없음");
        assert!(plain[0]["controls"].as_array().unwrap().is_empty());
        d.undo().unwrap();
        assert_eq!(caption(&mut d, &t), replaced);
        d.undo().unwrap();
        assert_eq!(caption(&mut d, &t), numbered);
        d.undo().unwrap();
        assert!(d.query(&json!({"kind":"paragraphs","list":{"section":0,"steps":[{"para":0,"control":t["control"],"index":1}]}}).to_string()).is_err());
    }
}

#[test]
fn page_breaks_report_in_format_runs_and_roundtrip() {
    for bytes in sources() {
        let mut d = Engine::new().open(bytes.to_vec()).unwrap();
        let runs = |d: &mut Doc| {
            query(
                d,
                json!({"kind":"formatRuns","list":{"section":0},"from":0,"count":1}),
            )
        };
        assert_eq!(runs(&mut d)[0]["pageBreak"], false);
        let at = end(&mut d);
        tx(&mut d, json!([{"op":"pageBreak","at":at,"on":true}]));
        assert_eq!(runs(&mut d)[0]["pageBreak"], true);
        for format in ["hwp5", "hwpx"] {
            let mut r = reopen(&mut d, format);
            assert_eq!(runs(&mut r)[0]["pageBreak"], true, "{format}");
        }
        d.undo().unwrap();
        assert_eq!(runs(&mut d)[0]["pageBreak"], false);
    }
}
