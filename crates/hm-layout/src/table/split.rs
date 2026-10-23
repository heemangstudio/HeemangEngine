//! Cutting a table across pages and columns.
//!
//! A fragment shows a horizontal band `[top, bottom)` of the table. Cuts
//! fall on row edges, or, when the table splits anywhere, also inside rows
//! at a height no cell line straddles (a line's bottom, its spacing left
//! out). Splitting between rows does not cut cells that span them.
//! Header rows (rows of header cells at the top) repeat above each
//! continuation when the table asks for it.
//!
//! The legal cuts of a table are found once, in one pass over its cells
//! and lines, so placing a long table costs a search per fragment.

use hm_model::Table;

use super::TableLayout;

/// How a table may break across pages: never, between rows only (split
/// mode 1, HWPX `TABLE`), or inside cells too (mode 2, `CELL`, which
/// Hancom's stored layouts show cutting cells' lines across pages).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitMode {
    Never,
    Rows,
    Anywhere,
}

impl SplitMode {
    pub fn of(t: &Table) -> Self {
        match t.split() {
            1 => SplitMode::Rows,
            2 => SplitMode::Anywhere,
            _ => SplitMode::Never,
        }
    }
}

/// The heights a table may be cut at, ascending, table end excluded.
#[derive(Clone, Debug, Default)]
pub struct Cuts {
    /// Row edges no cell spans.
    rows: Vec<i32>,
    /// Those, row edges no line straddles, and line bottoms no line
    /// straddles.
    anywhere: Vec<i32>,
}

impl Cuts {
    pub fn new(t: &Table, tl: &TableLayout) -> Self {
        let rows = tl.row_y.len().saturating_sub(1);
        // Cells spanning edge `r` (the edge above row `r`), by a running sum.
        let mut crossing = vec![0i32; rows + 1];
        for c in &t.cells {
            let (top, end) = (c.row as usize + 1, (c.end_row() as usize).min(rows));
            if top < end {
                crossing[top] += 1;
                crossing[end] -= 1;
            }
        }
        let lines = Straddled::new(tl);
        let within = |y: i32| 0 < y && y < tl.height;
        let mut out = Cuts::default();
        let mut open = crossing[0];
        for (r, &y) in tl.row_y.iter().enumerate().take(rows).skip(1) {
            open += crossing[r];
            let clean = open == 0;
            if !within(y) {
                continue;
            }
            if clean {
                out.rows.push(y);
            }
            if clean || !lines.at(y) {
                out.anywhere.push(y);
            }
        }
        for cl in &tl.cells {
            for (_, y) in spans(cl) {
                if y <= cl.y + cl.h && within(y) && !lines.at(y) {
                    out.anywhere.push(y);
                }
            }
        }
        out.rows.dedup();
        out.anywhere.sort_unstable();
        out.anywhere.dedup();
        out
    }

    fn of(&self, mode: SplitMode) -> &[i32] {
        match mode {
            SplitMode::Never => &[],
            SplitMode::Rows => &self.rows,
            SplitMode::Anywhere => &self.anywhere,
        }
    }
}

/// What a cell's content occupies in table heights, `(top, bottom)`: its
/// lines and the tables flowing among them, which no cut may pass through.
fn spans(cl: &super::CellLayout) -> Vec<(i32, i32)> {
    let top = cl.content_y();
    let mut out: Vec<(i32, i32)> = cl
        .lines
        .iter()
        .map(|l| (top + l.y, top + l.y + line_height(cl, l)))
        .collect();
    out.extend(cl.tables.iter().map(|t| (top + t.y, top + t.y + t.height)));
    out
}

/// [`spans`] sorted by top.
pub(crate) fn sorted_spans(cl: &super::CellLayout) -> Vec<(i32, i32)> {
    let mut v = spans(cl);
    v.sort_unstable();
    v
}

fn line_height(cl: &super::CellLayout, l: &super::CellLine) -> i32 {
    cl.paras
        .get(l.para as usize)
        .and_then(|p| p.lines.get(l.line as usize))
        .map_or(0, |ln| ln.height)
}

/// The open spans `(top, bottom)` of the cell lines, each clipped to its
/// cell, merged where they overlap.
struct Straddled(Vec<(i32, i32)>);

