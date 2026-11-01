//! Layout caches, page packets, picture patches and rendered exports.
use super::*;

/// The live layout is Hancom's editing screen: it shows the guide text of
/// unfilled click-here fields, which print and PDF leave out.
pub(crate) const SCREEN: hm_layout::Options = hm_layout::Options {
    stored_lines: true,
    guides: true,
};

/// A click-here field Hancom has not marked changed, anywhere in `paras`:
/// its guide text may show on screen and not in print.
fn unfilled_click_here(paras: &[hm_model::Paragraph]) -> bool {
    paras.iter().flat_map(|p| &p.controls).any(|c| {
        c.as_field()
            .is_some_and(|f| f.kind() == hm_model::FieldKind::ClickHere && !f.dirty())
            || hm_edit::ops::control_lists(c)
                .into_iter()
                .any(|l| unfilled_click_here(l))
    })
}

impl Inner {
    /// A failed atomic edit restores content, but may replace model addresses.
    /// Keep the layout and epochs; rebuild address-dependent page metadata lazily.
    pub(super) fn edit<T, E>(
        &mut self,
        change: impl FnOnce(&mut Editor) -> std::result::Result<T, E>,
    ) -> std::result::Result<T, E> {
        let result = change(&mut self.editor);
        if result.is_err() {
            self.clear_indexes();
        }
        result
    }

    /// Lays out after a committed edit. Footnotes numbered per page then
    /// store the page-local numbers this layout shows, as part of the same
    /// undo step, and the document is laid out once more.
    pub(super) fn relayout_edited(&mut self) {
        // An edit that removed, moved or pasted notes without saying so
        // (deleting table rows, a caption, a range) renumbers them too.
        if self.editor.notes_moved() {
            let ops = crate::notes::renumber(&self.editor.doc);
            if !ops.is_empty() {
                let _ = self.editor.amend(ops);
            }
        }
        // So do caption numbers when numbered pictures, tables or
        // equations come, go or move.
        if self.editor.numbers_moved() {
            let ops = crate::agent_api::renumber_captions(&self.editor.doc);
            if !ops.is_empty() {
                let _ = self.editor.amend(ops);
            }
        }
        self.relayout();
        let ops = crate::notes::page_local(&self.editor.doc, &self.layout);
        if !ops.is_empty() && matches!(self.editor.amend(ops), Ok(true)) {
            self.relayout();
        }
    }

    /// The layout print and PDF use: the screen's, unless an unfilled
    /// click-here guide shows on screen. Hancom's print leaves the guide
    /// out and closes up its line.
    pub(super) fn print_layout(&mut self) -> Option<Rc<DocLayout>> {
        if !self
            .editor
            .doc
            .sections
            .iter()
            .any(|s| unfilled_click_here(&s.paras))
        {
            return None;
        }
        if let Some((epoch, l)) = &self.print
            && *epoch == self.layout_epoch
        {
            return Some(l.clone());
        }
        let fonts = self.fonts.borrow();
        let l = Rc::new(hm_layout::layout_with(
            &self.editor.doc,
            &fonts,
            hm_layout::Options {
                guides: false,
                ..SCREEN
            },
        ));
        drop(fonts);
        self.print = Some((self.layout_epoch, l.clone()));
        Some(l)
    }

    fn clear_indexes(&mut self) {
        self.pages.clear();
        self.indexes.clear();
        self.paths = None;
    }

    /// A picture decoded at its own size, the last one kept.
    pub(super) fn decoded(
        &mut self,
        pic: &hm_render::draw::ImageItem,
    ) -> Option<Rc<hm_render::image::Raster>> {
        let key = pic.data.identity();
        if let Some((k, r)) = &self.last_picture
            && *k == key
        {
            return Some(r.clone());
        }
        let r = Rc::new(hm_render::image::decode(&pic.data)?);
        self.last_picture = Some((key, r.clone()));
        Some(r)
    }

    pub(super) fn relayout(&mut self) {
        self.laid = true;
        self.laid_with = self.metrics.get();
        let fonts = self.fonts.borrow();
        fonts.set_layout_running(true);
        let last = std::mem::take(&mut self.layout);
        self.layout = self.editor.relayout_with(&fonts, SCREEN, last);
        fonts.set_layout_running(false);
        drop(fonts);
        self.sigs = Default::default();
        self.layout_epoch += 1;
        self.clear_indexes();
    }

