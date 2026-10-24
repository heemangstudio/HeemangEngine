//! Typed dialog edits translated into the native reversible operations.

use crate::types::*;
use hm_edit::{ListPath, Op, Pos, Step, ops};
use hm_model::*;
use serde_json::{Value, json};

fn bad(message: impl std::fmt::Display) -> EngineError {
    EngineError::new("BAD_REQUEST", message)
}
fn address(message: impl std::fmt::Display) -> EngineError {
    EngineError::new("BAD_ADDRESS", message)
}
fn para<'a>(doc: &'a Document, at: &Pos) -> Result<&'a Paragraph> {
    ops::list(doc, &at.list)
        .and_then(|p| p.get(at.para as usize))
        .ok_or_else(|| address("no such paragraph"))
}
fn replace(at: &Pos, mut p: Paragraph) -> Op {
    p.src = None;
    p.line_segs.clear();
    Op::Replace {
        list: at.list.clone(),
        start: at.para,
        count: 1,
        paras: vec![p],
    }
}
fn control_pos(doc: &Document, target: &TableAddress) -> Result<Pos> {
    let mut pos = Address {
        section: target.section,
        steps: target.steps.clone(),
        para: target.para,
        offset: 0,
    }
    .pos();
    let p = para(doc, &pos)?;
    pos.offset = p
        .units()
        .find_map(|(o, u)| {
            matches!(u, Unit::Extended{index,..} if index == target.control).then_some(o)
        })
        .ok_or_else(|| address("no such control"))?;
    Ok(pos)
}
fn insert(doc: &Document, at: &Pos, kind: ControlKind, code: u16) -> Result<(Op, u32)> {
    let mut p = para(doc, at)?.clone();
    ops::check(&p, at.offset).map_err(address)?;
    // Controls and their wire blocks have the same order.
    let index = p
        .units()
        .filter(|(o, u)| *o < at.offset && matches!(u, Unit::Extended { .. }))
        .count();
    let c = Control { kind, src: None };
    let id = c.id().0;
    let units = [
        code,
        (id & 0xffff) as u16,
        (id >> 16) as u16,
        0,
        0,
        0,
        0,
        code,
    ];
    ops::insert_units(&mut p, at.offset, &units);
    p.controls.insert(index, c);
    Ok((replace(at, p), index as u32))
}
fn empty_list() -> ParaList {
    ParaList {
        count: 1,
        paras: vec![Paragraph {
            text: vec![PARA_END],
            runs: vec![CharRun::default()],
            ..Default::default()
        }],
        ..Default::default()
    }
}
fn side_bits(side: Side) -> u32 {
    match side {
        Side::Both => 0,
        Side::Even => 1,
        Side::Odd => 2,
    }
}
fn section_pos(doc: &Document, section: u32) -> Result<Pos> {
    let at = Pos {
        list: ListPath {
            section,
            steps: vec![],
        },
        para: 0,
        offset: 0,
    };
    para(doc, &at)?;
    Ok(at)
}
fn section_edit(
    doc: &Document,
    section: u32,
    edit: impl FnOnce(&mut SectionDef) -> Result<()>,
) -> Result<Op> {
    let at = section_pos(doc, section)?;
    let mut p = para(doc, &at)?.clone();
    let c = p
        .controls
        .iter_mut()
        .find(|c| matches!(c.kind, ControlKind::Section(_)))
        .ok_or_else(|| address("section has no definition"))?;
    let ControlKind::Section(def) = &mut c.kind else {
        unreachable!()
    };
    edit(def)?;
    c.src = None;
    Ok(replace(&at, p))
}
macro_rules! patch {
    ($to:expr, $from:expr, $($field:ident),+ $(,)?) => {$(if let Some(value) = &$from.$field { $to.$field = value.clone(); })+};
}

