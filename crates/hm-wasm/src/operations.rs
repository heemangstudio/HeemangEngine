//! Translate typed requests into atomic model operations and formatting patches.
use super::*;

impl Inner {
    /// Typed operations as model operations, with the caret after them.
    pub(super) fn ops(
        doc: &mut hm_model::Document,
        fonts: &FontSystem,
        req: Vec<EditOp>,
    ) -> Result<(Vec<Op>, Option<Pos>)> {
        let mut ops = Vec::new();
        let mut caret = None;
        let bad = |e: hm_edit::EditError| EngineError::new("BAD_ADDRESS", e);
        for op in req {
            match op {
                EditOp::SetCharShape {
                    from,
                    to,
                    char_shape_id,
                } => {
                    if doc.info.char_shape(char_shape_id).is_none() {
                        return Err(EngineError::new("BAD_REQUEST", "no such character shape"));
                    }
                    for (from, to) in Self::spans_in(doc, &from.pos(), &to.pos())? {
                        if from != to {
                            ops.push(Op::SetCharShape {
                                from,
                                to,
                                shape: char_shape_id,
                            });
                        }
                    }
                }
                EditOp::SetParaShape {
                    from,
                    to,
                    para_shape_id,
                } => {
                    if doc.info.para_shape(para_shape_id as u32).is_none() {
                        return Err(EngineError::new("BAD_REQUEST", "no such paragraph shape"));
                    }
                    for (from, to) in Self::spans_in(doc, &from.pos(), &to.pos())? {
                        ops.push(Op::SetParaShape {
                            list: from.list,
                            first: from.para,
                            last: to.para,
                            shape: para_shape_id,
                        });
                    }
                }
                EditOp::CreateStyle { style } => {
                    ops.extend(definitions::style(doc, fonts, None, style)?)
                }
                EditOp::UpdateStyle { id, style } => {
                    ops.extend(definitions::style(doc, fonts, Some(id), style)?)
                }
                EditOp::DeleteStyle { id } => ops.extend(definitions::delete_style(doc, id)?),
                EditOp::EnsureList { definition } => {
                    ops.extend(definitions::ensure_list(doc, definition)?)
                }
                EditOp::CreateTable {
                    at,
                    rows,
                    columns,
                    column_widths,
                    row_heights,
                    inline,
                } => {
                    let c = object_api::create_table(
                        doc,
                        &at,
                        rows,
                        columns,
                        column_widths,
                        row_heights,
                        inline.unwrap_or(true),
                    )?;
                    ops.push(object_api::insert(doc, &at, c)?);
                    caret = Some(Pos {
                        offset: at.offset + 8,
                        ..at.pos()
                    });
                }
                EditOp::InsertPicture {
                    at,
                    bytes,
                    extension,
                    description,
                    width,
                    height,
                    natural_width,
                    natural_height,
                    inline,
                    horz_offset,
                    vert_offset,
                } => {
                    let c = object_api::create_picture(
                        doc,
                        bytes,
                        extension,
                        description,
                        width,
                        height,
                        natural_width,
                        natural_height,
                        inline.unwrap_or(true),
                        horz_offset.unwrap_or(0),
                        vert_offset.unwrap_or(0),
                    )?;
                    ops.push(object_api::insert(doc, &at, c)?);
                    caret = Some(Pos {
                        offset: at.offset + 8,
                        ..at.pos()
                    });
                }
                EditOp::AssignPictureImage {
                    object,
                    bytes,
                    extension,
                    natural_width,
                    natural_height,
                } => {
                    ops.push(object_api::assign_picture_image(
                        doc,
                        &object,
                        bytes,
                        extension,
                        natural_width,
                        natural_height,
                    )?);
                }
                EditOp::CreateShape {
                    at,
                    kind,
                    connector,
                    width,
                    height,
                    text,
                    inline,
                    horz_offset,
                    vert_offset,
                } => {
                    let mut c = object_api::create_shape(
                        doc,
                        &at,
                        &kind,
                        width,
                        height,
                        text,
                        inline.unwrap_or(false),
                        horz_offset.unwrap_or(0),
                        vert_offset.unwrap_or(0),
                    )?;
                    if let Some(spec) = connector {
                        connectors::attach(&mut c, spec)?;
                    }
                    ops.push(object_api::insert(doc, &at, c)?);
                    caret = Some(Pos {
                        offset: at.offset + 8,
                        ..at.pos()
                    });
                }
                EditOp::DeleteObject { object } => {
                    let (op, at) = object_api::delete(doc, &object)?;
                    ops.push(op);
                    caret = Some(at);
                }
                EditOp::RelocateObject { object, at } => {
                    let (moved, end) = object_api::relocate(doc, &object, &at)?;
                    ops.extend(moved);
                    caret = Some(end);
                }
                EditOp::SetObjectProperties { object, patch } => {
                    ops.push(object_api::set_object(doc, fonts, &object, patch)?)
                }
                EditOp::SetTableProperties { table, patch } => {
                    ops.push(object_api::set_table(doc, fonts, &table, patch)?)
                }
                EditOp::SetCellProperties {
                    table,
                    row,
                    column,
                    patch,
                } => ops.push(object_api::set_cell(
                    doc, fonts, &table, row, column, patch,
                )?),
                EditOp::SetCellZone {
                    table,
                    start_row,
                    start_column,
                    end_row,
                    end_column,
                    border_fill,
                } => ops.push(object_api::set_zone(
                    doc,
                    &table,
                    (start_row, start_column, end_row, end_column),
                    &border_fill,
                )?),
                EditOp::InsertText { at, text, format } => {
                    let at = at.pos();
                    if text.is_empty() {
                        continue;
                    }
                    // Each paragraph piece of the text: where it ends.
                    let mut pieces = Vec::new();
                    let mut end = at.clone();
                    let mut parts = text.split('\n');
                    end.offset += wire_len(parts.next().unwrap_or(""));
                    pieces.push(end.clone());
                    for part in parts {
                        end.para += 1;
                        end.offset = wire_len(part);
                        pieces.push(end.clone());
                    }
                    caret = Some(end);
                    // Inserted text takes the shape before `at` (see
                    // `formatContext`); a format lays a copy of it over.
                    let shape = match &format {
                        Some(f) => {
                            let base = hm_edit::ops::list(doc, &at.list)
                                .and_then(|l| l.get(at.para as usize))
                                .map(|p| format::caret_shape(p, at.offset))
                                .ok_or_else(|| bad(hm_edit::EditError::NoSuchParagraph))?;
                            let p = Self::char_patch(doc, fonts, f)?;
                            Some(hm_edit::script::derive_char_shape(doc, base, &|c| {
                                p.apply(c)
                            }))
                        }
                        None => None,
                    };
                    ops.push(Op::InsertText {
                        at: at.clone(),
                        text,
                    });
                    if let Some(shape) = shape {
                        let empty_tail = hm_edit::ops::list(doc, &at.list)
                            .and_then(|l| l.get(at.para as usize))
                            .is_some_and(|p| at.offset == p.len().saturating_sub(1));
                        let last = pieces.len() - 1;
                        let mut from = at;
                        for (i, to) in pieces.into_iter().enumerate() {
                            let mut through = to.clone();
                            // The inserted paragraph break owns its end mark;
                            // a new empty tail also needs the typing size.
                            if i < last || (i > 0 && to.offset == 0 && empty_tail) {
                                through.offset += 1;
                            }
                            if through.offset > from.offset {
                                ops.push(Op::SetCharShape {
                                    from,
                                    to: through,
                                    shape,
                                });
                            }
                            from = Pos {
                                para: to.para + 1,
                                offset: 0,
                                ..to
                            };
                        }
                    }
                }
                EditOp::Delete { from, to } => {
                    let spans = Self::spans_in(doc, &from.pos(), &to.pos())?;
                    caret = spans.first().map(|s| s.0.clone());
                    for (from, to) in spans {
                        if from != to {
                            ops.push(Op::Delete { from, to });
                        }
                    }
                }
                EditOp::Split { at, page_break } => {
                    let at = at.pos();
                    caret = Some(Pos {
                        list: at.list.clone(),
                        para: at.para + 1,
                        offset: 0,
                    });
                    ops.push(Op::Split { at, page_break });
                }
                EditOp::FormatChars { from, to, patch } => {
                    let p = Self::char_patch(doc, fonts, &patch)?;
                    for (from, to) in Self::spans_in(doc, &from.pos(), &to.pos())? {
                        ops.extend(hm_edit::script::char_ops(doc, &p, &from, &to).map_err(bad)?);
                    }
                }
                EditOp::FormatParas { from, to, patch } => {
                    let p = Self::para_patch(&patch)?;
                    for (from, to) in Self::spans_in(doc, &from.pos(), &to.pos())? {
                        ops.extend(
                            hm_edit::script::para_ops(doc, &p, &from.list, from.para, to.para)
                                .map_err(bad)?,
                        );
                    }
                }
                EditOp::ApplyStyle { from, to, style_id } => {
                    let style = doc
                        .info
                        .styles
                        .get(style_id as usize)
                        .filter(|_| style_id <= u8::MAX as u32)
                        .cloned()
                        .ok_or_else(|| {
                            EngineError::new("BAD_REQUEST", format!("no style {style_id}"))
                        })?;
                    for (from, to) in Self::spans_in(doc, &from.pos(), &to.pos())? {
                        if style.attr & 7 == 1 {
                            // A character style formats the range only.
                            if from != to {
                                ops.push(Op::SetCharShape {
                                    from,
                                    to,
                                    shape: style.char_shape as u32,
                                });
                            }
                        } else {
                            ops.push(Op::SetStyle {
                                list: from.list,
                                first: from.para,
                                last: to.para,
                                style: style_id as u8,
                                para_shape: style.para_shape,
                                char_shape: Some(style.char_shape as u32),
                            });
                        }
                    }
                }
                EditOp::SetList { from, to, list } => {
                    let set = Self::list_patch(doc, list.as_ref())?;
                    for (from, to) in Self::spans_in(doc, &from.pos(), &to.pos())? {
                        let bases: Vec<u16> = {
                            let paras = hm_edit::ops::list(doc, &from.list)
                                .ok_or_else(|| bad(hm_edit::EditError::NoSuchList))?;
                            (from.para..=to.para)
                                .map(|i| paras.get(i as usize).map(|p| p.header.para_shape))
                                .collect::<Option<_>>()
                                .ok_or_else(|| bad(hm_edit::EditError::NoSuchParagraph))?
                        };
                        for (i, base) in (from.para..=to.para).zip(bases) {
                            let shape = hm_edit::script::derive_para_shape(doc, base, &|s| {
                                s.attr1 &= !(3 << 23 | 7 << 25);
                                if let Some((kind, id, level)) = set {
                                    s.attr1 |= kind << 23 | level << 25;
                                    s.numbering = id;
                                }
                            });
                            ops.push(Op::SetParaShape {
                                list: from.list.clone(),
                                first: i,
                                last: i,
                                shape,
                            });
                        }
                    }
                }
                EditOp::PageBreak { at, on } => {
                    let at = at.pos();
                    ops.push(Op::SetPageBreak {
                        list: at.list,
                        para: at.para,
                        on,
                    });
                }
                EditOp::SetLineEnds { object, start, end } => ops.push(grouping::set_line_ends(
                    doc,
                    &object,
                    (start[0], start[1]),
                    (end[0], end[1]),
                )?),
                EditOp::UngroupObject { object } => {
                    let (edits, end) = grouping::ungroup(doc, &object)?;
                    ops.extend(edits);
                    caret = Some(end);
                }
                EditOp::GroupObjects { .. } => {
                    return Err(EngineError::new(
                        "BAD_REQUEST",
                        "groupObjects is a transaction operation",
                    ));
                }
                EditOp::MoveObject { object, dx, dy } => {
                    ops.push(objects::edit(
                        doc,
                        fonts,
                        &object,
                        objects::Action::Move { dx, dy },
                    )?);
                }
                EditOp::ResizeObject {
                    object,
                    width,
                    height,
                } => {
                    let action = objects::Action::Resize { width, height };
                    ops.push(objects::edit(doc, fonts, &object, action)?);
                }
                EditOp::ResizeTableCells { table, cells } => {
                    ops.push(objects::edit(
                        doc,
                        fonts,
                        &table,
                        objects::Action::Cells(cells),
                    )?);
                }
                EditOp::InsertEquation {
                    at,
                    script,
                    size,
                    color,
                    font,
                    baseline,
                } => {
                    let c =
                        agent_api::create_equation(fonts, &script, size, color, font, baseline)?;
                    ops.push(agent_api::insert(doc, &at, c)?);
                    caret = Some(agent_api::after(&at));
                }
                EditOp::PromoteOleEquation { object } => {
                    ops.push(agent_api::promote_ole_equation(doc, &object)?)
                }
                EditOp::SetEquation { object, patch } => {
                    ops.push(agent_api::set_equation(doc, fonts, &object, patch)?)
                }
                EditOp::SetTableCaption {
                    table,
                    text,
                    with_number,
                } => ops.push(agent_api::set_table_caption(
                    doc,
                    &table,
                    &text,
                    with_number.unwrap_or(false),
                )?),
                EditOp::SetObjectCaption {
                    object,
                    text,
                    with_number,
                } => ops.push(agent_api::set_object_caption(
                    doc,
                    &object,
                    &text,
                    with_number.unwrap_or(false),
                )?),
                other => {
                    if let Some((table, action)) = tables::request(&other) {
                        ops.push(tables::edit(doc, fonts, table, action)?.0);
                    } else {
                        let other = dialogs::resolve_page_border(doc, other)?;
                        let (edits, next) = dialogs::edit(doc, other)?;
                        ops.extend(edits);
                        if next.is_some() {
                            caret = next;
                        }
                    }
                }
            }
        }
        Ok((ops, caret))
    }