    pub(super) fn page(&mut self, i: usize) -> Result<Built> {
        if i >= self.layout.pages.len() {
            return Err(EngineError::new("NO_SUCH_PAGE", format!("no page {i}")));
        }
        if let Some(k) = self.pages.iter().position(|(p, _)| *p == i) {
            let entry = self.pages.remove(k);
            let built = entry.1.clone();
            self.pages.push(entry);
            return Ok(built);
        }
        let (list, boxes, cells) = {
            let fonts = self.fonts.borrow();
            hm_render::build_page_indexed(&self.editor.doc, &self.layout, &fonts, i)
        };
        // Painting finds the lines a line index found without painting.
        let index = match self.indexes.get(&i) {
            Some(index) => index.clone(),
            None => {
                let index = Rc::new(self.line_index(i, boxes, cells));
                self.indexes.insert(i, index.clone());
                index
            }
        };
        let built = Rc::new((list, index));
        self.pages.push((i, built.clone()));
        if self.pages.len() > KEPT_PAGES {
            self.pages.remove(0);
        }
        Ok(built)
    }

    /// The line index of page `i`, found without painting the page.
    pub(super) fn index(&mut self, i: usize) -> Result<Rc<PageIndex>> {
        if let Some(index) = self.indexes.get(&i) {
            return Ok(index.clone());
        }
        if i >= self.layout.pages.len() {
            return Err(EngineError::new("NO_SUCH_PAGE", format!("no page {i}")));
        }
        let (boxes, cells) = {
            let fonts = self.fonts.borrow();
            hm_render::page_index(&self.editor.doc, &self.layout, &fonts, i)
        };
        let index = Rc::new(self.line_index(i, boxes, cells));
        self.indexes.insert(i, index.clone());
        Ok(index)
    }

    /// Page `i`'s line index from its line boxes. The paths of the page's
    /// own paragraphs place every box the document's paragraphs reach;
    /// otherwise those of the whole document, kept until the next layout.
    fn line_index(
        &mut self,
        i: usize,
        boxes: Vec<hm_render::LineBox>,
        cells: Vec<hm_render::CellBox>,
    ) -> PageIndex {
        if self.paths.is_none() {
            let (local, unreached) = Paths::of_page(&self.editor.doc, &self.layout.pages[i]);
            if boxes
                .iter()
                .all(|b| local.contains(b.para) || unreached.contains(&b.para))
            {
                return PageIndex::new(boxes, cells, &local);
            }
        }
        let paths = self
            .paths
            .get_or_insert_with(|| Paths::new(&self.editor.doc));
        PageIndex::new(boxes, cells, paths)
    }

    pub(super) fn page_info(&self, index: usize, p: &hm_layout::Page) -> PageInfo {
        let def = self
            .editor
            .doc
            .sections
            .get(p.section as usize)
            .and_then(|s| s.definition())
            .and_then(|s| s.page_def());
        let m = |v: u32| v.min(i32::MAX as u32) as i32;
        let margins = def.map_or_else(types::Margins::default, |g| types::Margins {
            left: m(g.margin_left),
            right: m(g.margin_right),
            top: m(g.margin_top),
            bottom: m(g.margin_bottom),
            header: m(g.margin_header),
            footer: m(g.margin_footer),
            gutter: m(g.gutter),
        });
        // Where the layout sets headers and footers.
        let header = types::Area {
            x: p.body.x,
            y: margins.top,
            width: p.body.w,
            height: margins.header,
        };
        let footer = types::Area {
            x: p.body.x,
            y: p.height - margins.bottom - margins.footer,
            width: p.body.w,
            height: margins.footer,
        };
        PageInfo {
            index,
            section: p.section,
            width: p.width,
            height: p.height,
            number: p.number,
            landscape: def.is_some_and(|g| g.landscape()),
            margins,
            body: p.body.into(),
            header,
            footer,
            columns: p.columns.iter().map(|&c| c.into()).collect(),
        }
    }
}

