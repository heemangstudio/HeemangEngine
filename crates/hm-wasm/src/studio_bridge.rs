//! Scoped paragraph captures and table transposition for editor commands.
use super::*;
use hm_model::{ControlKind, Table};
use serde::Deserialize;
use types::TableAddress;

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "camelCase")]
enum TransposeRequest {
    #[serde(rename_all = "camelCase")]
    Copy {
        table: TableAddress,
        start_row: u16,
        start_column: u16,
        end_row: u16,
        end_column: u16,
        transpose: Option<bool>,
    },
    #[serde(rename_all = "camelCase")]
    Paste {
        table: TableAddress,
        start_row: u16,
        start_column: u16,
        transpose: Option<bool>,
    },
    InPlace {
        table: TableAddress,
    },
    Insert {
        at: Address,
    },
}

fn bad(message: &str) -> EngineError {
    EngineError::new("BAD_REQUEST", message)
}

fn block(source: &Table, row: u16, col: u16, end_row: u16, end_col: u16) -> Result<Table> {
    if row > end_row || col > end_col || end_row >= source.rows || end_col >= source.cols {
        return Err(bad("table selection is outside the grid"));
    }
    let mut t = source.clone();
    t.cells.retain(|c| {
        c.row <= end_row && c.row + c.row_span > row && c.col <= end_col && c.col + c.col_span > col
    });
    for c in &mut t.cells {
        if c.row < row
            || c.col < col
            || c.row + c.row_span > end_row + 1
            || c.col + c.col_span > end_col + 1
        {
            return Err(bad("table selection cuts a merged cell"));
        }
        c.row -= row;
        c.col -= col;
    }
    t.rows = end_row - row + 1;
    t.cols = end_col - col + 1;
    t.caption = None;
    t.row_cells = (0..t.rows)
        .map(|row| t.cells.iter().filter(|c| c.row == row).count() as u16)
        .collect();
    t.header.common.width = t
        .cells
        .iter()
        .filter(|c| c.row == 0)
        .fold(0i32, |sum, c| sum.saturating_add(c.width));
    t.header.common.height = t
        .cells
        .iter()
        .filter(|c| c.col == 0)
        .fold(0i32, |sum, c| sum.saturating_add(c.height));
    t.zones = source.zones.as_ref().map(|zones| {
        zones
            .iter()
            .filter_map(|z| {
                if z.start_row > end_row
                    || z.end_row < row
                    || z.start_col > end_col
                    || z.end_col < col
                {
                    return None;
                }
                Some(hm_model::CellZone {
                    start_row: z.start_row.max(row) - row,
                    end_row: z.end_row.min(end_row) - row,
                    start_col: z.start_col.max(col) - col,
                    end_col: z.end_col.min(end_col) - col,
                    border_fill: z.border_fill,
                })
            })
            .collect()
    });
    Ok(t)
}

/// Pastes copied cells over `target` from the cell at `row`, `col` as Hancom's
/// 셀 붙이기 does by default (덮어쓰기): each copied cell that fits replaces the
/// cell it lands on, which keeps its place and size, and what runs past the
/// table's edge is left out (clipboard case c13).
pub(crate) fn overwrite_cells(
    target: &mut Table,
    copied: &Table,
    row: u16,
    col: u16,
) -> Result<()> {
    if row >= target.rows || col >= target.cols || copied.rows == 0 || copied.cols == 0 {
        return Err(bad("the paste starts outside the table"));
    }
    let rows = copied.rows.min(target.rows - row);
    let cols = copied.cols.min(target.cols - col);
    let under = block(target, row, col, row + rows - 1, col + cols - 1)?;
    for c in copied
        .cells
        .iter()
        .filter(|c| c.row + c.row_span <= rows && c.col + c.col_span <= cols)
    {
        under
            .cells
            .iter()
            .find(|d| {
                d.row == c.row
                    && d.col == c.col
                    && d.row_span == c.row_span
                    && d.col_span == c.col_span
            })
            .ok_or_else(|| bad("the copied cells are merged differently from the table's"))?;
        let dest = target
            .cells
            .iter_mut()
            .find(|d| d.row == c.row + row && d.col == c.col + col)
            .ok_or_else(|| bad("the copied cells are merged differently from the table's"))?;
        let geometry = (dest.row, dest.col, dest.width, dest.height);
        *dest = c.clone();
        (dest.row, dest.col, dest.width, dest.height) = geometry;
    }
    Ok(())
}

