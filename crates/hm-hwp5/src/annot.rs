//! Fields, bookmarks, index marks and form objects: their control headers,
//! control data and form records.

use hm_model::{
    Bookmark, Control, ControlKind, CtrlId, Diagnostic, Field, FormObject, HwpString, IndexMark,
    ParaList, Paragraph, Params, RawRecord, SharedBytes,
};

use crate::record::{Reader, Record, Writer, read_records, tag, write_record};

/// Control data (parameter sets naming fields and bookmarks).
pub const CTRL_DATA: u16 = 0x57;
/// A form object's type and property sets.
pub const FORM_OBJECT: u16 = 0x5B;

/// The parameter item id Hancom uses for a field or bookmark name.
const NAME_ITEM: u16 = 0x4000;

fn raw(rec: &Record, level: u16) -> RawRecord {
    RawRecord {
        tag: rec.tag,
        level: rec.level.saturating_sub(level),
        payload: rec.payload.clone(),
    }
}

/// The name item of a control data parameter set.
fn data_name(data: &[RawRecord]) -> Option<HwpString> {
    let rec = data.iter().find(|r| r.tag == CTRL_DATA && r.level == 1)?;
    let mut r = Reader::new(&rec.payload);
    r.u16().ok()?;
    // Hancom stores the item count in 32 bits.
    let count = r.i32().ok()?;
    for _ in 0..count.clamp(0, 64) {
        let id = r.u16().ok()?;
        let kind = r.u16().ok()?;
        match kind {
            1 => {
                let s = r.string().ok()?;
                if id == NAME_ITEM {
                    return Some(s);
                }
            }
            0 | 2..=9 => {
                r.u32().ok()?;
            }
            _ => return None,
        }
    }
    None
}

/// Reads an annotation control; `None` leaves it raw.
pub(crate) fn parse(
    id: CtrlId,
    payload: &SharedBytes,
    children: &[Record],
    level: u16,
) -> Option<ControlKind> {
    let data: Vec<RawRecord> = children.iter().map(|r| raw(r, level)).collect();
    let mut r = Reader::new(payload);
    r.u32().ok()?;
    if id.chars()[0] == b'%' {
        if data.iter().any(|d| d.tag != CTRL_DATA) {
            return None;
        }
        let attr = r.u32().ok()?;
        let props = r.u8().ok()?;
        let command = r.string().ok()?;
        let instance = r.u32().ok()?;
        return Some(ControlKind::Field(Box::new(Field {
            id,
            attr,
            props,
            command,
            instance,
            tail: r.tail(),
            name: data_name(&data),
            data,
            memo: None,
        })));
    }
    match id {
        CtrlId::BOOKMARK if data.iter().all(|d| d.tag == CTRL_DATA) => {
            Some(ControlKind::Bookmark(Bookmark {
                name: data_name(&data),
                data,
                tail: r.tail(),
            }))
        }
        CtrlId::INDEX_MARK if data.is_empty() => {
            let keys = [r.string().ok()?, r.string().ok()?];
            Some(ControlKind::IndexMark(IndexMark {
                keys,
                tail: r.tail(),
            }))
        }
        CtrlId::FORM => {
            let [rec] = children else {
                return None;
            };
            if rec.tag != FORM_OBJECT || rec.level != level + 1 {
                return None;
            }
            let header = crate::object::object_header(payload)?;
            let mut f = Reader::new(&rec.payload);
            let kind = CtrlId(f.u32().ok()?);
            f.u32().ok()?;
            let count = f.u32().ok()? as usize;
            f.u16().ok()?;
            let mut units = Vec::with_capacity(count);
            for _ in 0..count {
                units.push(f.u16().ok()?);
            }
            if f.remaining() != 0 {
                return None;
            }
            let text = String::from_utf16_lossy(&units);
            let props = Params::parse(&text);
            // Keep only what reproduces the record exactly.
            if props.to_text() != text {
                return None;
            }
            Some(ControlKind::Form(Box::new(FormObject {
                header,
                kind,
                props,
                items: Vec::new(),
                tail: SharedBytes::empty(),
            })))
        }
        _ => None,
    }
}

/// Encodes an annotation control at `level`; false for other kinds.
pub(crate) fn encode(out: &mut Vec<u8>, kind: &ControlKind, level: u16) -> bool {
    let mut w = Writer::default();
    match kind {
        ControlKind::Field(f) => {
            w.u32(f.id.0);
            w.u32(f.attr);
            w.u8(f.props);
            w.string(&f.command);
            w.u32(f.instance);
            w.bytes(&f.tail);
            write_record(out, tag::CTRL_HEADER, level, &w.0);
            write_data(out, &f.data, level);
        }
        ControlKind::Bookmark(b) => {
            w.u32(CtrlId::BOOKMARK.0);
            w.bytes(&b.tail);
            write_record(out, tag::CTRL_HEADER, level, &w.0);
            write_data(out, &b.data, level);
        }
        ControlKind::IndexMark(m) => {
            w.u32(CtrlId::INDEX_MARK.0);
            w.string(&m.keys[0]);
            w.string(&m.keys[1]);
            w.bytes(&m.tail);
            write_record(out, tag::CTRL_HEADER, level, &w.0);
        }
        ControlKind::Form(f) => {
            w.u32(CtrlId::FORM.0);
            crate::object::encode_header(&mut w, &f.header);
            write_record(out, tag::CTRL_HEADER, level, &w.0);
            let text: Vec<u16> = f.props.to_text().encode_utf16().collect();
            let mut r = Writer::default();
            r.u32(f.kind.0);
            r.u32(f.kind.0);
            r.u32(text.len() as u32);
            r.u16(text.len() as u16);
            text.iter().for_each(|&u| r.u16(u));
            write_record(out, FORM_OBJECT, level + 1, &r.0);
        }
        _ => return false,
    }
    true
}

