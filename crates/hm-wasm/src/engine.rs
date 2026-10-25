//! Font registration, document creation and opening.
use super::*;

#[wasm_bindgen]
impl Engine {
    #[wasm_bindgen(constructor)]
    #[allow(clippy::new_without_default)]
    pub fn new() -> Engine {
        Engine {
            fonts: Rc::new(RefCell::new(FontSystem::new())),
            generation: Rc::new(Cell::new(0)),
            metrics: Rc::new(Cell::new(0)),
            // Digits, Latin letters and punctuation turn up everywhere.
            chars: Rc::new(RefCell::new({
                let mut s = text::CharSet::default();
                s.extend(' '..='~');
                s
            })),
            clipboard: Rc::new(RefCell::new(None)),
        }
    }

    fn register<S: hm_fonts::FontSource + ?Sized>(&self, label: &str, src: &S) -> usize {
        let chars: Vec<char> = self.chars.borrow().iter().collect();
        let added = self.fonts.borrow_mut().add_compact(label, src, &chars);
        self.generation.set(self.generation.get() + 1);
        if !added.refilled || added.relayout {
            self.metrics.set(self.metrics.get() + 1);
        }
        added.faces
    }

    fn bump(&self) {
        self.generation.set(self.generation.get() + 1);
        self.metrics.set(self.metrics.get() + 1);
    }

    /// Characters whose outlines compact font registration retains, including
    /// the documents opened so far and the engine's basic Latin seed.
    #[wasm_bindgen(js_name = fontCharacters)]
    pub fn font_characters(&self) -> String {
        self.chars.borrow().iter().collect()
    }

    /// Retained and pending glyph IDs for canonical source labels, as
    /// `[label, faceIndex, glyphIds][]`. Sparse refills must keep their union.
    #[wasm_bindgen(js_name = glyphCoverage)]
    pub fn glyph_coverage(&self, labels: &str) -> Result<String> {
        let labels: Vec<String> = parse(labels)?;
        Ok(json(&self.fonts.borrow().glyph_coverage(&labels)))
    }

    /// Registers a TTF, OTF, TTC or HFT file; returns the faces added.
    /// Documents lay out again with it on their next request.
    ///
    /// The engine keeps of a TrueType face its metrics and the outlines of
    /// the characters in the documents opened so far, not the file. Other
    /// outlines come empty (`em` 0 in `glyphOutlines`) and name the file in
    /// [`Engine::outline_requests`]; registering the same label again fills
    /// them in and bumps the font generation, without a new layout unless
    /// one measured a missing outline.
    #[wasm_bindgen(js_name = addFont)]
    pub fn add_font(&self, label: &str, bytes: &[u8]) -> usize {
        self.register(label, bytes)
    }

    /// Registers a font file the host holds, like [`Engine::add_font`],
    /// reading through `file.read(offset, length)` (a `Uint8Array`) only
    /// the parts the engine keeps, so the file never enters the engine's
    /// memory whole.
    #[wasm_bindgen(js_name = addFontFile)]
    pub fn add_font_file(&self, label: &str, file: &FontFile) -> usize {
        self.register(label, &HostFile(file))
    }

    /// Unregisters the font files registered under the labels of a JSON
    /// `string[]`; returns the faces removed. Documents lay out again on
    /// their next request.
    #[wasm_bindgen(js_name = unregisterFonts)]
    pub fn unregister_fonts(&self, labels: &str) -> Result<usize> {
        let labels: Vec<String> = parse(labels)?;
        let mut fonts = self.fonts.borrow_mut();
        let n = labels.iter().map(|l| fonts.remove(l)).sum();
        drop(fonts);
        self.bump();
        Ok(n)
    }

    /// Labels of registered font files with outlines asked for but not
    /// kept, as a JSON `string[]`: register each again with its bytes.
    #[wasm_bindgen(js_name = outlineRequests)]
    pub fn outline_requests(&self) -> String {
        json(&self.fonts.borrow().outline_requests())
    }

    /// Bytes of font data the engine holds.
    #[wasm_bindgen(getter, js_name = fontBytes)]
    pub fn font_bytes(&self) -> usize {
        self.fonts.borrow().resident_bytes()
    }

