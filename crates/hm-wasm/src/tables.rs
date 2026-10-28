//! Structural table edits become paragraph replacements in the native history.

use hm_edit::Op;
use hm_model::{Cell, ControlKind, Document, Table};

use crate::types::{
    Address, CellRef, CellRemap, EditOp, EngineError, Result, TableAddress, TableChange,
};

pub enum Action {
    Insert {
        columns: bool,
        at: u16,
        count: u16,
        like: Option<u16>,
    },
    Delete {
        columns: bool,
        at: u16,
        count: u16,
    },
    Merge {
        first: CellRef,
        last: CellRef,
    },
    Split {
        at: CellRef,
        rows: u16,
        columns: u16,
        equal: bool,
    },
    Edge {
        at: CellRef,
        columns: bool,
        delta: i32,
    },
}

fn bad(message: &str) -> EngineError {
    EngineError::new("BAD_REQUEST", message)
}

pub fn request(op: &EditOp) -> Option<(TableAddress, Action)> {
    Some(match op {
        EditOp::InsertTableRows {
            table,
            before_row,
            count,
            copy_row,
        } => (
            table.clone(),
            Action::Insert {
                columns: false,
                at: *before_row,
                count: *count,
                like: *copy_row,
            },
        ),
        EditOp::InsertTableColumns {
            table,
            before_column,
            count,
            copy_column,
        } => (
            table.clone(),
            Action::Insert {
                columns: true,
                at: *before_column,
                count: *count,
                like: *copy_column,
            },
        ),
        EditOp::DeleteTableRows {
            table,
            start_row,
            count,
        } => (
            table.clone(),
            Action::Delete {
                columns: false,
                at: *start_row,
                count: *count,
            },
        ),
        EditOp::DeleteTableColumns {
            table,
            start_column,
            count,
        } => (
            table.clone(),
            Action::Delete {
                columns: true,
                at: *start_column,
                count: *count,
            },
        ),
        EditOp::MergeTableCells {
            table,
            start_row,
            start_column,
            end_row,
            end_column,
        } => (
            table.clone(),
            Action::Merge {
                first: CellRef {
                    row: *start_row,
                    column: *start_column,
                },
                last: CellRef {
                    row: *end_row,
                    column: *end_column,
                },
            },
        ),
        EditOp::SplitTableCell {
            table,
            row,
            column,
            rows,
            columns,
            equal_row_height,
        } => (
            table.clone(),
            Action::Split {
                at: CellRef {
                    row: *row,
                    column: *column,
                },
                rows: *rows,
                columns: *columns,
                equal: *equal_row_height,
            },
        ),
        EditOp::MoveTableCellEdge {
            table,
            row,
            column,
            side,
            delta,
        } => (
            table.clone(),
            Action::Edge {
                at: CellRef {
                    row: *row,
                    column: *column,
                },
                columns: side != "bottom",
                delta: *delta,
            },
        ),
        _ => return None,
    })
}

pub fn path(table: &TableAddress) -> hm_edit::ListPath {
    Address {
        section: table.section,
        steps: table.steps.clone(),
        para: table.para,
        offset: 0,
    }
    .pos()
    .list
}

fn origin(c: &Cell) -> CellRef {
    CellRef {
        row: c.row,
        column: c.col,
    }
}

fn axis(c: &Cell, columns: bool) -> (u16, u16, i32) {
    if columns {
        (c.col, c.col_span, c.width)
    } else {
        (c.row, c.row_span, c.height)
    }
}

fn set_axis(c: &mut Cell, columns: bool, start: u16, span: u16, size: i32) {
    if columns {
        if c.width != size {
            for p in &mut c.paras {
                p.src = None;
                p.line_segs.clear();
            }
        }
        c.col = start;
        c.col_span = span;
        c.width = size;
    } else {
        c.row = start;
        c.row_span = span;
        c.height = size;
    }
}

fn count(t: &Table, columns: bool) -> u16 {
    if columns { t.cols } else { t.rows }
}

fn set_count(t: &mut Table, columns: bool, n: u16) {
    if columns {
        t.cols = n;
    } else {
        t.rows = n;
    }
}

