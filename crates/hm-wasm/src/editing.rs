//! Atomic transactions and IME composition.
use super::*;

#[wasm_bindgen]
impl Doc {
    /// Applies edits atomically as one undo step and lays out again;
    /// returns a `Receipt`. With `expectedEpoch` set, a document edited
    /// since fails with `STALE_EPOCH` and nothing changes.
    pub fn transact(&mut self, request: &str) -> Result<String> {
        let req: TransactRequest = parse(request)?;
        let d = self.inner()?;
        d.not_composing()?;
        if let Some(e) = req.expected_epoch
            && e != d.content_epoch
        {
            return Err(EngineError::new(
                "STALE_EPOCH",
                format!("expected epoch {e}, the document is at {}", d.content_epoch),
            ));
        }
        let before = d.page_sigs();
        let mut caret = None;
        let explicit_after = req.after_selection.clone();
        let mut after = req.after_selection.or_else(|| req.selection.clone());
        let mut table_changes = Vec::new();
        let fonts = d.fonts.clone();
        // Grouping places members by where the layout before this
        // transaction shows them.
        let boxes = if req
            .ops
            .iter()
            .any(|op| matches!(op, EditOp::GroupObjects { .. }))
        {
            d.object_boxes()
        } else {
            Default::default()
        };
        // Ops that add, remove or restart notes or change a note shape are
        // followed, once, by a renumbering translated against the document
        // the last op left. Numbers move no addresses.
        let renumber = std::cell::Cell::new(false);
        // Deletions that cut a field in two are followed, once, by removing the markers
        // they left alone, after every requested op so the requested addresses hold.
        let mut cut_fields: Vec<(hm_edit::ListPath, hm_edit::ops::FieldMark)> = Vec::new();
        let balance = std::cell::Cell::new(false);
        let mut queue = req.ops.into_iter();
        let requests = std::iter::from_fn(|| {
            queue
                .next()
                .or_else(|| renumber.replace(false).then_some(EditOp::RenumberNotes {}))
                .or_else(|| balance.replace(false).then_some(EditOp::BalanceFields {}))
        });
        let changed = d.edit(|editor| {
            editor.transact_mapped(
                requests,
                req.selection.as_ref().map(|v| v.to_string()),
                |doc, op| {
                    if Inner::changes_note_numbers(&op) {
                        renumber.set(true);
                    }
                    if matches!(op, EditOp::BalanceFields {}) {
                        let ops = Inner::balance_fields(
                            doc,
                            &std::mem::take(&mut cut_fields),
                            &mut caret,
                        );
                        if explicit_after.is_none()
                            && let Some(c) = &caret
                        {
                            after = Some(Inner::collapsed(&Address::of(c)));
                        }
                        return Ok::<_, EngineError>((ops, after.as_ref().map(|v| v.to_string())));
                    }
                    let (ops, next) = match tables::request(&op) {
                        _ if matches!(op, EditOp::GroupObjects { .. }) => {
                            let EditOp::GroupObjects { objects } = op else {
                                unreachable!()
                            };
                            let (ops, at) = grouping::group(doc, &objects, &boxes)?;
                            (ops, Some(at))
                        }
                        Some((table, action)) => {
                            let (op, change) = tables::edit(doc, &fonts.borrow(), table, action)?;
                            if explicit_after.is_none() {
                                after = after.take().map(|v| tables::remap_selection(v, &change));
                            }
                            if let Some(pos) = caret.take() {
                                let mut address = Address::of(&pos);
                                if tables::remap_address(&mut address, &change) {
                                    caret = Some(address.pos());
                                }
                            }
                            table_changes.push(change);
                            (vec![op], None)
                        }
                        None => Inner::ops(doc, &fonts.borrow(), vec![op])?,
                    };
                    for op in &ops {
                        if let Op::Delete { from, to } = op
                            && let Some(list) = hm_edit::ops::list(doc, &from.list)
                        {
                            for mark in hm_edit::ops::cut_field_partners(list, from, to) {
                                cut_fields.push((from.list.clone(), mark));
                                balance.set(true);
                            }
                        }
                    }
                    if let Some(next) = next {
                        if explicit_after.is_none() {
                            after = Some(Inner::collapsed(&Address::of(&next)));
                        }
                        caret = Some(next);
                    }
                    Ok::<_, EngineError>((ops, after.as_ref().map(|v| v.to_string())))
                },
            )
        })?;
        if !changed {
            return Ok(json(&d.receipt(None)));
        }
        d.content_epoch += 1;
        d.relayout_edited();
        let scopes = d.editor.changed().map(<[_]>::to_vec);
        let mut receipt = d.receipt_after(
            caret.as_ref().map(Address::of),
            &before,
            scopes.as_deref(),
            after,
        );
        receipt.table_changes = table_changes;
        Ok(json(&receipt))
    }