    /// Registers a Hancom code conversion table (`u300_*.dat`).
    #[wasm_bindgen(js_name = addCodeTable)]
    pub fn add_code_table(&self, bytes: &[u8]) -> bool {
        let ok = self.fonts.borrow_mut().add_code_table(bytes);
        self.bump();
        ok
    }

    /// Registers Hancom's font mapping table (`FontMap.dat`).
    #[wasm_bindgen(js_name = addFontMap)]
    pub fn add_font_map(&self, bytes: &[u8]) -> bool {
        let ok = self.fonts.borrow_mut().add_font_map(bytes);
        self.bump();
        ok
    }

    /// Registers Hancom's HFT catalog (`hftinfo.dat`), before HFT files.
    #[wasm_bindgen(js_name = addHftInfo)]
    pub fn add_hft_info(&self, bytes: &[u8]) -> bool {
        let ok = self.fonts.borrow_mut().add_hft_info(bytes);
        self.bump();
        ok
    }

    #[wasm_bindgen(getter, js_name = fontGeneration)]
    pub fn font_generation(&self) -> u32 {
        self.generation.get()
    }

    /// Outlines for `(face, glyph)` pairs as a page's `GLYF` block lists
    /// them (see [`packet::glyph_outlines`]).
    #[wasm_bindgen(js_name = glyphOutlines)]
    pub fn glyph_outlines(&self, pairs: &[u32]) -> Vec<u8> {
        packet::glyph_outlines(&self.fonts.borrow(), pairs)
    }

    /// A new document from a template: the template's pages, styles,
    /// headers and footers, with each section's text emptied to one
    /// paragraph and no history.
    pub fn create(&self, template: Vec<u8>) -> Result<Doc> {
        let mut doc = self.open(template)?;
        let d = doc.raw()?;
        let sections = d.editor.doc.sections.len();
        for si in 0..sections {
            let list = hm_edit::ListPath {
                section: si as u32,
                steps: Vec::new(),
            };
            let paras = &d.editor.doc.sections[si].paras;
            let n = paras.len() as u32;
            let Some(first) = paras.first() else { continue };
            let lead = hm_edit::ops::lead(first);
            let end = first.len().saturating_sub(1);
            let at = |offset| Pos {
                list: list.clone(),
                para: 0,
                offset,
            };
            let mut ops = Vec::new();
            if n > 1 {
                ops.push(Op::Replace {
                    list: list.clone(),
                    start: 1,
                    count: n - 1,
                    paras: Vec::new(),
                });
            }
            if end > lead {
                ops.push(Op::Delete {
                    from: at(lead),
                    to: at(end),
                });
            }
            for op in ops {
                hm_edit::apply(&mut d.editor.doc, &op)
                    .map_err(|e| EngineError::new("MALFORMED_INPUT", format!("template: {e}")))?;
            }
        }
        Ok(doc)
    }

    /// Opens HWP 5, HWPX, HML or HWP 3 bytes; layout waits for a request.
    pub fn open(&self, bytes: Vec<u8>) -> Result<Doc> {
        let bytes = SharedBytes::from(bytes);
        let doc = if hm_hwpx::is_hwpx(&bytes) {
            hm_hwpx::read(bytes).map_err(|e| EngineError::new("MALFORMED_INPUT", e))?
        } else if hm_hwp5::is_hwp5(&bytes) {
            hm_hwp5::read(bytes).map_err(|e| EngineError::new("MALFORMED_INPUT", e))?
        } else if hm_hml::is_hml(&bytes) {
            hm_hml::read(bytes).map_err(|e| EngineError::new("MALFORMED_INPUT", e))?
        } else if hm_hwp3::is_hwp3(&bytes) {
            hm_hwp3::read(bytes).map_err(|e| EngineError::new("MALFORMED_INPUT", e))?
        } else {
            return Err(EngineError::new(
                "UNSUPPORTED_FORMAT",
                "neither an HWP 5 compound file nor an HWPX package",
            ));
        };
        let used = Rc::new(text::font_use(&doc));
        self.chars.borrow_mut().extend(used.chars.iter());
        // The layout waits for the first request that needs it, so fonts
        // the document names (`Doc.fontNames`) can be registered first.
        Ok(Doc {
            inner: Some(Inner {
                editor: Editor::new(doc),
                layout: DocLayout::default(),
                laid: false,
                fonts: self.fonts.clone(),
                generation: self.generation.clone(),
                metrics: self.metrics.clone(),
                clipboard: self.clipboard.clone(),
                composition: None,
                checkpoints: Default::default(),
                next_checkpoint: 1,
                transforms: Default::default(),
                paragraph_captures: Default::default(),
                transpose_clipboard: None,
                cell_clipboard: None,
                laid_with: self.metrics.get(),
                drawn_with: self.generation.get(),
                instance: INSTANCES.fetch_add(1, Ordering::Relaxed),
                content_epoch: 0,
                layout_epoch: 0,
                saved_epoch: 0,
                saved_history: 0,
                pages: Vec::new(),
                indexes: Default::default(),
                paths: None,
                font_use: (0, used),
                picture_sizes: Default::default(),
                last_picture: None,
            }),
        })
    }
}