/// Bound the occupancy map before allocating it, including for hostile requests.
fn area(rows: u16, cols: u16) -> Result<usize> {
    let n = rows as usize * cols as usize;
    if n == 0 || n > 1_000_000 {
        return Err(bad("table grid must contain 1 to 1000000 positions"));
    }
    Ok(n)
}

fn grid(t: &Table, holes: bool) -> Result<Vec<usize>> {
    let mut grid = vec![usize::MAX; area(t.rows, t.cols)?];
    for (i, c) in t.cells.iter().enumerate() {
        if c.row_span == 0
            || c.col_span == 0
            || c.end_row() > t.rows as u32
            || c.end_col() > t.cols as u32
        {
            return Err(bad("cell span lies outside the table"));
        }
        for r in c.row..c.row + c.row_span {
            for col in c.col..c.col + c.col_span {
                let slot = &mut grid[r as usize * t.cols as usize + col as usize];
                if *slot != usize::MAX {
                    return Err(bad("table cells overlap"));
                }
                *slot = i;
            }
        }
    }
    if !holes && grid.contains(&usize::MAX) {
        return Err(bad("table grid has missing cells"));
    }
    Ok(grid)
}

/// Narrow cells define individual bands; wider cells fill any unknown bands.
fn sizes(t: &Table, columns: bool) -> Vec<i32> {
    let mut sizes = vec![0; count(t, columns) as usize];
    let mut cells: Vec<_> = t.cells.iter().collect();
    cells.sort_by_key(|c| axis(c, columns).1);
    for c in cells {
        let (start, span, size) = axis(c, columns);
        let bands = &mut sizes[start as usize..(start + span) as usize];
        let unknown = bands.iter().filter(|&&v| v == 0).count() as i32;
        if unknown == 0 {
            continue;
        }
        let left = (size - bands.iter().sum::<i32>()).max(unknown);
        for (k, band) in bands.iter_mut().filter(|v| **v == 0).enumerate() {
            *band = left / unknown + i32::from((k as i32) < left % unknown);
        }
    }
    sizes.iter_mut().for_each(|v| *v = (*v).max(1));
    sizes
}

fn blank(c: &Cell) -> Cell {
    let template = c.paras.first().cloned().unwrap_or_default();
    let shape = template.runs.first().map_or(0, |r| r.shape);
    Cell {
        paras: vec![hm_edit::ops::empty_paragraph(&template, shape)],
        ..c.clone()
    }
}

fn zone_axis(t: &mut Table, columns: bool, map: impl Fn(u16) -> u16) {
    for z in t.zones.iter_mut().flatten() {
        let (a, b) = if columns {
            (&mut z.start_col, &mut z.end_col)
        } else {
            (&mut z.start_row, &mut z.end_row)
        };
        *a = map(*a);
        *b = map(b.saturating_add(1)).saturating_sub(1).max(*a);
    }
}

