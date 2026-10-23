//! The engine for Studio, as WebAssembly.
//!
//! JS registers font files with an [`Engine`] and opens HWP 5 or HWPX bytes
//! as a [`Doc`]. A page comes as one binary packet ([`packet`]): paint ops
//! for a canvas player, the page's line index ([`index`]) and the glyphs it
//! paints, whose outlines the engine hands out once each. Pictures and
//! brushes canvas cannot draw come as pixel patches. Requests and results
//! with structure travel as JSON ([`types`]); failures are [`EngineError`]s
//! with a code.
//!
//! Edits go through `hm-edit` as atomic transactions with grouped undo,
//! and the layout after one formats only the changed paragraphs. Every
//! successful edit, undo or redo bumps the content epoch; every layout
//! bumps the layout epoch. Positions are wire offsets in paragraph lists
//! reached from a section body through control steps: table cells and
//! captions, text boxes, headers, footers, notes and master pages.

pub mod index;
pub mod packet;
pub mod types;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use hm_edit::{Editor, Op, Pos};
use hm_fonts::FontSystem;
use hm_layout::DocLayout;
use hm_model::{Format, SharedBytes, Unit};
use hm_render::DisplayList;
use wasm_bindgen::prelude::*;

use index::{Line, PageIndex, Paths, caret_range};
use types::{
    Address, Affinity, Caret, ControlInfo, DocState, EditOp, EngineError, LocateRequest, Located,
    Motion, NavigateRequest, PageInfo, ParagraphInfo, QueryRequest, Receipt, Rect, Result,
    SelectRequest, TransactRequest, json, parse,
};

/// Contract version of the JSON requests and results.
pub const CONTRACT: u32 = 1;
/// Pages whose display lists and line indexes are kept.
const KEPT_PAGES: usize = 12;

static INSTANCES: AtomicU32 = AtomicU32::new(1);

/// What this build offers, as JSON (`Capabilities`).
#[wasm_bindgen]
pub fn capabilities() -> String {
    format!(
        concat!(
            r#"{{"contract":{},"packet":{},"open":["hwp5","hwpx"],"serialize":["same-format"],"#,
            r#""scopes":["body","cell","tableCaption","textBox","objectCaption","header","footer","note","masterPage"],"#,
            r#""ops":["insertText","delete","split","formatChars","formatParas","pageBreak"],"#,
            r#""locate":["point","caret"],"motions":["left","right","lineStart","lineEnd","up","down"],"#,
            r#""query":["paragraphs"],"missing":["create","unregisterFonts","checkpoint","clipboard","search","compare","merge","crossFormatSave"]}}"#
        ),
        CONTRACT,
        packet::VERSION
    )
}

/// The family names of a font file (TTF, OTF, TTC or HFT) as a JSON
/// `string[]`, for a host's font catalog; empty when unreadable.
#[wasm_bindgen(js_name = familyNames)]
pub fn family_names(bytes: Vec<u8>) -> String {
    let mut fs = FontSystem::new();
    fs.add("probe", Arc::new(bytes));
    let mut out: Vec<String> = Vec::new();
    for e in fs.entries() {
        for f in &e.families {
            if !out.contains(f) {
                out.push(f.clone());
            }
        }
    }
    json(&out)
}

/// Fonts shared by the documents an engine opens.
#[wasm_bindgen]
pub struct Engine {
    fonts: Rc<RefCell<FontSystem>>,
    generation: Rc<Cell<u32>>,
}

#[wasm_bindgen]
impl Engine {
    #[wasm_bindgen(constructor)]
    #[allow(clippy::new_without_default)]
    pub fn new() -> Engine {
        Engine {
            fonts: Rc::new(RefCell::new(FontSystem::new())),
            generation: Rc::new(Cell::new(0)),
        }
    }

    fn bump(&self) {
        self.generation.set(self.generation.get() + 1);
    }

