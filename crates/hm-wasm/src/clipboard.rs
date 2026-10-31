//! Engine-native document fragments and paste transactions.
use super::*;

#[wasm_bindgen]
impl Doc {
    /// Copies `[from, to)` (`CopyRequest`) as `Copied` text and HTML for
    /// the system clipboard. The fragment is shared by this Engine's
    /// documents until the next copy, survives source closure, and carries
    /// face names for paste into another document.
    pub fn copy(&mut self, request: &str) -> Result<String> {
        let req: CopyRequest = parse(request)?;
        let d = self.inner()?;
        let range = if req.from.section != req.to.section
            && req.from.steps.is_empty()
            && req.to.steps.is_empty()
        {
            range::Range::Text(req.from.pos(), req.to.pos())
        } else {
            d.range(&req.from.pos(), &req.to.pos())?
        };
        let (from, to) = match range {
            range::Range::Text(from, to) => (from, to),
            range::Range::Cells {
                list,
                para,
                control,
                rows,
                cols,
            } => {
                // A block of cells copies as rows of tab-separated text and
                // an HTML table.
                let t = d.table_at(&list, para, control)?;
                let mut text = String::new();
                let mut html = String::from("<table>");
                for r in rows.0..=rows.1 {
                    let mut row: Vec<String> = Vec::new();
                    for i in range::block_cells(t, rows, cols) {
                        let c = &t.cells[i];
                        if c.row != r {
                            continue;
                        }
                        let cell: Vec<String> =
                            c.paras.iter().map(hm_edit::export::para_text).collect();
                        row.push(cell.join("\n"));
                    }
                    if row.is_empty() {
                        continue;
                    }
                    text.push_str(
                        &row.iter()
                            .map(|c| c.replace('\n', " "))
                            .collect::<Vec<_>>()
                            .join("\t"),
                    );
                    text.push('\n');
                    html.push_str("<tr>");
                    for c in &row {
                        let escaped = c
                            .replace('&', "&amp;")
                            .replace('<', "&lt;")
                            .replace('>', "&gt;")
                            .replace('\n', "<br>");
                        html.push_str(&format!("<td>{escaped}</td>"));
                    }
                    html.push_str("</tr>");
                }
                html.push_str("</table>");
                *d.clipboard.borrow_mut() = None;
                return Ok(json(&Copied {
                    text,
                    html,
                    objects: 0,
                }));
            }
        };
        let mut frag = clip::copy(&d.editor.doc, d.instance, &from, &to)
            .map_err(|e| Inner::edit_error(0, e))?;
        frag.origin = d.origin(&frag, &from)?;
        let out = Copied {
            text: frag.text(),
            html: frag.html(),
            objects: frag.objects(),
        };
        *d.clipboard.borrow_mut() = Some(frag);
        Ok(json(&out))
    }

    /// Pastes at `at`, or over `replace` (one list), as one undo step
    /// (`PasteRequest`): the engine's fragment when `text` is absent or is
    /// the fragment's text, else `text` as typed. A `Receipt` with the caret
    /// after the paste; on failure nothing changes, epochs included.
    pub fn paste(&mut self, request: &str) -> Result<String> {
        let req: PasteRequest = parse(request)?;
        self.paste_request(req, None)
    }

    /// Imports a source range through the same atomic paste path, without
    /// reading or replacing the Engine's user clipboard.
    #[wasm_bindgen(js_name = pasteFrom)]
    pub fn paste_from(&mut self, source: &Doc, range: &str, request: &str) -> Result<String> {
        let range: CopyRequest = parse(range)?;
        let req: PasteRequest = parse(request)?;
        let source = source
            .inner
            .as_ref()
            .ok_or_else(|| EngineError::new("CLOSED", "the source document is closed"))?;
        let from = range.from.pos();
        let to = range.to.pos();
        source.check(&from)?;
        source.check(&to)?;
        let fragment = clip::copy(&source.editor.doc, source.instance, &from, &to)
            .map_err(|e| Inner::edit_error(0, e))?;
        self.paste_request(req, Some(fragment))
    }
}