/// New bands copy the cells of band `like`: each new cell takes the
/// cross-axis span of the template cell beside it and the template band's
/// size, as Hancom adds a row below a row of merged cells.
fn insert(
    t: &mut Table,
    columns: bool,
    at: u16,
    n: u16,
    like: Option<u16>,
    ids: &mut Vec<Option<usize>>,
) -> Result<()> {
    let old = count(t, columns);
    let new = old
        .checked_add(n)
        .ok_or_else(|| bad("table dimension overflow"))?;
    if at > old || n == 0 || like.is_some_and(|v| v >= old) {
        return Err(bad("invalid table insertion range"));
    }
    area(
        if columns { t.rows } else { new },
        if columns { new } else { t.cols },
    )?;
    let bands = sizes(t, columns);
    let cross = sizes(t, !columns);
    let sample = like.unwrap_or(at.min(old - 1));
    let size = bands[sample as usize];
    let added = size
        .checked_mul(n as i32)
        .ok_or_else(|| bad("table size overflow"))?;
    let templates = t.clone();
    let old_grid = grid(&templates, false)?;
    for c in &mut t.cells {
        let (start, span, extent) = axis(c, columns);
        if start >= at {
            set_axis(c, columns, start + n, span, extent);
        } else if start + span > at {
            // A cell spanning the new columns widens with them. Heights are
            // minimums: Hancom keeps a spanning cell's stored height.
            let extent = if columns {
                extent
                    .checked_add(added)
                    .ok_or_else(|| bad("cell size overflow"))?
            } else {
                extent
            };
            set_axis(c, columns, start, span + n, extent);
        }
    }
    set_count(t, columns, new);
    let occupied = grid(t, true)?;
    let slot = |a: u16, b: u16| {
        let (r, col) = if columns { (b, a) } else { (a, b) };
        r as usize * t.cols as usize + col as usize
    };
    let mut fresh = Vec::new();
    for a in at..at + n {
        let mut b = 0;
        while b < count(t, !columns) {
            if occupied[slot(a, b)] != usize::MAX {
                b += 1;
                continue;
            }
            let (sr, sc) = if columns { (b, sample) } else { (sample, b) };
            let template =
                &templates.cells[old_grid[sr as usize * templates.cols as usize + sc as usize]];
            let (cross_start, cross_span, _) = axis(template, !columns);
            let mut end = (cross_start + cross_span).max(b + 1);
            if let Some(stop) = (b + 1..end).find(|&e| occupied[slot(a, e)] != usize::MAX) {
                end = stop;
            }
            let mut cell = blank(template);
            let extent: i32 = cross[b as usize..end as usize].iter().sum();
            let extent = if end - b == cross_span && b == cross_start {
                axis(template, !columns).2
            } else {
                extent
            };
            set_axis(&mut cell, columns, a, 1, size);
            set_axis(&mut cell, !columns, b, end - b, extent);
            fresh.push(cell);
            b = end;
        }
    }
    for cell in fresh {
        t.cells.push(cell);
        ids.push(None);
    }
    zone_axis(t, columns, |v| if v >= at { v + n } else { v });
    let extent = if columns {
        &mut t.header.common.width
    } else {
        &mut t.header.common.height
    };
    *extent = extent
        .checked_add(added)
        .ok_or_else(|| bad("table size overflow"))?;
    Ok(())
}

fn delete(
    t: &mut Table,
    columns: bool,
    at: u16,
    n: u16,
    ids: &mut Vec<Option<usize>>,
) -> Result<()> {
    let old = count(t, columns);
    let end = at
        .checked_add(n)
        .ok_or_else(|| bad("table deletion overflow"))?;
    if n == 0 || end > old || n == old {
        return Err(bad("cannot delete this table range or its last row/column"));
    }
    let bands = sizes(t, columns);
    let mut cells = Vec::new();
    let mut kept = Vec::new();
    for (mut c, id) in std::mem::take(&mut t.cells).into_iter().zip(ids.drain(..)) {
        let (start, span, size) = axis(&c, columns);
        let finish = start + span;
        let removed = finish.min(end).saturating_sub(start.max(at));
        if removed == span {
            continue;
        }
        let shrink: i32 = bands
            [start.max(at) as usize..finish.min(end).max(start.max(at)) as usize]
            .iter()
            .sum();
        let mapped = start - start.min(end).saturating_sub(at);
        set_axis(
            &mut c,
            columns,
            mapped,
            span - removed,
            (size - shrink).max(1),
        );
        cells.push(c);
        kept.push(id);
    }
    t.cells = cells;
    *ids = kept;
    set_count(t, columns, old - n);
    if let Some(zones) = &mut t.zones {
        zones.retain(|z| {
            let (first, last) = if columns {
                (z.start_col, z.end_col)
            } else {
                (z.start_row, z.end_row)
            };
            first < at || last >= end
        });
    }
    zone_axis(t, columns, |v| v - v.min(end).saturating_sub(at));
    if let Some(zones) = &mut t.zones {
        zones.retain(|z| z.start_row < t.rows && z.start_col < t.cols);
    }
    let removed: i32 = bands[at as usize..end as usize].iter().sum();
    let extent = if columns {
        &mut t.header.common.width
    } else {
        &mut t.header.common.height
    };
    *extent = (*extent - removed).max(1);
    Ok(())
}