    /// Registers a TTF, OTF, TTC or HFT file; returns the faces added.
    /// Documents lay out again with it on their next request.
    #[wasm_bindgen(js_name = addFont)]
    pub fn add_font(&self, label: &str, bytes: Vec<u8>) -> usize {
        let n = self.fonts.borrow_mut().add(label, Arc::new(bytes));
        self.bump();
        n
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

    /// Opens HWP 5 or HWPX bytes and lays them out.
    pub fn open(&self, bytes: Vec<u8>) -> Result<Doc> {
        let bytes = SharedBytes::from(bytes);
        let doc = if hm_hwpx::is_hwpx(&bytes) {
            hm_hwpx::read(bytes).map_err(|e| EngineError::new("MALFORMED_INPUT", e))?
        } else if hm_hwp5::is_hwp5(&bytes) {
            hm_hwp5::read(bytes).map_err(|e| EngineError::new("MALFORMED_INPUT", e))?
        } else {
            return Err(EngineError::new(
                "UNSUPPORTED_FORMAT",
                "neither an HWP 5 compound file nor an HWPX package",
            ));
        };
        // The layout waits for the first request that needs it, so fonts
        // the document names (`Doc.fontNames`) can be registered first.
        Ok(Doc {
            inner: Some(Inner {
                editor: Editor::new(doc),
                layout: DocLayout::default(),
                laid: false,
                fonts: self.fonts.clone(),
                generation: self.generation.clone(),
                laid_with: self.generation.get(),
                instance: INSTANCES.fetch_add(1, Ordering::Relaxed),
                content_epoch: 0,
                layout_epoch: 0,
                saved_epoch: 0,
                pages: Vec::new(),
                paths: None,
            }),
        })
    }
}

type Built = Rc<(DisplayList, PageIndex)>;

struct Inner {
    editor: Editor,
    layout: DocLayout,
    fonts: Rc<RefCell<FontSystem>>,
    generation: Rc<Cell<u32>>,
    /// The font generation the layout was made with.
    laid_with: u32,
    /// The document has been laid out.
    laid: bool,
    instance: u32,
    content_epoch: u32,
    layout_epoch: u32,
    saved_epoch: u32,
    /// Built pages, most recent last.
    pages: Vec<(usize, Built)>,
    /// Paragraph addresses to list positions, for the current document.
    paths: Option<Paths>,
}

/// An open document.
#[wasm_bindgen]
pub struct Doc {
    inner: Option<Inner>,
}

impl Doc {
    /// The open document, without laying it out.
    fn raw(&mut self) -> Result<&mut Inner> {
        self.inner
            .as_mut()
            .ok_or_else(|| EngineError::new("CLOSED", "the document is closed"))
    }

    /// The open document, laid out with the current fonts.
    fn inner(&mut self) -> Result<&mut Inner> {
        let d = self.raw()?;
        if !d.laid || d.laid_with != d.generation.get() {
            if d.laid {
                d.editor.forget_layout();
            }
            d.relayout();
        }
        Ok(d)
    }
}

impl Inner {
    fn relayout(&mut self) {
        self.laid = true;
        self.laid_with = self.generation.get();
        self.layout = self.editor.layout(&self.fonts.borrow());
        self.layout_epoch += 1;
        self.pages.clear();
        self.paths = None;
    }

    fn page(&mut self, i: usize) -> Result<Built> {
        if i >= self.layout.pages.len() {
            return Err(EngineError::new("NO_SUCH_PAGE", format!("no page {i}")));
        }
        if let Some(k) = self.pages.iter().position(|(p, _)| *p == i) {
            let entry = self.pages.remove(k);
            let built = entry.1.clone();
            self.pages.push(entry);
            return Ok(built);
        }
        let (list, boxes) = {
            let fonts = self.fonts.borrow();
            hm_render::build_page_indexed(&self.editor.doc, &self.layout, &fonts, i)
        };
        let paths = self
            .paths
            .get_or_insert_with(|| Paths::new(&self.editor.doc));
        let built = Rc::new((list, PageIndex::new(boxes, paths)));
        self.pages.push((i, built.clone()));
        if self.pages.len() > KEPT_PAGES {
            self.pages.remove(0);
        }
        Ok(built)
    }

