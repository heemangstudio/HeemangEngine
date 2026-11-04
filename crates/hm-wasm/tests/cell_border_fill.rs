//! Cell border and fill edits store what Hancom Office 2022 stores for the
//! same edit on `hancom-table-4x4.hwpx` (COM captures in
//! heemang-cleanroom/parity/notes/formatting-20261104/T/captures).
use hm_wasm::{Doc, Engine};
use serde_json::{Value, json};

fn open() -> (Doc, Value) {
    let mut d = Engine::new()
        .open(include_bytes!("fixtures/hancom-table-4x4.hwpx").to_vec())
        .unwrap();
    let paras = query(&mut d, json!({"kind":"paragraphs","list":{"section":0}}));
    let table = paras
        .as_array()
        .unwrap()
        .iter()
        .find_map(|p| {
            p["controls"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["kind"] == "table")
                .map(|c| json!({"section":0,"para":p["index"],"control":c["index"]}))
        })
        .unwrap();
    (d, table)
}
fn query(d: &mut Doc, v: Value) -> Value {
    serde_json::from_str(&d.query(&v.to_string()).unwrap()).unwrap()
}
fn tx(d: &mut Doc, ops: Value) {
    d.transact(&json!({ "ops": ops }).to_string()).unwrap();
}
fn cell(d: &mut Doc, t: &Value, row: u16, column: u16) -> Value {
    query(
        d,
        json!({"kind":"cellProperties","table":t,"row":row,"column":column}),
    )["borderFill"]
        .clone()
}
/// Border fill ids of the 4x4 grid, row by row.
fn ids(d: &mut Doc, t: &Value) -> Vec<Vec<u64>> {
    (0..4)
        .map(|r| {
            (0..4)
                .map(|c| cell(d, t, r, c)["id"].as_u64().unwrap())
                .collect()
        })
        .collect()
}
/// A side as (style, width index, color).
fn side(b: &Value, name: &str) -> (i64, i64, i64) {
    let l = &b[name];
    (
        l["style"].as_i64().unwrap(),
        l["widthIndex"].as_i64().unwrap(),
        l["color"].as_i64().unwrap(),
    )
}
fn block(t: &Value, patch: Value, inner: Value) -> Value {
    let mut op = json!({"op":"setCellBorderFill","table":t,"startRow":1,"startColumn":1,"endRow":2,"endColumn":2,"borderFill":patch});
    for (k, v) in inner.as_object().unwrap() {
        op[k] = v.clone();
    }
    op
}
const SOLID: (i64, i64, i64) = (1, 1, 0);

#[test]
fn block_sides_are_its_outer_edges_and_inner_lines_are_separate() {
    let none = json!({"style":0,"width":1,"color":0});
    // e-block-each-left-none: only the block's left edge and the cells left of it.
    let (mut d, t) = open();
    tx(&mut d, json!([block(&t, json!({"left":none}), json!({}))]));
    assert_eq!(
        ids(&mut d, &t),
        [[3, 3, 3, 3], [4, 5, 3, 3], [4, 5, 3, 3], [3, 3, 3, 3]]
    );
    assert_eq!(side(&cell(&mut d, &t, 1, 0), "right"), (0, 1, 0));
    assert_eq!(side(&cell(&mut d, &t, 1, 1), "left"), (0, 1, 0));
    assert_eq!(side(&cell(&mut d, &t, 1, 2), "left"), SOLID);
    // f-block-each-horz-none: the inner horizontal line only.
    let (mut d, t) = open();
    tx(
        &mut d,
        json!([block(&t, json!({}), json!({"horizontal":none}))]),
    );
    assert_eq!(
        ids(&mut d, &t),
        [[3, 3, 3, 3], [3, 4, 4, 3], [3, 5, 5, 3], [3, 3, 3, 3]]
    );
    assert_eq!(side(&cell(&mut d, &t, 1, 1), "bottom"), (0, 1, 0));
    assert_eq!(side(&cell(&mut d, &t, 2, 2), "top"), (0, 1, 0));
    assert_eq!(side(&cell(&mut d, &t, 2, 2), "bottom"), SOLID);
    // r2-block-each-outer-red: per cell, the neighbors beyond its outer edges
    // take new border fills before the cell does.
    let (mut d, t) = open();
    let red = json!({"style":1,"width":4,"color":255});
    tx(
        &mut d,
        json!([block(
            &t,
            json!({"left":red,"right":red,"top":red,"bottom":red}),
            json!({})
        )]),
    );
    assert_eq!(
        ids(&mut d, &t),
        [[3, 5, 5, 3], [4, 6, 8, 7], [4, 10, 11, 7], [3, 9, 9, 3]]
    );
    assert_eq!(side(&cell(&mut d, &t, 2, 2), "right"), (1, 4, 255));
    assert_eq!(side(&cell(&mut d, &t, 2, 2), "top"), SOLID);
    // Fill and diagonals go to every cell of the block (b-block-each-solid).
    let (mut d, t) = open();
    tx(
        &mut d,
        json!([block(
            &t,
            json!({"fill":{"kind":"solid","background":255},"backslash":2}),
            json!({})
        )]),
    );
    assert_eq!(
        ids(&mut d, &t),
        [[3, 3, 3, 3], [3, 4, 4, 3], [3, 4, 4, 3], [3, 3, 3, 3]]
    );
}