/// One paragraph with no text and no controls.
fn empty_cell(c: &Cell) -> bool {
    matches!(c.paras.as_slice(), [p] if p.is_empty() && p.controls.is_empty())
}

fn merge(
    t: &mut Table,
    first: CellRef,
    last: CellRef,
    ids: &mut Vec<Option<usize>>,
    redirects: &mut [Option<usize>],
    shifts: &mut [u32],
) -> Result<()> {
    if first.row > last.row
        || first.column > last.column
        || last.row >= t.rows
        || last.column >= t.cols
    {
        return Err(bad("invalid merge rectangle"));
    }
    let selected: Vec<_> = t
        .cells
        .iter()
        .enumerate()
        .filter(|(_, c)| {
            c.row <= last.row
                && c.end_row() > first.row as u32
                && c.col <= last.column
                && c.end_col() > first.column as u32
        })
        .map(|(i, _)| i)
        .collect();
    if selected.iter().any(|&i| {
        let c = &t.cells[i];
        c.row < first.row
            || c.col < first.column
            || c.end_row() > last.row as u32 + 1
            || c.end_col() > last.column as u32 + 1
    }) {
        return Err(bad("merge rectangle cuts an existing span"));
    }
    let survivor = selected
        .iter()
        .copied()
        .find(|&i| origin(&t.cells[i]) == first)
        .ok_or_else(|| bad("merge must begin at a cell origin"))?;
    let widths = sizes(t, true);
    let heights = sizes(t, false);
    let mut merged = t.cells[survivor].clone();
    merged.col_span = last.column - first.column + 1;
    merged.row_span = last.row - first.row + 1;
    merged.width = widths[first.column as usize..=last.column as usize]
        .iter()
        .sum();
    merged.height = heights[first.row as usize..=last.row as usize].iter().sum();
    let mut ordered = selected.clone();
    ordered.sort_by_key(|&i| (t.cells[i].row, t.cells[i].col));
    // Hancom keeps the content of every non-empty cell in row order and
    // drops cells holding only one empty paragraph; all empty keeps one.
    let kept: Vec<usize> = ordered
        .iter()
        .copied()
        .filter(|&i| !empty_cell(&t.cells[i]))
        .collect();
    let kept = if kept.is_empty() {
        vec![survivor]
    } else {
        kept
    };
    let mut shift = 0;
    for &i in &ordered {
        let keep = kept.contains(&i);
        if let Some(id) = ids[i] {
            // A dropped cell's caret goes to the last kept paragraph before it.
            shifts[id] = if keep { shift } else { shift.saturating_sub(1) };
        }
        if keep {
            shift += t.cells[i].paras.len() as u32;
        }
    }
    merged.paras = kept
        .iter()
        .flat_map(|&i| t.cells[i].paras.clone())
        .collect();
    for p in &mut merged.paras {
        p.src = None;
        p.line_segs.clear();
    }
    let target = ids[survivor];
    let mut cells = Vec::new();
    let mut kept = Vec::new();
    for (i, (c, id)) in std::mem::take(&mut t.cells)
        .into_iter()
        .zip(ids.drain(..))
        .enumerate()
    {
        if i == survivor {
            cells.push(merged.clone());
            kept.push(id);
        } else if selected.contains(&i) {
            if let Some(id) = id {
                redirects[id] = target;
            }
        } else {
            cells.push(c);
            kept.push(id);
        }
    }
    t.cells = cells;
    *ids = kept;
    Ok(())
}

/// Places one grid line inside band `band`: later cells move one band on,
/// cells crossing it span one more. Sizes are untouched; they follow from
/// the cells.
fn add_line(t: &mut Table, columns: bool, band: u16) -> Result<()> {
    let new = count(t, columns)
        .checked_add(1)
        .ok_or_else(|| bad("split grid overflow"))?;
    area(
        if columns { t.rows } else { new },
        if columns { new } else { t.cols },
    )?;
    for c in &mut t.cells {
        let (start, span, size) = axis(c, columns);
        if start > band {
            set_axis(c, columns, start + 1, span, size);
        } else if start + span > band {
            set_axis(c, columns, start, span + 1, size);
        }
    }
    zone_axis(t, columns, |v| v + u16::from(v > band));
    set_count(t, columns, new);
    Ok(())
}

