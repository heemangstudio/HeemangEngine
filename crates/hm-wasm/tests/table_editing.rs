//! Structural table edits and typing, checked against what Hancom Office
//! 2022 stored for the same edits of the same Hancom-made 4x4 table
//! (`hancom-table-4x4.hwpx`; captures in the cleanroom's
//! `parity/notes/hancom-edit-captures/`).
use hm_wasm::{Doc, Engine};
use serde_json::{Value, json};

const TABLE: &[u8] = include_bytes!("fixtures/hancom-table-4x4.hwpx");

fn tx(d: &mut Doc, ops: Value) -> Value {
    serde_json::from_str(&d.transact(&json!({ "ops": ops }).to_string()).unwrap()).unwrap()
}
fn query(d: &mut Doc, q: Value) -> Value {
    serde_json::from_str(&d.query(&q.to_string()).unwrap()).unwrap()
}
fn open(bytes: Vec<u8>) -> Doc {
    Engine::new().open(bytes).unwrap()
}
/// The table control in the document's body.
fn address(d: &mut Doc) -> Value {
    let paras = query(d, json!({"kind":"paragraphs","list":{"section":0}}));
    for (para, p) in paras.as_array().unwrap().iter().enumerate() {
        if let Some(c) = p["controls"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["kind"] == "table")
        {
            return json!({"section":0,"para":para,"control":c["index"]});
        }
    }
    panic!("no table");
}
/// Rows, columns and each cell as `row,col rowSpan x colSpan width height [paragraphs]`.
fn summary(d: &mut Doc) -> (u64, u64, Vec<String>) {
    let table = address(d);
    let info = query(d, json!({"kind":"table","table":table}));
    let cells = info["cells"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            let props = query(
                d,
                json!({"kind":"cellProperties","table":table,"row":c["row"],"column":c["column"]}),
            );
            let paras = query(d, json!({"kind":"paragraphs","list":c["list"]}));
            let texts: Vec<String> = paras
                .as_array()
                .unwrap()
                .iter()
                .map(|p| p["text"].as_str().unwrap().to_string())
                .collect();
            format!(
                "{},{} {}x{} {} {} {:?}",
                c["row"],
                c["column"],
                c["rowSpan"],
                c["columnSpan"],
                props["width"],
                props["height"],
                texts
            )
        })
        .collect();
    (
        info["rows"].as_u64().unwrap(),
        info["columns"].as_u64().unwrap(),
        cells,
    )
}
fn cell<'a>(cells: &'a [String], at: &str) -> &'a str {
    cells
        .iter()
        .find(|c| c.starts_with(&format!("{at} ")))
        .unwrap()
}
fn table_height(d: &mut Doc) -> i64 {
    let table = address(d);
    query(d, json!({"kind":"objectProperties","object":table}))["height"]
        .as_i64()
        .unwrap()
}
fn reopened(d: &mut Doc, check: impl Fn(&mut Doc)) {
    for format in ["hwpx", "hwp5"] {
        let mut r = open(d.serialize_as(format, false).unwrap());
        check(&mut r);
    }
}
fn merge(d: &mut Doc, r0: u16, c0: u16, r1: u16, c1: u16) {
    let table = address(d);
    tx(
        d,
        json!([{"op":"mergeTableCells","table":table,"startRow":r0,"startColumn":c0,"endRow":r1,"endColumn":c1}]),
    );
}
fn split(d: &mut Doc, row: u16, column: u16, rows: u16, columns: u16) {
    let table = address(d);
    tx(
        d,
        json!([{"op":"splitTableCell","table":table,"row":row,"column":column,"rows":rows,"columns":columns,"equalRowHeight":false}]),
    );
}