#[wasm_bindgen]
impl Doc {
    /// `PageInfo[]` as JSON, sizes in HU.
    pub fn pages(&mut self) -> Result<String> {
        let d = self.inner()?;
        let v: Vec<PageInfo> = d
            .layout
            .pages
            .iter()
            .enumerate()
            .map(|(index, p)| d.page_info(index, p))
            .collect();
        Ok(json(&v))
    }

    /// Metadata for just one page, without serializing the whole document.
    #[wasm_bindgen(js_name = pageInfo)]
    pub fn page_info(&mut self, index: usize) -> Result<String> {
        let d = self.inner()?;
        let page = d
            .layout
            .pages
            .get(index)
            .ok_or_else(|| EngineError::new("NO_SUCH_PAGE", format!("no page {index}")))?;
        Ok(json(&d.page_info(index, page)))
    }

    /// Page `i` as a packet (see [`packet`]).
    pub fn page(&mut self, i: usize) -> Result<Vec<u8>> {
        let d = self.inner()?;
        let built = d.page(i)?;
        let paint = packet::paint_block(&built.0, &d.fonts.borrow());
        let header = packet::Header {
            page: i as u32,
            content_epoch: d.content_epoch,
            layout_epoch: d.layout_epoch,
            font_generation: d.drawn_with,
        };
        Ok(packet::packet(
            &header,
            &built.0,
            &[
                (*b"PANT", paint),
                (*b"LINE", built.1.block()),
                (*b"GLYF", packet::glyph_block(&built.0)),
            ],
        ))
    }

    /// Pixels of patch op `item` of page `i` at `dpi`: `u32` width and
    /// height, then straight RGBA rows; empty when it draws nothing.
    pub fn patch(&mut self, i: usize, item: usize, dpi: f32) -> Result<Vec<u8>> {
        let d = self.inner()?;
        let built = d.page(i)?;
        let it = built
            .0
            .items
            .get(item)
            .ok_or_else(|| EngineError::new("BAD_REQUEST", "no such item"))?;
        let (x, y, w, h) = packet::patch_box(it)
            .ok_or_else(|| EngineError::new("BAD_REQUEST", "not a patch item"))?;
        let one = DisplayList {
            width: w,
            height: h,
            items: vec![packet::shifted(it, -x, -y)],
        };
        let fonts = d.fonts.borrow();
        let Some(pixmap) = hm_render::raster::render_clear(&one, &fonts, dpi) else {
            return Ok(Vec::new());
        };
        let mut out = Vec::with_capacity(8 + pixmap.data().len());
        out.extend_from_slice(&pixmap.width().to_le_bytes());
        out.extend_from_slice(&pixmap.height().to_le_bytes());
        for p in pixmap.pixels() {
            let c = p.demultiply();
            out.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
        }
        Ok(out)
    }

    /// Patch op `item` of page `i` as a picture the host decodes and
    /// scales itself (`PictureSource`), or `null` when the patch has to be
    /// rendered by [`Doc::patch`]. JPEG, PNG, GIF and BMP files come as
    /// stored; other raster formats as straight RGBA rows
    /// (`image/x-rgba`). Hosts decode such pictures off the critical path,
    /// so a first paint need not wait for the engine to decode and
    /// resample them.
    #[wasm_bindgen(js_name = pictureSource)]
    pub fn picture_source(&mut self, i: usize, item: usize) -> Result<String> {
        let d = self.inner()?;
        let built = d.page(i)?;
        let it = built
            .0
            .items
            .get(item)
            .ok_or_else(|| EngineError::new("BAD_REQUEST", "no such item"))?;
        let (pic, mime, size) = match hm_render::vector::browser_picture(it) {
            Some(found) => found,
            None => {
                let Some(pic) = hm_render::vector::plain_picture(it) else {
                    return Ok("null".into());
                };
                let key = pic.data.identity();
                let size = match d.picture_sizes.get(&key) {
                    Some(&size) => size,
                    None => {
                        let size = d.decoded(pic).map(|r| (r.w, r.h));
                        d.picture_sizes.insert(key, size);
                        size
                    }
                };
                let Some(size) = size else {
                    return Ok("null".into());
                };
                (pic, "image/x-rgba", size)
            }
        };
        Ok(json(&types::PictureSource {
            key: format!(
                "{}-{:x}-{:x}-{:x}",
                d.instance,
                pic.data.identity().0,
                pic.data.identity().1,
                pic.data.identity().2
            ),
            mime,
            width: size.0,
            height: size.1,
            matrix: hm_render::vector::picture_matrix(pic, size),
            clip: (!hm_render::vector::uncropped(pic, size))
                .then(|| hm_render::vector::frame_corners(pic)),
        }))
    }

