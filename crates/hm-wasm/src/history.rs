//! History receipts, checkpoints, undo, redo and object transforms.
use super::*;

impl Inner {
    /// Fails while IME text is in progress: it must be committed or
    /// cancelled first.
    pub(super) fn not_composing(&self) -> Result<()> {
        if self.composition.is_some() {
            return Err(EngineError::new(
                "COMPOSING",
                "commit or cancel the composition first",
            ));
        }
        Ok(())
    }

    /// Takes the IME text out again; returns where it started.
    pub(super) fn uncompose(&mut self) -> Option<Pos> {
        let (at, _, inverse) = self.composition.take()?;
        // The inverse of an insertion just made always applies.
        let _ = hm_edit::apply(&mut self.editor.doc, &inverse);
        Some(at)
    }

    pub(super) fn edit_error(i: usize, e: hm_edit::EditError) -> EngineError {
        EngineError::new("BAD_ADDRESS", format!("operation {i}: {e}"))
    }

    pub(super) fn receipt(&self, caret: Option<Address>) -> Receipt {
        self.receipt_after(caret, &self.page_sigs(), Some(&[]), None)
    }

    /// A fingerprint of each laid-out page: what sits where on it. Kept
    /// until the next layout.
    pub(super) fn page_sigs(&self) -> Rc<[u64]> {
        self.sigs.get_or_init(|| self.fingerprints()).clone()
    }

    fn fingerprints(&self) -> Rc<[u64]> {
        use std::hash::{Hash, Hasher};
        if !self.laid {
            return Rc::new([]);
        }
        self.layout
            .pages
            .iter()
            .map(|p| {
                let mut h = std::collections::hash_map::DefaultHasher::new();
                (p.section, p.number, p.width, p.height).hash(&mut h);
                for l in &p.lines {
                    (l.para, l.line, l.x, l.y, l.vpos).hash(&mut h);
                }
                for t in &p.tables {
                    (t.para, t.control, t.x, t.y, t.top, t.bottom).hash(&mut h);
                    (t.tail, t.tail_lines, t.header, t.caption).hash(&mut h);
                    t.projected.as_ref().map(|p| &p.row_y).hash(&mut h);
                    t.carried
                        .as_deref()
                        .map(|c| {
                            (
                                &c.cells, c.until, c.height, c.end, c.minimum, &c.next, c.spanning,
                            )
                        })
                        .hash(&mut h);
                }
                for o in &p.objects {
                    (
                        o.para, o.control, o.rect.x, o.rect.y, o.rect.w, o.rect.h, o.z,
                    )
                        .hash(&mut h);
                }
                h.finish()
            })
            .collect()
    }

    /// The pages to paint again after an edit: those that hold a changed
    /// paragraph, those laid out differently than before (`before` holds
    /// the earlier fingerprints) and those that went away; `None` for all
    /// (unknown changes, or changes to headers, footers, notes or master
    /// pages).
    pub(super) fn changed_pages(
        &self,
        before: &[u64],
        scopes: Option<&[hm_edit::Changed]>,
    ) -> Option<Vec<usize>> {
        let scopes = scopes?;
        let doc = &self.editor.doc;
        let mut paras: Vec<(u32, u32, u32)> = Vec::new();
        for (list, start, count) in scopes {
            if *count == u32::MAX {
                return None;
            }
            match list.steps.first() {
                None => paras.push((list.section, *start, (*count).max(1))),
                Some(step) => {
                    let c = doc
                        .sections
                        .get(list.section as usize)?
                        .paras
                        .get(step.para as usize)?
                        .controls
                        .get(step.control as usize)?;
                    if matches!(
                        c.kind,
                        hm_model::ControlKind::HeaderFooter(_)
                            | hm_model::ControlKind::Note(_)
                            | hm_model::ControlKind::Section(_)
                    ) {
                        return None;
                    }
                    paras.push((list.section, step.para, 1));
                }
            }
        }
        let now = self.page_sigs();
        let holds = |p: &hm_layout::Page, q: u32| {
            p.lines.iter().any(|l| l.para == q)
                || p.tables.iter().any(|t| t.para == q)
                || p.objects.iter().any(|o| o.para == q)
        };
        let mut out: Vec<usize> = Vec::new();
        for (i, p) in self.layout.pages.iter().enumerate() {
            let moved = before.get(i) != now.get(i);
            let edited = paras.iter().any(|&(s, a, n)| {
                p.section == s && (a.saturating_sub(1)..a.saturating_add(n)).any(|q| holds(p, q))
            });
            if moved || edited {
                out.push(i);
            }
        }
        out.extend(now.len()..before.len());
        Some(out)
    }