/// Matching field ends may occur in a later paragraph; nested field pairs count.
fn field_end(doc: &Document, at: &Pos) -> Option<Pos> {
    let list = ops::list(doc, &at.list)?;
    let mut depth = 0;
    for (pi, p) in list.iter().enumerate().skip(at.para as usize) {
        for (offset, unit) in p.units() {
            if pi == at.para as usize && offset <= at.offset {
                continue;
            }
            match unit {
                Unit::Extended { code: 3, .. } => depth += 1,
                Unit::Inline(4) if depth == 0 => {
                    return Some(Pos {
                        list: at.list.clone(),
                        para: pi as u32,
                        offset,
                    });
                }
                Unit::Inline(4) => depth -= 1,
                _ => {}
            }
        }
    }
    None
}
fn field_value(doc: &Document, at: &Pos, end: &Pos) -> String {
    let list = ops::list(doc, &at.list).expect("field list");
    let mut out = String::new();
    for pi in at.para..=end.para {
        if pi > at.para {
            out.push('\n');
        }
        for (offset, unit) in list[pi as usize].units() {
            if pi == at.para && offset < at.offset + 8 || pi == end.para && offset >= end.offset {
                continue;
            }
            match unit {
                Unit::Char(c) => out.push(c),
                Unit::Inline(9) => out.push('\t'),
                Unit::CharControl(10) => out.push('\u{2028}'),
                _ => {}
            }
        }
    }
    out
}
fn field_kind(f: &Field) -> &'static str {
    match f.kind() {
        FieldKind::ClickHere => "clickHere",
        FieldKind::Hyperlink => "hyperlink",
        FieldKind::Bookmark => "bookmark",
        FieldKind::Date if f.command.to_string_lossy().contains("HH") => "time",
        FieldKind::Date => "date",
        FieldKind::Path => "fileName",
        FieldKind::Summary => "title",
        _ => "unknown",
    }
}