/// Cuts one axis of the cell starting at band `start` at offsets `cuts`
/// (ascending, from the cell's start). A cut on an existing inner grid line
/// (offsets `inner`) reuses it; any other adds a line, so neighbours keep
/// their size and content. Returns, per cut, its line's number of bands
/// from the cell's start.
fn cut_axis(
    t: &mut Table,
    columns: bool,
    start: u16,
    inner: &[i32],
    cuts: &[i32],
) -> Result<Vec<u16>> {
    let mut lines: Vec<(i32, bool)> = inner.iter().map(|&v| (v, false)).collect();
    let mut taken = Vec::with_capacity(cuts.len());
    for &cut in cuts {
        let k = match lines.iter().position(|&(v, used)| v == cut && !used) {
            Some(k) => k,
            None => {
                // After every line at or before the cut, so equal cuts stack.
                let k = lines.partition_point(|&(v, _)| v <= cut);
                add_line(t, columns, start + k as u16)?;
                lines.insert(k, (cut, false));
                for line in taken.iter_mut() {
                    if *line >= k {
                        *line += 1;
                    }
                }
                k
            }
        };
        lines[k].1 = true;
        taken.push(k);
    }
    Ok(taken.into_iter().map(|k| k as u16 + 1).collect())
}

/// `total` shared by `n` pieces, the remainder going to the first ones.
fn shares(total: i32, n: u16) -> Vec<i32> {
    let n = n as i32;
    (0..n)
        .map(|k| total / n + i32::from(k < total % n))
        .collect()
}

/// One axis of a split: the pieces' boundaries from the cell's start and
/// their sizes. A span the pieces divide keeps its grid lines, each piece
/// taking its bands' sizes (`lines` are the inner grid lines, `total` the
/// cell's size along the axis); otherwise pieces of `sizes` are laid end to
/// end.
fn piece_cuts(
    lines: &[i32],
    total: i32,
    span: u16,
    n: u16,
    sizes: Vec<i32>,
) -> (Vec<i32>, Vec<i32>) {
    if span % n == 0 {
        let step = (span / n) as usize;
        let edges: Vec<i32> = std::iter::once(0)
            .chain((1..n as usize).map(|k| lines[k * step - 1]))
            .chain(std::iter::once(total))
            .collect();
        let sizes = edges.windows(2).map(|w| w[1] - w[0]).collect();
        return (edges[1..n as usize].to_vec(), sizes);
    }
    let cuts = sizes[..n as usize - 1]
        .iter()
        .scan(0, |at, size| {
            *at += size;
            Some(*at)
        })
        .collect();
    (cuts, sizes)
}