#[test]
fn a_cell_side_change_numbers_the_neighbors_first() {
    // g-cell-top-red-thick: B1 (bottom) before B2.
    let (mut d, t) = open();
    let red = json!({"style":1,"width":6,"color":255});
    tx(
        &mut d,
        json!([{"op":"setCellProperties","table":t,"row":1,"column":1,"patch":{"borderFill":{"top":red}}}]),
    );
    assert_eq!(
        ids(&mut d, &t),
        [[3, 4, 3, 3], [3, 5, 3, 3], [3, 3, 3, 3], [3, 3, 3, 3]]
    );
    // x6-cell-top-right: the right neighbor, then the top one, then the cell.
    let (mut d, t) = open();
    let none = json!({"style":0,"width":1,"color":0});
    let red = json!({"style":1,"width":1,"color":255});
    tx(
        &mut d,
        json!([{"op":"setCellProperties","table":t,"row":1,"column":1,"patch":{"borderFill":{"top":red,"right":none}}}]),
    );
    assert_eq!(
        ids(&mut d, &t),
        [[3, 5, 3, 3], [3, 6, 4, 3], [3, 3, 3, 3], [3, 3, 3, 3]]
    );
}

#[test]
fn a_new_cell_zone_starts_from_hancoms_empty_zone() {
    // v5-zone-diag: no sides at 0.12 mm and a 0.1 mm solid diagonal line.
    let (mut d, t) = open();
    let zone = |t: &Value, r0: u16, c0: u16, r1: u16, c1: u16, b: Value| json!({"op":"setCellZone","table":t,"startRow":r0,"startColumn":c0,"endRow":r1,"endColumn":c1,"borderFill":b});
    tx(
        &mut d,
        json!([zone(&t, 1, 1, 2, 2, json!({"backslash":2}))]),
    );
    let zones =
        |d: &mut Doc| query(d, json!({"kind":"tableProperties","table":t}))["zones"].clone();
    let z = zones(&mut d);
    let b = &z[0]["borderFill"];
    for s in ["left", "right", "top", "bottom"] {
        assert_eq!(side(b, s), (0, 1, 0), "{s}");
    }
    assert_eq!(side(b, "diagonal"), (1, 0, 0));
    assert_eq!(
        (b["backslash"].as_u64(), b["slash"].as_u64()),
        (Some(2), Some(0))
    );
    assert_eq!(b["fill"]["kind"], 0);
    assert!(b["fill"]["solid"].is_null());
    // w10-zone-twice: the same block's zone takes the next patch.
    tx(&mut d, json!([zone(&t, 1, 1, 2, 2, json!({"slash":2}))]));
    let z = zones(&mut d);
    assert_eq!(z.as_array().unwrap().len(), 1);
    assert_eq!(
        (
            z[0]["borderFill"]["slash"].as_u64(),
            z[0]["borderFill"]["backslash"].as_u64()
        ),
        (Some(2), Some(2))
    );
    // w14-zone-overlap: another block adds a zone over the first.
    tx(&mut d, json!([zone(&t, 2, 2, 3, 3, json!({"slash":2}))]));
    assert_eq!(zones(&mut d).as_array().unwrap().len(), 2);
    // The cells keep their own border fills.
    assert_eq!(ids(&mut d, &t), vec![vec![3; 4]; 4]);
}

