//! Table controls: reading the `tbl ` record subtree into a [`Table`] and
//! writing it back.
//!
//! Below the control header at level `L` Hancom stores, at level `L + 1`:
//! an optional caption (a list header followed by its paragraphs), the
//! TABLE record, then each cell as a list header followed by its
//! paragraphs. List paragraphs sit at the same level as their list header.
//! Records the model does not know (field data, for one) keep their slot
//! among these parts.

use hm_model::{Cell, CellZone, Diagnostic, ExtraRecord, Paragraph, RawRecord, SharedBytes, Table};

use crate::Error;
use crate::body::{encode_list, paragraph_run};
use crate::object::{caption_head, encode_caption, encode_header, object_header};
use crate::record::{Reader, Record, Writer, tag, write_record};

/// Parses a table control. `records` is the control's subtree without its
/// header, `level` the header's level. `None` when the subtree does not
/// have the expected shape; the caller keeps the control raw then.
pub fn parse(
    stream: &SharedBytes,
    header: &SharedBytes,
    records: &[Record],
    level: u16,
    diags: &mut Vec<Diagnostic>,
) -> Option<Table> {
    let child = level + 1;
    let mut t = Table {
        header: object_header(header)?,
        ..Table::default()
    };
    let mut slot = 0u32;
    let mut seen_table = false;
    let mut cells_left = 0usize;
    let mut i = 0;
    while i < records.len() {
        let r = &records[i];
        if r.level == child && r.tag == tag::LIST_HEADER && (seen_table || t.caption.is_none()) {
            if seen_table && cells_left == 0 {
                return None;
            }
            let count = list_count(&r.payload)?;
            let (paras, used) = paragraph_run(stream, &records[i + 1..], child, count, diags);
            if paras.len() != count {
                return None;
            }
            if seen_table {
                t.cells.push(cell(&r.payload, paras).ok()?);
                cells_left -= 1;
            } else {
                let mut c = caption_head(&r.payload)?;
                c.paras = paras;
                t.caption = Some(c);
            }
            slot += 1;
            i += 1 + used;
            continue;
        }
        if r.level == child && r.tag == tag::TABLE && !seen_table {
            table_record(&r.payload, &mut t).ok()?;
            cells_left = t.row_cells.iter().map(|&n| n as usize).sum();
            seen_table = true;
            slot += 1;
            i += 1;
            continue;
        }
        if r.level <= level {
            return None;
        }
        t.extra.push(ExtraRecord {
            slot,
            record: RawRecord {
                tag: r.tag,
                level: r.level - level,
                payload: r.payload.clone(),
            },
        });
        i += 1;
    }
    (seen_table && cells_left == 0).then_some(t)
}

/// The paragraph count at the start of a list header.
fn list_count(b: &[u8]) -> Option<usize> {
    Some(u32::from_le_bytes(b.get(0..4)?.try_into().ok()?) as usize)
}

fn table_record(payload: &SharedBytes, t: &mut Table) -> Result<(), Error> {
    let mut r = Reader::new(payload);
    t.attr = r.u32()?;
    t.rows = r.u16()?;
    t.cols = r.u16()?;
    t.spacing = r.i16()?;
    t.padding = [r.i16()?, r.i16()?, r.i16()?, r.i16()?];
    t.row_cells = (0..t.rows).map(|_| r.u16()).collect::<Result<_, _>>()?;
    t.border_fill = r.u16()?;
    t.zones = r
        .try_read(|r| {
            let n = r.u16()? as usize;
            (0..n)
                .map(|_| {
                    // Stored row first, unlike the public table.
                    Ok(CellZone {
                        start_row: r.u16()?,
                        start_col: r.u16()?,
                        end_row: r.u16()?,
                        end_col: r.u16()?,
                        border_fill: r.u16()?,
                    })
                })
                .collect::<Result<Vec<_>, Error>>()
        })
        .ok();
    t.tail = r.tail();
    Ok(())
}