/// Splits a cell into `rows` x `columns` pieces, as Hancom does. Rows use
/// the height the cell is drawn at: across new grid lines the first piece
/// keeps what its content needs and the others share the rest (or all
/// share it with `equal`). New column lines share the cell's width evenly.
/// Grid lines the pieces fall on are reused.
fn split(
    t: &mut Table,
    at: CellRef,
    rows: u16,
    columns: u16,
    equal: bool,
    laid: &hm_layout::table::TableLayout,
    ids: &mut Vec<Option<usize>>,
) -> Result<()> {
    if rows == 0 || columns == 0 {
        return Err(bad("split counts must be positive"));
    }
    let i = t
        .cells
        .iter()
        .position(|c| origin(c) == at)
        .ok_or_else(|| bad("split must address a cell origin"))?;
    let old = t.cells[i].clone();
    let drawn = laid
        .cells
        .get(i)
        .ok_or_else(|| bad("cell is not laid out"))?;
    if old.width < columns as i32 {
        return Err(bad("cell too small to split"));
    }
    let inner = |e: &[i32], start: u16, span: u16| -> Vec<i32> {
        (start + 1..start + span)
            .map(|k| e[k as usize] - e[start as usize])
            .collect()
    };
    let mut col_x = vec![0];
    for w in sizes(t, true) {
        col_x.push(col_x.last().unwrap() + w);
    }
    let col_lines = inner(&col_x, old.col, old.col_span);
    let width = col_x[(old.col + old.col_span) as usize] - col_x[old.col as usize];
    let (col_cuts, piece_widths) = piece_cuts(
        &col_lines,
        width,
        old.col_span,
        columns,
        shares(width, columns),
    );
    // Keep the cell's own width when the bands disagree with it.
    let piece_widths = if width == old.width {
        piece_widths
    } else {
        shares(old.width, columns)
    };
    let row_y = &laid.row_y;
    let row_lines = inner(row_y, old.row, old.row_span);
    let height = row_y[(old.row + old.row_span) as usize] - row_y[old.row as usize];
    let heights = if equal || rows == 1 {
        shares(height, rows)
    } else {
        let need = (drawn.content + drawn.padding[2] + drawn.padding[3]).clamp(0, height);
        std::iter::once(need)
            .chain(shares(height - need, rows - 1))
            .collect()
    };
    let (row_cuts, piece_heights) = piece_cuts(&row_lines, height, old.row_span, rows, heights);
    let piece_heights = if equal {
        shares(height, rows)
    } else {
        piece_heights
    };
    let col_lines = cut_axis(t, true, old.col, &col_lines, &col_cuts)?;
    let row_lines = cut_axis(t, false, old.row, &row_lines, &row_cuts)?;
    let cell = t.cells[i].clone();
    let bounds = |lines: &[u16], span: u16| -> Vec<u16> {
        std::iter::once(0)
            .chain(lines.iter().copied())
            .chain(std::iter::once(span))
            .collect()
    };
    let col_bounds = bounds(&col_lines, cell.col_span);
    let row_bounds = bounds(&row_lines, cell.row_span);
    for r in 0..rows as usize {
        for c in 0..columns as usize {
            let first = r == 0 && c == 0;
            let mut part = if first { cell.clone() } else { blank(&cell) };
            set_axis(
                &mut part,
                true,
                cell.col + col_bounds[c],
                col_bounds[c + 1] - col_bounds[c],
                piece_widths[c],
            );
            set_axis(
                &mut part,
                false,
                cell.row + row_bounds[r],
                row_bounds[r + 1] - row_bounds[r],
                piece_heights[r],
            );
            if first {
                t.cells[i] = part;
            } else {
                t.cells.push(part);
                ids.push(None);
            }
        }
    }
    Ok(())
}

/// Drops grid lines no cell begins or ends at along one axis.
fn drop_unused_lines(t: &mut Table, columns: bool) {
    let mut line = 1;
    while line < count(t, columns) {
        let used = t.cells.iter().any(|c| {
            let (start, span, _) = axis(c, columns);
            start == line || start + span == line
        });
        if used {
            line += 1;
            continue;
        }
        for c in &mut t.cells {
            let (start, span, size) = axis(c, columns);
            if start > line {
                set_axis(c, columns, start - 1, span, size);
            } else if start + span > line {
                set_axis(c, columns, start, span - 1, size);
            }
        }
        zone_axis(t, columns, |v| v - u16::from(v > line));
        let n = count(t, columns) - 1;
        set_count(t, columns, n);
    }
}

