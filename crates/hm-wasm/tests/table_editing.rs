//! Structural table edits and typing, checked against what Hancom Office
//! 2022 stored for the same edits of the same Hancom-made 4x4 table
//! (`hancom-table-4x4.hwpx`).
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
fn table_width(d: &mut Doc) -> i64 {
    let table = address(d);
    query(d, json!({"kind":"objectProperties","object":table}))["width"]
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

#[test]
fn moving_one_cell_edge_adds_a_grid_line_and_keeps_the_table_width() {
    // Hancom resize-shift-right: F5 on B2 and Shift+Right three times widen
    // B2 by 849 and narrow C2 by as much; the other rows keep their cells, a
    // grid line is added and the table stays 41952 wide.
    let mut d = open(TABLE.to_vec());
    let table = address(&mut d);
    for _ in 0..3 {
        tx(
            &mut d,
            json!([{"op":"moveTableCellEdge","table":table,"row":1,"column":1,"side":"right","delta":283}]),
        );
    }
    let check = |d: &mut Doc| {
        let (rows, cols, cells) = summary(d);
        assert_eq!((rows, cols), (4, 5), "{cells:?}");
        assert!(
            cell(&cells, "1,1").starts_with(r#"1,1 1x2 11337 282 ["B2"]"#),
            "{cells:?}"
        );
        assert!(
            cell(&cells, "1,3").starts_with(r#"1,3 1x1 9639 282 ["C2"]"#),
            "{cells:?}"
        );
        assert!(
            cell(&cells, "0,2").starts_with(r#"0,2 1x2 10488 282 ["C1"]"#),
            "{cells:?}"
        );
        assert!(
            cell(&cells, "0,1").starts_with(r#"0,1 1x1 10488 "#),
            "{cells:?}"
        );
        assert_eq!(table_width(d), 41952);
    };
    check(&mut d);
    reopened(&mut d, check);
    // Moving it back takes the added line away again.
    let table = address(&mut d);
    tx(
        &mut d,
        json!([{"op":"moveTableCellEdge","table":table,"row":1,"column":1,"side":"right","delta":-849}]),
    );
    let (rows, cols, cells) = summary(&mut d);
    assert_eq!((rows, cols), (4, 4), "{cells:?}");
    assert!(
        cell(&cells, "1,1").starts_with("1,1 1x1 10488 "),
        "{cells:?}"
    );
    // A cell on the table's right border has nothing across its edge.
    let table = address(&mut d);
    assert!(
        d.transact(
            &json!({"ops":[{"op":"moveTableCellEdge","table":table,"row":1,"column":3,"side":"right","delta":283}]})
                .to_string()
        )
        .is_err()
    );
}

/// A merge leaving a column no cell starts in removes it, as Hancom does (formatting
/// V/probes/p3 b6, b8): the 4x4 table merged whole is 1x1, and merging A1:B4 leaves three
/// columns, the old C and D now second and third. Both formats keep that grid.
#[test]
fn a_merge_leaving_a_column_without_cells_removes_it() {
    let mut d = open(TABLE.to_vec());
    merge(&mut d, 0, 0, 3, 3);
    let (rows, columns, cells) = summary(&mut d);
    assert_eq!((rows, columns, cells.len()), (1, 1, 1));
    assert!(cells[0].starts_with("0,0 1x1"));
    let mut d = open(TABLE.to_vec());
    merge(&mut d, 0, 0, 3, 1);
    let shape = |d: &mut Doc| {
        let (rows, columns, cells) = summary(d);
        let spans: Vec<String> = cells
            .iter()
            .map(|c| c.split(' ').take(2).collect::<Vec<_>>().join(" "))
            .collect();
        (rows, columns, spans)
    };
    let want = shape(&mut d);
    assert_eq!((want.0, want.1), (4, 3));
    assert!(want.2.contains(&"0,0 4x1".to_string()) && want.2.contains(&"3,2 1x1".to_string()));
    for format in ["hwpx", "hwp5"] {
        let mut reopened = open(d.serialize_as(format, false).unwrap());
        assert_eq!(shape(&mut reopened), want);
    }
}

/// A merged cell keeps the top left cell's border fill but takes the bottom right cell's right
/// and bottom sides, as Hancom merges (formatting V/probes/p8 m1-m10): inner sides and the
/// other cells' outer sides go.
#[test]
fn a_merged_cell_takes_its_right_and_bottom_sides_from_the_bottom_right_cell() {
    const RED: u32 = 0x0000FF;
    const BLUE: u32 = 0xFF0000;
    let set = |d: &mut Doc, row: u16, column: u16, key: &str, color: u32| {
        let table = address(d);
        tx(
            d,
            json!([{"op":"setCellProperties","table":table,"row":row,"column":column,
                    "patch":{"borderFill":{key:{"style":1,"width":6,"color":color}}}}]),
        );
    };
    let sides = |d: &mut Doc| {
        let table = address(d);
        let c = query(
            d,
            json!({"kind":"cellProperties","table":table,"row":1,"column":1}),
        );
        ["left", "right", "top", "bottom"].map(|k| c["borderFill"][k]["color"].as_u64().unwrap())
    };
    let mut d = open(TABLE.to_vec());
    set(&mut d, 1, 1, "bottom", RED); // B2's inner bottom
    set(&mut d, 2, 1, "left", RED); // B3's left
    set(&mut d, 2, 2, "right", BLUE); // C3's right
    set(&mut d, 1, 2, "top", RED); // C2's top
    let before = sides(&mut d);
    merge(&mut d, 1, 1, 2, 2);
    let after = sides(&mut d);
    assert_eq!(after[0], before[0]); // B2's left
    assert_eq!(after[2], before[2]); // B2's top
    assert_eq!(after[1], BLUE as u64); // C3's right
    assert_ne!(after[3], RED as u64); // C3's bottom, not B2's inner one
}

/// Hancom's 셀 테두리 on one cell carries a changed side over to the neighbors along it only
/// when all of them lie within that side (formatting V/probes/p3 b1-b4, p6 c5-c7): under a row
/// merged across the table, a cell's top leaves the merged cell's bottom; a merged cell's bottom
/// over two single cells changes both; next to one neighbor reaching past it, none changes.
#[test]
fn a_cell_side_carries_over_only_to_neighbors_within_it() {
    const RED: u32 = 0x0000FF;
    let red = json!({"style":1,"width":6,"color":RED});
    let side = |d: &mut Doc, row: u16, column: u16, key: &str| {
        let table = address(d);
        let c = query(
            d,
            json!({"kind":"cellProperties","table":table,"row":row,"column":column}),
        );
        c["borderFill"][key]["color"] == json!(RED)
    };
    let set = |d: &mut Doc, row: u16, column: u16, key: &str| {
        let table = address(d);
        tx(
            d,
            json!([{"op":"setCellProperties","table":table,"row":row,"column":column,
                    "patch":{"borderFill":{key:red}}}]),
        );
    };
    // B2's top under A1:D1.
    let mut d = open(TABLE.to_vec());
    merge(&mut d, 0, 0, 0, 3);
    set(&mut d, 1, 1, "top");
    assert!(side(&mut d, 1, 1, "top"));
    assert!(!side(&mut d, 0, 0, "bottom"));
    // A1:B1's bottom over A2 and B2.
    let mut d = open(TABLE.to_vec());
    merge(&mut d, 0, 0, 0, 1);
    set(&mut d, 0, 0, "bottom");
    assert!(side(&mut d, 1, 0, "top") && side(&mut d, 1, 1, "top"));
    // C2:D2's top under B1:C1 (reaching past it) and D1 (within it).
    let mut d = open(TABLE.to_vec());
    merge(&mut d, 0, 1, 0, 2);
    merge(&mut d, 1, 2, 1, 3);
    set(&mut d, 1, 2, "top");
    assert!(side(&mut d, 1, 2, "top"));
    assert!(!side(&mut d, 0, 1, "bottom") && !side(&mut d, 0, 3, "bottom"));
}

/// 각 셀마다 over a cell block carries its outer sides over to each neighbor lying within the
/// block's edge (formatting V/probes/p6): a merged cell exactly over the block takes the block's
/// top, one reaching past it keeps its line, and of two neighbors the one within takes it.
#[test]
fn a_block_side_carries_over_to_the_neighbors_within_the_block_edge() {
    const RED: u32 = 0x0000FF;
    let red = json!({"style":1,"width":6,"color":RED});
    let bottom_red = |d: &mut Doc, row: u16, column: u16| {
        let table = address(d);
        let c = query(
            d,
            json!({"kind":"cellProperties","table":table,"row":row,"column":column}),
        );
        c["borderFill"]["bottom"]["color"] == json!(RED)
    };
    let block_top = |d: &mut Doc, c0: u16, c1: u16| {
        let table = address(d);
        tx(
            d,
            json!([{"op":"setCellBorderFill","table":table,"startRow":1,"startColumn":c0,
                    "endRow":1,"endColumn":c1,"borderFill":{"top":red}}]),
        );
    };
    // B1:C1 over the block B2:C2 (c3).
    let mut d = open(TABLE.to_vec());
    merge(&mut d, 0, 1, 0, 2);
    block_top(&mut d, 1, 2);
    assert!(bottom_red(&mut d, 0, 1));
    // A1:D1 over B2:C2 (c2).
    let mut d = open(TABLE.to_vec());
    merge(&mut d, 0, 0, 0, 3);
    block_top(&mut d, 1, 2);
    assert!(!bottom_red(&mut d, 0, 0));
    // B1:C1 and D1 over C2:D2 (c4).
    let mut d = open(TABLE.to_vec());
    merge(&mut d, 0, 1, 0, 2);
    block_top(&mut d, 2, 3);
    assert!(!bottom_red(&mut d, 0, 1) && bottom_red(&mut d, 0, 3));
}

/// Cell zones after a merge, as Hancom stores them (formatting V2/probes p11, pS, pZ5): a merge
/// inside a zone keeps it, ending it at the merged cell when it covers the zone's end cell; a
/// merge across a zone's edge removes that zone and the last one, and the check goes on from
/// the removed zone's place.
#[test]
fn a_merge_across_a_zone_removes_it_and_the_last_zone() {
    let zoned = || {
        let mut d = open(TABLE.to_vec());
        let t = address(&mut d);
        let zone = |(r0, c0, r1, c1): (u16, u16, u16, u16)| {
            json!({"op":"setCellZone","table":t,"startRow":r0,"startColumn":c0,"endRow":r1,
                   "endColumn":c1,"borderFill":{"fill":{"kind":"solid","background":0xFF}}})
        };
        let zones = [(0, 0, 1, 1), (0, 3, 1, 3), (3, 0, 3, 1), (3, 2, 3, 3)];
        tx(&mut d, Value::Array(zones.into_iter().map(zone).collect()));
        d
    };
    let zones = |d: &mut Doc| -> Vec<[u64; 4]> {
        let t = address(d);
        query(d, json!({"kind":"tableProperties","table":t}))["zones"]
            .as_array()
            .unwrap()
            .iter()
            .map(|z| ["startRow", "startColumn", "endRow", "endColumn"].map(|k| z[k].as_u64().unwrap()))
            .collect()
    };
    let mut d = zoned();
    assert_eq!(zones(&mut d), [[0, 0, 1, 1], [0, 3, 1, 3], [3, 0, 3, 1], [3, 2, 3, 3]]);
    // Across the first zone's right edge: it and the last go (pZ5 q1).
    merge(&mut d, 0, 1, 0, 2);
    assert_eq!(zones(&mut d), [[0, 3, 1, 3], [3, 0, 3, 1]]);
    // The last zone merged whole ends at the merged cell (p11 z06).
    let mut d = zoned();
    merge(&mut d, 3, 2, 3, 3);
    assert_eq!(zones(&mut d)[3], [3, 2, 3, 2]);
    // Across the first two: the check goes on from the first one's place (pZ5 q6).
    let mut d = zoned();
    merge(&mut d, 0, 0, 0, 3);
    assert_eq!(zones(&mut d), Vec::<[u64; 4]>::new());
}