    /// A receipt after an edit made with the pages fingerprinted as
    /// `before`, the history changes in `scopes`, and the selection to show.
    pub(super) fn receipt_after(
        &self,
        caret: Option<Address>,
        before: &[u64],
        scopes: Option<&[hm_edit::Changed]>,
        selection: Option<serde_json::Value>,
    ) -> Receipt {
        let (undo, redo) = self.editor.history();
        Receipt {
            content_epoch: self.content_epoch,
            layout_epoch: self.layout_epoch,
            page_count: self.layout.pages.len(),
            caret,
            undo,
            redo,
            changed_pages: self.changed_pages(before, scopes),
            selection,
            history_id: self.editor.history_id(),
            table_changes: Vec::new(),
        }
    }

    /// Every top-level object's box as laid out, by section, paragraph
    /// and control.
    pub(super) fn object_boxes(&self) -> grouping::Boxes {
        let fonts = self.fonts.borrow();
        let mut out = grouping::Boxes::new();
        for i in 0..self.layout.pages.len() {
            for o in objects::page_objects(&self.editor.doc, &self.layout, &fonts, i) {
                if o.steps.is_empty() && !o.furniture {
                    out.entry((o.section, o.para, o.control))
                        .or_insert((i, o.x, o.y, o.width, o.height));
                }
            }
        }
        out
    }

    /// The selection collapsed at `caret`.
    pub(super) fn collapsed(caret: &Address) -> serde_json::Value {
        serde_json::json!({
            "anchor": caret, "focus": caret,
            "anchorAffinity": "downstream", "focusAffinity": "downstream",
        })
    }
}

#[wasm_bindgen]
impl Doc {
    /// Keeps the document and its history to come back to; returns the
    /// checkpoint's handle.
    pub fn checkpoint(&mut self) -> Result<u32> {
        let d = self.inner()?;
        d.not_composing()?;
        let id = d.next_checkpoint;
        d.next_checkpoint += 1;
        let snap = d.editor.snapshot();
        d.checkpoints.insert(id, snap);
        Ok(id)
    }

    /// Puts back a checkpoint's document and history, keeping the
    /// checkpoint; a `Receipt`.
    pub fn restore(&mut self, checkpoint: u32) -> Result<String> {
        let d = self.inner()?;
        d.not_composing()?;
        let snap = d.checkpoints.get(&checkpoint).cloned().ok_or_else(|| {
            EngineError::new("STALE_HANDLE", format!("no checkpoint {checkpoint}"))
        })?;
        d.editor.restore(snap);
        d.content_epoch += 1;
        d.relayout();
        Ok(json(&d.receipt_after(None, &[], None, None)))
    }

    /// Takes `other`'s document in place of this one's, with an empty
    /// history, and closes `other`. This document's checkpoints stay, so a
    /// host can put back what it held before; the result counts as unsaved.
    /// A `Receipt`.
    pub fn adopt(&mut self, mut other: Doc) -> Result<String> {
        let next = other
            .inner
            .take()
            .ok_or_else(|| EngineError::new("CLOSED", "the adopted document is closed"))?;
        let d = self.raw()?;
        d.not_composing()?;
        d.editor.replace_document(next.editor.doc);
        d.saved_history = u64::MAX;
        d.content_epoch += 1;
        d.picture_sizes.clear();
        d.last_picture = None;
        d.relayout();
        Ok(json(&d.receipt_after(None, &[], None, None)))
    }