pub fn edit(doc: &Document, op: EditOp) -> Result<(Vec<Op>, Option<Pos>)> {
    let mut out = Vec::new();
    let mut caret = None;
    match op {
        EditOp::SetPageSettings { section, patch: p } => {
            out.push(section_edit(doc, section, |s| {
                if !s
                    .children
                    .iter()
                    .any(|c| matches!(c, SectionChild::Page(_)))
                {
                    s.children.push(SectionChild::Page(PageDef::default()));
                }
                let page = s
                    .children
                    .iter_mut()
                    .find_map(|c| {
                        if let SectionChild::Page(p) = c {
                            Some(p)
                        } else {
                            None
                        }
                    })
                    .unwrap();
                patch!(
                    page,
                    p,
                    width,
                    height,
                    margin_left,
                    margin_right,
                    margin_top,
                    margin_bottom,
                    margin_header,
                    margin_footer,
                    gutter
                );
                if let Some(v) = p.landscape {
                    page.attr = (page.attr & !1) | u32::from(v);
                }
                if let Some(v) = p.binding {
                    if v > 2 {
                        return Err(bad("binding must be 0..2"));
                    }
                    page.attr = (page.attr & !6) | (v << 1);
                }
                let (_, _, w, h) = page.body();
                if page.width == 0
                    || page.height == 0
                    || page.width > i32::MAX as u32
                    || page.height > i32::MAX as u32
                    || w <= 0
                    || h <= 0
                {
                    return Err(bad("paper and body dimensions must be positive HU"));
                }
                Ok(())
            })?)
        }
        EditOp::SetSectionSettings { section, patch: p } => {
            out.push(section_edit(doc, section, |s| {
                patch!(
                    s,
                    p,
                    attr,
                    column_gap,
                    outline_numbering,
                    page_number,
                    figure_number,
                    table_number,
                    equation_number
                );
                if let Some(v) = p.language {
                    s.language = Some(v);
                }
                if let Some(v) = p.tab_stop {
                    if v <= 0 || v > i32::MAX / 2 {
                        return Err(bad("tabStop must be positive HU"));
                    }
                    s.default_tab = v * 2;
                }
                Ok(())
            })?)
        }
        EditOp::SetNoteShape {
            section,
            kind,
            patch: p,
        } => out.push(section_edit(doc, section, |s| {
            let wanted = usize::from(kind == NoteKind::Endnote);
            while s.note_shapes().count() <= wanted {
                s.children.push(SectionChild::NoteShape(NoteShape {
                    start: 1,
                    ..Default::default()
                }));
            }
            let shape = s
                .children
                .iter_mut()
                .filter_map(|c| {
                    if let SectionChild::NoteShape(n) = c {
                        Some(n)
                    } else {
                        None
                    }
                })
                .nth(wanted)
                .unwrap();
            patch!(
                shape,
                p,
                attr,
                user_char,
                prefix,
                suffix,
                start,
                separator_length,
                space_above,
                space_below,
                space_between,
                line_kind,
                line_width,
                line_color
            );
            Ok(())
        })?),
        EditOp::SetPageBorder {
            section,
            side,
            patch: p,
        } => out.push(section_edit(doc, section, |s| {
            let wanted = side_bits(side) as usize;
            while s.page_borders().count() <= wanted {
                s.children
                    .push(SectionChild::PageBorder(PageBorderFill::default()));
            }
            let b = s
                .children
                .iter_mut()
                .filter_map(|c| {
                    if let SectionChild::PageBorder(b) = c {
                        Some(b)
                    } else {
                        None
                    }
                })
                .nth(wanted)
                .unwrap();
            patch!(b, p, attr, offsets, border_fill);
            Ok(())
        })?),
        EditOp::SetColumns { at, patch: p } => {
            let at = at.pos();
            let para = para(doc, &at)?;
            let existing = para
                .controls
                .iter()
                .position(|c| matches!(c.kind, ControlKind::Columns(_)));
            let mut col = existing
                .and_then(|i| {
                    if let ControlKind::Columns(c) = &para.controls[i].kind {
                        Some(c.clone())
                    } else {
                        None
                    }
                })
                .unwrap_or(ColumnDef {
                    attr: (1 << 12) | (1 << 2),
                    ..Default::default()
                });
            if let Some(v) = p.count {
                if !(1..=255).contains(&v) {
                    return Err(bad("column count must be 1..255"));
                }
                col.attr = (col.attr & !(255 << 2)) | (v << 2);
            }
            if let Some(v) = p.kind {
                if v > 2 {
                    return Err(bad("column kind must be 0..2"));
                }
                col.attr = (col.attr & !3) | v;
            }
            if let Some(v) = p.direction {
                if v > 2 {
                    return Err(bad("column direction must be 0..2"));
                }
                col.attr = (col.attr & !(3 << 10)) | (v << 10);
            }
            if let Some(v) = p.equal_widths {
                col.attr = (col.attr & !(1 << 12)) | (u16::from(v) << 12);
            }
            patch!(col, p, gap, columns, line_kind, line_width, line_color);
            if col.gap < 0
                || !col.equal_widths()
                    && (col.columns.len() != col.count() as usize
                        || col.columns.iter().any(|(w, _)| *w == 0))
            {
                return Err(bad(
                    "unequal columns need one nonzero width per column; gap must be nonnegative",
                ));
            }
            if let Some(i) = existing {
                let mut para = para.clone();
                para.controls[i] = Control {
                    kind: ControlKind::Columns(col),
                    src: None,
                };
                out.push(replace(&at, para));
            } else {
                out.push(insert(doc, &at, ControlKind::Columns(col), 2)?.0);
            }
        }
        EditOp::CreateHeaderFooter {
            section,
            kind,
            side,
        } => {
            let mut at = section_pos(doc, section)?;
            let paras = ops::list(doc, &at.list).unwrap();
            let found = paras.iter().enumerate().find_map(|(pi, p)| {
                p.controls.iter().enumerate().find_map(|(ci, c)| {
                    c.as_header_footer()
                        .filter(|h| {
                            h.footer == (kind == FurnitureKind::Footer)
                                && h.attr & 3 == side_bits(side)
                        })
                        .map(|_| (pi as u32, ci as u32))
                })
            });
            let (pi, ci) = if let Some(found) = found {
                found
            } else {
                at.offset = ops::lead(&paras[0]);
                let (op, ci) = insert(
                    doc,
                    &at,
                    ControlKind::HeaderFooter(Box::new(HeaderFooter {
                        footer: kind == FurnitureKind::Footer,
                        attr: side_bits(side),
                        list: empty_list(),
                        ..Default::default()
                    })),
                    16,
                )?;
                out.push(op);
                (0, ci)
            };
            at.list.steps.push(Step {
                para: pi,
                control: ci,
                index: 0,
            });
            at.para = 0;
            at.offset = 0;
            caret = Some(at);
        }
        EditOp::InsertNote { at, kind } => {
            let mut at = at.pos();
            let endnote = kind == NoteKind::Endnote;
            let mut records = Vec::new();
            scan_document(doc, None, &mut records, false)?;
            let n = records
                .iter()
                .filter(|r| r["kind"] == if endnote { "endnote" } else { "footnote" })
                .count() as u32;
            let start = doc
                .sections
                .get(at.list.section as usize)
                .and_then(|s| s.definition())
                .and_then(|s| s.note_shapes().nth(usize::from(endnote)))
                .map(|n| n.start)
                .filter(|n| *n != 0)
                .unwrap_or(if endnote {
                    doc.info.properties.endnote_start.max(1)
                } else {
                    doc.info.properties.footnote_start.max(1)
                });
            let mut list = empty_list();
            // The note's own automatic-number mark is native content, not display text.
            let mut npara = std::mem::take(&mut list.paras[0]);
            let auto = Control {
                kind: ControlKind::AutoNumber(AutoNumber {
                    attr: if endnote { 2 } else { 1 },
                    number: (start as u32 + n).min(u16::MAX as u32) as u16,
                    ..Default::default()
                }),
                src: None,
            };
            let id = auto.id().0;
            npara
                .text
                .splice(0..0, [18, id as u16, (id >> 16) as u16, 0, 0, 0, 0, 18]);
            npara.controls.push(auto);
            list.paras[0] = npara;
            let (op, ci) = insert(
                doc,
                &at,
                ControlKind::Note(Box::new(Note {
                    endnote,
                    number: start as u32 + n,
                    list,
                    ..Default::default()
                })),
                17,
            )?;
            out.push(op);
            at.list.steps.push(Step {
                para: at.para,
                control: ci,
                index: 0,
            });
            at.para = 0;
            at.offset = 8;
            caret = Some(at);
        }
        EditOp::SetPageHide { at, flags } => {
            if flags > 63 {
                return Err(bad("page hide flags must be 0..63"));
            }
            let at = at.pos();
            let mut p = para(doc, &at)?.clone();
            if p.controls
                .iter()
                .any(|c| matches!(c.kind, ControlKind::PageHide(_)))
            {
                let index = p
                    .controls
                    .iter()
                    .position(|c| matches!(c.kind, ControlKind::PageHide(_)))
                    .unwrap() as u32;
                if flags == 0 {
                    let offset = p
                        .units()
                        .find_map(|(o, u)| {
                            matches!(u,Unit::Extended{index:i,..} if i==index).then_some(o)
                        })
                        .unwrap();
                    ops::remove_units(&mut p, offset, offset + 8);
                } else {
                    let c = &mut p.controls[index as usize];
                    let ControlKind::PageHide(h) = &mut c.kind else {
                        unreachable!()
                    };
                    h.attr = (h.attr & !63) | flags;
                    c.src = None;
                }
                out.push(replace(&at, p));
            } else if flags != 0 {
                out.push(
                    insert(
                        doc,
                        &at,
                        ControlKind::PageHide(PageHide {
                            attr: flags,
                            ..Default::default()
                        }),
                        21,
                    )?
                    .0,
                );
            }
        }
        EditOp::InsertBookmark { at, name } => {
            if name.is_empty() {
                return Err(bad("bookmark name must not be empty"));
            }
            out.push(
                insert(
                    doc,
                    &at.pos(),
                    ControlKind::Bookmark(Bookmark {
                        name: Some(HwpString::from_text(&name)),
                        ..Default::default()
                    }),
                    22,
                )?
                .0,
            );
        }
        EditOp::RenameBookmark { target, name } => {
            if name.is_empty() {
                return Err(bad("bookmark name must not be empty"));
            }
            let at = control_pos(doc, &target)?;
            let mut p = para(doc, &at)?.clone();
            let c = &mut p.controls[target.control as usize];
            match &mut c.kind {
                ControlKind::Bookmark(b) => b.name = Some(HwpString::from_text(&name)),
                _ => return Err(bad("target is not a point bookmark")),
            };
            c.src = None;
            out.push(replace(&at, p));
        }
        EditOp::DeleteControl { target } => {
            let at = control_pos(doc, &target)?;
            let p = para(doc, &at)?;
            if matches!(
                p.controls[target.control as usize].kind,
                ControlKind::Section(_) | ControlKind::Columns(_)
            ) {
                return Err(bad(
                    "section and column definitions cannot be deleted as furniture",
                ));
            }
            if matches!(
                p.controls[target.control as usize].kind,
                ControlKind::Field(_)
            ) {
                let end = field_end(doc, &at).ok_or_else(|| bad("field has no matching end"))?;
                let mut to = end.clone();
                to.offset += 8;
                out.push(Op::Delete { from: end, to });
            }
            let mut to = at.clone();
            to.offset += 8;
            out.push(Op::Delete { from: at, to });
        }
        EditOp::InsertField {
            at,
            kind,
            to,
            command,
            name,
            editable,
            text,
        } => {
            let at = at.pos();
            if kind == FieldKindJs::PageNumber {
                if to.is_some() || text.is_some() {
                    return Err(bad(
                        "pageNumber is an automatic-number control and cannot wrap text",
                    ));
                }
                let (op, _) = insert(doc, &at, ControlKind::AutoNumber(AutoNumber::default()), 18)?;
                out.push(op);
                let mut next = at;
                next.offset += 8;
                caret = Some(next);
            } else {
                let id = match kind {
                    FieldKindJs::ClickHere => b"%clk",
                    FieldKindJs::Hyperlink => b"%hlk",
                    FieldKindJs::Bookmark => b"%bmk",
                    FieldKindJs::Date | FieldKindJs::Time => b"%dte",
                    FieldKindJs::FileName => b"%pat",
                    FieldKindJs::Title => b"%smr",
                    FieldKindJs::PageCount => {
                        return Err(EngineError::new(
                            "UNSUPPORTED",
                            "pageCount has no verified native field representation",
                        ));
                    }
                    FieldKindJs::PageNumber => unreachable!(),
                };
                let mut records = Vec::new();
                scan_document(doc, None, &mut records, true)?;
                let instance = records
                    .iter()
                    .filter_map(|r| r["fieldId"].as_u64())
                    .max()
                    .unwrap_or(0)
                    .checked_add(1)
                    .filter(|v| *v <= u32::MAX as u64)
                    .ok_or_else(|| bad("field id space exhausted"))?
                    as u32;
                let command = match command {
                    Some(command) => command,
                    None if matches!(
                        kind,
                        FieldKindJs::Date
                            | FieldKindJs::Time
                            | FieldKindJs::FileName
                            | FieldKindJs::Title
                    ) =>
                    {
                        return Err(EngineError::new(
                            "UNSUPPORTED",
                            "this field kind requires a verified native command supplied by the caller",
                        ));
                    }
                    None => String::new(),
                };
                let field = Field {
                    id: CtrlId::from_chars(id),
                    instance,
                    command: HwpString::from_text(&command),
                    name: name.map(|s| HwpString::from_text(&s)),
                    attr: u32::from(editable.unwrap_or(false)),
                    ..Default::default()
                };
                let (mut op, _) = insert(doc, &at, ControlKind::Field(Box::new(field)), 3)?;
                let Op::Replace { paras, .. } = &mut op else {
                    unreachable!()
                };
                let end = if let Some(to) = to {
                    let to = to.pos();
                    if to.list != at.list
                        || to.para != at.para
                        || to.offset < at.offset
                        || text.is_some()
                    {
                        return Err(bad(
                            "field wrapping requires an ordered range in one paragraph and no text",
                        ));
                    }
                    ops::check(para(doc, &at)?, to.offset).map_err(address)?;
                    to.offset + 8
                } else {
                    let value = text.unwrap_or_default();
                    if value.chars().any(|c| c < ' ') {
                        return Err(bad(
                            "insertField text must be a single paragraph without control characters",
                        ));
                    }
                    let units: Vec<u16> = value.encode_utf16().collect();
                    let end = at.offset + 8;
                    ops::insert_units(&mut paras[0], end, &units);
                    end + units.len() as u32
                };
                ops::insert_units(
                    &mut paras[0],
                    end,
                    &[4, instance as u16, (instance >> 16) as u16, 0, 0, 0, 0, 4],
                );
                let mut next = at;
                next.offset = end;
                caret = Some(next);
                out.push(op);
            }
        }
        EditOp::UpdateField { target, patch } => {
            let at = control_pos(doc, &target)?;
            let mut p = para(doc, &at)?.clone();
            let c = &mut p.controls[target.control as usize];
            let ControlKind::Field(f) = &mut c.kind else {
                return Err(bad("target is not a field"));
            };
            if let Some(s) = patch.command {
                f.command = HwpString::from_text(&s);
            }
            if let Some(s) = patch.name {
                f.name = Some(HwpString::from_text(&s));
            }
            if let Some(b) = patch.editable {
                f.attr = (f.attr & !1) | u32::from(b);
            }
            f.attr |= 1 << 15;
            c.src = None;
            out.push(replace(&at, p));
            if let Some(text) = patch.value {
                let end = field_end(doc, &at).ok_or_else(|| bad("field has no matching end"))?;
                let mut start = at;
                start.offset += 8;
                out.push(Op::Delete {
                    from: start.clone(),
                    to: end,
                });
                out.push(Op::InsertText { at: start, text });
            }
        }
        EditOp::SetDocumentProperties { patch: p } => {
            let mut properties = doc.info.properties.clone();
            properties.section_count =
                u16::try_from(doc.sections.len()).map_err(|_| bad("too many sections"))?;
            patch!(
                properties,
                p,
                page_start,
                footnote_start,
                endnote_start,
                figure_start,
                table_start,
                equation_start,
                caret_list,
                caret_para,
                caret_pos
            );
            properties.src = None;
            out.push(Op::SetProperties { properties });
        }
        EditOp::ColumnBreak { at } => {
            let at = at.pos();
            let mut copy = doc.clone();
            let split = Op::Split {
                at: at.clone(),
                page_break: false,
            };
            hm_edit::apply(&mut copy, &split).map_err(address)?;
            let mut next = at.clone();
            next.para += 1;
            next.offset = 0;
            let mut p = para(&copy, &next)?.clone();
            p.header.break_kind = (p.header.break_kind & !break_kind::PAGE) | break_kind::COLUMN;
            out.push(split);
            out.push(replace(&next, p));
            caret = Some(next);
        }
        EditOp::InsertNewNumber { at, number } => out.push(
            insert(
                doc,
                &at.pos(),
                ControlKind::NewNumber(NewNumber {
                    number,
                    ..Default::default()
                }),
                21,
            )?
            .0,
        ),
        _ => return Err(bad("unknown dialog edit")),
    }
    Ok((out, caret))
}