#[test]
fn a_color_fill_stores_no_hatch_as_hancom_and_reuses_its_border_fill() {
    let (mut d, t) = open();
    let red = json!({"fill":{"kind":"solid","background":255,"patternColor":0,"pattern":0}});
    tx(
        &mut d,
        json!([{"op":"setCellProperties","table":t,"row":1,"column":1,"patch":{"borderFill":red}}]),
    );
    assert_eq!(cell(&mut d, &t, 1, 1)["fill"]["solid"]["pattern"], -1);
    // Reopened from either format, the same fill on another cell is the same
    // border fill (a3-cell-solid-stylem1, w12-cell-solid-hwp).
    for format in ["hwpx", "hwp5"] {
        let bytes = d.serialize_as(format, false).unwrap();
        let mut r = Engine::new().open(bytes).unwrap();
        let id = cell(&mut r, &t, 1, 1)["id"].clone();
        tx(
            &mut r,
            json!([{"op":"setCellProperties","table":t,"row":2,"column":2,"patch":{"borderFill":red}}]),
        );
        assert_eq!(cell(&mut r, &t, 2, 2)["id"], id, "{format}");
    }
    // Back to no fill takes the table's plain border fill again (n-cell-none-after-solid).
    tx(
        &mut d,
        json!([{"op":"setCellProperties","table":t,"row":1,"column":1,"patch":{"borderFill":{"fill":{"kind":"none"}}}}]),
    );
    assert_eq!(cell(&mut d, &t, 1, 1)["id"], 3);
}

#[test]
fn bent_and_turned_diagonals_save_as_hancom() {
    let (mut d, t) = open();
    let set = |d: &mut Doc, column: u16, b: Value| {
        tx(
            d,
            json!([{"op":"setCellProperties","table":t,"row":1,"column":column,"patch":{"borderFill":b}}]),
        );
    };
    // y-bs5: a bent backslash is the 2 of the slash's Crooked (HWP 5 bit 9).
    set(&mut d, 0, json!({"backslash":2,"backslashCrooked":true}));
    // y-slK: a turned slash to the bottom edge.
    set(&mut d, 1, json!({"slash":3,"slashCounter":true}));
    // A diagonal after a center line replaces it.
    set(&mut d, 2, json!({"centerLine":"vertical"}));
    set(&mut d, 2, json!({"slash":2}));
    let c = cell(&mut d, &t, 1, 2);
    assert_eq!(
        (c["centerLine"].as_str(), c["slash"].as_u64()),
        (Some("none"), Some(2))
    );
    assert_eq!(c["slashCrooked"], false);
    let saved = d.serialize_as("hwpx", false).unwrap();
    let xml = hm_hwpx::zip::Archive::open(saved.into())
        .unwrap()
        .read_name("Contents/header.xml")
        .unwrap()
        .unwrap();
    let xml = String::from_utf8_lossy(&xml);
    assert!(xml.contains(r#"<hh:slash type="NONE" Crooked="2" isCounter="0"/><hh:backSlash type="CENTER" Crooked="0" isCounter="0"/>"#));
    assert!(xml.contains(r#"<hh:slash type="CENTER_BELOW" Crooked="0" isCounter="1"/><hh:backSlash type="NONE" Crooked="0" isCounter="0"/>"#));
    for format in ["hwpx", "hwp5"] {
        let mut r = Engine::new()
            .open(d.serialize_as(format, false).unwrap())
            .unwrap();
        let b = cell(&mut r, &t, 1, 0);
        assert_eq!(
            (b["backslash"].as_u64(), b["backslashCrooked"].as_bool()),
            (Some(2), Some(true)),
            "{format}"
        );
        let b = cell(&mut r, &t, 1, 1);
        assert_eq!(
            (b["slash"].as_u64(), b["slashCounter"].as_bool()),
            (Some(3), Some(true)),
            "{format}"
        );
    }
}
