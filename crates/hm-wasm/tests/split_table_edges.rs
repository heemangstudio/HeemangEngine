//! Edges where a table row is cut across pages, as Hancom draws them
//! (heemang-cleanroom parity/notes/formatting-20261104/T2/split: row 3 of a
//! 4x4 table is taller than a page; its cells have a red 1 mm top and a blue
//! 1 mm bottom, the table's split line is a red 0.4 mm dashed line).
use hm_wasm::Engine;

fn red_lines(svg: &str) -> Vec<(i64, bool)> {
    svg.split("<line ")
        .skip(1)
        .filter(|l| l.contains("stroke=\"#ff0000\""))
        .map(|l| {
            let y = l.split("y1=\"").nth(1).unwrap().split('"').next().unwrap();
            (
                y.parse::<f64>().unwrap() as i64,
                l.contains("stroke-dasharray"),
            )
        })
        .collect()
}

#[test]
fn a_tables_split_line_closes_both_sides_of_a_cut() {
    let engine = Engine::new();
    let mut doc = engine
        .open(include_bytes!("fixtures/split-row-splitline.hwpx").to_vec())
        .unwrap();
    // Page 1 ends the cut row with the dashed split line, not its blue bottom.
    let first = doc.page_svg(0, 72.0).unwrap();
    assert!(!first.contains("#0000ff"));
    let lowest = red_lines(&first).into_iter().max().unwrap();
    assert!(lowest.1, "page 1 bottom is the split line");
    // Page 2 starts it with the split line, not its red 1 mm top.
    let second = doc.page_svg(1, 72.0).unwrap();
    let lines = red_lines(&second);
    assert!(!lines.is_empty() && lines.iter().all(|l| l.1));
}

#[test]
fn repeated_header_rows_close_a_cut_row_with_their_own_bottom() {
    let engine = Engine::new();
    let mut doc = engine
        .open(include_bytes!("fixtures/split-row-header.hwpx").to_vec())
        .unwrap();
    // Without a split line page 1 closes the cut row with its own bottom.
    assert!(doc.page_svg(0, 72.0).unwrap().contains("#0000ff"));
    // On page 2 the header's black bottom stands above the rest of the row:
    // its red top is not drawn again.
    assert!(red_lines(&doc.page_svg(1, 72.0).unwrap()).is_empty());
}
