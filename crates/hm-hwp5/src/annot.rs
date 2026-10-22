//! Fields, bookmarks, index marks and form objects: their control headers,
//! control data and form records.

use hm_model::{
    Bookmark, ControlKind, CtrlId, Field, FormObject, HwpString, IndexMark, Params, RawRecord,
    SharedBytes,
};

use crate::record::{Reader, Record, Writer, tag, write_record};

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