/// Moves the right (`columns`) or bottom edge of the cell at `at` by
/// `delta`, as Hancom resizes one cell of a cell block (F5, Shift+arrow):
/// only that cell and the cells beside it across the edge change size, a
/// grid line is added where the edge lands, and lines no cell uses go.
/// Widths start from the stored ones, heights from the rows as drawn.
fn move_edge(
    t: &mut Table,
    at: CellRef,
    columns: bool,
    delta: i32,
    laid: &hm_layout::table::TableLayout,
) -> Result<()> {
    let i = t
        .cells
        .iter()
        .position(|c| origin(c) == at)
        .ok_or_else(|| bad("the edge must belong to a cell origin"))?;
    let (start, span, _) = axis(&t.cells[i], columns);
    let (from, across, _) = axis(&t.cells[i], !columns);
    let edge = start + span;
    let beside: Vec<usize> = (0..t.cells.len())
        .filter(|&k| {
            let (s, _, _) = axis(&t.cells[k], columns);
            let (a, n, _) = axis(&t.cells[k], !columns);
            s == edge && a < from + across && a + n > from
        })
        .collect();
    if beside.is_empty() {
        return Err(bad("no cell lies across this edge"));
    }
    if beside.iter().any(|&k| {
        let (a, n, _) = axis(&t.cells[k], !columns);
        a < from || a + n > from + across
    }) {
        return Err(bad("a cell across this edge also borders other cells"));
    }
    let size = |k: usize| {
        if columns {
            t.cells[k].width
        } else {
            laid.cells.get(k).map_or(t.cells[k].height, |l| l.h)
        }
    };
    let own = size(i);
    let besides: Vec<(usize, i32)> = beside.iter().map(|&k| (k, size(k))).collect();
    let lowest = besides.iter().map(|&(_, v)| v).min().unwrap_or(0);
    let delta = delta.clamp(
        crate::objects::MIN_CELL - own,
        lowest - crate::objects::MIN_CELL,
    );
    if delta == 0 {
        return Err(bad("the edge cannot move further"));
    }
    let lines: Vec<i32> = if columns {
        let mut x = vec![0];
        for w in sizes(t, true) {
            x.push(x.last().unwrap() + w);
        }
        x
    } else {
        laid.row_y.iter().map(|y| y - laid.row_y[0]).collect()
    };
    let target = lines[edge as usize] + delta;
    let line = match lines.iter().position(|&v| v == target) {
        Some(k) => k as u16,
        None => {
            let band = lines.partition_point(|&v| v < target) as u16 - 1;
            add_line(t, columns, band)?;
            band + 1
        }
    };
    let (s, _, _) = axis(&t.cells[i], columns);
    set_axis(&mut t.cells[i], columns, s, line - s, own + delta);
    for (k, extent) in besides {
        let (s, n, _) = axis(&t.cells[k], columns);
        set_axis(&mut t.cells[k], columns, line, s + n - line, extent - delta);
    }
    drop_unused_lines(t, columns);
    Ok(())
}

pub fn edit(
    doc: &Document,
    fonts: &hm_fonts::FontSystem,
    table: TableAddress,
    action: Action,
) -> Result<(Op, TableChange)> {
    let list = path(&table);
    let mut para = hm_edit::ops::list(doc, &list)
        .and_then(|p| p.get(table.para as usize))
        .cloned()
        .ok_or_else(|| EngineError::new("BAD_ADDRESS", "no such table paragraph"))?;
    let control = para
        .controls
        .get_mut(table.control as usize)
        .ok_or_else(|| EngineError::new("BAD_ADDRESS", "no such table control"))?;
    let ControlKind::Table(t) = &mut control.kind else {
        return Err(EngineError::new("BAD_ADDRESS", "control is not a table"));
    };
    grid(t, false)?;
    let old: Vec<_> = t
        .cells
        .iter()
        .map(|c| (origin(c), c.row_span, c.col_span))
        .collect();
    let mut ids: Vec<_> = (0..old.len()).map(Some).collect();
    let mut redirects: Vec<_> = (0..old.len()).map(Some).collect();
    let mut shifts = vec![0; old.len()];
    let tab = doc
        .sections
        .get(table.section as usize)
        .and_then(|s| s.definition())
        .map(|d| hm_model::units::unpack_distance(d.default_tab, || 1000))
        .filter(|&v| v > 0)
        .unwrap_or(4000);
    let mut styles = hm_layout::style::Styles::new(&doc.info, fonts);
    match action {
        Action::Insert {
            columns,
            at,
            count,
            like,
        } => insert(t, columns, at, count, like, &mut ids)?,
        Action::Delete { columns, at, count } => delete(t, columns, at, count, &mut ids)?,
        Action::Merge { first, last } => {
            merge(t, first, last, &mut ids, &mut redirects, &mut shifts)?
        }
        Action::Split {
            at,
            rows,
            columns,
            equal,
        } => {
            // Split heights follow the cell as drawn, not its stored minimum.
            let drawn = hm_layout::table::layout(t, &doc.info, &mut styles, tab);
            split(t, at, rows, columns, equal, &drawn, &mut ids)?
        }
        Action::Edge { at, columns, delta } => {
            let drawn = hm_layout::table::layout(t, &doc.info, &mut styles, tab);
            move_edge(t, at, columns, delta, &drawn)?
        }
    }
    grid(t, false)?;
    let destinations: Vec<_> = (0..old.len())
        .map(|id| {
            ids.iter()
                .position(|&k| k == Some(id))
                .map(|i| origin(&t.cells[i]))
        })
        .collect();
    t.cells.sort_by_key(|c| (c.row, c.col));
    let cells = old
        .into_iter()
        .enumerate()
        .map(|(i, (from, rows, columns))| CellRemap {
            span: (rows, columns),
            from,
            to: redirects[i].and_then(|id| destinations[id]),
            index: redirects[i]
                .and_then(|id| destinations[id])
                .and_then(|at| t.cells.iter().position(|c| origin(c) == at))
                .map(|i| i as u32),
            paragraph_shift: shifts[i],
        })
        .collect();
    t.row_cells = vec![0; t.rows as usize];
    for c in &t.cells {
        t.row_cells[c.row as usize] += 1;
    }
    // Only the control changed. Keep the host paragraph's stored spans,
    // just as a cell text edit does, so anchored furniture keeps its flow.
    let laid = hm_layout::table::layout(t, &doc.info, &mut styles, tab);
    // Page-relative wrapping uses the common header's size. Hancom grows it
    // to the cells' content on open, so keep the preview's exclusion box in
    // sync with the new grid before pagination and serialization.
    t.header.common.width = laid.width;
    t.header.common.height = laid.height;
    control.src = None;
    Ok((
        Op::Replace {
            list,
            start: table.para,
            count: 1,
            paras: vec![para],
        },
        TableChange { table, cells },
    ))
}

