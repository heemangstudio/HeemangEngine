//! Page furniture controls: headers, footers, notes, numbers, page number
//! position, page hiding, the odd/even adjustment and master page lists.
//!
//! A control becomes typed only when its records have exactly the expected
//! shape; anything else stays raw so it still saves byte for byte.

use hm_model::{
    AutoNumber, ControlKind, CtrlId, Diagnostic, HeaderFooter, NewNumber, Note, PageHide,
    PageNumberPos, PageParity, ParaList, SharedBytes,
};

use crate::body::{encode_list, paragraph_run};
use crate::record::{Reader, Record, Writer, tag, write_record};

/// Parses a list header and the paragraphs after it at `level`. Returns
/// the list and the number of records consumed (list header included).
pub(crate) fn para_list(
    stream: &SharedBytes,
    records: &[Record],
    level: u16,
    diags: &mut Vec<Diagnostic>,
) -> Option<(ParaList, usize)> {
    let head = records.first()?;
    if head.tag != tag::LIST_HEADER || head.level != level {
        return None;
    }
    let mut r = Reader::new(&head.payload);
    let count = r.u32().ok()?;
    let attr = r.u32().ok()?;
    // Lists with a text area (headers, footers, master pages) store its
    // size and two reference bytes; note lists store eight zero bytes.
    let (text_size, refs) = if r.remaining() >= 10 {
        (
            Some((r.i32().ok()?, r.i32().ok()?)),
            Some((r.u8().ok()?, r.u8().ok()?)),
        )
    } else {
        (None, None)
    };
    let tail = r.tail();
    let (paras, used) = paragraph_run(stream, &records[1..], level, count as usize, diags);
    if paras.len() != count as usize {
        return None;
    }
    Some((
        ParaList {
            count,
            attr,
            text_size,
            refs,
            tail,
            paras,
        },
        used + 1,
    ))
}

pub(crate) fn encode_para_list(out: &mut Vec<u8>, list: &ParaList, level: u16) {
    let mut w = Writer::default();
    w.u32(list.paras.len() as u32);
    w.u32(list.attr);
    if let (Some((width, height)), Some((text, number))) = (list.text_size, list.refs) {
        w.i32(width);
        w.i32(height);
        w.u8(text);
        w.u8(number);
    }
    w.bytes(&list.tail);
    write_record(out, tag::LIST_HEADER, level, &w.0);
    encode_list(out, &list.paras, level);
}

/// A typed furniture control from its header payload and descendant
/// records (`children` excludes the control header at `level`).
pub(crate) fn parse(
    id: CtrlId,
    stream: &SharedBytes,
    payload: &SharedBytes,
    children: &[Record],
    level: u16,
    diags: &mut Vec<Diagnostic>,
) -> Option<ControlKind> {
    let mut r = Reader::new(payload);
    r.u32().ok()?;
    let with_list = |diags: &mut Vec<Diagnostic>| {
        let (list, used) = para_list(stream, children, level + 1, diags)?;
        (used == children.len()).then_some(list)
    };
    let kind = match id {
        CtrlId::HEADER | CtrlId::FOOTER => {
            let attr = r.u32().ok()?;
            let tail = r.tail();
            ControlKind::HeaderFooter(Box::new(HeaderFooter {
                footer: id == CtrlId::FOOTER,
                attr,
                tail,
                list: with_list(diags)?,
            }))
        }
        CtrlId::FOOTNOTE | CtrlId::ENDNOTE => {
            let number = r.u32().ok()?;
            let prefix = r.u16().ok()?;
            let suffix = r.u16().ok()?;
            let tail = r.tail();
            ControlKind::Note(Box::new(Note {
                endnote: id == CtrlId::ENDNOTE,
                number,
                prefix,
                suffix,
                tail,
                list: with_list(diags)?,
            }))
        }
        _ if !children.is_empty() => return None,
        CtrlId::AUTO_NUMBER => ControlKind::AutoNumber(AutoNumber {
            attr: r.u32().ok()?,
            number: r.u16().ok()?,
            user_char: r.u16().ok()?,
            prefix: r.u16().ok()?,
            suffix: r.u16().ok()?,
            tail: r.tail(),
        }),
        CtrlId::NEW_NUMBER => ControlKind::NewNumber(NewNumber {
            attr: r.u32().ok()?,
            number: r.u16().ok()?,
            tail: r.tail(),
        }),
        CtrlId::PAGE_NUMBER_POS => ControlKind::PageNumber(PageNumberPos {
            attr: r.u32().ok()?,
            user_char: r.u16().ok()?,
            prefix: r.u16().ok()?,
            suffix: r.u16().ok()?,
            side: r.u16().ok()?,
            tail: r.tail(),
        }),
        CtrlId::PAGE_HIDE => ControlKind::PageHide(PageHide {
            attr: r.u32().ok()?,
            tail: r.tail(),
        }),
        CtrlId::PAGE_PARITY => ControlKind::PageParity(PageParity {
            attr: r.u32().ok()?,
            tail: r.tail(),
        }),
        _ => return None,
    };
    Some(kind)
}