impl Straddled {
    fn new(tl: &TableLayout) -> Self {
        let mut v: Vec<(i32, i32)> = Vec::new();
        for cl in &tl.cells {
            for (top, bottom) in spans(cl) {
                let (a, b) = (top.max(cl.y), bottom.min(cl.y + cl.h));
                if a < b {
                    v.push((a, b));
                }
            }
        }
        v.sort_unstable();
        let mut merged: Vec<(i32, i32)> = Vec::with_capacity(v.len());
        for (a, b) in v {
            match merged.last_mut() {
                // Open spans that only touch leave the shared end free.
                Some(last) if a < last.1 => last.1 = last.1.max(b),
                _ => merged.push((a, b)),
            }
        }
        Straddled(merged)
    }

    /// True when a cut at `y` would split a line.
    fn at(&self, y: i32) -> bool {
        let i = self.0.partition_point(|&(a, _)| a < y);
        i > 0 && y < self.0[i - 1].1
    }
}

/// Height of the header band repeated on continuation fragments, when
/// repeating is on: the rows from the top whose cells are all header
/// cells, merged cells among them taken whole.
pub fn header_height(t: &Table, tl: &TableLayout) -> i32 {
    if !t.repeat_header() {
        return 0;
    }
    let rows = (t.rows as usize).min(tl.row_y.len().saturating_sub(1));
    // The block ends before the first row holding a cell that is not a
    // header cell, at an edge no cell crosses.
    let plain = t
        .cells
        .iter()
        .filter(|c| !c.header())
        .map(|c| c.row as usize)
        .min()
        .unwrap_or(rows)
        .min(rows);
    let mut ends = vec![0usize; plain];
    for c in t.cells.iter().filter(|c| (c.row as usize) < plain) {
        let e = &mut ends[c.row as usize];
        *e = (*e).max(c.end_row() as usize);
    }
    let mut reach = 0;
    let mut block = 0;
    for (k, &e) in ends.iter().enumerate() {
        reach = reach.max(e);
        if reach <= k + 1 {
            block = k + 1;
        }
    }
    if block == 0 || block >= rows {
        return 0;
    }
    tl.row_y[block]
}

/// The lowest cut (the largest height) in `(from, limit]`, or `None` when
/// none fits.
pub fn cut(t: &Table, tl: &TableLayout, mode: SplitMode, from: i32, limit: i32) -> Option<i32> {
    if limit >= tl.height {
        return Some(tl.height);
    }
    let list = tl.cuts(t).of(mode);
    let i = list.partition_point(|&y| y <= limit);
    (i > 0 && list[i - 1] > from).then(|| list[i - 1])
}

/// Where a fragment cut inside cells at table height `limit` (the page
/// end) ends and the next one begins, when the cells crossing the limit hold
/// no lines below it: the fragment is drawn down to the limit and the next
/// begins at the following row edge, skipping the rows' empty rest.
/// `None` when a line runs on past the limit.
pub fn fill_cut(tl: &TableLayout, limit: i32) -> Option<(i32, i32)> {
    if limit <= 0 || limit >= tl.height {
        return None;
    }
    let mut next = None::<i32>;
    for (k, cl) in tl
        .cells
        .iter()
        .enumerate()
        .filter(|(_, cl)| cl.y < limit && limit < cl.y + cl.h)
    {
        if tl.cell_spans(k).iter().any(|&(_, b)| b > limit) {
            return None;
        }
        let end = cl.y + cl.h;
        next = Some(next.map_or(end, |n: i32| n.max(end)));
    }
    let next = next?;
    (next > limit && next < tl.height).then_some((limit, next))
}

/// Cells cut at a page end whose lines left over go on at the top of the
/// next fragment, each below its own top padding, as Hancom splits the
/// cells of a row across pages.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Carried {
    /// For each cell cut: its index and the table height of the top of its
    /// first line shown here (`None` when it has nothing left).
    pub cells: Vec<(u32, Option<i32>)>,
    /// The row edge where the cut cells end.
    pub until: i32,
    /// Height the carried part takes in this fragment.
    pub height: i32,
}