fn list_json(path: &ListPath) -> Value {
    json!({"section":path.section,"steps":path.steps.iter().map(|s|json!({"para":s.para,"control":s.control,"index":s.index})).collect::<Vec<_>>()})
}
fn scan(doc: &Document, path: &ListPath, out: &mut Vec<Value>, fields_only: bool) -> Result<()> {
    let paras = ops::list(doc, path).ok_or_else(|| address("no such paragraph list"))?;
    for (pi, p) in paras.iter().enumerate() {
        for (offset, u) in p.units() {
            let Unit::Extended { index, .. } = u else {
                continue;
            };
            let Some(c) = p.controls.get(index as usize) else {
                continue;
            };
            let at = Pos {
                list: path.clone(),
                para: pi as u32,
                offset,
            };
            let mut target = list_json(path);
            target["para"] = json!(pi);
            target["control"] = json!(index);
            let nested: Vec<_> = ops::control_lists(c)
                .iter()
                .enumerate()
                .map(|(i, _)| {
                    let mut path = path.clone();
                    path.steps.push(Step {
                        para: pi as u32,
                        control: index,
                        index: i as u32,
                    });
                    path
                })
                .collect();
            let mut r = json!({"target":target,"offset":offset,"kind":crate::control_kind(c),"lists":nested.iter().map(list_json).collect::<Vec<_>>()});
            match &c.kind {
                ControlKind::HeaderFooter(h) => {
                    r["side"] = json!(match h.side() {
                        PageSide::Both => "both",
                        PageSide::Even => "even",
                        PageSide::Odd => "odd",
                    })
                }
                ControlKind::Note(n) => r["number"] = json!(n.number),
                ControlKind::PageHide(h) => r["flags"] = json!(h.attr & 63),
                ControlKind::Field(f) => {
                    r["fieldId"] = json!(f.instance);
                    r["fieldKind"] = json!(field_kind(f));
                    r["command"] = json!(f.command.to_string_lossy());
                    r["name"] = json!(
                        f.name
                            .as_ref()
                            .map(HwpString::to_string_lossy)
                            .unwrap_or_default()
                    );
                    r["editable"] = json!(f.editable());
                    r["guide"] = json!(f.guide().unwrap_or_default());
                    if let Some(end) = field_end(doc, &at) {
                        r["end"] = json!(Address::of(&end));
                        r["value"] = json!(field_value(doc, &at, &end));
                    }
                }
                ControlKind::Bookmark(b) => {
                    r["name"] = json!(
                        b.name
                            .as_ref()
                            .map(HwpString::to_string_lossy)
                            .unwrap_or_default()
                    )
                }
                ControlKind::AutoNumber(a) => {
                    r["number"] = json!(a.number);
                    if a.kind() == NumberKind::Page {
                        r["fieldKind"] = json!("pageNumber");
                    }
                }
                ControlKind::NewNumber(n) => r["number"] = json!(n.number),
                _ => {}
            }
            if !fields_only || matches!(c.kind, ControlKind::Field(_) | ControlKind::Bookmark(_)) {
                out.push(r);
            }
            for path in nested {
                scan(doc, &path, out, fields_only)?;
            }
        }
    }
    Ok(())
}
fn scan_document(
    doc: &Document,
    list: Option<ListAddress>,
    out: &mut Vec<Value>,
    fields_only: bool,
) -> Result<()> {
    if let Some(l) = list {
        scan(
            doc,
            &ListPath {
                section: l.section,
                steps: l
                    .steps
                    .iter()
                    .map(|s| Step {
                        para: s.para,
                        control: s.control,
                        index: s.index,
                    })
                    .collect(),
            },
            out,
            fields_only,
        )?;
    } else {
        for section in 0..doc.sections.len() {
            scan(
                doc,
                &ListPath {
                    section: section as u32,
                    steps: vec![],
                },
                out,
                fields_only,
            )?;
        }
    }
    Ok(())
}
fn note_shape(n: &NoteShape) -> Value {
    json!({"attr":n.attr,"userChar":n.user_char,"prefix":n.prefix,"suffix":n.suffix,"start":n.start,"separatorLength":n.separator_length,"spaceAbove":n.space_above,"spaceBelow":n.space_below,"spaceBetween":n.space_between,"lineKind":n.line_kind,"lineWidth":n.line_width,"lineColor":n.line_color})
}
fn definition(doc: &Document, section: u32) -> Result<Option<&SectionDef>> {
    Ok(doc
        .sections
        .get(section as usize)
        .ok_or_else(|| address("no such section"))?
        .definition())
}
pub fn query(doc: &Document, req: QueryRequest) -> Result<Value> {
    match req {
        QueryRequest::Controls { list } => {
            let mut out = Vec::new();
            scan_document(doc, list, &mut out, false)?;
            Ok(json!(out))
        }
        QueryRequest::Fields { list } => {
            let mut out = Vec::new();
            scan_document(doc, list, &mut out, true)?;
            Ok(json!(out))
        }
        QueryRequest::DocumentProperties => {
            let p = &doc.info.properties;
            Ok(
                json!({"sectionCount":doc.sections.len(),"pageStart":p.page_start,"footnoteStart":p.footnote_start,"endnoteStart":p.endnote_start,"figureStart":p.figure_start,"tableStart":p.table_start,"equationStart":p.equation_start,"caretList":p.caret_list,"caretPara":p.caret_para,"caretPos":p.caret_pos}),
            )
        }
        QueryRequest::NoteShape { section, note_kind } => Ok(definition(doc, section)?
            .and_then(|s| {
                s.note_shapes()
                    .nth(usize::from(note_kind == NoteKind::Endnote))
            })
            .map(note_shape)
            .unwrap_or(Value::Null)),
        QueryRequest::SectionSettings { section } => {
            let sec = doc
                .sections
                .get(section as usize)
                .ok_or_else(|| address("no such section"))?;
            let s = sec.definition();
            let page=s.and_then(|s|s.page_def()).map(|p|json!({"width":p.width,"height":p.height,"marginLeft":p.margin_left,"marginRight":p.margin_right,"marginTop":p.margin_top,"marginBottom":p.margin_bottom,"marginHeader":p.margin_header,"marginFooter":p.margin_footer,"gutter":p.gutter,"landscape":p.landscape(),"binding":(p.attr>>1)&3}));
            let settings=s.map(|s|json!({"attr":s.attr,"columnGap":s.column_gap,"tabStop":s.default_tab/2,"outlineNumbering":s.outline_numbering,"pageNumber":s.page_number,"figureNumber":s.figure_number,"tableNumber":s.table_number,"equationNumber":s.equation_number,"language":s.language}));
            let columns=sec.paras.iter().flat_map(|p|&p.controls).find_map(|c|if let ControlKind::Columns(c)=&c.kind {Some(json!({"count":c.count(),"kind":c.kind(),"direction":(c.attr>>10)&3,"equalWidths":c.equal_widths(),"gap":c.gap,"columns":c.columns,"lineKind":c.line_kind,"lineWidth":c.line_width,"lineColor":c.line_color}))}else{None});
            let borders: Vec<_> = s
                .into_iter()
                .flat_map(|s| s.page_borders())
                .map(|b| json!({"attr":b.attr,"offsets":b.offsets,"borderFill":b.border_fill}))
                .collect();
            Ok(
                json!({"section":section,"page":page,"settings":settings,"columns":columns,"noteShapes":{"footnote":s.and_then(|s|s.note_shapes().next()).map(note_shape),"endnote":s.and_then(|s|s.note_shapes().nth(1)).map(note_shape)},"pageBorders":borders}),
            )
        }
        _ => Err(bad("unknown dialog query")),
    }
}