/// Encodes a furniture control at `level`; false for other kinds.
pub(crate) fn encode(out: &mut Vec<u8>, kind: &ControlKind, level: u16) -> bool {
    let mut w = Writer::default();
    let list = match kind {
        ControlKind::HeaderFooter(h) => {
            w.u32(
                if h.footer {
                    CtrlId::FOOTER
                } else {
                    CtrlId::HEADER
                }
                .0,
            );
            w.u32(h.attr);
            w.bytes(&h.tail);
            Some(&h.list)
        }
        ControlKind::Note(n) => {
            w.u32(
                if n.endnote {
                    CtrlId::ENDNOTE
                } else {
                    CtrlId::FOOTNOTE
                }
                .0,
            );
            w.u32(n.number);
            w.u16(n.prefix);
            w.u16(n.suffix);
            w.bytes(&n.tail);
            Some(&n.list)
        }
        ControlKind::AutoNumber(a) => {
            w.u32(CtrlId::AUTO_NUMBER.0);
            w.u32(a.attr);
            for v in [a.number, a.user_char, a.prefix, a.suffix] {
                w.u16(v);
            }
            w.bytes(&a.tail);
            None
        }
        ControlKind::NewNumber(n) => {
            w.u32(CtrlId::NEW_NUMBER.0);
            w.u32(n.attr);
            w.u16(n.number);
            w.bytes(&n.tail);
            None
        }
        ControlKind::PageNumber(p) => {
            w.u32(CtrlId::PAGE_NUMBER_POS.0);
            w.u32(p.attr);
            for v in [p.user_char, p.prefix, p.suffix, p.side] {
                w.u16(v);
            }
            w.bytes(&p.tail);
            None
        }
        ControlKind::PageHide(h) => {
            w.u32(CtrlId::PAGE_HIDE.0);
            w.u32(h.attr);
            w.bytes(&h.tail);
            None
        }
        ControlKind::PageParity(p) => {
            w.u32(CtrlId::PAGE_PARITY.0);
            w.u32(p.attr);
            w.bytes(&p.tail);
            None
        }
        _ => return false,
    };
    write_record(out, tag::CTRL_HEADER, level, &w.0);
    if let Some(list) = list {
        encode_para_list(out, list, level + 1);
    }
    true
}

/// The last-page master page Hancom stores at the end of a section: a list
/// header at level 1 after the last paragraph, whose byte 18 is 3 (7 marks
/// a master page for a chosen page).
pub(crate) fn last_page_master(
    stream: &SharedBytes,
    diags: &mut Vec<Diagnostic>,
) -> Option<ParaList> {
    let (records, _) = crate::record::read_records(stream);
    let at = records
        .iter()
        .position(|r| r.tag == tag::LIST_HEADER && r.level == 1 && r.payload.get(18) == Some(&3))?;
    para_list(stream, &records[at..], 1, diags).map(|(list, _)| list)
}