/// The cells a cut at table height `cut` passes through, each with the top
/// of its first line ending below the cut. `None` when the cut does not
/// fall inside a row, a cut cell reaches past the row edge after the cut,
/// or no cut cell has lines left.
pub fn cut_cells(tl: &TableLayout, cut: i32) -> Option<Carried> {
    let next = *tl.row_y.iter().find(|&&y| y > cut)?;
    let mut cells = Vec::new();
    // Some of the row must show above the cut, or it moves on whole.
    let mut shows = false;
    for (k, cl) in tl.cells.iter().enumerate() {
        if !(cl.y < cut && cut < cl.y + cl.h) {
            continue;
        }
        if cl.y + cl.h > next {
            return None;
        }
        let items = tl.cell_spans(k);
        shows |= items.iter().any(|&(t, b)| t >= cl.y && b <= cut);
        let first = items
            .iter()
            .copied()
            .filter(|&(_, b)| b > cut)
            .map(|(t, _)| t)
            .min();
        cells.push((k as u32, first));
    }
    (shows && cells.iter().any(|c| c.1.is_some())).then_some(Carried {
        cells,
        until: next,
        height: 0,
    })
}

/// Height the rest of carried cell `cl` (content `spans`, sorted) takes
/// from line top `first`: its paddings and the lines from there on.
pub fn carried_height(cl: &super::CellLayout, spans: &[(i32, i32)], first: Option<i32>) -> i32 {
    let pads = cl.padding[2] + cl.padding[3];
    let Some(first) = first else {
        return pads;
    };
    let from = spans.partition_point(|&(t, _)| t < first);
    let last = spans[from..].iter().map(|&(_, b)| b).max().unwrap_or(first);
    pads + last - first
}

/// The top of the first line of carried cell `cl` (shown from line top
/// `first`) that does not fit in `room`, or `None` when the rest fits.
/// With `force` at least one line goes in, so a line taller than any page
/// still moves on.
pub fn carried_break(
    cl: &super::CellLayout,
    spans: &[(i32, i32)],
    first: Option<i32>,
    room: i32,
    force: bool,
) -> Option<i32> {
    let first = first?;
    let from = spans.partition_point(|&(t, _)| t < first);
    for (k, &(t, b)) in spans[from..].iter().enumerate() {
        // What starts with the first item goes with it.
        let may_stop = !force || k > 0;
        if may_stop && t > first && cl.padding[2] + b - first > room {
            return Some(t);
        }
    }
    None
}

/// Where the fragment after a cut at table height `cut` begins: at the
/// first line below the cut in the cells crossing it, less that cell's top
/// padding, so the line keeps its padding at the top of the next page;
/// never above the cut.
pub fn resume_after(tl: &TableLayout, cut: i32) -> i32 {
    // A cut on a row edge goes on with the next row as it stands.
    let Some(&next) = tl.row_y.iter().find(|&&y| y > cut) else {
        return cut;
    };
    if tl.row_y.contains(&cut) {
        return cut;
    }
    let mut resume = None::<i32>;
    for (k, cl) in tl
        .cells
        .iter()
        .enumerate()
        .filter(|(_, cl)| cl.y < cut && cut < cl.y + cl.h)
    {
        let first = tl
            .cell_spans(k)
            .iter()
            .map(|&(t, _)| t)
            .filter(|&y| y >= cut)
            .min();
        if let Some(y) = first {
            let at = (y - cl.padding[2]).max(cut);
            resume = Some(resume.map_or(at, |r: i32| r.min(at)));
        }
    }
    resume.unwrap_or(cut).min(next)
}

/// The first cut below `from`, or the table end.
pub fn first_cut(t: &Table, tl: &TableLayout, mode: SplitMode, from: i32) -> i32 {
    let list = tl.cuts(t).of(mode);
    let i = list.partition_point(|&y| y <= from);
    list.get(i).copied().unwrap_or(tl.height)
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
        let (t, tl) = table(4, 1, false);
        assert_eq!(cut(&t, &tl, SplitMode::of(&t), 0, 2500), Some(2000));
        assert_eq!(cut(&t, &tl, SplitMode::of(&t), 0, 999), None);
        // A spanning cell keeps its rows together.
        let (t, tl) = table(4, 1, true);
        assert_eq!(cut(&t, &tl, SplitMode::of(&t), 0, 1500), None);
    }

    #[test]
    fn splitting_anywhere_cuts_between_lines() {
        let (t, tl) = table(4, 2, false);
        // Inside the third row the last whole line ends at 2600.
        assert_eq!(cut(&t, &tl, SplitMode::of(&t), 0, 2750), Some(2600));
        // Never splitting only takes the whole table.
        let (t, tl) = table(4, 0, false);
        assert_eq!(cut(&t, &tl, SplitMode::of(&t), 0, 3999), None);
        assert_eq!(cut(&t, &tl, SplitMode::of(&t), 0, 4000), Some(4000));
    }
}