    fn check(&self, pos: &Pos) -> Result<(u32, u32)> {
        let (lo, hi) = caret_range(&self.editor.doc, &pos.list, pos.para).ok_or_else(|| {
            EngineError::new("BAD_ADDRESS", "no such paragraph list or paragraph")
        })?;
        Ok((lo, hi))
    }

    /// Pages likely to show a position, nearest first: the hint, then the
    /// pages drawing its section's top paragraph.
    fn pages_of(&self, pos: &Pos, hint: Option<u32>) -> Vec<usize> {
        let s = pos.list.section;
        let para = pos.list.steps.first().map_or(pos.para, |c| c.para);
        let mut v: Vec<(u32, usize)> = self
            .layout
            .pages
            .iter()
            .enumerate()
            .filter(|(_, p)| p.section == s)
            .filter_map(|(i, p)| {
                let paras = p
                    .lines
                    .iter()
                    .map(|l| l.para)
                    .chain(p.tables.iter().map(|t| t.para))
                    .chain(p.objects.iter().map(|o| o.para));
                let (lo, hi) = paras.fold((u32::MAX, 0), |(lo, hi), q| (lo.min(q), hi.max(q)));
                if lo > hi {
                    return None;
                }
                let d = if para < lo {
                    lo - para
                } else {
                    para.saturating_sub(hi)
                };
                (d <= 1).then_some((d, i))
            })
            .collect();
        v.sort();
        let mut out: Vec<usize> = hint.map(|h| h as usize).into_iter().collect();
        out.extend(
            v.into_iter()
                .map(|(_, i)| i)
                .filter(|i| Some(*i as u32) != hint),
        );
        // Furniture shows on pages far from its anchor: the first page of
        // the section as a last resort.
        if !pos.list.steps.is_empty()
            && let Some(first) = self.layout.pages.iter().position(|p| p.section == s)
            && !out.contains(&first)
        {
            out.push(first);
        }
        out
    }

    fn caret(&mut self, pos: &Pos, affinity: Affinity, hint: Option<u32>) -> Result<Option<Caret>> {
        for i in self.pages_of(pos, hint) {
            if i >= self.layout.pages.len() {
                continue;
            }
            if let Some(c) = self.page(i)?.1.caret(pos, affinity) {
                return Ok(Some(Caret {
                    page: i as u32,
                    x: c[0],
                    y: c[1],
                    height: c[2],
                }));
            }
        }
        Ok(None)
    }

    fn receipt(&self, caret: Option<Address>) -> Receipt {
        let (undo, redo) = self.editor.history();
        Receipt {
            content_epoch: self.content_epoch,
            layout_epoch: self.layout_epoch,
            page_count: self.layout.pages.len(),
            caret,
            undo,
            redo,
        }
    }

    /// Typed operations as model operations, with the caret after them.
    fn ops(&mut self, req: Vec<EditOp>) -> Result<(Vec<Op>, Option<Pos>)> {
        let mut ops = Vec::new();
        let mut caret = None;
        let bad = |e: hm_edit::EditError| EngineError::new("BAD_ADDRESS", e);
        for op in req {
            match op {
                EditOp::InsertText { at, text } => {
                    let at = at.pos();
                    let mut end = at.clone();
                    let mut parts = text.split('\n');
                    end.offset += parts.next().unwrap_or("").encode_utf16().count() as u32;
                    for part in parts {
                        end.para += 1;
                        end.offset = part.encode_utf16().count() as u32;
                    }
                    caret = Some(end);
                    ops.push(Op::InsertText { at, text });
                }
                EditOp::Delete { from, to } => {
                    caret = Some(from.pos());
                    ops.push(Op::Delete {
                        from: from.pos(),
                        to: to.pos(),
                    });
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
                    let p = hm_edit::script::CharPatch {
                        bold: patch.bold,
                        italic: patch.italic,
                        underline: patch.underline,
                        size: patch.size,
                        color: patch.color,
                    };
                    ops.extend(
                        hm_edit::script::char_ops(&mut self.editor.doc, &p, &from.pos(), &to.pos())
                            .map_err(bad)?,
                    );
                }
                EditOp::FormatParas { from, to, patch } => {
                    let align = match patch.align.as_deref() {
                        None => None,
                        Some(a) => Some(hm_edit::script::align_code(a).ok_or_else(|| {
                            EngineError::new("BAD_REQUEST", format!("unknown align {a:?}"))
                        })?),
                    };
                    let p = hm_edit::script::ParaPatch {
                        align,
                        space_before: patch.space_before,
                        space_after: patch.space_after,
                        left: patch.left,
                        indent: patch.indent,
                        line_spacing: patch.line_spacing,
                    };
                    let (from, to) = (from.pos(), to.pos());
                    if from.list != to.list || to.para < from.para {
                        return Err(EngineError::new(
                            "BAD_ADDRESS",
                            "range ends in different lists or reversed",
                        ));
                    }
                    ops.extend(
                        hm_edit::script::para_ops(
                            &mut self.editor.doc,
                            &p,
                            &from.list,
                            from.para,
                            to.para,
                        )
                        .map_err(bad)?,
                    );
                }
                EditOp::PageBreak { at, on } => {
                    let at = at.pos();
                    ops.push(Op::SetPageBreak {
                        list: at.list,
                        para: at.para,
                        on,
                    });
                }
            }
        }
        Ok((ops, caret))
    }