#[wasm_bindgen]
impl Doc {
    /// Actual glyphs drawn on every current page, grouped by canonical font
    /// source label and source face index. Touches residency so an unavailable
    /// outline also appears in Engine.outlineRequests before visible painting.
    #[wasm_bindgen(js_name = glyphRequests)]
    pub fn glyph_requests(&mut self) -> Result<String> {
        let d = self.inner()?;
        let mut pairs = std::collections::BTreeSet::new();
        for page in 0..d.layout.pages.len() {
            let built = d.page(page)?;
            for item in &built.0.items {
                let run = match item {
                    hm_render::Item::Glyphs(run) | hm_render::Item::Outline { run, .. } => run,
                    _ => continue,
                };
                for &(glyph, _) in &run.glyphs {
                    pairs.insert((run.face.0, glyph));
                }
            }
        }
        let fonts = d.fonts.borrow();
        let mut requests: std::collections::BTreeMap<(String, u32), Vec<u16>> = Default::default();
        for (face, glyph) in pairs {
            let id = hm_fonts::FaceId(face);
            let entry = fonts.entry(id);
            requests
                .entry((entry.label.clone(), entry.index))
                .or_default()
                .push(glyph);
            if let Some(face) = fonts.face(id) {
                // Existing outlines need no path decoding during preflight.
                if !face.resident(glyph) {
                    face.outline(glyph);
                }
            }
        }
        let requests: Vec<_> = requests
            .into_iter()
            .map(|((label, index), glyphs)| {
                serde_json::json!({"label":label,"index":index,"glyphs":glyphs})
            })
            .collect();
        Ok(json(&requests))
    }

