//! Selections whose ends lie in different paragraph lists.
//!
//! Hancom selects such ranges by their nearest common list: an end inside a
//! table, text box or other control there takes the whole control. Two ends
//! in different cells of one table select the block of cells between them,
//! grown to take in merged cells it cuts.

use hm_edit::{ListPath, Pos};
use hm_model::{ControlKind, Document, Table, Unit};

/// A normalized selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Range {
    /// `[from, to)` in one list, `from` first.
    Text(Pos, Pos),
    /// A block of cells: rows and columns, both ends included.
    Cells {
        list: ListPath,
        para: u32,
        control: u32,
        rows: (u16, u16),
        cols: (u16, u16),
    },
}

/// The wire span `[start, end)` of extended control `index` in a paragraph.
pub fn control_span(p: &hm_model::Paragraph, index: u32) -> Option<(u32, u32)> {
    p.units().find_map(|(pos, u)| match u {
        Unit::Extended { code, index: i } if i == index => {
            Some((pos, pos + hm_edit::ops::unit_width(code)))
        }
        _ => None,
    })
}

fn table<'a>(doc: &'a Document, list: &ListPath, para: u32, control: u32) -> Option<&'a Table> {
    let p = hm_edit::ops::list(doc, list)?.get(para as usize)?;
    match &p.controls.get(control as usize)?.kind {
        ControlKind::Table(t) => Some(t),
        _ => None,
    }
}

/// Grows a block of cells until every cell it touches lies inside it.
pub fn grow(t: &Table, mut rows: (u16, u16), mut cols: (u16, u16)) -> ((u16, u16), (u16, u16)) {
    loop {
        let before = (rows, cols);
        for c in &t.cells {
            let (r0, r1) = (c.row, c.row + c.row_span.max(1) - 1);
            let (c0, c1) = (c.col, c.col + c.col_span.max(1) - 1);
            if r1 >= rows.0 && r0 <= rows.1 && c1 >= cols.0 && c0 <= cols.1 {
                rows = (rows.0.min(r0), rows.1.max(r1));
                cols = (cols.0.min(c0), cols.1.max(c1));
            }
        }
        if (rows, cols) == before {
            return (rows, cols);
        }
    }
}

/// The cells of a block, by index in the table.
pub fn block_cells(t: &Table, rows: (u16, u16), cols: (u16, u16)) -> Vec<usize> {
    (0..t.cells.len())
        .filter(|&i| {
            let c = &t.cells[i];
            c.row >= rows.0 && c.row <= rows.1 && c.col >= cols.0 && c.col <= cols.1
        })
        .collect()
}

/// A place in a list: paragraph and wire offset.
type Spot = (u32, u32);

/// The selection from `a` to `b` (either order).
pub fn normalize(doc: &Document, a: &Pos, b: &Pos) -> Result<Range, &'static str> {
    if a.list.section != b.list.section {
        return Err("a selection across sections");
    }
    if a.list == b.list {
        let (from, to) = if (b.para, b.offset) < (a.para, a.offset) {
            (b, a)
        } else {
            (a, b)
        };
        return Ok(Range::Text(from.clone(), to.clone()));
    }
    let (sa, sb) = (&a.list.steps, &b.list.steps);
    let k = sa.iter().zip(sb).take_while(|(x, y)| x == y).count();
    let common = ListPath {
        section: a.list.section,
        steps: sa[..k].to_vec(),
    };
    if let (Some(x), Some(y)) = (sa.get(k), sb.get(k))
        && (x.para, x.control) == (y.para, y.control)
        && let Some(t) = table(doc, &common, x.para, x.control)
        && let (Some(ca), Some(cb)) = (t.cells.get(x.index as usize), t.cells.get(y.index as usize))
    {
        let rows = (ca.row.min(cb.row), ca.row.max(cb.row));
        let cols = (ca.col.min(cb.col), ca.col.max(cb.col));
        let (rows, cols) = grow(t, rows, cols);
        return Ok(Range::Cells {
            list: common,
            para: x.para,
            control: x.control,
            rows,
            cols,
        });
    }
    let paras = hm_edit::ops::list(doc, &common).ok_or("no such list")?;
    // Each end as a span in the common list: a point, or a whole control.
    let span = |p: &Pos| -> Result<(Spot, Spot), &'static str> {
        match p.list.steps.get(k) {
            None => Ok(((p.para, p.offset), (p.para, p.offset))),
            Some(s) => {
                let para = paras.get(s.para as usize).ok_or("no such paragraph")?;
                let (lo, hi) = control_span(para, s.control).ok_or("no such control")?;
                Ok(((s.para, lo), (s.para, hi)))
            }
        }
    };
    let (a0, a1) = span(a)?;
    let (b0, b1) = span(b)?;
    let (from, to) = (a0.min(b0), a1.max(b1));
    let at = |(para, offset): (u32, u32)| Pos {
        list: common.clone(),
        para,
        offset,
    };
    Ok(Range::Text(at(from), at(to)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(row: u16, col: u16, row_span: u16, col_span: u16) -> hm_model::Cell {
        hm_model::Cell {
            row,
            col,
            row_span,
            col_span,
            ..Default::default()
        }
    }

    #[test]
    fn blocks_grow_to_take_in_merged_cells() {
        // A 3x3 grid whose top row's last two cells are merged, and whose
        // first column's lower two cells are merged.
        let t = Table {
            cells: vec![
                cell(0, 0, 1, 1),
                cell(0, 1, 1, 2),
                cell(1, 0, 2, 1),
                cell(1, 1, 1, 1),
                cell(1, 2, 1, 1),
                cell(2, 1, 1, 1),
                cell(2, 2, 1, 1),
            ],
            ..Default::default()
        };
        assert_eq!(grow(&t, (0, 0), (2, 2)), ((0, 0), (1, 2)));
        assert_eq!(grow(&t, (2, 2), (0, 0)), ((1, 2), (0, 0)));
        assert_eq!(block_cells(&t, (1, 2), (0, 1)), vec![2, 3, 5]);
    }
}