    /// The caret position after `motion`, with its affinity.
    fn navigate(&mut self, req: &NavigateRequest) -> Result<Option<(Pos, Affinity)>> {
        let pos = req.position.pos();
        let (lo, hi) = self.check(&pos)?;
        let stops = |doc: &hm_model::Document, pos: &Pos| -> Vec<u32> {
            let Some(p) = hm_edit::ops::list(doc, &pos.list).and_then(|l| l.get(pos.para as usize))
            else {
                return Vec::new();
            };
            let (lo, hi) = (hm_edit::ops::lead(p), p.len().saturating_sub(1));
            p.units()
                .map(|(q, _): (u32, Unit)| q)
                .filter(|&q| q >= lo && q <= hi)
                .collect()
        };
        let doc = &self.editor.doc;
        let count = hm_edit::ops::list(doc, &pos.list).map_or(0, |l| l.len() as u32);
        match req.motion {
            Motion::Left | Motion::Right => {
                let s = stops(doc, &pos);
                let left = req.motion == Motion::Left;
                let next = if left {
                    s.iter().rev().find(|&&q| q < pos.offset).copied()
                } else {
                    s.iter().find(|&&q| q > pos.offset).copied()
                };
                if let Some(offset) = next {
                    return Ok(Some((Pos { offset, ..pos }, Affinity::Downstream)));
                }
                let para = if left {
                    pos.para.checked_sub(1)
                } else {
                    (pos.para + 1 < count).then_some(pos.para + 1)
                };
                let Some(para) = para else {
                    return Ok(Some((pos, req.affinity)));
                };
                let target = Pos {
                    para,
                    ..pos.clone()
                };
                let (lo, hi) = caret_range(doc, &target.list, para).unwrap_or((0, 0));
                let offset = if left { hi } else { lo };
                Ok(Some((Pos { offset, ..target }, Affinity::Downstream)))
            }
            Motion::LineStart | Motion::LineEnd => {
                let Some(c) = self.caret(&pos, req.affinity, req.page)? else {
                    return Ok(None);
                };
                let built = self.page(c.page as usize)?;
                let Some(line) = built.1.line_of(&pos, req.affinity) else {
                    return Ok(None);
                };
                if req.motion == Motion::LineStart {
                    let offset = line.start().max(lo);
                    Ok(Some((Pos { offset, ..pos }, Affinity::Downstream)))
                } else if line.end() > hi {
                    Ok(Some((Pos { offset: hi, ..pos }, Affinity::Downstream)))
                } else {
                    let offset = line.end();
                    Ok(Some((Pos { offset, ..pos }, Affinity::Upstream)))
                }
            }
            Motion::Up | Motion::Down => {
                let Some(c) = self.caret(&pos, req.affinity, req.page)? else {
                    return Ok(None);
                };
                let x = req.preferred_x.unwrap_or(c.x);
                let up = req.motion == Motion::Up;
                let built = self.page(c.page as usize)?;
                let here = built.1.line_of(&pos, req.affinity).map(|l| (l.y, l.height));
                let (top, bottom) = here.map_or((c.y, c.y + c.height), |(y, h)| (y, y + h));
                // The nearest line above or below that overlaps in x, else
                // the nearest one at all.
                let beyond = |l: &Line| {
                    if up {
                        l.y + l.height <= top
                    } else {
                        l.y >= bottom
                    }
                };
                let gap = |l: &Line| {
                    let dy = if up {
                        top - (l.y + l.height)
                    } else {
                        l.y - bottom
                    };
                    let dx = if x < l.left() {
                        l.left() - x
                    } else {
                        (x - l.right()).max(0)
                    };
                    (dx > 0, dy, dx)
                };
                let pick = |idx: &PageIndex| {
                    idx.lines
                        .iter()
                        .filter(|l| beyond(l))
                        .min_by_key(|l| gap(l))
                        .map(|l| (l.y, l.height))
                };
                let mut target = pick(&built.1).map(|t| (c.page as usize, t));
                if target.is_none() {
                    let next = if up {
                        (c.page as usize).checked_sub(1)
                    } else {
                        Some(c.page as usize + 1).filter(|&p| p < self.layout.pages.len())
                    };
                    if let Some(p) = next {
                        let idx = self.page(p)?;
                        let edge = if up {
                            idx.1.lines.iter().max_by_key(|l| l.y + l.height)
                        } else {
                            idx.1.lines.iter().min_by_key(|l| l.y)
                        };
                        target = edge.map(|l| (p, (l.y, l.height)));
                    }
                }
                let Some((p, (y, h))) = target else {
                    return Ok(Some((pos, req.affinity)));
                };
                let idx = self.page(p)?;
                let hit = idx.1.hit(&self.editor.doc, x, y + h / 2, &|l: &Line| {
                    l.y == y && l.height == h
                });
                Ok(hit.map(|(pos, aff, _)| (pos, aff)))
            }
        }
    }
}

#[wasm_bindgen]
impl Doc {
    /// Releases the document; later calls fail with `CLOSED`.
    pub fn close(&mut self) {
        self.inner = None;
    }