fn transpose(mut t: Table) -> Table {
    std::mem::swap(&mut t.rows, &mut t.cols);
    std::mem::swap(&mut t.header.common.width, &mut t.header.common.height);
    for c in &mut t.cells {
        std::mem::swap(&mut c.row, &mut c.col);
        std::mem::swap(&mut c.row_span, &mut c.col_span);
        std::mem::swap(&mut c.width, &mut c.height);
    }
    if let Some(zones) = &mut t.zones {
        for z in zones {
            std::mem::swap(&mut z.start_row, &mut z.start_col);
            std::mem::swap(&mut z.end_row, &mut z.end_col);
        }
    }
    t.cells.sort_by_key(|c| (c.row, c.col));
    t.row_cells = (0..t.rows)
        .map(|r| t.cells.iter().filter(|c| c.row == r).count() as u16)
        .collect();
    t
}

fn table_replacement(d: &Inner, at: &TableAddress, table: Table) -> Result<Op> {
    let mut para = object_api::replacement(&d.editor.doc, at)?;
    para.controls[at.control as usize].kind = ControlKind::Table(Box::new(table));
    Ok(object_api::replace(at, para))
}

fn commit(d: &mut Inner, op: Op) -> Result<Receipt> {
    d.not_composing()?;
    let before = d.page_sigs();
    d.edit(|editor| editor.transact(vec![op]))
        .map_err(|(i, e)| Inner::edit_error(i, e))?;
    d.content_epoch += 1;
    d.relayout_edited();
    let scopes = d.editor.changed().map(<[_]>::to_vec);
    Ok(d.receipt_after(None, &before, scopes.as_deref(), None))
}

#[wasm_bindgen]
impl Doc {
    /// Keeps one complete paragraph, including nested controls, in this document.
    #[wasm_bindgen(js_name = captureParagraph)]
    pub fn capture_paragraph(&mut self, address: &str) -> Result<u32> {
        let at: Address = parse(address)?;
        let d = self.raw()?;
        let p = hm_edit::ops::list(&d.editor.doc, &at.pos().list)
            .and_then(|l| l.get(at.para as usize))
            .ok_or_else(|| EngineError::new("BAD_ADDRESS", "no such paragraph"))?
            .clone();
        let id = d.next_checkpoint;
        d.next_checkpoint += 1;
        d.paragraph_captures.insert(id, p);
        Ok(id)
    }

    /// Restores only the addressed paragraph as one undoable edit; keeps the capture.
    #[wasm_bindgen(js_name = restoreParagraph)]
    pub fn restore_paragraph(&mut self, handle: u32, address: &str) -> Result<String> {
        let at: Address = parse(address)?;
        let d = self.inner()?;
        let p = d
            .paragraph_captures
            .get(&handle)
            .ok_or_else(|| EngineError::new("STALE_HANDLE", "no such paragraph capture"))?
            .clone();
        let op = Op::Replace {
            list: at.pos().list,
            start: at.para,
            count: 1,
            paras: vec![p],
        };
        Ok(json(&commit(d, op)?))
    }

    #[wasm_bindgen(js_name = discardParagraphCapture)]
    pub fn discard_paragraph_capture(&mut self, handle: u32) -> Result<bool> {
        Ok(self.raw()?.paragraph_captures.remove(&handle).is_some())
    }