#[test]
fn merging_keeps_the_text_of_non_empty_cells_and_drops_empty_ones() {
    // Hancom: merge-empty, merge-mixed, merge-multipara, merge-text.
    let mut d = open(TABLE.to_vec());
    merge(&mut d, 3, 0, 3, 1);
    merge(&mut d, 2, 0, 2, 1);
    merge(&mut d, 0, 2, 0, 3);
    let check = |d: &mut Doc| {
        let (_, _, cells) = summary(d);
        assert!(cell(&cells, "3,0").ends_with(r#"[""]"#), "{cells:?}");
        assert!(cell(&cells, "2,0").ends_with(r#"["A3"]"#), "{cells:?}");
        assert!(
            cell(&cells, "0,2").ends_with(r#"["C1", "D1", "D1-2"]"#),
            "{cells:?}"
        );
    };
    check(&mut d);
    reopened(&mut d, check);
    let mut e = open(TABLE.to_vec());
    merge(&mut e, 0, 0, 1, 1);
    let (_, _, cells) = summary(&mut e);
    assert!(
        cell(&cells, "0,0").ends_with(r#"["A1", "B1", "A2", "B2"]"#),
        "{cells:?}"
    );
}

#[test]
fn merging_empty_cells_leaves_the_table_height() {
    let mut d = open(TABLE.to_vec());
    let before = table_height(&mut d);
    merge(&mut d, 3, 0, 3, 1);
    assert_eq!(table_height(&mut d), before);
}

#[test]
fn splitting_rows_gives_the_first_piece_its_content_height_and_the_rest_what_remains() {
    // Hancom split-rows: B2 drawn 1282 high (stored 282) becomes 1282 + 0;
    // the cells beside it span both rows and keep their heights.
    let mut d = open(TABLE.to_vec());
    split(&mut d, 1, 1, 2, 1);
    let check = |d: &mut Doc| {
        let (rows, cols, cells) = summary(d);
        assert_eq!((rows, cols), (5, 4));
        assert!(
            cell(&cells, "1,1").starts_with("1,1 1x1 10488 1282 "),
            "{cells:?}"
        );
        assert!(
            cell(&cells, "2,1").starts_with("2,1 1x1 10488 0 "),
            "{cells:?}"
        );
        assert!(
            cell(&cells, "1,0").starts_with("1,0 2x1 10488 282 "),
            "{cells:?}"
        );
        assert!(
            cell(&cells, "1,3").starts_with("1,3 2x1 10488 282 "),
            "{cells:?}"
        );
    };
    check(&mut d);
    reopened(&mut d, check);
}

#[test]
fn splitting_a_merged_block_into_three_columns_adds_only_the_grid_lines_it_needs() {
    // Hancom split-block-merge: A1:B2 merged and split into 1x3 gives six
    // columns, the pieces 6992 wide, the lower rows' cells spanning two.
    let mut d = open(TABLE.to_vec());
    merge(&mut d, 0, 0, 1, 1);
    split(&mut d, 0, 0, 1, 3);
    let check = |d: &mut Doc| {
        let (rows, cols, cells) = summary(d);
        assert_eq!((rows, cols), (4, 6), "{cells:?}");
        assert!(
            cell(&cells, "0,0").starts_with("0,0 2x1 6992 "),
            "{cells:?}"
        );
        assert!(
            cell(&cells, "0,1").starts_with("0,1 2x2 6992 "),
            "{cells:?}"
        );
        assert!(
            cell(&cells, "0,3").starts_with("0,3 2x1 6992 "),
            "{cells:?}"
        );
        assert!(
            cell(&cells, "2,0").starts_with("2,0 1x2 10488 "),
            "{cells:?}"
        );
        assert!(
            cell(&cells, "2,2").starts_with("2,2 1x2 10488 "),
            "{cells:?}"
        );
        assert!(cell(&cells, "0,4").ends_with(r#"["C1"]"#), "{cells:?}");
    };
    check(&mut d);
    reopened(&mut d, check);
}

#[test]
fn splitting_a_merged_cell_along_its_rows_takes_the_drawn_row_heights() {
    // Hancom split-merged-cell: A3:B4 merged and split into 2x2.
    let mut d = open(TABLE.to_vec());
    merge(&mut d, 2, 0, 3, 1);
    split(&mut d, 2, 0, 2, 2);
    let (rows, cols, cells) = summary(&mut d);
    assert_eq!((rows, cols), (4, 4));
    for at in ["2,0", "2,1", "3,0", "3,1"] {
        assert!(
            cell(&cells, at).starts_with(&format!("{at} 1x1 10488 1282 ")),
            "{cells:?}"
        );
    }
    assert!(cell(&cells, "2,0").ends_with(r#"["A3"]"#));
}

#[test]
fn an_inserted_row_copies_the_spans_and_height_of_the_row_it_copies() {
    // Hancom ins-row-merged-corpus: a row added below a row of merged cells
    // repeats its column spans and its height.
    let mut d = open(TABLE.to_vec());
    merge(&mut d, 0, 0, 0, 1);
    let table = address(&mut d);
    tx(
        &mut d,
        json!([{"op":"insertTableRows","table":table,"beforeRow":1,"count":1,"copyRow":0}]),
    );
    let check = |d: &mut Doc| {
        let (rows, _, cells) = summary(d);
        assert_eq!(rows, 5);
        assert!(
            cell(&cells, "1,0").starts_with(r#"1,0 1x2 20976 282 [""]"#),
            "{cells:?}"
        );
        assert!(
            cell(&cells, "1,2").starts_with(r#"1,2 1x1 10488 282 [""]"#),
            "{cells:?}"
        );
        assert!(cell(&cells, "2,0").ends_with(r#"["A2"]"#), "{cells:?}");
    };
    check(&mut d);
    reopened(&mut d, check);
    // Hancom ins-row-merged-corpus: a row added inside cells spanning rows
    // widens their span and keeps their stored height.
    let mut f = open(TABLE.to_vec());
    merge(&mut f, 1, 0, 2, 0);
    let (_, _, cells) = summary(&mut f);
    let before = cell(&cells, "1,0").split(' ').nth(3).unwrap().to_string();
    let table = address(&mut f);
    tx(
        &mut f,
        json!([{"op":"insertTableRows","table":table,"beforeRow":2,"count":1,"copyRow":1}]),
    );
    let (_, _, cells) = summary(&mut f);
    assert!(
        cell(&cells, "1,0").starts_with(&format!("1,0 3x1 10488 {before} ")),
        "{cells:?}"
    );
    // Without a row to copy, the row at the insertion point is the template.
    let mut e = open(TABLE.to_vec());
    merge(&mut e, 0, 0, 0, 1);
    let table = address(&mut e);
    tx(
        &mut e,
        json!([{"op":"insertTableRows","table":table,"beforeRow":1,"count":1}]),
    );
    let (_, _, cells) = summary(&mut e);
    assert!(cell(&cells, "1,1").starts_with("1,1 1x1 "), "{cells:?}");
}

#[test]
fn typing_saves_the_grown_table_height_and_keeps_cell_heights() {
    // Hancom type-grow / shrink-after-grow: the table's stored height follows
    // the rows as drawn; the cell keeps its stored 282.
    let mut d = open(TABLE.to_vec());
    let table = address(&mut d);
    let base = table_height(&mut d);
    let info = query(&mut d, json!({"kind":"table","table":table}));
    let b2 = info["cells"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["row"] == 1 && c["column"] == 1)
        .unwrap()
        .clone();
    let at = json!({"section":0,"steps":b2["list"]["steps"],"para":0,"offset":2});
    let text = "가나다라마바사아자차카타파하 ".repeat(3);
    tx(&mut d, json!([{"op":"insertText","at":at,"text":text}]));
    let grown = |d: &mut Doc| {
        let h = table_height(d);
        let (_, _, cells) = summary(d);
        assert!(
            cell(&cells, "1,1").starts_with("1,1 1x1 10488 282 "),
            "{cells:?}"
        );
        h
    };
    // Hancom stored 13128 for this text (five lines in B2).
    reopened(&mut d, |r| {
        assert_eq!(grown(r), 13128, "the saved table height grows")
    });
    tx(
        &mut d,
        json!([{"op":"delete","from":at,"to":{"section":0,"steps":b2["list"]["steps"],"para":0,"offset":2 + text.encode_utf16().count()}}]),
    );
    reopened(&mut d, |r| {
        assert_eq!(grown(r), base, "deleting the text shrinks it back")
    });
}