fn cell(payload: &SharedBytes, paras: Vec<Paragraph>) -> Result<Cell, Error> {
    let mut r = Reader::new(payload);
    r.u32()?;
    let mut cell = Cell {
        list_attr: r.u32()?,
        col: r.u16()?,
        row: r.u16()?,
        col_span: r.u16()?,
        row_span: r.u16()?,
        width: r.u32()? as i32,
        height: r.u32()? as i32,
        padding: [r.i16()?, r.i16()?, r.i16()?, r.i16()?],
        border_fill: r.u16()?,
        tail: r.tail(),
        field_name: None,
        paras,
    };
    cell.field_name = cell_name(&cell.tail);
    Ok(cell)
}

/// Writes a table control at `level`.
pub fn encode(out: &mut Vec<u8>, t: &Table, level: u16) {
    let mut h = Writer::default();
    h.u32(hm_model::CtrlId::TABLE.0);
    encode_header(&mut h, &t.header);
    write_record(out, tag::CTRL_HEADER, level, &h.0);
    let child = level + 1;
    let mut next = 0usize;
    let mut flush = |out: &mut Vec<u8>, upto: u32| {
        while let Some(e) = t.extra.get(next).filter(|e| e.slot <= upto) {
            write_record(out, e.record.tag, level + e.record.level, &e.record.payload);
            next += 1;
        }
    };
    let mut slot = 0u32;
    if let Some(c) = &t.caption {
        flush(out, slot);
        encode_caption(out, c, child);
        slot += 1;
    }
    flush(out, slot);
    let mut w = Writer::default();
    w.u32(t.attr);
    w.u16(t.rows);
    w.u16(t.cols);
    w.i16(t.spacing);
    t.padding.iter().for_each(|&p| w.i16(p));
    t.row_cells.iter().for_each(|&n| w.u16(n));
    w.u16(t.border_fill);
    if let Some(zones) = &t.zones {
        w.u16(zones.len() as u16);
        for z in zones {
            for v in [
                z.start_row,
                z.start_col,
                z.end_row,
                z.end_col,
                z.border_fill,
            ] {
                w.u16(v);
            }
        }
    }
    w.bytes(&t.tail);
    write_record(out, tag::TABLE, child, &w.0);
    slot += 1;
    for c in &t.cells {
        flush(out, slot);
        let mut w = Writer::default();
        w.u32(c.paras.len() as u32);
        w.u32(c.list_attr);
        for v in [c.col, c.row, c.col_span, c.row_span] {
            w.u16(v);
        }
        w.u32(c.width as u32);
        w.u32(c.height as u32);
        c.padding.iter().for_each(|&p| w.i16(p));
        w.u16(c.border_fill);
        if cell_name(&c.tail) == c.field_name {
            w.bytes(&c.tail);
        } else {
            let mut tail = vec![0; 15];
            if c.tail.len() >= 15 && !c.tail.starts_with(b"<") {
                tail.copy_from_slice(&c.tail[..15]);
            }
            let name = c
                .field_name
                .as_ref()
                .map(|n| n.to_string_lossy())
                .unwrap_or_default();
            let units: Vec<u16> = name.encode_utf16().take(u16::MAX as usize).collect();
            tail.extend_from_slice(&(units.len() as u16).to_le_bytes());
            for unit in units {
                tail.extend_from_slice(&unit.to_le_bytes());
            }
            w.bytes(&tail);
        }
        write_record(out, tag::LIST_HEADER, child, &w.0);
        encode_list(out, &c.paras, child);
        slot += 1;
    }
    flush(out, u32::MAX);
}

fn cell_name(tail: &[u8]) -> Option<hm_model::HwpString> {
    let n = u16::from_le_bytes(tail.get(15..17)?.try_into().ok()?) as usize;
    if n == 0 {
        return None;
    }
    let units: Vec<u16> = tail
        .get(17..17 + n * 2)?
        .chunks_exact(2)
        .map(|u| u16::from_le_bytes([u[0], u[1]]))
        .collect();
    Some(hm_model::HwpString::from_text(&String::from_utf16_lossy(
        &units,
    )))
}
