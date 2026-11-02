//! Fields, bookmarks, index marks, form objects, overlaid characters and
//! ruby text: their control headers, control data and form records.

use hm_model::{
    Bookmark, Compose, ComposeFrame, Control, ControlKind, CtrlId, Diagnostic, Dutmal, Field,
    FormObject, HwpString, IndexMark, ParaList, Paragraph, Params, RawRecord, SharedBytes,
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
            memo_in_last_section: false,
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
        CtrlId::COMPOSE if data.is_empty() => {
            let text = r.string().ok()?;
            let frame = r
                .try_read(|r| {
                    let shape = r.u8()?;
                    let inner_size = r.i8()?;
                    let kind = r.u8()?;
                    let n = r.u8()?;
                    let mut char_shapes = Vec::with_capacity(n as usize);
                    for _ in 0..n {
                        char_shapes.push(r.u32()?);
                    }
                    Ok(ComposeFrame {
                        shape,
                        inner_size,
                        kind,
                        char_shapes,
                    })
                })
                .ok();
            Some(ControlKind::Compose(Box::new(Compose {
                text,
                frame,
                tail: r.tail(),
            })))
        }
        CtrlId::DUTMAL if data.is_empty() => {
            let main = r.string().ok()?;
            let sub = r.string().ok()?;
            let mut v = [0u32; 5];
            for x in &mut v {
                *x = r.u32().ok()?;
            }
            let [position, size_ratio, option, style, align] = v;
            Some(ControlKind::Dutmal(Box::new(Dutmal {
                main,
                sub,
                position,
                size_ratio,
                option,
                style,
                align,
                tail: r.tail(),
            })))
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
            write_named_data(out, &f.data, f.name.as_ref(), level);
        }
        ControlKind::Bookmark(b) => {
            w.u32(CtrlId::BOOKMARK.0);
            w.bytes(&b.tail);
            write_record(out, tag::CTRL_HEADER, level, &w.0);
            write_named_data(out, &b.data, b.name.as_ref(), level);
        }
        ControlKind::IndexMark(m) => {
            w.u32(CtrlId::INDEX_MARK.0);
            w.string(&m.keys[0]);
            w.string(&m.keys[1]);
            w.bytes(&m.tail);
            write_record(out, tag::CTRL_HEADER, level, &w.0);
        }
        ControlKind::Compose(c) => {
            w.u32(CtrlId::COMPOSE.0);
            w.string(&c.text);
            if let Some(f) = &c.frame {
                w.u8(f.shape);
                w.i8(f.inner_size);
                w.u8(f.kind);
                w.u8(f.char_shapes.len() as u8);
                f.char_shapes.iter().for_each(|&id| w.u32(id));
            }
            w.bytes(&c.tail);
            write_record(out, tag::CTRL_HEADER, level, &w.0);
        }
        ControlKind::Dutmal(d) => {
            w.u32(CtrlId::DUTMAL.0);
            w.string(&d.main);
            w.string(&d.sub);
            for v in [d.position, d.size_ratio, d.option, d.style, d.align] {
                w.u32(v);
            }
            w.bytes(&d.tail);
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

/// The typed name may have changed since the raw parameter record was read.
/// Patch only that string, preserving unrelated parameter bytes and records.
fn write_named_data(out: &mut Vec<u8>, data: &[RawRecord], name: Option<&HwpString>, level: u16) {
    let Some(name) = name else {
        write_data(out, data, level);
        return;
    };
    if data_name(data).as_ref() == Some(name) {
        write_data(out, data, level);
        return;
    }
    fn name_span(payload: &SharedBytes) -> Option<(usize, usize)> {
        let mut r = Reader::new(payload);
        r.u16().ok()?;
        let count = r.i32().ok()?;
        for _ in 0..count.clamp(0, 64) {
            let id = r.u16().ok()?;
            let kind = r.u16().ok()?;
            let start = r.pos();
            match kind {
                1 => {
                    r.string().ok()?;
                    if id == NAME_ITEM {
                        return Some((start, r.pos()));
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
    let found = data.iter().enumerate().find_map(|(i, r)| {
        (r.tag == CTRL_DATA && r.level == 1)
            .then(|| name_span(&r.payload))
            .flatten()
            .map(|span| (i, span))
    });
    let mut string = Writer::default();
    string.string(name);
    if let Some((i, (start, end))) = found {
        let mut changed = data.to_vec();
        let mut bytes = changed[i].payload.to_vec();
        bytes.splice(start..end, string.0);
        changed[i].payload = SharedBytes::from(bytes);
        write_data(out, &changed, level);
    } else {
        let mut w = Writer::default();
        w.u16(0);
        w.i32(1);
        w.u16(NAME_ITEM);
        w.u16(1);
        w.bytes(&string.0);
        write_record(out, CTRL_DATA, level + 1, &w.0);
        write_data(out, data, level);
    }
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
    let Some((start, memos)) = paras.last().and_then(|p| memo_lists(p, diags)) else {
        return;
    };
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

/// Hancom keeps the memos of a document with several sections at the end of
/// the last one, whichever section holds their fields. Their fields get the
/// text; the records stay in that paragraph for a save in this format.
pub(crate) fn take_memos_of_other_sections(
    sections: &mut [hm_model::Section],
    diags: &mut Vec<Diagnostic>,
) {
    let Some((_, memos)) = sections
        .last()
        .and_then(|s| s.paras.last())
        .and_then(|p| memo_lists(p, diags))
    else {
        return;
    };
    let mut fields = Vec::new();
    for s in sections.iter_mut() {
        memo_fields(&mut s.paras, &mut fields);
    }
    for (number, list) in memos {
        let mut hits = fields
            .iter_mut()
            .filter(|f| f.memo_number() == Some(number) && f.memo.is_none());
        if let (Some(f), None) = (hits.next(), hits.next()) {
            f.memo = Some(list);
            f.memo_in_last_section = true;
        }
    }
}

/// The memo lists after a paragraph's own records: where they start among its
/// extra records, and each one's number and text.
fn memo_lists(
    last: &Paragraph,
    diags: &mut Vec<Diagnostic>,
) -> Option<(usize, Vec<(u32, ParaList)>)> {
    let start = last
        .extra
        .iter()
        .position(|e| e.record.tag == MEMO_LIST && e.record.level == 1)?;
    let direct = (last.text.len() > 1) as u32
        + !last.runs.is_empty() as u32
        + !last.line_segs.is_empty() as u32
        + !last.range_tags.is_empty() as u32
        + last.controls.len() as u32;
    if last.extra[start..].iter().any(|e| e.slot < direct) {
        return None;
    }
    let mut bytes = Vec::new();
    for e in &last.extra[start..] {
        write_record(&mut bytes, e.record.tag, e.record.level, &e.record.payload);
    }
    let stream = SharedBytes::from(bytes);
    let (records, err) = read_records(&stream);
    if err.is_some() {
        return None;
    }
    let mut memos = Vec::new();
    let mut i = 0;
    while i < records.len() {
        let rec = &records[i];
        if rec.tag != MEMO_LIST || rec.level != 1 || rec.payload.len() != 4 {
            return None;
        }
        let number = u32::from_le_bytes(rec.payload[..4].try_into().unwrap());
        let (list, used) = crate::furniture::para_list(&stream, &records[i + 1..], 1, diags)?;
        memos.push((number, list));
        i += 1 + used;
    }
    Some((start, memos))
}

/// The memo field controls of a paragraph list and the lists nested in it.
fn memo_controls<'a>(paras: &'a [Paragraph], out: &mut Vec<&'a Control>) {
    for p in paras {
        for c in &p.controls {
            match &c.kind {
                ControlKind::Field(f) if f.memo.is_some() && !f.memo_in_last_section => out.push(c),
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

#[cfg(test)]
mod other_section_memos {
    use super::*;
    use hm_model::{ExtraRecord, Section};

    fn field(memo: Option<ParaList>) -> Paragraph {
        Paragraph {
            text: vec![13],
            controls: vec![Control {
                src: None,
                kind: ControlKind::Field(Box::new(Field {
                    id: CtrlId::from_chars(b"%unk"),
                    command: HwpString::from_text("MEMO/65535/4/1/2/USER/;"),
                    memo,
                    ..Default::default()
                })),
            }],
            ..Default::default()
        }
    }

    #[test]
    fn memos_at_the_end_of_the_last_section_reach_fields_in_other_sections() {
        let text = ParaList {
            paras: vec![Paragraph {
                text: vec![b'm' as u16, 13],
                ..Default::default()
            }],
            ..Default::default()
        };
        // Records as Hancom writes them, at the end of the last section.
        let (records, _) = encode_memos(&[field(Some(text))]);
        let stream = SharedBytes::from(records);
        let (read, err) = read_records(&stream);
        assert!(err.is_none());
        let last = Paragraph {
            text: vec![13],
            extra: read
                .iter()
                .map(|r| ExtraRecord {
                    slot: 0,
                    record: RawRecord {
                        tag: r.tag,
                        level: r.level,
                        payload: r.payload.clone(),
                    },
                })
                .collect(),
            ..Default::default()
        };
        let mut sections = vec![
            Section {
                paras: vec![field(None)],
                ..Default::default()
            },
            Section {
                paras: vec![last],
                ..Default::default()
            },
        ];
        let mut diags = Vec::new();
        take_memos_of_other_sections(&mut sections, &mut diags);
        let f = sections[0].paras[0].controls[0].as_field().unwrap();
        let memo = f.memo.as_ref().expect("the field has its text");
        assert_eq!(memo.paras[0].text[0], b'm' as u16);
        // The file's records stay; the section holding the field adds none.
        assert!(!sections[1].paras[0].extra.is_empty());
        assert!(encode_memos(&sections[0].paras).0.is_empty());
    }
}