    /// Shows IME text in progress at `at` (or where the composition began),
    /// replacing the text shown before; laid out as typed text but kept out
    /// of the history. Returns a `Receipt` whose caret ends the text.
    pub fn compose(&mut self, request: &str) -> Result<String> {
        let req: ComposeRequest = parse(request)?;
        let shown = req.text.unwrap_or_default();
        if shown.contains('\n') {
            return Err(EngineError::new(
                "BAD_REQUEST",
                "composition text has a line break",
            ));
        }
        let d = self.inner()?;
        let before = d.page_sigs();
        let at = match d.uncompose() {
            Some(at) => at,
            None => req
                .at
                .ok_or_else(|| EngineError::new("BAD_REQUEST", "a new composition needs `at`"))?
                .pos(),
        };
        d.check(&at)?;
        let len = wire_len(&shown);
        if len > 0 {
            let op = Op::InsertText {
                at: at.clone(),
                text: shown,
            };
            let inverse =
                hm_edit::apply(&mut d.editor.doc, &op).map_err(|e| Inner::edit_error(0, e))?;
            d.editor.touch(&at.list, at.para);
            d.composition = Some((at.clone(), len, inverse));
        }
        d.content_epoch += 1;
        d.relayout();
        let scope = [(at.list.clone(), at.para, 1)];
        let end = Pos {
            offset: at.offset + len,
            ..at
        };
        Ok(json(&d.receipt_after(
            Some(Address::of(&end)),
            &before,
            Some(&scope),
            None,
        )))
    }

    /// Ends the composition with `text` (the shown text when absent) as one
    /// undo step; a `Receipt`.
    #[wasm_bindgen(js_name = commitComposition)]
    pub fn commit_composition(&mut self, request: &str) -> Result<String> {
        let req: ComposeRequest = parse(request)?;
        let d = self.inner()?;
        let shown = d
            .composition
            .as_ref()
            .map(|(at, len, _)| (at.clone(), *len));
        let Some((at, len)) = shown else {
            return Err(EngineError::new(
                "BAD_REQUEST",
                "no composition in progress",
            ));
        };
        let text = match req.text {
            Some(t) => t,
            None => {
                let p = hm_edit::ops::list(&d.editor.doc, &at.list)
                    .and_then(|l| l.get(at.para as usize))
                    .ok_or_else(|| EngineError::new("BAD_ADDRESS", "no such paragraph"))?;
                let (s, offsets) = text::text(p);
                s.chars()
                    .zip(&offsets)
                    .filter(|(_, o)| **o >= at.offset && **o < at.offset + len)
                    .map(|(c, _)| c)
                    .collect()
            }
        };
        let before = d.page_sigs();
        d.uncompose();
        let end = Pos {
            offset: at.offset + wire_len(&text),
            ..at.clone()
        };
        let scope = [(at.list.clone(), at.para, 1)];
        let typed = !text.is_empty();
        if typed {
            let after = Some(Inner::collapsed(&Address::of(&end)).to_string());
            d.edit(|editor| editor.transact_with(vec![Op::InsertText { at, text }], None, after))
                .map_err(|(i, e)| Inner::edit_error(i, e))?;
        }
        d.content_epoch += 1;
        if typed {
            d.relayout_edited();
        } else {
            d.relayout();
        }
        Ok(json(&d.receipt_after(
            Some(Address::of(&end)),
            &before,
            Some(&scope),
            None,
        )))
    }

    /// Takes the IME text out; a `Receipt` with the caret where it began,
    /// or `null` when nothing was composing.
    #[wasm_bindgen(js_name = cancelComposition)]
    pub fn cancel_composition(&mut self) -> Result<String> {
        let d = self.inner()?;
        let before = d.page_sigs();
        let Some(at) = d.uncompose() else {
            return Ok("null".into());
        };
        d.content_epoch += 1;
        d.relayout();
        let scope = [(at.list.clone(), at.para, 1)];
        Ok(json(&d.receipt_after(
            Some(Address::of(&at)),
            &before,
            Some(&scope),
            None,
        )))
    }
}

impl Inner {
    /// Deletes the field markers `cut` names that have no partner left, last first, and
    /// moves the caret back over those before it in its paragraph.
    pub(crate) fn balance_fields(
        doc: &hm_model::Document,
        cut: &[(hm_edit::ListPath, hm_edit::ops::FieldMark)],
        caret: &mut Option<Pos>,
    ) -> Vec<Op> {
        let mut lists: Vec<&hm_edit::ListPath> = Vec::new();
        for (l, _) in cut {
            if !lists.contains(&l) {
                lists.push(l);
            }
        }
        let mut ops = Vec::new();
        for path in lists {
            let marks: Vec<_> = cut
                .iter()
                .filter(|(l, _)| l == path)
                .map(|(_, m)| *m)
                .collect();
            let Some(list) = hm_edit::ops::list(doc, path) else {
                continue;
            };
            let mut gone = hm_edit::ops::unmatched_field_marks(list, &marks);
            gone.sort_by(|a, b| b.cmp(a));
            for &(para, offset) in &gone {
                let from = Pos {
                    list: path.clone(),
                    para,
                    offset,
                };
                let to = Pos {
                    offset: offset + 8,
                    ..from.clone()
                };
                ops.push(Op::Delete { from, to });
            }
            if let Some(c) = caret.as_mut()
                && &c.list == path
            {
                let before = gone
                    .iter()
                    .filter(|&&(p, o)| p == c.para && o < c.offset)
                    .count() as u32;
                c.offset -= 8 * before;
            }
        }
        ops
    }
}