fn write_data(out: &mut Vec<u8>, data: &[RawRecord], level: u16) {
    for d in data {
        write_record(out, d.tag, level + d.level, &d.payload);
    }
}

/// A memo's number and text list (`MEMO_LIST`, then a list header and its
/// paragraphs), stored after the records of a section's last paragraph.
pub const MEMO_LIST: u16 = 0x5D;

/// The memo fields of a paragraph list and every list nested in it.
fn memo_fields<'a>(paras: &'a mut [Paragraph], out: &mut Vec<&'a mut Field>) {
    for p in paras {
        for c in &mut p.controls {
            match &mut c.kind {
                ControlKind::Field(f) if f.memo_number().is_some() => out.push(f),
                ControlKind::Table(t) => {
                    for cell in &mut t.cells {
                        memo_fields(&mut cell.paras, out);
                    }
                }
                ControlKind::HeaderFooter(h) => memo_fields(&mut h.list.paras, out),
                ControlKind::Note(n) => memo_fields(&mut n.list.paras, out),
                _ => {}
            }
        }
    }
}

/// Moves the memo lists after a section's last paragraph into their
/// fields. Leaves everything as stored unless every list finds its field.
pub(crate) fn take_memos(paras: &mut [Paragraph], diags: &mut Vec<Diagnostic>) {
    let Some(last) = paras.last() else { return };
    let Some(start) = last
        .extra
        .iter()
        .position(|e| e.record.tag == MEMO_LIST && e.record.level == 1)
    else {
        return;
    };
    let direct = (last.text.len() > 1) as u32
        + !last.runs.is_empty() as u32
        + !last.line_segs.is_empty() as u32
        + !last.range_tags.is_empty() as u32
        + last.controls.len() as u32;
    if last.extra[start..].iter().any(|e| e.slot < direct) {
        return;
    }
    let mut bytes = Vec::new();
    for e in &last.extra[start..] {
        write_record(&mut bytes, e.record.tag, e.record.level, &e.record.payload);
    }
    let stream = SharedBytes::from(bytes);
    let (records, err) = read_records(&stream);
    if err.is_some() {
        return;
    }
    let mut memos = Vec::new();
    let mut i = 0;
    while i < records.len() {
        let rec = &records[i];
        if rec.tag != MEMO_LIST || rec.level != 1 || rec.payload.len() != 4 {
            return;
        }
        let number = u32::from_le_bytes(rec.payload[..4].try_into().unwrap());
        let Some((list, used)) = crate::furniture::para_list(&stream, &records[i + 1..], 1, diags)
        else {
            return;
        };
        memos.push((number, list));
        i += 1 + used;
    }
    let mut fields = Vec::new();
    memo_fields(paras, &mut fields);
    let mut slots = Vec::with_capacity(memos.len());
    for (number, _) in &memos {
        let mut hits = fields
            .iter()
            .enumerate()
            .filter(|(_, f)| f.memo_number() == Some(*number) && f.memo.is_none());
        match (hits.next(), hits.next()) {
            (Some((k, _)), None) if !slots.contains(&k) => slots.push(k),
            _ => return,
        }
    }
    for ((_, list), k) in memos.into_iter().zip(slots) {
        fields[k].memo = Some(list);
    }
    drop(fields);
    if let Some(last) = paras.last_mut() {
        last.extra.truncate(start);
    }
}

/// The memo field controls of a paragraph list and the lists nested in it.
fn memo_controls<'a>(paras: &'a [Paragraph], out: &mut Vec<&'a Control>) {
    for p in paras {
        for c in &p.controls {
            match &c.kind {
                ControlKind::Field(f) if f.memo.is_some() => out.push(c),
                ControlKind::Table(t) => {
                    for cell in &t.cells {
                        memo_controls(&cell.paras, out);
                    }
                }
                ControlKind::HeaderFooter(h) => memo_controls(&h.list.paras, out),
                ControlKind::Note(n) => memo_controls(&n.list.paras, out),
                _ => {}
            }
        }
    }
}

/// A section's memo lists in number order, encoded for the end of its last
/// paragraph, and whether all of them are as read (the last paragraph's
/// source then still holds them).
pub(crate) fn encode_memos(paras: &[Paragraph]) -> (Vec<u8>, bool) {
    let mut controls = Vec::new();
    memo_controls(paras, &mut controls);
    let clean = controls.iter().all(|c| c.is_clean());
    let mut memos: Vec<(u32, &ParaList)> = controls
        .iter()
        .filter_map(|c| {
            let f = c.as_field()?;
            Some((f.memo_number()?, f.memo.as_ref()?))
        })
        .collect();
    memos.sort_by_key(|m| m.0);
    let mut out = Vec::new();
    for (number, list) in memos {
        write_record(&mut out, MEMO_LIST, 1, &number.to_le_bytes());
        crate::furniture::encode_para_list(&mut out, list, 1);
    }
    (out, clean)
}