impl Inner {
    /// The origin of a fragment holding one floating object of the body
    /// and nothing else.
    fn origin(&mut self, frag: &clip::Fragment, from: &Pos) -> Result<Option<clip::Origin>> {
        let [p] = frag.paras.as_slice() else {
            return Ok(None);
        };
        let [(_, c)] = p.objects.as_slice() else {
            return Ok(None);
        };
        let Some(common) = c.object_common().filter(|o| !o.treat_as_char()) else {
            return Ok(None);
        };
        if !from.list.steps.is_empty() || p.runs.iter().any(|r| !r.text.is_empty()) {
            return Ok(None);
        }
        let control = hm_edit::ops::list(&self.editor.doc, &from.list)
            .and_then(|l| l.get(from.para as usize))
            .and_then(|q| {
                q.units().find_map(|(offset, u)| match u {
                    Unit::Extended { index, .. } if offset >= from.offset => Some(index),
                    _ => None,
                })
            });
        let Some(control) = control else {
            return Ok(None);
        };
        let top = self.lines(from)?.first().map(|l| (l.page, l.y));
        Ok(Some(clip::Origin {
            at: (from.list.section, from.para, control),
            common,
            top,
            pasted: Default::default(),
        }))
    }

    /// Places a pasted copy of a floating object as Hancom does while the
    /// original still stands: one more step right and down for every copy
    /// pasted, on the page where the original is, even when anchored to
    /// another paragraph (h09a: offset 5000 in a paragraph 3200 lower
    /// becomes 3217). After a cut the stored offsets stay (lane h26-paste
    /// c3). Returns the copy count to keep once the paste succeeds.
    fn place_copy(&mut self, frag: &mut clip::Fragment, at: &Pos) -> Result<Option<CopyCount>> {
        let Some(o) = frag.origin.clone() else {
            return Ok(None);
        };
        if frag.instance != self.instance || !at.list.steps.is_empty() {
            return Ok(None);
        }
        let (section, para, control) = o.at;
        let standing = self
            .editor
            .doc
            .sections
            .get(section as usize)
            .and_then(|s| s.paras.get(para as usize))
            .and_then(|p| p.controls.get(control as usize))
            .and_then(|c| c.object_common())
            == Some(o.common);
        if !standing {
            return Ok(None);
        }
        let n = o.pasted.get() + 1;
        let step = clip::COPY_STEP.saturating_mul(n);
        let mut dy = step;
        if o.common.vert_rel_to() == 2 {
            let here = self.lines(at)?.first().map(|l| (l.page, l.y));
            if let (Some((_, was)), Some((_, now))) = (o.top, here) {
                dy = dy.saturating_add(was - now);
            }
        }
        let c = &mut frag.paras[0].objects[0].1;
        crate::object_api::shift(c, step, dy);
        Ok(Some((o.pasted, n)))
    }
}

/// A fragment's count of pasted copies and the value it takes when the
/// paste placing the next copy succeeds.
type CopyCount = (std::rc::Rc<std::cell::Cell<i32>>, i32);