    /// Font families the document may draw with, as a JSON `string[]`:
    /// each face name with its alternates, aliases, font-map stand-ins and
    /// class defaults. Doesn't lay the document out.
    #[wasm_bindgen(js_name = fontNames)]
    pub fn font_names(&mut self) -> Result<String> {
        let d = self.raw()?;
        let fonts = d.fonts.borrow();
        let mut out: Vec<String> = Vec::new();
        for f in d.editor.doc.info.faces.iter().flatten() {
            let name = f.name.to_string_lossy();
            let sub = f.substitute.as_ref().map(|(_, n)| n.to_string_lossy());
            let def = f.default_name.as_ref().map(|n| n.to_string_lossy());
            let alternates: Vec<&str> = sub.iter().chain(def.iter()).map(String::as_str).collect();
            for n in fonts.wanted_families(&name, &alternates) {
                if !out.contains(&n) {
                    out.push(n);
                }
            }
        }
        Ok(json(&out))
    }

    /// `DocState` as JSON.
    pub fn state(&mut self) -> Result<String> {
        let d = self.inner()?;
        let (undo, redo) = d.editor.history();
        Ok(json(&DocState {
            instance: d.instance,
            content_epoch: d.content_epoch,
            layout_epoch: d.layout_epoch,
            font_generation: d.laid_with,
            format: match d.editor.doc.meta.format {
                Format::Hwp5 => "hwp5",
                Format::Hwpx => "hwpx",
            },
            page_count: d.layout.pages.len(),
            dirty: d.content_epoch != d.saved_epoch,
            undo,
            redo,
        }))
    }