    pub(super) fn para_patch(patch: &types::ParaPatchJs) -> Result<hm_edit::script::ParaPatch> {
        let align = match patch.align.as_deref() {
            None => None,
            Some(a) => Some(
                hm_edit::script::align_code(a)
                    .ok_or_else(|| EngineError::new("BAD_REQUEST", "unknown alignment"))?,
            ),
        };
        let line_spacing_kind = match patch.line_spacing_kind.as_deref() {
            None => None,
            Some("percent") => Some(0),
            Some("fixed") => Some(1),
            Some("added") => Some(2),
            Some("atLeast") => Some(3),
            Some(_) => return Err(EngineError::new("BAD_REQUEST", "unknown line spacing kind")),
        };
        Ok(hm_edit::script::ParaPatch {
            align,
            space_before: patch.space_before,
            space_after: patch.space_after,
            left: patch.left,
            right: patch.right,
            indent: patch.indent,
            line_spacing: patch.line_spacing,
            line_spacing_kind,
        })
    }

    /// A host character patch for the editor: the face found or added in
    /// every slot (it must name a registered family), spacing checked.
    pub(super) fn char_patch(
        doc: &mut hm_model::Document,
        fonts: &FontSystem,
        p: &CharPatchJs,
    ) -> Result<hm_edit::script::CharPatch> {
        let spacing = match p.char_spacing {
            Some(v @ -50..=50) => Some(v as i8),
            Some(v) => {
                return Err(EngineError::new(
                    "BAD_REQUEST",
                    format!("character spacing {v} is outside -50 to 50"),
                ));
            }
            None => None,
        };
        fn slots(
            values: &Option<Vec<Option<i32>>>,
            min: i32,
            max: i32,
        ) -> Result<[Option<i32>; 7]> {
            let mut out = [None; 7];
            if let Some(values) = values {
                if values.len() > 7 || values.iter().flatten().any(|v| !(min..=max).contains(v)) {
                    return Err(EngineError::new(
                        "BAD_REQUEST",
                        "slot patch has invalid length or value",
                    ));
                }
                for (slot, v) in out.iter_mut().zip(values) {
                    *slot = *v;
                }
            }
            Ok(out)
        }
        let spacings = slots(&p.spacings, -50, 50)?.map(|v| v.map(|v| v as i8));
        let ratios = slots(&p.ratios, 50, 200)?.map(|v| v.map(|v| v as u8));
        let rel_sizes = slots(&p.rel_sizes, 10, 250)?.map(|v| v.map(|v| v as u8));
        let offsets = slots(&p.offsets, -100, 100)?.map(|v| v.map(|v| v as i8));
        let code = |v: Option<u32>, max: u32, what: &str| -> Result<Option<u32>> {
            match v {
                Some(v) if v > max => Err(EngineError::new(
                    "BAD_REQUEST",
                    format!("{what} {v} is over {max}"),
                )),
                v => Ok(v),
            }
        };
        let underline_kind = match p.underline_kind.as_deref() {
            None => None,
            Some("none") => Some(0),
            Some("below") => Some(1),
            Some("above") => Some(3),
            Some(k) => {
                return Err(EngineError::new(
                    "BAD_REQUEST",
                    format!("unknown underline kind {k:?}"),
                ));
            }
        };
        let shadow_offset = match p.shadow_offset.as_deref() {
            None => None,
            Some(&[x, y]) if (-100..=100).contains(&x) && (-100..=100).contains(&y) => {
                Some([x as i8, y as i8])
            }
            Some(_) => {
                return Err(EngineError::new(
                    "BAD_REQUEST",
                    "shadowOffset is two percents from -100 to 100",
                ));
            }
        };
        let mut names: [Option<&str>; 7] = [p.font_face.as_deref(); 7];
        if let Some(per_slot) = &p.font_faces {
            if per_slot.len() > 7 {
                return Err(EngineError::new(
                    "BAD_REQUEST",
                    "fontFaces names at most seven slots",
                ));
            }
            for (slot, name) in names.iter_mut().zip(per_slot) {
                if let Some(name) = name {
                    *slot = Some(name);
                }
            }
        }
        let mut faces = [None; 7];
        for (k, name) in names.into_iter().enumerate() {
            let Some(name) = name else { continue };
            if !fonts.resolves(name) {
                return Err(EngineError::new(
                    "BAD_REQUEST",
                    format!("no registered font family {name:?}"),
                ));
            }
            let f = clip::face_named(doc, name);
            faces[k] = Some(clip::face_id(doc, k, &f));
        }
        Ok(hm_edit::script::CharPatch {
            bold: p.bold,
            italic: p.italic,
            underline: p.underline,
            size: p.size,
            color: p.color,
            strike: p.strike,
            faces,
            spacing,
            spacings,
            ratios,
            rel_sizes,
            offsets,
            superscript: p.superscript,
            subscript: p.subscript,
            emboss: p.emboss,
            engrave: p.engrave,
            kerning: p.kerning,
            underline_kind,
            underline_shape: code(p.underline_shape, 15, "underline shape")?,
            underline_color: p.underline_color,
            strike_shape: code(p.strike_shape, 15, "strike shape")?,
            strike_color: p.strike_color,
            outline: code(p.outline, 6, "outline")?,
            shadow: code(p.shadow, 2, "shadow")?,
            shadow_color: p.shadow_color,
            shadow_offset,
            emphasis: code(p.emphasis, 12, "emphasis")?,
            shade_color: p.shade_color,
        })
    }

