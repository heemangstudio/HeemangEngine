//! Bounded model queries and text search.
use super::*;

#[wasm_bindgen]
impl Doc {
    /// Bounded reads of the document (`QueryRequest`), as JSON:
    /// `paragraphs` gives `ParagraphInfo[]` for a list, `formatContext` a
    /// `FormatContext`, `styles` `StyleInfo[]` and `style` one (or null),
    /// `lists` `ListInfo[]` and `list` one (or null), `count` a
    /// `CharacterCount` and `formatRuns` `ParagraphFormat[]`.
    pub fn query(&mut self, request: &str) -> Result<String> {
        let req: QueryRequest = parse(request)?;
        let (list, from, count) = match req {
            QueryRequest::Fonts => {
                let faces: Vec<Vec<_>> = self.raw()?.editor.doc.info.faces.iter().map(|slot| slot.iter().enumerate().map(|(id, f)| serde_json::json!({"id": id, "name": f.name.to_string_lossy()})).collect()).collect();
                return Ok(json(&faces));
            }
            QueryRequest::FontCandidates { names } => {
                let fonts = self.raw()?.fonts.borrow();
                let out: Vec<types::FaceCandidates> = names
                    .into_iter()
                    .map(|name| {
                        let (candidates, stand_ins) = fonts.face_candidates(&name, &[]);
                        let resolves = fonts.resolves(&name);
                        types::FaceCandidates {
                            name,
                            candidates,
                            stand_ins,
                            resolves,
                        }
                    })
                    .collect();
                return Ok(json(&out));
            }
            QueryRequest::CharShape { id } => {
                let doc = &self.raw()?.editor.doc;
                return Ok(json(
                    &doc.info.char_shape(id).map(|_| format::characters(doc, id)),
                ));
            }
            QueryRequest::ParaShape { id } => {
                let doc = &self.raw()?.editor.doc;
                return Ok(json(
                    &doc.info
                        .para_shape(id)
                        .map(|_| format::paragraphs(doc, id as u16, 1000)),
                ));
            }
            req @ (QueryRequest::SectionSettings { .. }
            | QueryRequest::NoteShape { .. }
            | QueryRequest::DocumentProperties
            | QueryRequest::Controls { .. }
            | QueryRequest::Fields { .. }) => {
                return Ok(json(&dialogs::query(&self.raw()?.editor.doc, req)?));
            }
            QueryRequest::ObjectProperties { object } => {
                return Ok(json(&object_api::object_properties(
                    &self.raw()?.editor.doc,
                    &object,
                )?));
            }
            QueryRequest::TableProperties { table } => {
                return Ok(json(&object_api::table_properties(
                    &self.raw()?.editor.doc,
                    &table,
                )?));
            }
            QueryRequest::CellProperties { table, row, column } => {
                return Ok(json(&object_api::cell_properties(
                    &self.raw()?.editor.doc,
                    &table,
                    row,
                    column,
                )?));
            }
            QueryRequest::PictureData { object } => {
                return Ok(json(&object_api::picture_data(
                    &self.raw()?.editor.doc,
                    &object,
                )?));
            }
            QueryRequest::Equation { object } => {
                return Ok(json(&agent_api::equation(
                    &self.raw()?.editor.doc,
                    &object,
                )?));
            }
            QueryRequest::EquationPreview {
                script,
                size,
                color,
                font,
            } => {
                let fonts = self.raw()?.fonts.clone();
                let fonts = fonts.borrow();
                return Ok(json(&agent_api::preview(
                    &fonts,
                    &script,
                    size,
                    color,
                    font.as_deref(),
                )?));
            }
            QueryRequest::Table { table, page } => {
                return self.inner()?.table_info(table, page);
            }
            QueryRequest::Paragraphs { list, from, count } => (list, from, count),
            QueryRequest::FormatContext { at, to } => {
                let d = self.raw()?;
                let at = at.pos();
                let to = to.map_or_else(|| at.clone(), |t| t.pos());
                let spans = d.spans(&at, &to)?;
                let doc = &d.editor.doc;
                let mut lists = Vec::new();
                for (a, b) in &spans {
                    let paras = hm_edit::ops::list(doc, &a.list)
                        .ok_or_else(|| EngineError::new("BAD_ADDRESS", "no such paragraph list"))?;
                    lists.push((paras.as_slice(), a.para, a.offset, b.para, b.offset));
                }
                return Ok(json(&format::context(doc, &lists)));
            }
            QueryRequest::Styles { from, count } => {
                let doc = &self.raw()?.editor.doc;
                let n = doc.info.styles.len() as u32;
                let end = count.map_or(n, |c| from.saturating_add(c).min(n));
                let out: Vec<_> = (from..end).filter_map(|i| format::style(doc, i)).collect();
                return Ok(json(&out));
            }
            QueryRequest::Style { id } => {
                return Ok(json(&format::style(&self.raw()?.editor.doc, id)));
            }
            QueryRequest::Lists => return Ok(json(&format::lists(&self.raw()?.editor.doc))),
            QueryRequest::Lines { list, para } => {
                let d = self.inner()?;
                let pos = Address {
                    section: list.section,
                    steps: list.steps,
                    para,
                    offset: 0,
                }
                .pos();
                d.check(&pos)?;
                return Ok(json(&d.lines(&pos)?));
            }
            QueryRequest::List { list_kind, id } => {
                return Ok(json(&format::list(&self.raw()?.editor.doc, &list_kind, id)));
            }
            QueryRequest::Count { list, from, to } => {
                let d = self.raw()?;
                let characters = match (from, to) {
                    (Some(from), Some(to)) => {
                        let mut n = 0;
                        for (a, b) in d.spans(&from.pos(), &to.pos())? {
                            let paras =
                                hm_edit::ops::list(&d.editor.doc, &a.list).ok_or_else(|| {
                                    EngineError::new("BAD_ADDRESS", "no such paragraph list")
                                })?;
                            n += count::range(paras, a.para, a.offset, b.para, b.offset);
                        }
                        n
                    }
                    (None, None) => match list {
                        Some(list) => {
                            let path = Address {
                                section: list.section,
                                steps: list.steps,
                                para: 0,
                                offset: 0,
                            }
                            .pos()
                            .list;
                            let paras =
                                hm_edit::ops::list(&d.editor.doc, &path).ok_or_else(|| {
                                    EngineError::new("BAD_ADDRESS", "no such paragraph list")
                                })?;
                            count::paragraphs(paras)
                        }
                        None => d
                            .editor
                            .doc
                            .sections
                            .iter()
                            .map(|s| count::paragraphs(&s.paras))
                            .sum(),
                    },
                    _ => {
                        return Err(EngineError::new(
                            "BAD_REQUEST",
                            "a count range needs both from and to",
                        ));
                    }
                };
                return Ok(json(&serde_json::json!({ "characters": characters })));
            }
            QueryRequest::FormatRuns { list, from, count } => {
                let d = self.raw()?;
                let path = Address {
                    section: list.section,
                    steps: list.steps,
                    para: 0,
                    offset: 0,
                }
                .pos()
                .list;
                let doc = &d.editor.doc;
                let paras = hm_edit::ops::list(doc, &path)
                    .ok_or_else(|| EngineError::new("BAD_ADDRESS", "no such paragraph list"))?;
                let end = count.map_or(paras.len(), |c| {
                    (from as usize + c as usize).min(paras.len())
                });
                let out: Vec<_> = paras
                    .iter()
                    .enumerate()
                    .take(end)
                    .skip(from as usize)
                    .map(|(i, p)| format::paragraph_format(doc, i as u32, p))
                    .collect();
                return Ok(json(&out));
            }
        };
        let d = self.inner()?;
        let path = Address {
            section: list.section,
            steps: list.steps,
            para: 0,
            offset: 0,
        }
        .pos()
        .list;
        let paras = hm_edit::ops::list(&d.editor.doc, &path)
            .ok_or_else(|| EngineError::new("BAD_ADDRESS", "no such paragraph list"))?;
        let end = count.map_or(paras.len(), |c| {
            (from as usize + c as usize).min(paras.len())
        });
        let out: Vec<ParagraphInfo> = paras
            .iter()
            .enumerate()
            .take(end)
            .skip(from as usize)
            .map(|(i, p)| ParagraphInfo {
                index: i as u32,
                text: p.plain_text(),
                offsets: text_offsets(p),
                length: p.len(),
                caret: [hm_edit::ops::lead(p), p.len().saturating_sub(1)],
                controls: p
                    .units()
                    .filter_map(|(offset, u)| match u {
                        Unit::Extended { index, .. } => {
                            let c = p.controls.get(index as usize)?;
                            Some(ControlInfo {
                                index,
                                offset,
                                kind: control_kind(c),
                                lists: hm_edit::ops::control_lists(c).len(),
                                script: c
                                    .as_object()
                                    .and_then(|o| o.equation())
                                    .map(|e| e.script.to_string_lossy()),
                                inline: c.object_common().map(|oc| oc.treat_as_char()),
                                side: match &c.kind {
                                    hm_model::ControlKind::HeaderFooter(h) => {
                                        Some(match h.side() {
                                            hm_model::PageSide::Both => "both",
                                            hm_model::PageSide::Even => "even",
                                            hm_model::PageSide::Odd => "odd",
                                        })
                                    }
                                    _ => None,
                                },
                                number: match &c.kind {
                                    hm_model::ControlKind::Note(n) => Some(n.number),
                                    _ => None,
                                },
                            })
                        }
                        _ => None,
                    })
                    .collect(),
            })
            .collect();
        Ok(json(&out))
    }

