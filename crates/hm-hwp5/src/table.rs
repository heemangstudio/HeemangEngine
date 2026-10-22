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
                    Ok(CellZone {
                        start_col: r.u16()?,
                        start_row: r.u16()?,
                        end_col: r.u16()?,
                        end_row: r.u16()?,
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
    Ok(Cell {
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
        paras,
    })
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
                z.start_col,
                z.start_row,
                z.end_col,
                z.end_row,
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
        w.bytes(&c.tail);
        write_record(out, tag::LIST_HEADER, child, &w.0);
        encode_list(out, &c.paras, child);
        slot += 1;
    }
    flush(out, u32::MAX);
}