    /// The head kind, definition id and level a list request sets (none to
    /// leave lists); a restart adds a copy of the numbering starting there.
    pub(super) fn list_patch(
        doc: &mut hm_model::Document,
        list: Option<&types::ListSpec>,
    ) -> Result<Option<(u32, u16, u32)>> {
        let Some(l) = list else { return Ok(None) };
        let info = &mut doc.info;
        let (kind, count) = match l.kind.as_str() {
            "numbering" => (2, info.numberings.len()),
            "bullet" => (3, info.bullets.len()),
            k => {
                return Err(EngineError::new(
                    "BAD_REQUEST",
                    format!("unknown list kind {k:?}"),
                ));
            }
        };
        if l.id == 0 || l.id as usize > count {
            return Err(EngineError::new(
                "BAD_REQUEST",
                format!("no {} {}", l.kind, l.id),
            ));
        }
        if l.level > 6 {
            return Err(EngineError::new(
                "BAD_REQUEST",
                format!("level {} is over 6", l.level),
            ));
        }
        let mut id = l.id;
        if let Some(n) = l.restart {
            if kind != 2 {
                return Err(EngineError::new("BAD_REQUEST", "only a numbering restarts"));
            }
            let mut copy = info.numberings[l.id as usize - 1].clone();
            let n16 = u16::try_from(n).map_err(|_| {
                EngineError::new("BAD_REQUEST", format!("restart {n} is too large"))
            })?;
            copy.start = Some(n16);
            let mut starts = copy
                .level_starts
                .unwrap_or([info.numberings[l.id as usize - 1].start.unwrap_or(1) as u32; 7]);
            starts[l.level as usize] = n;
            copy.level_starts = Some(starts);
            copy.src = None;
            info.numberings.push(copy);
            info.id_mappings.src = None;
            id = info.numberings.len() as u32;
        }
        let id =
            u16::try_from(id).map_err(|_| EngineError::new("BAD_REQUEST", "too many lists"))?;
        Ok(Some((kind, id, l.level)))
    }

    /// Whether `op` adds, removes or restarts a footnote or endnote.
    pub(super) fn changes_note_numbers(doc: &hm_model::Document, op: &EditOp) -> Result<bool> {
        Ok(match op {
            EditOp::InsertNote { .. } | EditOp::InsertNewNumber { .. } => true,
            EditOp::DeleteControl { target } => {
                let list = Address {
                    section: target.section,
                    steps: target.steps.clone(),
                    para: target.para,
                    offset: 0,
                }
                .pos()
                .list;
                notes::is_counted(doc, &list, target.para, target.control)
            }
            EditOp::Delete { from, to } => Self::spans_in(doc, &from.pos(), &to.pos())
                .map(|spans| spans.iter().any(|(a, b)| notes::in_range(doc, a, b)))
                .unwrap_or(false),
            _ => false,
        })
    }
}