    /// The bytes of the picture [`Doc::picture_source`] describes: the file
    /// as stored, or straight RGBA rows for `image/x-rgba`.
    #[wasm_bindgen(js_name = pictureBytes)]
    pub fn picture_bytes(&mut self, i: usize, item: usize) -> Result<Vec<u8>> {
        let d = self.inner()?;
        let built = d.page(i)?;
        let it = built
            .0
            .items
            .get(item)
            .ok_or_else(|| EngineError::new("BAD_REQUEST", "no such item"))?;
        if let Some((p, _, _)) = hm_render::vector::browser_picture(it) {
            return Ok(p.data.to_vec());
        }
        hm_render::vector::plain_picture(it)
            .and_then(|p| d.decoded(p))
            .map(|r| r.rgba.clone())
            .ok_or_else(|| EngineError::new("BAD_REQUEST", "not a picture the host decodes"))
    }

    /// Page `i` as an SVG document: text and rules as vectors, pictures
    /// and brushes as PNG patches at `dpi`.
    #[wasm_bindgen(js_name = pageSvg)]
    pub fn page_svg(&mut self, i: usize, dpi: f32) -> Result<String> {
        let d = self.inner()?;
        let built = d.page(i)?;
        Ok(hm_render::svg::page(&built.0, &d.fonts.borrow(), dpi))
    }

    /// [`Doc::page_svg`] of page `i` of the print layout
    /// ([`Doc::print_page_count`] pages), which leaves out the guide text of
    /// unfilled click-here fields as Hancom's print does.
    #[wasm_bindgen(js_name = printPageSvg)]
    pub fn print_page_svg(&mut self, i: usize, dpi: f32) -> Result<String> {
        let d = self.inner()?;
        let Some(l) = d.print_layout() else {
            let built = d.page(i)?;
            return Ok(hm_render::svg::page(&built.0, &d.fonts.borrow(), dpi));
        };
        if i >= l.pages.len() {
            return Err(EngineError::new("NO_SUCH_PAGE", format!("no page {i}")));
        }
        let fonts = d.fonts.borrow();
        let dl = hm_render::build_page(&d.editor.doc, &l, &fonts, i);
        Ok(hm_render::svg::page(&dl, &fonts, dpi))
    }

    /// The number of pages print and PDF give: the screen's, or fewer
    /// when the unfilled click-here guides the screen shows take lines.
    #[wasm_bindgen(js_name = printPageCount)]
    pub fn print_page_count(&mut self) -> Result<usize> {
        let d = self.inner()?;
        Ok(match d.print_layout() {
            Some(l) => l.pages.len(),
            None => d.layout.pages.len(),
        })
    }

    /// Pages `from..to` of the print layout as one PDF for printing or
    /// export: text and rules as vectors, pictures and brushes as images
    /// at `dpi`.
    pub fn pdf(&mut self, from: usize, to: usize, dpi: f32) -> Result<Vec<u8>> {
        let d = self.inner()?;
        let print = d.print_layout();
        let layout = print.as_deref().unwrap_or(&d.layout);
        let to = to.min(layout.pages.len());
        if from >= to {
            return Err(EngineError::new(
                "NO_SUCH_PAGE",
                format!("no pages {from}..{to}"),
            ));
        }
        let fonts = d.fonts.clone();
        let fonts = fonts.borrow();
        let mut w = hm_render::pdf::Writer::new(&fonts, dpi);
        for i in from..to {
            // Built one at a time so a long document never holds them all.
            let dl = hm_render::build_page(&d.editor.doc, layout, &fonts, i);
            w.page(&dl);
        }
        Ok(w.finish())
    }