impl Doc {
    fn paste_request(
        &mut self,
        req: PasteRequest,
        fragment: Option<clip::Fragment>,
    ) -> Result<String> {
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
        let kept = req.selection.clone().or_else(|| {
            req.replace
                .as_ref()
                .map(|r| {
                    serde_json::json!({
                        "anchor": r.from, "focus": r.to,
                        "anchorAffinity": "downstream", "focusAffinity": "downstream",
                    })
                })
                .or_else(|| req.at.as_ref().map(Inner::collapsed))
        });
        let (at, cut) = match (req.at, req.replace) {
            (Some(at), None) => {
                let at = at.pos();
                d.check(&at)?;
                (at, None)
            }
            (None, Some(r)) if r.from.pos().list != r.to.pos().list => {
                return Err(EngineError::new(
                    "BAD_REQUEST",
                    "a paste replaces a range in one paragraph list",
                ));
            }
            (None, Some(r)) => match d.range(&r.from.pos(), &r.to.pos())? {
                range::Range::Text(from, to) => {
                    let cut = (from != to).then(|| Op::Delete {
                        from: from.clone(),
                        to,
                    });
                    (from, cut)
                }
                range::Range::Cells { .. } => {
                    return Err(EngineError::new(
                        "BAD_REQUEST",
                        "a paste replaces a range in one paragraph list",
                    ));
                }
            },
            _ => {
                return Err(EngineError::new(
                    "BAD_REQUEST",
                    "a paste takes exactly one of `at` and `replace`",
                ));
            }
        };
        let mut frag = fragment.or_else(|| d.clipboard.borrow().clone());
        let mut copies = None;
        if let Some(f) = &mut frag {
            copies = d.place_copy(f, &at)?;
        }
        let text = req
            .text
            .map(|t| t.replace("\r\n", "\n").replace('\r', "\n"));
        let frag = frag.filter(|f| text.as_ref().is_none_or(|t| *t == f.text()));
        if frag.is_none() {
            copies = None;
        }
        // Pasting may add shapes and faces to the document tables; a
        // failure anywhere puts them back.
        let tables = d.editor.doc.info.clone();
        let bins = d.editor.doc.bins.clone();
        let made = match (frag, text) {
            (Some(f), _) => clip::paste(&mut d.editor.doc, d.instance, &f, &at, cut.as_ref())
                .map_err(|e| Inner::edit_error(0, e)),
            (None, Some(t)) => Inner::ops(
                &mut d.editor.doc,
                &d.fonts.borrow(),
                vec![EditOp::InsertText {
                    at: Address::of(&at),
                    text: t,
                    format: None,
                }],
            )
            .map(|(ops, caret)| (ops, caret.unwrap_or_else(|| at.clone()))),
            (None, None) => Err(EngineError::new("BAD_REQUEST", "nothing to paste")),
        };
        let (mut ops, end) = match made {
            Ok(x) => x,
            Err(e) => {
                d.editor.doc.info = tables;
                d.editor.doc.bins = bins;
                return Err(e);
            }
        };
        // A cut that takes one marker of a field removes the field's other marker once
        // the paste is in place, as typing over a selection does.
        let cut_fields: Vec<_> = match &cut {
            Some(Op::Delete { from, to }) => hm_edit::ops::list(&d.editor.doc, &from.list)
                .map(|l| hm_edit::ops::cut_field_partners(l, from, to))
                .unwrap_or_default()
                .into_iter()
                .map(|m| (from.list.clone(), m))
                .collect(),
            _ => Vec::new(),
        };
        // The paste's operations hold after the cut: it only removes text
        // after `at`, and the shape pasted over is the one before it.
        if let Some(cut) = cut {
            ops.insert(0, cut);
        }
        let before = d.page_sigs();
        let explicit_after = req.after_selection;
        let mut end = Some(end);
        let mut after = None;
        let mut ops = Some(ops);
        if let Err((i, e)) = d.edit(|editor| {
            editor.transact_mapped(
                [false, true],
                kept.map(|v| v.to_string()),
                |doc, balance| {
                    if !balance {
                        return Ok((ops.take().unwrap_or_default(), None));
                    }
                    let tail = Inner::balance_fields(doc, &cut_fields, &mut end);
                    let a = explicit_after.clone().unwrap_or_else(|| {
                        Inner::collapsed(&Address::of(end.as_ref().expect("paste end")))
                    });
                    after = Some(a.clone());
                    Ok::<_, (usize, hm_edit::EditError)>((tail, Some(a.to_string())))
                },
            )
        }) {
            d.editor.doc.info = tables;
            d.editor.doc.bins = bins;
            return Err(Inner::edit_error(i, e));
        }
        // Only a copy that was pasted counts toward the next one's step.
        if let Some((count, n)) = copies {
            count.set(n);
        }
        d.content_epoch += 1;
        d.relayout_edited();
        let scopes = d.editor.changed().map(<[_]>::to_vec);
        Ok(json(&d.receipt_after(
            end.as_ref().map(Address::of),
            &before,
            scopes.as_deref(),
            after,
        )))
    }
}