    /// Text matches within paragraphs (`SearchRequest` to `SearchResult`),
    /// in document order: each list, then the lists its paragraphs' controls
    /// hold. Doesn't lay the document out.
    pub fn search(&mut self, request: &str) -> Result<String> {
        let req: SearchRequest = parse(request)?;
        let d = self.raw()?;
        let doc = &d.editor.doc;
        let scope = req.list.map(|l| {
            Address {
                section: l.section,
                steps: l.steps,
                para: 0,
                offset: 0,
            }
            .pos()
            .list
        });
        let limit = if req.from.is_some() {
            usize::MAX
        } else {
            req.limit.unwrap_or(1000)
        };
        let mut found = text::search(doc, &req.text, req.case_sensitive, scope.as_ref(), limit);
        if let Some(from) = req.from {
            let order = text::lists(doc);
            let key = |p: &Pos| {
                let l = order
                    .iter()
                    .position(|l| *l == p.list)
                    .unwrap_or(usize::MAX);
                (l, p.para, p.offset)
            };
            let here = key(&from.pos());
            let pick = if req.backward {
                found
                    .iter()
                    .rposition(|(a, _)| key(a) < here)
                    .or_else(|| found.len().checked_sub(1))
            } else {
                found
                    .iter()
                    .position(|(a, _)| key(a) > here)
                    .or_else(|| (!found.is_empty()).then_some(0))
            };
            found = pick.map(|k| found.swap_remove(k)).into_iter().collect();
        }
        Ok(json(&SearchResult {
            content_epoch: d.content_epoch,
            matches: found
                .iter()
                .map(|(a, b)| Match {
                    from: Address::of(a),
                    to: Address::of(b),
                })
                .collect(),
        }))
    }
}
