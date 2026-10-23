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
        } => (
            table.clone(),
            Action::Insert {
                columns: false,
                at: *before_row,
                count: *count,
            },
        ),
        EditOp::InsertTableColumns {
            table,
            before_column,
            count,
        } => (
            table.clone(),
            Action::Insert {
                columns: true,
                at: *before_column,
                count: *count,
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

fn insert(
    t: &mut Table,
    columns: bool,
    at: u16,
    n: u16,
    ids: &mut Vec<Option<usize>>,
) -> Result<()> {
    let old = count(t, columns);
    let new = old
        .checked_add(n)
        .ok_or_else(|| bad("table dimension overflow"))?;
    if at > old || n == 0 {
        return Err(bad("invalid table insertion range"));
    }
    area(
        if columns { t.rows } else { new },
        if columns { new } else { t.cols },
    )?;
    let bands = sizes(t, columns);
    let cross = sizes(t, !columns);
    let size = bands[at.min(old - 1) as usize];
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
            set_axis(
                c,
                columns,
                start,
                span + n,
                extent
                    .checked_add(added)
                    .ok_or_else(|| bad("cell size overflow"))?,
            );
        }
    }
    set_count(t, columns, new);
    let occupied = grid(t, true)?;
    for a in at..at + n {
        for b in 0..count(t, !columns) {
            let (r, col) = if columns { (b, a) } else { (a, b) };
            if occupied[r as usize * t.cols as usize + col as usize] != usize::MAX {
                continue;
            }
            let sample = at.min(old - 1);
            let (sr, sc) = if columns { (b, sample) } else { (sample, b) };
            let k = old_grid[sr as usize * templates.cols as usize + sc as usize];
            let mut cell = blank(&templates.cells[k]);
            set_axis(&mut cell, columns, a, 1, size);
            set_axis(&mut cell, !columns, b, 1, cross[b as usize]);
            t.cells.push(cell);
            ids.push(None);
        }
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
    let mut shift = 0;
    for &i in &ordered {
        if let Some(id) = ids[i] {
            shifts[id] = shift;
        }
        shift += t.cells[i].paras.len() as u32;
    }
    merged.paras = ordered
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

/// Add boundaries inside the chosen cell's bands without changing the size
/// or content of any other cell crossed by those boundaries.
fn refine(t: &mut Table, columns: bool, start: u16, span: u16, pieces: u16) -> Result<()> {
    if pieces == 0 {
        return Err(bad("split counts must be positive"));
    }
    let mut a = span;
    let mut b = pieces;
    while b != 0 {
        (a, b) = (b, a % b);
    }
    let factor = pieces / a;
    let end = start + span;
    let added = (span as u32) * (factor as u32 - 1);
    let new =
        u16::try_from(count(t, columns) as u32 + added).map_err(|_| bad("split grid overflow"))?;
    area(
        if columns { t.rows } else { new },
        if columns { new } else { t.cols },
    )?;
    let map = |v: u16| -> u16 {
        (v as u32 + v.min(end).saturating_sub(start) as u32 * (factor as u32 - 1)) as u16
    };
    for c in &mut t.cells {
        let (pos, n, size) = axis(c, columns);
        set_axis(c, columns, map(pos), map(pos + n) - map(pos), size);
    }
    zone_axis(t, columns, map);
    set_count(t, columns, new);
    Ok(())
}

fn split(
    t: &mut Table,
    at: CellRef,
    rows: u16,
    columns: u16,
    equal: bool,
    ids: &mut Vec<Option<usize>>,
) -> Result<()> {
    let i = t
        .cells
        .iter()
        .position(|c| origin(c) == at)
        .ok_or_else(|| bad("split must address a cell origin"))?;
    let old = t.cells[i].clone();
    refine(t, false, old.row, old.row_span, rows)?;
    refine(t, true, old.col, old.col_span, columns)?;
    let cell = t.cells[i].clone();
    if cell.width < columns as i32 || cell.height < rows as i32 {
        return Err(bad("cell too small to split"));
    }
    let heights = sizes(t, false);
    let heights: Vec<i32> = (0..rows)
        .map(|r| {
            if equal {
                cell.height / rows as i32 + i32::from((r as i32) < cell.height % rows as i32)
            } else {
                let a = cell.row + r * (cell.row_span / rows);
                heights[a as usize..(a + cell.row_span / rows) as usize]
                    .iter()
                    .sum()
            }
        })
        .collect();
    for r in 0..rows {
        for col in 0..columns {
            let first = r == 0 && col == 0;
            let mut part = if first { cell.clone() } else { blank(&cell) };
            let width =
                cell.width / columns as i32 + i32::from((col as i32) < cell.width % columns as i32);
            set_axis(
                &mut part,
                true,
                cell.col + col * (cell.col_span / columns),
                cell.col_span / columns,
                width,
            );
            set_axis(
                &mut part,
                false,
                cell.row + r * (cell.row_span / rows),
                cell.row_span / rows,
                heights[r as usize],
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
    match action {
        Action::Insert { columns, at, count } => insert(t, columns, at, count, &mut ids)?,
        Action::Delete { columns, at, count } => delete(t, columns, at, count, &mut ids)?,
        Action::Merge { first, last } => {
            merge(t, first, last, &mut ids, &mut redirects, &mut shifts)?
        }
        Action::Split {
            at,
            rows,
            columns,
            equal,
        } => split(t, at, rows, columns, equal, &mut ids)?,
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
    let tab = doc
        .sections
        .get(table.section as usize)
        .and_then(|s| s.definition())
        .map(|d| hm_model::units::unpack_distance(d.default_tab, || 1000))
        .filter(|&v| v > 0)
        .unwrap_or(4000);
    let mut styles = hm_layout::style::Styles::new(&doc.info, fonts);
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