    /// What fonts the document draws with (`FontRequests`): per face name
    /// its text, paragraph heads and equations use, its candidate families
    /// in the order the engine tries them and its font-map stand-ins; the
    /// class default lists; and `fallbacks`, the unregistered families the
    /// engine looks in, in order, for characters the registered faces lack.
    ///
    /// Register the first available candidate of each face and the first
    /// available family of the first default list (Hancom's default face;
    /// failing that, of the others), then ask again and register the
    /// available fallbacks before the first layout. Doesn't lay the
    /// document out. Once laid out, the faces also include every name the
    /// layout asked for and the fallbacks what it looked for; registering
    /// what is new and available lays it out again.
    #[wasm_bindgen(js_name = fontRequests)]
    pub fn font_requests(&mut self) -> Result<String> {
        let d = self.raw()?;
        if d.font_use.0 != d.content_epoch {
            d.font_use = (d.content_epoch, Rc::new(text::font_use(&d.editor.doc)));
        }
        let used = d.font_use.1.clone();
        let fonts = d.fonts.borrow();
        let info = &d.editor.doc.info;
        let mut text_names = std::collections::BTreeSet::new();
        for (shape, chars) in &used.shaped {
            let Some(cs) = info.char_shape(*shape) else {
                continue;
            };
            for ch in chars.iter() {
                if ch.is_whitespace() || ch.is_control() {
                    continue;
                }
                let Some(script) = hm_layout::script::script_of(ch) else {
                    continue;
                };
                if let Some(face) = info.faces[script.index()].get(cs.face(script) as usize) {
                    text_names.insert(face.name.to_string_lossy());
                }
            }
        }
        let mut faces: Vec<FaceRequest> = Vec::new();
        let mut named: Vec<&hm_model::FaceName> = used
            .faces
            .iter()
            .filter_map(|&(script, id)| info.faces[script].get(id as usize))
            .collect();
        // The layout knows exactly which names its text asked for.
        let current_layout = d.laid && d.laid_with == d.metrics.get();
        if current_layout {
            for n in &d.layout.requested_faces {
                if let Some(f) = info
                    .faces
                    .iter()
                    .flatten()
                    .find(|f| f.name.to_string_lossy() == *n)
                {
                    named.push(f);
                }
            }
        }
        for f in named {
            let name = f.name.to_string_lossy();
            if faces.iter().any(|r| r.name == name) {
                continue;
            }
            let sub = f.substitute.as_ref().map(|(_, n)| n.to_string_lossy());
            let def = f.default_name.as_ref().map(|n| n.to_string_lossy());
            let alternates: Vec<&str> = sub.iter().chain(def.iter()).map(String::as_str).collect();
            let (candidates, stand_ins) = fonts.face_candidates(&name, &alternates);
            faces.push(FaceRequest {
                text: text_names.contains(&name),
                name,
                candidates,
                stand_ins,
            });
        }
        for name in used.equations.iter().cloned() {
            let mut candidates: Vec<String> = name.into_iter().collect();
            for n in hm_layout::math::EQUATION_FAMILIES {
                if !candidates.iter().any(|c| c.eq_ignore_ascii_case(n)) {
                    candidates.push(n.to_string());
                }
            }
            let name = candidates[0].clone();
            if faces.iter().any(|r| r.name == name) {
                continue;
            }
            faces.push(FaceRequest {
                name,
                candidates,
                stand_ins: Vec::new(),
                text: true,
            });
        }
        // Characters the registered faces lack, found without a layout.
        let mut styles = hm_layout::style::Styles::new(info, &fonts);
        if !fonts.entries().is_empty() {
            for (cs, set) in &used.shaped {
                let mut primary: [Option<Option<Arc<hm_fonts::Face>>>; 7] = Default::default();
                for c in set.iter() {
                    let Some(script) = hm_layout::script::script_of(c) else {
                        continue;
                    };
                    let face = primary[script.index()].get_or_insert_with(|| {
                        let si = styles.style_for(*cs, script, None);
                        styles.face(si).cloned()
                    });
                    if face.as_ref().is_some_and(|f| !f.has(c)) {
                        styles.style_for(*cs, script, Some(c));
                    }
                }
            }
        }
        let mut fallbacks: Vec<String> = Vec::new();
        let laid = current_layout.then_some(&d.layout.fallback_families);
        // Equations draw what their face lacks with every text family.
        let math = (!used.equations.is_empty())
            .then_some(hm_layout::math::TEXT_FAMILIES)
            .into_iter()
            .flatten()
            .map(|s| s.to_string());
        for n in styles
            .fallback_families()
            .into_iter()
            .chain(laid.into_iter().flatten().cloned())
            .chain(math)
        {
            if fonts.family(&n).is_empty() && !fallbacks.contains(&n) {
                fallbacks.push(n);
            }
        }
        Ok(json(&FontRequests {
            faces,
            class_defaults: hm_fonts::class_defaults()
                .iter()
                .map(|l| l.iter().map(|s| s.to_string()).collect())
                .collect(),
            fallbacks,
        }))
    }

    /// Every family [`Doc::font_requests`] names, as a JSON `string[]`.
    #[wasm_bindgen(js_name = fontNames)]
    pub fn font_names(&mut self) -> Result<String> {
        let r: FontRequests = serde_json::from_str(&self.font_requests()?).unwrap_or_default();
        let mut out: Vec<String> = Vec::new();
        let all = r
            .faces
            .into_iter()
            .flat_map(|f| f.candidates.into_iter().chain(f.stand_ins))
            .chain(r.class_defaults.into_iter().flatten())
            .chain(r.fallbacks);
        for n in all {
            if !out.contains(&n) {
                out.push(n);
            }
        }
        Ok(json(&out))
    }
}
