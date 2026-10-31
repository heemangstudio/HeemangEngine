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
                other => Self::other_op(doc, fonts, other, &mut ops, &mut caret)?,
            }
        }
        Ok((ops, caret))
    }

    // Keep uncommon request branches out of the code compiled for first typing.
    // Share the accumulators: an operation without a caret retains the preceding one.
    #[inline(never)]
    fn other_op(
        doc: &mut hm_model::Document,
        fonts: &FontSystem,
        op: EditOp,
        ops: &mut Vec<Op>,
        caret: &mut Option<Pos>,
    ) -> Result<()> {
        let bad = |e: hm_edit::EditError| EngineError::new("BAD_ADDRESS", e);
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
            EditOp::DeleteStyle { id, replacement } => {
                ops.extend(definitions::delete_style(doc, id, replacement)?)
            }
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
                *caret = Some(Pos {
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
                *caret = Some(Pos {
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
                points,
            } => {
                let mut c = object_api::create_shape(
                    doc,
                    &at,
                    &kind,
                    width,
                    height,
                    points,
                    text,
                    inline.unwrap_or(false),
                    horz_offset.unwrap_or(0),
                    vert_offset.unwrap_or(0),
                )?;
                if let Some(spec) = connector {
                    connectors::attach(&mut c, spec)?;
                }
                ops.push(object_api::insert(doc, &at, c)?);
                *caret = Some(Pos {
                    offset: at.offset + 8,
                    ..at.pos()
                });
            }
            EditOp::DeleteObject { object } => {
                let (op, at) = object_api::delete(doc, &object)?;
                ops.push(op);
                *caret = Some(at);
            }
            EditOp::RelocateObject { object, at } => {
                let (moved, end) = object_api::relocate(doc, &object, &at)?;
                ops.extend(moved);
                *caret = Some(end);
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
            EditOp::Split {
                at,
                page_break,
                next_style,
            } => {
                let at = at.pos();
                *caret = Some(Pos {
                    list: at.list.clone(),
                    para: at.para + 1,
                    offset: 0,
                });
                let restyle = Self::split_restyle(doc, &at, next_style);
                ops.push(Op::Split { at, page_break });
                ops.extend(restyle);
            }
            EditOp::ColumnBreak { at, on } if on != Some(false) => {
                let pos = at.pos();
                let (edits, next) = dialogs::edit(doc, EditOp::ColumnBreak { at, on })?;
                // A column break that splits the paragraph gives the new one what Enter does.
                let restyle = if edits.iter().any(|op| matches!(op, Op::Split { .. })) {
                    Self::split_restyle(doc, &pos, None)
                } else {
                    None
                };
                ops.extend(edits);
                ops.extend(restyle);
                *caret = next;
            }
            EditOp::FormatChars { from, to, patch } => {
                // A border or background change without a base id changes each run's own
                // border fill, so the half the patch leaves out stays as it was.
                let relative = match (&patch.border_fill, patch.border_fill_id) {
                    (Some(value), None) => Some(
                        serde_json::from_value::<object_api::BorderFillPatch>(value.clone())
                            .map_err(|e| EngineError::new("BAD_REQUEST", e))?,
                    ),
                    _ => None,
                };
                let p = match relative {
                    Some(_) => Self::char_patch(
                        doc,
                        fonts,
                        &types::CharPatchJs {
                            border_fill: None,
                            ..patch.clone()
                        },
                    )?,
                    None => Self::char_patch(doc, fonts, &patch)?,
                };
                for (from, mut to) in Self::spans_in(doc, &from.pos(), &to.pos())? {
                    // A range that reaches the last character takes the paragraph's end
                    // mark too, as Hancom's selection to the end does: its shape sizes an
                    // empty last line and a paragraph head.
                    if (from.para, from.offset) < (to.para, to.offset)
                        && let Some(p) =
                            hm_edit::ops::list(doc, &to.list).and_then(|l| l.get(to.para as usize))
                        && to.offset + 1 == p.len()
                    {
                        to.offset += 1;
                    }
                    let Some(bp) = &relative else {
                        ops.extend(hm_edit::script::char_ops(doc, &p, &from, &to).map_err(bad)?);
                        continue;
                    };
                    for (s, e, fill) in Self::border_pieces(doc, &from, &to)? {
                        let q = hm_edit::script::CharPatch {
                            border_fill: Some(object_api::apply_border_fill(doc, fill, bp)?),
                            ..p
                        };
                        ops.extend(hm_edit::script::char_ops(doc, &q, &s, &e).map_err(bad)?);
                    }
                }
            }
            EditOp::FormatParas { from, to, patch } => {
                let (p, tabs) = Self::para_patch(doc, &patch)?;
                for (from, to) in Self::spans_in(doc, &from.pos(), &to.pos())? {
                    let Some(tabs) = &tabs else {
                        ops.extend(
                            hm_edit::script::para_ops(doc, &p, &from.list, from.para, to.para)
                                .map_err(bad)?,
                        );
                        continue;
                    };
                    // Each paragraph keeps what of its own tab definition the patch leaves.
                    for para in from.para..=to.para {
                        let base = hm_edit::ops::list(doc, &from.list)
                            .and_then(|l| l.get(para as usize))
                            .map(|p| p.header.para_shape)
                            .ok_or_else(|| bad(hm_edit::EditError::NoSuchParagraph))?;
                        let p = hm_edit::script::ParaPatch {
                            tab_def: Some(Self::tab_def(doc, base, tabs)?),
                            ..p
                        };
                        ops.extend(
                            hm_edit::script::para_ops(doc, &p, &from.list, para, para)
                                .map_err(bad)?,
                        );
                    }
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
                let at_caret = from == to;
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
                    } else if let Some(old) = at_caret
                        .then(|| {
                            let p = hm_edit::ops::list(doc, &from.list)?.get(from.para as usize)?;
                            let old = doc.info.styles.get(p.header.style as usize)?;
                            Some((p.clone(), old.para_shape, old.char_shape as u32))
                        })
                        .flatten()
                    {
                        // With only the caret in it, Hancom moves the paragraph to the style
                        // attribute by attribute: what it sets itself stays. Applying the
                        // style it already has changes nothing.
                        let (p, para_shape, char_shape) = old;
                        let to = (style.para_shape, style.char_shape as u32);
                        let mut restyle =
                            hm_edit::script::Restyle::new((para_shape, char_shape), to);
                        if let Some(q) = restyle.paragraph(doc, &p, style_id as u8) {
                            ops.push(Op::Replace {
                                list: from.list,
                                start: from.para,
                                count: 1,
                                paras: vec![q],
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
            EditOp::SetListRestart { at, restart } => {
                let at = at.pos();
                let mut p = hm_edit::ops::list(doc, &at.list)
                    .and_then(|l| l.get(at.para as usize))
                    .cloned()
                    .ok_or_else(|| bad(hm_edit::EditError::NoSuchParagraph))?;
                hm_edit::ops::check(&p, at.offset).map_err(bad)?;
                p.list_restart = restart.map(|r| match r {
                    types::ListRestartSpec::Continue => hm_model::ListRestart::Continue,
                    types::ListRestartSpec::Restart { start } => {
                        hm_model::ListRestart::Start(start.max(1))
                    }
                });
                p.src = None;
                p.line_segs.clear();
                ops.push(Op::Replace {
                    list: at.list,
                    start: at.para,
                    count: 1,
                    paras: vec![p],
                });
            }
            EditOp::SetList { from, to, list } => {
                let set = Self::list_patch(doc, list.as_ref())?;
                for (from, to) in Self::spans_in(doc, &from.pos(), &to.pos())? {
                    let bases: Vec<u16> = {
                        let paras = hm_edit::ops::list(doc, &from.list).ok_or_else(|| {
                            EngineError::new("BAD_ADDRESS", hm_edit::EditError::NoSuchList)
                        })?;
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
                *caret = Some(end);
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
                let c = agent_api::create_equation(fonts, &script, size, color, font, baseline)?;
                ops.push(agent_api::insert(doc, &at, c)?);
                *caret = Some(agent_api::after(&at));
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
                        *caret = next;
                    }
                }
            }
        }
        Ok(())
    }

    /// A host paragraph patch for the editor, checked against the document, and the tab
    /// definition change it asks for (resolved per paragraph by [`Inner::tab_def`]).
    pub(super) fn para_patch(
        doc: &mut hm_model::Document,
        patch: &types::ParaPatchJs,
    ) -> Result<(hm_edit::script::ParaPatch, Option<TabChange>)> {
        let bad = |what: String| EngineError::new("BAD_REQUEST", what);
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
        // Distances beyond every page of the document cannot be meant, and would overflow
        // the packed fields. Hancom's own line spacing steps stop at 0 and 500 percent.
        let side = doc
            .sections
            .iter()
            .filter_map(|s| s.definition()?.page_def())
            .map(|p| p.width.max(p.height))
            .max()
            .map_or(MAX_DISTANCE, |m| m.min(MAX_DISTANCE as u32) as i32);
        for (name, v) in [
            ("left", patch.left),
            ("right", patch.right),
            ("indent", patch.indent),
            ("spaceBefore", patch.space_before),
            ("spaceAfter", patch.space_after),
        ] {
            if let Some(v) = v
                && !(-side..=side).contains(&v)
            {
                return Err(bad(format!("{name} {v} is outside -{side}..{side} HU")));
            }
        }
        if let Some(v) = patch.line_spacing {
            // A value without a kind keeps the original API's percent meaning.
            let max = match line_spacing_kind {
                None | Some(0) => 500,
                _ => side as u32,
            };
            if v > max {
                return Err(bad(format!("line spacing {v} is over {max}")));
            }
        }
        let code = |v: Option<u32>, max: u32, what: &str| -> Result<Option<u32>> {
            match v {
                Some(v) if v > max => Err(bad(format!("{what} {v} is over {max}"))),
                v => Ok(v),
            }
        };
        if let Some(id) = patch.border_fill_id
            && id as usize > doc.info.border_fills.len()
        {
            return Err(bad(format!("no border fill {id}")));
        }
        let border_offsets = match patch.border_offsets {
            Some(o) if o.iter().any(|v| i16::try_from(*v).is_err()) => {
                return Err(bad("border offsets are -32768..32767 HU".into()));
            }
            o => o.map(|o| o.map(|v| v as i16)),
        };
        let tabs = match &patch.tab_stops {
            None => None,
            Some(tabs) => {
                if tabs.len() > 256
                    || tabs
                        .iter()
                        .any(|t| t.position < 0 || t.kind > 3 || t.fill > 17)
                {
                    return Err(EngineError::new("BAD_REQUEST", "invalid tab stops"));
                }
                Some(
                    tabs.iter()
                        .map(|t| hm_model::TabStop {
                            position: t.position.saturating_mul(2),
                            kind: t.kind,
                            fill: t.fill,
                            reserved: 0,
                        })
                        .collect(),
                )
            }
        };
        let tab_change =
            (tabs.is_some() || patch.auto_tab_left.is_some() || patch.auto_tab_right.is_some())
                .then(|| TabChange {
                    tabs,
                    left: patch.auto_tab_left,
                    right: patch.auto_tab_right,
                });
        let heading = match &patch.heading {
            None => None,
            // A plain paragraph keeps whatever definition id and level it carries, so a
            // reported head goes back unchanged.
            Some(h) if h.kind == "none" => {
                if h.level > 7 {
                    return Err(bad(format!("level {} is over 7", h.level)));
                }
                Some((0, h.id, h.level))
            }
            Some(h) => {
                let list = types::ListSpec {
                    kind: h.kind.clone(),
                    id: h.id as u32,
                    level: h.level,
                    restart: None,
                };
                Self::list_patch(doc, Some(&list))?.map(|(kind, id, level)| (kind, id, level))
            }
        };
        let p = hm_edit::script::ParaPatch {
            align,
            space_before: patch.space_before,
            space_after: patch.space_after,
            left: patch.left,
            right: patch.right,
            indent: patch.indent,
            line_spacing: patch.line_spacing,
            line_spacing_kind,
            tab_def: None,
            keep_with_next: patch.keep_with_next,
            keep_lines: patch.keep_lines,
            widow_orphan: patch.widow_orphan,
            page_break_before: patch.page_break_before,
            break_latin_word: code(patch.break_latin_word, 2, "breakLatinWord")?,
            break_non_latin_word: code(patch.break_non_latin_word, 1, "breakNonLatinWord")?,
            snap_to_grid: patch.snap_to_grid,
            condense: code(patch.condense, 75, "condense")?,
            font_line_height: patch.font_line_height,
            vertical_align: code(patch.vertical_align, 3, "verticalAlign")?,
            line_wrap: code(patch.line_wrap, 2, "lineWrap")?,
            auto_space_latin: patch.e_asian_eng,
            auto_space_number: patch.e_asian_num,
            border_fill: patch.border_fill_id,
            border_offsets,
            border_connect: patch.border_connect,
            border_ignore_margin: patch.border_ignore_margin,
            heading,
        };
        Ok((p, tab_change))
    }

    /// The tab definition paragraph shape `base` takes under `change`: its own with the
    /// stops or automatic tabs replaced, an equal existing one, else a new entry.
    pub(super) fn tab_def(
        doc: &mut hm_model::Document,
        base: u16,
        change: &TabChange,
    ) -> Result<u16> {
        let current = doc.info.para_shapes.get(base as usize).map(|s| s.tab_def);
        let own = current.and_then(|id| doc.info.tab_defs.get(id as usize));
        let mut attr = own.map_or(0, |d| d.attr);
        let tabs = match &change.tabs {
            Some(tabs) => tabs.clone(),
            None => own.map(|d| d.tabs.clone()).unwrap_or_default(),
        };
        for (bit, on) in [(1, change.left), (2, change.right)] {
            match on {
                Some(true) => attr |= bit,
                Some(false) => attr &= !bit,
                None => {}
            }
        }
        let same = |d: &hm_model::TabDef| d.attr == attr && d.tabs == tabs;
        if let Some(id) = current.filter(|&id| doc.info.tab_defs.get(id as usize).is_some_and(same))
        {
            return Ok(id);
        }
        let id = match doc.info.tab_defs.iter().position(same) {
            Some(id) => id,
            None => {
                doc.info.tab_defs.push(hm_model::TabDef {
                    attr,
                    tabs,
                    ..Default::default()
                });
                doc.info.id_mappings.src = None;
                doc.info.tab_defs.len() - 1
            }
        };
        u16::try_from(id).map_err(|_| EngineError::new("BAD_REQUEST", "too many tab definitions"))
    }

    /// The change splitting the paragraph at `at` brings to the new one: the style's next
    /// style at the paragraph's end (unless `next_style` is false), otherwise the shape
    /// without "page break before" ([`Inner::break_before_stays`]).
    fn split_restyle(
        doc: &mut hm_model::Document,
        at: &Pos,
        next_style: Option<bool>,
    ) -> Option<Op> {
        match next_style {
            Some(false) => None,
            _ => definitions::next_style(doc, at),
        }
        .or_else(|| Self::break_before_stays(doc, at))
    }

    /// Splitting a paragraph that always starts a page leaves that with the first half, as
    /// Hancom's Enter, page break and column break do (parashape case pagebreak-before,
    /// fixes/_paragraphs_review pbb-*): the new paragraph's shape without "page break before".
    fn break_before_stays(doc: &mut hm_model::Document, at: &Pos) -> Option<Op> {
        let base = hm_edit::ops::list(doc, &at.list)?
            .get(at.para as usize)?
            .header
            .para_shape;
        if !doc.info.para_shape(base as u32)?.page_break_before() {
            return None;
        }
        let shape = hm_edit::script::derive_para_shape(doc, base, &|s| s.attr1 &= !(1 << 19));
        Some(Op::SetParaShape {
            list: at.list.clone(),
            first: at.para + 1,
            last: at.para + 1,
            shape,
        })
    }

    /// `[from, to)` split where the border fill of the character shape changes, with that
    /// border fill (0 for none).
    fn border_pieces(
        doc: &hm_model::Document,
        from: &Pos,
        to: &Pos,
    ) -> Result<Vec<(Pos, Pos, u16)>> {
        let paras = hm_edit::ops::list(doc, &from.list)
            .ok_or_else(|| EngineError::new("BAD_ADDRESS", hm_edit::EditError::NoSuchList))?;
        let mut out: Vec<(Pos, Pos, u16)> = Vec::new();
        for pi in from.para..=to.para {
            let p = paras.get(pi as usize).ok_or_else(|| {
                EngineError::new("BAD_ADDRESS", hm_edit::EditError::NoSuchParagraph)
            })?;
            let s = if pi == from.para { from.offset } else { 0 };
            let e = if pi == to.para { to.offset } else { p.len() };
            let mut bounds: Vec<u32> = vec![s];
            bounds.extend(p.runs.iter().map(|r| r.pos).filter(|&q| q > s && q < e));
            bounds.push(e);
            for w in bounds.windows(2).filter(|w| w[1] > w[0]) {
                let fill = doc
                    .info
                    .char_shapes
                    .get(p.shape_at(w[0]) as usize)
                    .and_then(|c| c.border_fill)
                    .unwrap_or(0);
                let at = |offset| Pos {
                    list: from.list.clone(),
                    para: pi,
                    offset,
                };
                match out.last_mut() {
                    Some(last) if last.2 == fill && last.1 == at(w[0]) => last.1 = at(w[1]),
                    _ => out.push((at(w[0]), at(w[1]), fill)),
                }
            }
        }
        Ok(out)
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
            Some("center") => Some(2),
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
        if let Some(size) = p.size
            && !(100..=409_600).contains(&size)
        {
            return Err(EngineError::new(
                "BAD_REQUEST",
                format!("size {size} is outside 100..409600 (1 to 4096 points)"),
            ));
        }
        let mut faces = [None; 7];
        for (k, name) in names.into_iter().enumerate() {
            let Some(name) = name else { continue };
            // A face the document already has in this slot is kept whether or not this
            // host can draw it: hosts send every slot back when one changes.
            let known = doc.info.faces[k]
                .iter()
                .any(|f| f.name.to_string_lossy() == name);
            if !known && !fonts.resolves(name) {
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
            use_font_space: p.use_font_space,
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
            border_fill: match &p.border_fill {
                Some(value) => {
                    let patch = serde_json::from_value(value.clone())
                        .map_err(|e| EngineError::new("BAD_REQUEST", e))?;
                    Some(object_api::apply_border_fill(
                        doc,
                        p.border_fill_id.unwrap_or(0),
                        &patch,
                    )?)
                }
                None => {
                    if let Some(id) = p.border_fill_id {
                        if id as usize > doc.info.border_fills.len() {
                            return Err(EngineError::new("BAD_REQUEST", "no such border fill"));
                        }
                    }
                    p.border_fill_id
                }
            },
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
            "outline" => (1, info.numberings.len()),
            "numbering" => (2, info.numberings.len()),
            "bullet" => (3, info.bullets.len()),
            k => {
                return Err(EngineError::new(
                    "BAD_REQUEST",
                    format!("unknown list kind {k:?}"),
                ));
            }
        };
        if (l.id == 0 && kind != 1) || l.id as usize > count {
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

    /// Whether `op` adds or restarts notes or changes how they number. Edits
    /// that remove or move notes are found from their content after the
    /// transaction ([`hm_edit::Editor::notes_moved`]).
    pub(super) fn changes_note_numbers(op: &EditOp) -> bool {
        match op {
            EditOp::InsertNote { .. }
            | EditOp::InsertNewNumber { .. }
            | EditOp::SetNoteShape { .. } => true,
            EditOp::SetDocumentProperties { patch } => {
                patch.footnote_start.is_some() || patch.endnote_start.is_some()
            }
            _ => false,
        }
    }
}

/// Tab stops and automatic tabs a paragraph patch sets.
pub(super) struct TabChange {
    tabs: Option<Vec<hm_model::TabStop>>,
    left: Option<bool>,
    right: Option<bool>,
}

/// The largest distance the packed paragraph fields hold (one bit marks character units).
const MAX_DISTANCE: i32 = (1 << 30) - 1;