    /// Keeps a drawing's placement and transform (its box, matrices,
    /// rotation, flips and crop) under a handle, for `swapObjectTransform`.
    #[wasm_bindgen(js_name = captureObjectTransform)]
    pub fn capture_object_transform(&mut self, object: &str) -> Result<u32> {
        let at: types::TableAddress = parse(object)?;
        let d = self.raw()?;
        let kept = object_api::capture_transform(&d.editor.doc, &at)?;
        let id = d.next_checkpoint;
        d.next_checkpoint += 1;
        d.transforms.insert(id, kept);
        Ok(id)
    }

    /// Exchanges a kept transform with the object's current one as one
    /// undo step, keeping the current one under the same handle; a
    /// `Receipt`.
    #[wasm_bindgen(js_name = swapObjectTransform)]
    pub fn swap_object_transform(&mut self, handle: u32) -> Result<String> {
        let d = self.inner()?;
        d.not_composing()?;
        let mut kept = d.transforms.remove(&handle).ok_or_else(|| {
            EngineError::new("STALE_HANDLE", format!("no object transform {handle}"))
        })?;
        let op = object_api::swap_transform(&d.editor.doc, &mut kept);
        d.transforms.insert(handle, kept);
        let before = d.page_sigs();
        let op = op?;
        d.edit(|editor| editor.transact(vec![op]))
            .map_err(|(i, e)| Inner::edit_error(i, e))?;
        d.content_epoch += 1;
        d.relayout();
        let scopes = d.editor.changed().map(<[_]>::to_vec);
        Ok(json(&d.receipt_after(
            None,
            &before,
            scopes.as_deref(),
            None,
        )))
    }

    /// Drops a kept transform; false when there was none.
    #[wasm_bindgen(js_name = discardObjectTransform)]
    pub fn discard_object_transform(&mut self, handle: u32) -> Result<bool> {
        Ok(self.raw()?.transforms.remove(&handle).is_some())
    }

    /// Drops a checkpoint; false when there was none.
    #[wasm_bindgen(js_name = releaseCheckpoint)]
    pub fn release_checkpoint(&mut self, checkpoint: u32) -> Result<bool> {
        Ok(self.raw()?.checkpoints.remove(&checkpoint).is_some())
    }

    /// Reverts the last transaction; a `Receipt`, or `null` when there is
    /// nothing to undo.
    pub fn undo(&mut self) -> Result<String> {
        let d = self.inner()?;
        d.not_composing()?;
        let before = d.page_sigs();
        let done = d
            .editor
            .undo()
            .map_err(|e| EngineError::new("EDIT_FAILED", e))?;
        if !done {
            return Ok("null".into());
        }
        d.content_epoch += 1;
        d.relayout();
        let scopes = d.editor.changed().map(<[_]>::to_vec);
        let selection = d
            .editor
            .returned()
            .and_then(|s| serde_json::from_str(s).ok());
        Ok(json(&d.receipt_after(
            None,
            &before,
            scopes.as_deref(),
            selection,
        )))
    }

    /// Applies the last undone transaction again; like [`Doc::undo`].
    pub fn redo(&mut self) -> Result<String> {
        let d = self.inner()?;
        d.not_composing()?;
        let before = d.page_sigs();
        let done = d
            .editor
            .redo()
            .map_err(|e| EngineError::new("EDIT_FAILED", e))?;
        if !done {
            return Ok("null".into());
        }
        d.content_epoch += 1;
        d.relayout();
        let scopes = d.editor.changed().map(<[_]>::to_vec);
        let selection = d
            .editor
            .returned()
            .and_then(|s| serde_json::from_str(s).ok());
        Ok(json(&d.receipt_after(
            None,
            &before,
            scopes.as_deref(),
            selection,
        )))
    }
}