    /// Copies or applies a transposed cell block; writes form one undo step.
    #[wasm_bindgen(js_name = transposeTable)]
    pub fn transpose_table(&mut self, request: &str) -> Result<String> {
        let req: TransposeRequest = parse(request)?;
        let d = self.inner()?;
        let source;
        let mut location = None;
        let mut receipt = None;
        let normal = matches!(
            &req,
            TransposeRequest::Copy {
                transpose: Some(false),
                ..
            } | TransposeRequest::Paste {
                transpose: Some(false),
                ..
            }
        );
        match req {
            TransposeRequest::Copy {
                table,
                start_row,
                start_column,
                end_row,
                end_column,
                transpose,
            } => {
                source = block(
                    d.table_at(&tables::path(&table), table.para, table.control)?,
                    start_row,
                    start_column,
                    end_row,
                    end_column,
                )?;
                if transpose.unwrap_or(true) {
                    d.transpose_clipboard = Some(source.clone());
                } else {
                    let p = &hm_edit::ops::list(&d.editor.doc, &tables::path(&table)).unwrap()
                        [table.para as usize];
                    let offset = p
                        .units()
                        .find_map(|(offset, u)| {
                            matches!(u,Unit::Extended {index,..} if index==table.control)
                                .then_some(offset)
                        })
                        .ok_or_else(|| bad("table has no inline anchor"))?;
                    let from = Pos {
                        list: tables::path(&table),
                        para: table.para,
                        offset,
                    };
                    let to = Pos {
                        offset: offset + 8,
                        ..from.clone()
                    };
                    let mut frag = clip::copy(&d.editor.doc, d.instance, &from, &to)
                        .map_err(|e| Inner::edit_error(0, e))?;
                    frag.paras[0].objects[0].1.kind = ControlKind::Table(Box::new(source.clone()));
                    frag.paras[0].objects[0].1.src = None;
                    *d.clipboard.borrow_mut() = Some(frag);
                    d.cell_clipboard = Some(source.clone());
                }
            }
            TransposeRequest::InPlace { table } => {
                source = d
                    .table_at(&tables::path(&table), table.para, table.control)?
                    .clone();
                receipt = Some(commit(
                    d,
                    table_replacement(d, &table, transpose(source.clone()))?,
                )?);
            }
            TransposeRequest::Paste {
                table,
                start_row,
                start_column,
                transpose: transposed,
            } => {
                source = if transposed.unwrap_or(true) {
                    &d.transpose_clipboard
                } else {
                    &d.cell_clipboard
                }
                .clone()
                .ok_or_else(|| bad("no table-cell clipboard"))?;
                let copied = if transposed.unwrap_or(true) {
                    transpose(source.clone())
                } else {
                    source.clone()
                };
                let mut target = d
                    .table_at(&tables::path(&table), table.para, table.control)?
                    .clone();
                // A transposed block must fit whole; a plain one is cut at the
                // table's edge as Hancom cuts it.
                if transposed.unwrap_or(true)
                    && (u32::from(start_row) + u32::from(copied.rows) > u32::from(target.rows)
                        || u32::from(start_column) + u32::from(copied.cols)
                            > u32::from(target.cols))
                {
                    return Err(bad("표 크기를 초과하는 행/열 바꿈 붙여넣기"));
                }
                overwrite_cells(&mut target, &copied, start_row, start_column)?;
                receipt = Some(commit(d, table_replacement(d, &table, target)?)?);
            }
            TransposeRequest::Insert { at } => {
                source = d
                    .transpose_clipboard
                    .clone()
                    .ok_or_else(|| bad("no transposed table clipboard"))?;
                let mut table = transpose(source.clone());
                // A new control gets its own instance identity.
                table.header.instance_id = None;
                let p = hm_edit::ops::list(&d.editor.doc, &at.pos().list)
                    .and_then(|l| l.get(at.para as usize))
                    .ok_or_else(|| EngineError::new("BAD_ADDRESS", "no such paragraph"))?;
                location = Some((
                    at.para,
                    p.units()
                        .filter(|(offset, u)| {
                            *offset < at.offset && matches!(u, Unit::Extended { .. })
                        })
                        .count(),
                ));
                let control = hm_model::Control {
                    kind: ControlKind::Table(Box::new(table)),
                    src: None,
                };
                let op = object_api::insert(&d.editor.doc, &at, control)?;
                receipt = Some(commit(d, op)?);
            }
        }
        let mut result = serde_json::json!({ "ok":true, "sourceRows":source.rows, "sourceCols":source.cols, "targetRows":source.cols, "targetCols":source.rows, "receipt":receipt });
        if normal {
            result["targetRows"] = source.rows.into();
            result["targetCols"] = source.cols.into();
            let mut rows = vec![vec![String::new(); source.cols as usize]; source.rows as usize];
            let escape = |s: String| {
                s.replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;")
                    .replace('\n', "<br>")
            };
            let mut html = String::from("<table>");
            for row in 0..source.rows {
                html.push_str("<tr>");
                for c in source.cells.iter().filter(|c| c.row == row) {
                    let text = c
                        .paras
                        .iter()
                        .map(hm_edit::export::para_text)
                        .collect::<Vec<_>>()
                        .join("\n");
                    rows[row as usize][c.col as usize] = text.clone();
                    html.push_str(&format!(
                        "<td rowspan=\"{}\" colspan=\"{}\">{}</td>",
                        c.row_span,
                        c.col_span,
                        escape(text)
                    ));
                }
                html.push_str("</tr>");
            }
            html.push_str("</table>");
            result["text"] = rows
                .into_iter()
                .map(|row| row.join("\t"))
                .collect::<Vec<_>>()
                .join("\n")
                .into();
            result["html"] = html.into();
        }
        if let Some((p, c)) = location {
            result["paraIdx"] = p.into();
            result["controlIdx"] = c.into();
        }
        Ok(json(&result))
    }

    #[wasm_bindgen(js_name = hasTableTransposeClipboard)]
    pub fn has_table_transpose_clipboard(&mut self) -> Result<bool> {
        Ok(self.raw()?.transpose_clipboard.is_some())
    }
}
