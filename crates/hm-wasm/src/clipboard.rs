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
        let frag = clip::copy(&d.editor.doc, d.instance, &from, &to)
            .map_err(|e| Inner::edit_error(0, e))?;
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
    /// The table holding the cell `at` lies in, with the cell's row and
    /// column.
    fn host_cell(doc: &hm_model::Document, at: &Pos) -> Option<(types::TableAddress, u16, u16)> {
        let (step, outer) = at.list.steps.split_last()?;
        let parent = hm_edit::ListPath {
            section: at.list.section,
            steps: outer.to_vec(),
        };
        let hm_model::ControlKind::Table(t) = &hm_edit::ops::list(doc, &parent)?
            .get(step.para as usize)?
            .controls
            .get(step.control as usize)?
            .kind
        else {
            return None;
        };
        let cell = t.cells.get(step.index as usize)?;
        let host = types::TableAddress {
            section: at.list.section,
            steps: outer
                .iter()
                .map(|s| types::StepJs {
                    para: s.para,
                    control: s.control,
                    index: s.index,
                })
                .collect(),
            para: step.para,
            control: step.control,
        };
        Some((host, cell.row, cell.col))
    }

    /// The operation pasting the copied `table` over the cells of `host` from
    /// `row`, `col`, its resources imported when it comes from another
    /// document.
    fn paste_cells(
        doc: &mut hm_model::Document,
        instance: u32,
        frag: &clip::Fragment,
        table: &hm_model::Control,
        host: &types::TableAddress,
        row: u16,
        col: u16,
    ) -> Result<Op> {
        let mut copied = table.clone();
        if frag.instance != instance {
            let mut import = clip_import::Import::paste(&frag.source, doc);
            import.control(&mut copied)?;
            import.finish();
        }
        let hm_model::ControlKind::Table(copied) = &copied.kind else {
            unreachable!("a lone table")
        };
        let mut para = object_api::replacement(doc, host)?;
        let hm_model::ControlKind::Table(target) = &mut para.controls[host.control as usize].kind
        else {
            return Err(EngineError::new("BAD_ADDRESS", "no such table"));
        };
        studio_bridge::overwrite_cells(target, copied, row, col)?;
        Ok(object_api::replace(host, para))
    }
}

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
        let frag = fragment.or_else(|| d.clipboard.borrow().clone());
        let text = req
            .text
            .map(|t| t.replace("\r\n", "\n").replace('\r', "\n"));
        let frag = frag.filter(|f| text.as_ref().is_none_or(|t| *t == f.text()));
        // A table pasted with the caret in a table cell fills the cells from
        // there, as Hancom's 셀 붙이기 does by default; `cells: "nest"` puts
        // it into the cell instead.
        let over = match (&frag, &cut, req.cells) {
            (Some(f), None, None | Some(types::CellPaste::Overwrite)) => clip::lone_table(f)
                .cloned()
                .zip(Inner::host_cell(&d.editor.doc, &at)),
            _ => None,
        };
        // Pasting may add shapes and faces to the document tables; a
        // failure anywhere puts them back.
        let tables = d.editor.doc.info.clone();
        let bins = d.editor.doc.bins.clone();
        let made = match (frag, text) {
            (Some(f), _) if over.is_some() => {
                let Some((table, (host, row, col))) = over else {
                    unreachable!("checked above")
                };
                Inner::paste_cells(&mut d.editor.doc, d.instance, &f, &table, &host, row, col)
                    .map(|op| (vec![op], at.clone()))
            }
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