    /// Page `i` rasterized by the engine as PNG, the reference the canvas
    /// player is checked against.
    #[wasm_bindgen(js_name = pagePng)]
    pub fn page_png(&mut self, i: usize, dpi: f32) -> Result<Vec<u8>> {
        let d = self.inner()?;
        let built = d.page(i)?;
        hm_render::raster::render_png(&built.0, &d.fonts.borrow(), dpi)
            .ok_or_else(|| EngineError::new("BAD_REQUEST", "page too large to rasterize"))
    }

    /// Paragraphs the last layout reused and formatted.
    #[wasm_bindgen(js_name = layoutStats)]
    pub fn layout_stats(&mut self) -> Result<Vec<f64>> {
        let d = self.inner()?;
        let (hits, misses) = d.editor.layout_stats();
        Ok(vec![hits as f64, misses as f64])
    }

    /// Face names that resolved to another family, as `requested\tused`
    /// lines: the fonts worth providing.
    pub fn substitutions(&mut self) -> Result<String> {
        let d = self.inner()?;
        Ok(d.layout
            .substitutions
            .iter()
            .map(|(a, b)| format!("{a}\t{b}\n"))
            .collect())
    }
}

#[wasm_bindgen]
impl Doc {
    /// A disposable document showing the selected header/footer on every page.
    /// Editing and the shared clipboard of the source document are untouched.
    #[wasm_bindgen(js_name = furniturePreview)]
    pub fn furniture_preview(&mut self, target: &str) -> Result<Doc> {
        let target: types::TableAddress = types::parse(target)?;
        let d = self.inner()?;
        let mut model = d.editor.doc.clone();
        let c = hm_edit::ops::list(&model, &crate::tables::path(&target))
            .and_then(|p| p.get(target.para as usize))
            .and_then(|p| p.controls.get(target.control as usize))
            .cloned()
            .ok_or_else(|| EngineError::new("BAD_ADDRESS", "no such header/footer"))?;
        let hm_model::ControlKind::HeaderFooter(mut chosen) = c.kind else {
            return Err(EngineError::new("BAD_REQUEST", "not a header/footer"));
        };
        chosen.attr &= !3;
        let mut cuts = Vec::new();
        for (si, section) in model.sections.iter().enumerate().rev() {
            for (pi, p) in section.paras.iter().enumerate().rev() {
                let offsets: Vec<u32> = p
                    .units()
                    .filter_map(|(offset, unit)| match unit {
                        hm_model::Unit::Extended { index, .. }
                            if p.controls[index as usize]
                                .as_header_footer()
                                .is_some_and(|h| h.footer == chosen.footer) =>
                        {
                            Some(offset)
                        }
                        _ => None,
                    })
                    .collect();
                for offset in offsets.into_iter().rev() {
                    let from = hm_edit::Pos {
                        list: hm_edit::ListPath {
                            section: si as u32,
                            steps: Vec::new(),
                        },
                        para: pi as u32,
                        offset,
                    };
                    let to = hm_edit::Pos {
                        offset: offset + 8,
                        ..from.clone()
                    };
                    cuts.push(hm_edit::Op::Delete { from, to });
                }
            }
        }
        for op in cuts {
            hm_edit::apply(&mut model, &op).map_err(|e| EngineError::new("BAD_ADDRESS", e))?;
        }
        let at = hm_edit::Pos {
            list: hm_edit::ListPath {
                section: 0,
                steps: Vec::new(),
            },
            para: 0,
            offset: hm_edit::ops::lead(&model.sections[0].paras[0]),
        };
        let c = hm_model::Control {
            kind: hm_model::ControlKind::HeaderFooter(chosen),
            src: None,
        };
        let op = hm_edit::ops::insert_control(&model, &at, c)
            .map_err(|e| EngineError::new("BAD_ADDRESS", e))?;
        hm_edit::apply(&mut model, &op).map_err(|e| EngineError::new("BAD_ADDRESS", e))?;
        model.source = None;
        let mut engine = Engine::new();
        engine.fonts = d.fonts.clone();
        engine.generation = d.generation.clone();
        engine.metrics = d.metrics.clone();
        engine.open(hm_hwpx::write_new(&model).map_err(|e| EngineError::new("SAVE_FAILED", e))?)
    }
}