    /// `PageInfo[]` as JSON, sizes in HU.
    pub fn pages(&mut self) -> Result<String> {
        let d = self.inner()?;
        let v: Vec<PageInfo> = d
            .layout
            .pages
            .iter()
            .enumerate()
            .map(|(index, p)| PageInfo {
                index,
                section: p.section,
                width: p.width,
                height: p.height,
                number: p.number,
            })
            .collect();
        Ok(json(&v))
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
            font_generation: d.laid_with,
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

    /// Page `i` rasterized by the engine as PNG, the reference the canvas
    /// player is checked against.
    #[wasm_bindgen(js_name = pagePng)]
    pub fn page_png(&mut self, i: usize, dpi: f32) -> Result<Vec<u8>> {
        let d = self.inner()?;
        let built = d.page(i)?;
        hm_render::raster::render_png(&built.0, &d.fonts.borrow(), dpi)
            .ok_or_else(|| EngineError::new("BAD_REQUEST", "page too large to rasterize"))
    }

    /// A point to a position (`Located`), or a position to its caret
    /// (`Caret`); `null` when nothing editable is there.
    pub fn locate(&mut self, request: &str) -> Result<String> {
        let req: LocateRequest = parse(request)?;
        let d = self.inner()?;
        match req {
            LocateRequest::Point { page, x, y } => {
                let built = d.page(page as usize)?;
                let Some((pos, affinity, c)) = built.1.hit(&d.editor.doc, x, y, &|_| true) else {
                    return Ok("null".into());
                };
                Ok(json(&Located {
                    position: Address::of(&pos),
                    affinity,
                    caret: Caret {
                        page,
                        x: c[0],
                        y: c[1],
                        height: c[2],
                    },
                }))
            }
            LocateRequest::Caret {
                position,
                affinity,
                page,
            } => {
                let pos = position.pos();
                d.check(&pos)?;
                Ok(match d.caret(&pos, affinity, page)? {
                    Some(c) => json(&c),
                    None => "null".into(),
                })
            }
        }
    }

    /// Selection rectangles (`Rect[]`) for `[from, to)` in one list.
    pub fn select(&mut self, request: &str) -> Result<String> {
        let req: SelectRequest = parse(request)?;
        let d = self.inner()?;
        let (mut from, mut to) = (req.from.pos(), req.to.pos());
        d.check(&from)?;
        d.check(&to)?;
        if (to.para, to.offset) < (from.para, from.offset) {
            std::mem::swap(&mut from, &mut to);
        }
        let pages: Vec<usize> = match req.page {
            Some(p) => vec![p as usize],
            None => {
                let a = d.caret(&from, Affinity::Downstream, None)?;
                let b = d.caret(&to, Affinity::Upstream, None)?;
                match (a, b) {
                    (Some(a), Some(b)) => (a.page as usize..=b.page as usize).collect(),
                    (Some(a), None) => vec![a.page as usize],
                    (None, Some(b)) => vec![b.page as usize],
                    (None, None) => Vec::new(),
                }
            }
        };
        let mut out = Vec::new();
        for p in pages {
            for [x, y, w, h] in d.page(p)?.1.selection(&from, &to) {
                out.push(Rect {
                    page: p as u32,
                    x,
                    y,
                    width: w,
                    height: h,
                });
            }
        }
        Ok(json(&out))
    }

    /// The caret after a movement (`Located`), `null` when the position is
    /// on no page.
    pub fn navigate(&mut self, request: &str) -> Result<String> {
        let req: NavigateRequest = parse(request)?;
        let d = self.inner()?;
        let Some((pos, affinity)) = d.navigate(&req)? else {
            return Ok("null".into());
        };
        let Some(caret) = d.caret(&pos, affinity, req.page)? else {
            return Ok("null".into());
        };
        Ok(json(&Located {
            position: Address::of(&pos),
            affinity,
            caret,
        }))
    }

    /// Applies edits atomically as one undo step and lays out again;
    /// returns a `Receipt`. With `expectedEpoch` set, a document edited
    /// since fails with `STALE_EPOCH` and nothing changes.
    pub fn transact(&mut self, request: &str) -> Result<String> {
        let req: TransactRequest = parse(request)?;
        let d = self.inner()?;
        if let Some(e) = req.expected_epoch
            && e != d.content_epoch
        {
            return Err(EngineError::new(
                "STALE_EPOCH",
                format!("expected epoch {e}, the document is at {}", d.content_epoch),
            ));
        }
        let (ops, caret) = d.ops(req.ops)?;
        if ops.is_empty() {
            return Ok(json(&d.receipt(None)));
        }
        d.editor.transact(ops).map_err(|(i, e)| {
            let code = match e {
                hm_edit::EditError::BadRange | hm_edit::EditError::BadOffset => "BAD_ADDRESS",
                hm_edit::EditError::NoSuchList | hm_edit::EditError::NoSuchParagraph => {
                    "BAD_ADDRESS"
                }
            };
            EngineError::new(code, format!("operation {i}: {e}"))
        })?;
        d.content_epoch += 1;
        d.relayout();
        Ok(json(&d.receipt(caret.as_ref().map(Address::of))))
    }

    /// Reverts the last transaction; a `Receipt`, or `null` when there is
    /// nothing to undo.
    pub fn undo(&mut self) -> Result<String> {
        let d = self.inner()?;
        let done = d
            .editor
            .undo()
            .map_err(|e| EngineError::new("EDIT_FAILED", e))?;
        if !done {
            return Ok("null".into());
        }
        d.content_epoch += 1;
        d.relayout();
        Ok(json(&d.receipt(None)))
    }

    /// Applies the last undone transaction again; like [`Doc::undo`].
    pub fn redo(&mut self) -> Result<String> {
        let d = self.inner()?;
        let done = d
            .editor
            .redo()
            .map_err(|e| EngineError::new("EDIT_FAILED", e))?;
        if !done {
            return Ok("null".into());
        }
        d.content_epoch += 1;
        d.relayout();
        Ok(json(&d.receipt(None)))
    }

    /// Bounded reads of the document (`QueryRequest`), as JSON:
    /// `paragraphs` gives `ParagraphInfo[]` for a list.
    pub fn query(&mut self, request: &str) -> Result<String> {
        let req: QueryRequest = parse(request)?;
        let d = self.inner()?;
        let QueryRequest::Paragraphs { list, from, count } = req;
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
                            })
                        }
                        _ => None,
                    })
                    .collect(),
            })
            .collect();
        Ok(json(&out))
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

    /// The document in its own format, edited paragraphs carrying line
    /// segments from our layout. `markSaved` clears the dirty state; the
    /// content epoch never changes.
    pub fn serialize(&mut self, mark_saved: bool) -> Result<Vec<u8>> {
        let d = self.inner()?;
        let mut doc = d.editor.doc.clone();
        hm_edit::lines::fill(&mut doc, &d.layout);
        let bytes = match doc.meta.format {
            Format::Hwpx => hm_hwpx::write(&doc).map_err(|e| EngineError::new("SAVE_FAILED", e))?,
            Format::Hwp5 => hm_hwp5::write(&doc).map_err(|e| EngineError::new("SAVE_FAILED", e))?,
        };
        if mark_saved {
            d.saved_epoch = d.content_epoch;
        }
        Ok(bytes)
    }
}

fn control_kind(c: &hm_model::Control) -> &'static str {
    use hm_model::ControlKind::*;
    match &c.kind {
        Section(_) => "section",
        Columns(_) => "columns",
        Table(_) => "table",
        Object(o) if o.equation().is_some() => "equation",
        Object(_) => "object",
        HeaderFooter(h) if h.footer => "footer",
        HeaderFooter(_) => "header",
        Note(n) if n.endnote => "endnote",
        Note(_) => "footnote",
        AutoNumber(_) => "autoNumber",
        NewNumber(_) => "newNumber",
        PageNumber(_) => "pageNumber",
        PageHide(_) => "pageHide",
        PageParity(_) => "pageParity",
        Field(_) => "field",
        Bookmark(_) => "bookmark",
        IndexMark(_) => "indexMark",
        Form(_) => "form",
        Compose(_) => "compose",
        Dutmal(_) => "dutmal",
        Raw(_) => "raw",
    }
}
