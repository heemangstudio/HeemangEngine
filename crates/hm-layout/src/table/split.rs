//! Cutting a table across pages and columns.
//!
//! A fragment shows a horizontal band `[top, bottom)` of the table. Cuts
//! fall on row edges, or, when the table splits anywhere, inside a row at
//! a height no cell line straddles (a line's bottom, its spacing left
//! out). Splitting between rows does not cut cells that span them.
//! Header rows (rows of header cells at the top) repeat above each
//! continuation when the table asks for it.

use hm_model::Table;

use super::TableLayout;

/// How a table may break across pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitMode {
    Never,
    Rows,
    Anywhere,
}

impl SplitMode {
    pub fn of(t: &Table) -> Self {
        match t.split() {
            1 => SplitMode::Anywhere,
            2 => SplitMode::Rows,
            _ => SplitMode::Never,
        }
    }
}

/// Height of the header band repeated on continuation fragments: the rows
/// from the top whose cells are all header cells, when repeating is on.
pub fn header_height(t: &Table, tl: &TableLayout) -> i32 {
    if !t.repeat_header() {
        return 0;
    }
    let mut rows = 0usize;
    'rows: for r in 0..t.rows as usize {
        let mut any = false;
        for c in &t.cells {
            if c.row as usize <= r && (c.end_row() as usize) > r {
                if !c.header() || c.end_row() as usize > r + 1 {
                    break 'rows;
                }
                any = true;
            }
        }
        if !any {
            break;
        }
        rows = r + 1;
    }
    if rows == 0 || rows >= tl.row_y.len() {
        return 0;
    }
    tl.row_y[rows]
}

/// True when no cell spans across row edge `r` (the edge above row `r`).
fn clean_edge(t: &Table, r: usize) -> bool {
    t.cells
        .iter()
        .all(|c| !((c.row as usize) < r && c.end_row() as usize > r))
}

/// True when a cut at table height `y` would split a cell line.
fn straddles(tl: &TableLayout, y: i32) -> bool {
    tl.cells.iter().any(|cl| {
        if y <= cl.y || y >= cl.y + cl.h {
            return false;
        }
        let top = cl.content_y();
        cl.lines.iter().any(|l| {
            let h = cl.paras[l.para as usize].lines[l.line as usize].height;
            let ly = top + l.y;
            ly < y && y < ly + h
        })
    })
}

/// The lowest cut in `(from, limit]`, or `None` when none fits.
pub fn cut(t: &Table, tl: &TableLayout, mode: SplitMode, from: i32, limit: i32) -> Option<i32> {
    if limit >= tl.height {
        return Some(tl.height);
    }
    if mode == SplitMode::Never {
        return None;
    }
    let rows = tl.row_y.len() - 1;
    // Row edges first: the edge above row `r` sits at `row_y[r]`.
    let mut best = None;
    for r in 1..rows {
        let y = tl.row_y[r];
        let legal = clean_edge(t, r) || (mode == SplitMode::Anywhere && !straddles(tl, y));
        if y > from && y <= limit && legal {
            best = Some(y);
        }
    }
    if mode == SplitMode::Rows {
        return best;
    }
    // Inside the row the limit falls in: line bottoms no line straddles.
    let r = tl.row_y.partition_point(|&y| y <= limit).saturating_sub(1);
    let (row_top, row_bottom) = (tl.row_y[r], tl.row_y[(r + 1).min(rows)]);
    let mut inside = None;
    for cl in tl
        .cells
        .iter()
        .filter(|cl| cl.y < row_bottom && cl.y + cl.h > row_top)
    {
        let top = cl.content_y();
        for l in &cl.lines {
            let h = cl.paras[l.para as usize].lines[l.line as usize].height;
            let y = top + l.y + h;
            if y > from.max(row_top)
                && y <= limit
                && inside.is_none_or(|b| y > b)
                && best.is_none_or(|b| y > b)
                && !straddles(tl, y)
            {
                inside = Some(y);
            }
        }
    }
    inside.or(best)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::para::{Line, ParaLayout};
    use crate::table::{CellLayout, CellLine};
    use hm_model::Cell;

    /// A table of `rows` rows of 1000, one cell per row with lines of 300.
    fn table(rows: u16, mode: u32, span_first: bool) -> (Table, TableLayout) {
        let mut t = Table {
            rows,
            cols: 1,
            attr: mode,
            ..Table::default()
        };
        let mut tl = TableLayout {
            row_y: (0..=rows as i32).map(|r| r * 1000).collect(),
            height: rows as i32 * 1000,
            ..TableLayout::default()
        };
        for r in 0..rows {
            let span = if span_first && r == 0 { 2 } else { 1 };
            if span_first && r == 1 {
                continue;
            }
            t.cells.push(Cell {
                row: r,
                row_span: span,
                col_span: 1,
                ..Cell::default()
            });
            let lines = 3 * span as u32;
            tl.cells.push(CellLayout {
                y: r as i32 * 1000,
                h: span as i32 * 1000,
                paras: vec![ParaLayout {
                    lines: vec![
                        Line {
                            height: 300,
                            ..Line::default()
                        };
                        lines as usize
                    ],
                    ..ParaLayout::default()
                }],
                lines: (0..lines)
                    .map(|k| CellLine {
                        para: 0,
                        line: k,
                        y: k as i32 * 300,
                    })
                    .collect(),
                ..CellLayout::default()
            });
        }
        (t, tl)
    }

    #[test]
    fn row_splits_stop_at_row_edges() {
        let (t, tl) = table(4, 2, false);
        assert_eq!(cut(&t, &tl, SplitMode::of(&t), 0, 2500), Some(2000));
        assert_eq!(cut(&t, &tl, SplitMode::of(&t), 0, 999), None);
        // A spanning cell keeps its rows together.
        let (t, tl) = table(4, 2, true);
        assert_eq!(cut(&t, &tl, SplitMode::of(&t), 0, 1500), None);
    }

    #[test]
    fn splitting_anywhere_cuts_between_lines() {
        let (t, tl) = table(4, 1, false);
        // Inside the third row the last whole line ends at 2600.
        assert_eq!(cut(&t, &tl, SplitMode::of(&t), 0, 2750), Some(2600));
        // Never splitting only takes the whole table.
        let (t, tl) = table(4, 0, false);
        assert_eq!(cut(&t, &tl, SplitMode::of(&t), 0, 3999), None);
        assert_eq!(cut(&t, &tl, SplitMode::of(&t), 0, 4000), Some(4000));
    }
}