/// Cell selections follow surviving origins. A selection in removed cells is
/// cleared; the host can use the per-cell receipt map to choose its next caret.
pub fn remap_selection(
    mut selection: serde_json::Value,
    change: &TableChange,
) -> serde_json::Value {
    if selection.get("kind").and_then(|v| v.as_str()) == Some("cells") {
        let matches = selection
            .get("table")
            .and_then(|v| serde_json::from_value::<TableAddress>(v.clone()).ok())
            .is_some_and(|v| v == change.table);
        if matches {
            for end in ["from", "to"] {
                let cell = selection
                    .get(end)
                    .and_then(|v| serde_json::from_value::<CellRef>(v.clone()).ok());
                let mapped = cell
                    .and_then(|cell| {
                        change.cells.iter().find(|m| {
                            cell.row >= m.from.row
                                && cell.column >= m.from.column
                                && (cell.row as u32) < m.from.row as u32 + m.span.0 as u32
                                && (cell.column as u32) < m.from.column as u32 + m.span.1 as u32
                        })
                    })
                    .and_then(|m| m.to);
                let Some(mapped) = mapped else {
                    return serde_json::Value::Null;
                };
                selection[end] = serde_json::json!(mapped);
            }
        }
    } else {
        for end in ["anchor", "focus"] {
            if let Some(mut address) = selection
                .get(end)
                .and_then(|v| serde_json::from_value::<Address>(v.clone()).ok())
            {
                if !remap_address(&mut address, change) {
                    return serde_json::Value::Null;
                }
                selection[end] = serde_json::json!(address);
            }
        }
    }
    selection
}

/// Returns false when the address was in a deleted cell.
pub fn remap_address(address: &mut Address, change: &TableChange) -> bool {
    let parent = &change.table;
    let depth = parent.steps.len();
    if address.section != parent.section || !address.steps.starts_with(&parent.steps) {
        return true;
    }
    let Some(step) = address.steps.get_mut(depth) else {
        return true;
    };
    if step.para != parent.para || step.control != parent.control {
        return true;
    }
    let Some(mapped) = change.cells.get(step.index as usize) else {
        return false;
    };
    let Some(index) = mapped.index else {
        return false;
    };
    step.index = index;
    if let Some(nested) = address.steps.get_mut(depth + 1) {
        nested.para += mapped.paragraph_shift;
    } else {
        address.para += mapped.paragraph_shift;
    }
    true
}
