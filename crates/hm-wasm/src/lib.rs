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

mod clip;
pub mod index;
pub mod packet;
mod text;
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
use types::{
    ComposeRequest, Copied, CopyRequest, FaceRequest, FontRequests, Match, ObjectTarget,
    PasteRequest, SearchRequest, SearchResult,
};
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
            r#"{{"contract":{},"packet":{},"open":["hwp5","hwpx","hwp3"],"serialize":["same-format"],"#,
            r#""scopes":["body","cell","tableCaption","textBox","objectCaption","header","footer","note","masterPage"],"#,
            r#""ops":["insertText","delete","split","formatChars","formatParas","pageBreak"],"#,
            r#""locate":["point","caret"],"motions":["left","right","lineStart","lineEnd","up","down"],"#,
            r#""query":["paragraphs"],"search":true,"export":["svg","pdf"],"fonts":["fontFile","outlineRequests","fallbacks","unregister"],"#,
            r#""missing":["compare","merge","crossFormatSave"]}}"#
        ),
        CONTRACT,
        packet::VERSION
    )
}

/// The family names of a font file (TTF, OTF, TTC or HFT) as a JSON
/// `string[]`, for a host's font catalog; empty when unreadable.
#[wasm_bindgen(js_name = familyNames)]
pub fn family_names(bytes: &[u8]) -> String {
    families_of(bytes)
}

/// [`family_names`] of a file the host holds, reading only its tables.
#[wasm_bindgen(js_name = fileFamilyNames)]
pub fn file_family_names(file: &FontFile) -> String {
    families_of(&HostFile(file))
}

fn families_of<S: hm_fonts::FontSource + ?Sized>(src: &S) -> String {
    let mut fs = FontSystem::new();
    fs.add_compact("probe", src, &[]);
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

#[wasm_bindgen]
extern "C" {
    /// A font file the host holds: `size` bytes, read in pieces.
    #[wasm_bindgen(typescript_type = "FontFileSource")]
    pub type FontFile;
    #[wasm_bindgen(method, getter, structural)]
    fn size(this: &FontFile) -> f64;
    #[wasm_bindgen(method, structural)]
    fn read(this: &FontFile, offset: f64, length: f64) -> Vec<u8>;
}

struct HostFile<'a>(&'a FontFile);

impl hm_fonts::FontSource for HostFile<'_> {
    fn size(&self) -> usize {
        self.0.size() as usize
    }
    fn read(&self, offset: usize, len: usize) -> Option<Vec<u8>> {
        if offset.checked_add(len)? > self.size() {
            return None;
        }
        let b = self.0.read(offset as f64, len as f64);
        (b.len() == len).then_some(b)
    }
}

/// Fonts shared by the documents an engine opens.
#[wasm_bindgen]
pub struct Engine {
    fonts: Rc<RefCell<FontSystem>>,
    /// Bumps on every font change, outlines included.
    generation: Rc<Cell<u32>>,
    /// Bumps on font changes that can move text: layouts redo then.
    metrics: Rc<Cell<u32>>,
    /// Characters of the documents opened, whose outlines font copies keep.
    chars: Rc<RefCell<text::CharSet>>,
    /// The last fragment copied from any of its documents.
    clipboard: Rc<RefCell<Option<clip::Fragment>>>,
}

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

    /// Opens HWP 5, HWPX or HWP 3 bytes and lays them out.
    pub fn open(&self, bytes: Vec<u8>) -> Result<Doc> {
        let bytes = SharedBytes::from(bytes);
        let doc = if hm_hwpx::is_hwpx(&bytes) {
            hm_hwpx::read(bytes).map_err(|e| EngineError::new("MALFORMED_INPUT", e))?
        } else if hm_hwp5::is_hwp5(&bytes) {
            hm_hwp5::read(bytes).map_err(|e| EngineError::new("MALFORMED_INPUT", e))?
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
                laid_with: self.metrics.get(),
                drawn_with: self.generation.get(),
                instance: INSTANCES.fetch_add(1, Ordering::Relaxed),
                content_epoch: 0,
                layout_epoch: 0,
                saved_epoch: 0,
                pages: Vec::new(),
                indexes: Default::default(),
                paths: None,
                font_use: (0, used),
            }),
        })
    }
}

type Built = Rc<(DisplayList, Rc<PageIndex>)>;

struct Inner {
    editor: Editor,
    layout: DocLayout,
    fonts: Rc<RefCell<FontSystem>>,
    generation: Rc<Cell<u32>>,
    metrics: Rc<Cell<u32>>,
    /// The metrics generation the layout was made with.
    laid_with: u32,
    /// The font generation the kept pages were drawn with.
    drawn_with: u32,
    /// The document has been laid out.
    laid: bool,
    instance: u32,
    content_epoch: u32,
    layout_epoch: u32,
    saved_epoch: u32,
    /// Built pages, most recent last.
    pages: Vec<(usize, Built)>,
    /// Line indexes of pages, built or not.
    indexes: std::collections::HashMap<usize, Rc<PageIndex>>,
    /// Paragraph addresses to list positions, for the current document.
    paths: Option<Paths>,
    clipboard: Rc<RefCell<Option<clip::Fragment>>>,
    /// The IME text in progress: where it starts, its length in wire units
    /// and the operation taking it out again.
    composition: Option<(Pos, u32, Op)>,
    /// What the text draws with, as of a content epoch.
    font_use: (u32, Rc<text::FontUse>),
    checkpoints: std::collections::HashMap<u32, hm_edit::Snapshot>,
    next_checkpoint: u32,
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
        if !d.laid || d.laid_with != d.metrics.get() {
            if d.laid {
                d.editor.forget_layout();
            }
            d.relayout();
        }
        if d.drawn_with != d.generation.get() {
            d.drawn_with = d.generation.get();
            d.pages.clear();
        }
        Ok(d)
    }
}

impl Inner {
    fn relayout(&mut self) {
        self.laid = true;
        self.laid_with = self.metrics.get();
        let fonts = self.fonts.borrow();
        fonts.set_layout_running(true);
        self.layout = self.editor.layout(&fonts);
        fonts.set_layout_running(false);
        drop(fonts);
        self.layout_epoch += 1;
        self.pages.clear();
        self.indexes.clear();
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
        let index = Rc::new(PageIndex::new(boxes, paths));
        self.indexes.insert(i, index.clone());
        let built = Rc::new((list, index));
        self.pages.push((i, built.clone()));
        if self.pages.len() > KEPT_PAGES {
            self.pages.remove(0);
        }
        Ok(built)
    }

    /// The line index of page `i`, found without painting the page.
    fn index(&mut self, i: usize) -> Result<Rc<PageIndex>> {
        if let Some(index) = self.indexes.get(&i) {
            return Ok(index.clone());
        }
        if i >= self.layout.pages.len() {
            return Err(EngineError::new("NO_SUCH_PAGE", format!("no page {i}")));
        }
        let boxes = {
            let fonts = self.fonts.borrow();
            hm_render::page_lines(&self.editor.doc, &self.layout, &fonts, i)
        };
        let paths = self
            .paths
            .get_or_insert_with(|| Paths::new(&self.editor.doc));
        let index = Rc::new(PageIndex::new(boxes, paths));
        self.indexes.insert(i, index.clone());
        Ok(index)
    }

    /// The body's floating objects and tables on page `i`, bottom to top.
    fn objects(&self, i: usize) -> Vec<ObjectTarget> {
        use hm_layout::objects::Layer;
        let Some(page) = self.layout.pages.get(i) else {
            return Vec::new();
        };
        let section = page.section;
        let Some(paras) = self
            .editor
            .doc
            .sections
            .get(section as usize)
            .map(|s| &s.paras)
        else {
            return Vec::new();
        };
        let control = |para: u32, k: u32| paras.get(para as usize)?.controls.get(k as usize);
        let layer = |l: Layer| match l {
            Layer::Behind => "behind",
            Layer::Text => "text",
            Layer::Front => "front",
        };
        let rank = |l: &str| match l {
            "behind" => 0,
            "text" => 1,
            _ => 2,
        };
        let mut out: Vec<(i32, ObjectTarget)> = Vec::new();
        for o in &page.objects {
            let Some(c) = control(o.para, o.control) else {
                continue;
            };
            out.push((
                o.z,
                ObjectTarget {
                    section,
                    steps: Vec::new(),
                    para: o.para,
                    control: o.control,
                    kind: control_kind(c),
                    layer: layer(o.layer),
                    page: i as u32,
                    x: o.rect.x,
                    y: o.rect.y,
                    width: o.rect.w,
                    height: o.rect.h,
                },
            ));
        }
        for t in &page.tables {
            let Some(c) = control(t.para, t.control) else {
                continue;
            };
            let width = self
                .layout
                .para(section, t.para)
                .table(t.control)
                .map_or(0, |tl| tl.width);
            let (l, z) = match c.object_common() {
                Some(oc) if !oc.treat_as_char() => (hm_layout::objects::layer_of(&oc), oc.z_order),
                _ => (Layer::Text, 0),
            };
            out.push((
                z,
                ObjectTarget {
                    section,
                    steps: Vec::new(),
                    para: t.para,
                    control: t.control,
                    kind: "table",
                    layer: layer(l),
                    page: i as u32,
                    x: t.x,
                    y: t.y,
                    width,
                    height: t.height(),
                },
            ));
        }
        out.sort_by_key(|(z, o)| (rank(o.layer), *z));
        out.into_iter().map(|(_, o)| o).collect()
    }

    /// Fails while IME text is in progress: it must be committed or
    /// cancelled first.
    fn not_composing(&self) -> Result<()> {
        if self.composition.is_some() {
            return Err(EngineError::new(
                "COMPOSING",
                "commit or cancel the composition first",
            ));
        }
        Ok(())
    }

    /// Takes the IME text out again; returns where it started.
    fn uncompose(&mut self) -> Option<Pos> {
        let (at, _, inverse) = self.composition.take()?;
        // The inverse of an insertion just made always applies.
        let _ = hm_edit::apply(&mut self.editor.doc, &inverse);
        Some(at)
    }

    fn edit_error(i: usize, e: hm_edit::EditError) -> EngineError {
        EngineError::new("BAD_ADDRESS", format!("operation {i}: {e}"))
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
            if let Some(c) = self.index(i)?.caret(pos, affinity) {
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
                    end.offset += wire_len(parts.next().unwrap_or(""));
                    for part in parts {
                        end.para += 1;
                        end.offset = wire_len(part);
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
            hm_edit::ops::list(doc, &pos.list)
                .and_then(|l| l.get(pos.para as usize))
                .map_or_else(Vec::new, text::stops)
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
                let built = self.index(c.page as usize)?;
                let Some(line) = built.line_of(&pos, req.affinity) else {
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
                let x = req.preferred_x.map_or(c.x, |v| v.round() as i32);
                let up = req.motion == Motion::Up;
                let built = self.index(c.page as usize)?;
                let here = built.line_of(&pos, req.affinity).map(|l| (l.y, l.height));
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
                let mut target = pick(&built).map(|t| (c.page as usize, t));
                if target.is_none() {
                    let next = if up {
                        (c.page as usize).checked_sub(1)
                    } else {
                        Some(c.page as usize + 1).filter(|&p| p < self.layout.pages.len())
                    };
                    if let Some(p) = next {
                        let idx = self.index(p)?;
                        let edge = if up {
                            idx.lines.iter().max_by_key(|l| l.y + l.height)
                        } else {
                            idx.lines.iter().min_by_key(|l| l.y)
                        };
                        target = edge.map(|l| (p, (l.y, l.height)));
                    }
                }
                let Some((p, (y, h))) = target else {
                    return Ok(Some((pos, req.affinity)));
                };
                let idx = self.index(p)?;
                let hit = idx.hit(&self.editor.doc, x, y + h / 2, &|l: &Line| {
                    l.y == y && l.height == h
                });
                Ok(hit.map(|(pos, aff, _, _)| (pos, aff)))
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
        let mut faces: Vec<FaceRequest> = Vec::new();
        let mut named: Vec<&hm_model::FaceName> = used
            .faces
            .iter()
            .filter_map(|&(script, id)| info.faces[script].get(id as usize))
            .collect();
        // The layout knows exactly which names its text asked for.
        if d.laid {
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
        let laid = d.laid.then_some(&d.layout.fallback_families);
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

    /// `DocState` as JSON.
    pub fn state(&mut self) -> Result<String> {
        let d = self.inner()?;
        let (undo, redo) = d.editor.history();
        Ok(json(&DocState {
            instance: d.instance,
            content_epoch: d.content_epoch,
            layout_epoch: d.layout_epoch,
            font_generation: d.drawn_with,
            format: match d.editor.doc.meta.format {
                Format::Hwp5 => "hwp5",
                Format::Hwpx => "hwpx",
                Format::Hwp3 => "hwp3",
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
            .map(|(index, p)| {
                let def = d
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

    /// Page `i` as an SVG document: text and rules as vectors, pictures
    /// and brushes as PNG patches at `dpi`.
    #[wasm_bindgen(js_name = pageSvg)]
    pub fn page_svg(&mut self, i: usize, dpi: f32) -> Result<String> {
        let d = self.inner()?;
        let built = d.page(i)?;
        Ok(hm_render::svg::page(&built.0, &d.fonts.borrow(), dpi))
    }

    /// Pages `from..to` as one PDF for printing or export: text and rules
    /// as vectors, pictures and brushes as images at `dpi`.
    pub fn pdf(&mut self, from: usize, to: usize, dpi: f32) -> Result<Vec<u8>> {
        let d = self.inner()?;
        let to = to.min(d.layout.pages.len());
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
            let dl = hm_render::build_page(&d.editor.doc, &d.layout, &fonts, i);
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

    /// A point to a position (`Located`), or a position to its caret
    /// (`Caret`); `null` when nothing editable is there.
    pub fn locate(&mut self, request: &str) -> Result<String> {
        let req: LocateRequest = parse(request)?;
        let d = self.inner()?;
        match req {
            LocateRequest::Point { page, x, y } => {
                let (x, y) = (x.round() as i32, y.round() as i32);
                let built = d.index(page as usize)?;
                let Some((pos, affinity, c, inside)) = built.hit(&d.editor.doc, x, y, &|_| true)
                else {
                    return Ok("null".into());
                };
                // The topmost object under the point, unless it lies behind
                // text the point is on.
                let object = d
                    .objects(page as usize)
                    .into_iter()
                    .rev()
                    .find(|o| x >= o.x && x < o.x + o.width && y >= o.y && y < o.y + o.height)
                    .filter(|o| o.layer != "behind" || !inside);
                Ok(json(&Located {
                    position: Address::of(&pos),
                    affinity,
                    caret: Caret {
                        page,
                        x: c[0],
                        y: c[1],
                        height: c[2],
                    },
                    object,
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

    /// The body's floating objects and tables on page `i`
    /// (`ObjectTarget[]`), bottom to top.
    pub fn objects(&mut self, i: usize) -> Result<String> {
        let d = self.inner()?;
        if i >= d.layout.pages.len() {
            return Err(EngineError::new("NO_SUCH_PAGE", format!("no page {i}")));
        }
        Ok(json(&d.objects(i)))
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
            for [x, y, w, h] in d.index(p)?.selection(&from, &to) {
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
            object: None,
        }))
    }

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
            d.composition = Some((at.clone(), len, inverse));
        }
        d.content_epoch += 1;
        d.relayout();
        let end = Pos {
            offset: at.offset + len,
            ..at
        };
        Ok(json(&d.receipt(Some(Address::of(&end)))))
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
        d.uncompose();
        let end = Pos {
            offset: at.offset + wire_len(&text),
            ..at.clone()
        };
        if !text.is_empty() {
            d.editor
                .transact(vec![Op::InsertText { at, text }])
                .map_err(|(i, e)| Inner::edit_error(i, e))?;
        }
        d.content_epoch += 1;
        d.relayout();
        Ok(json(&d.receipt(Some(Address::of(&end)))))
    }

    /// Takes the IME text out; a `Receipt` with the caret where it began,
    /// or `null` when nothing was composing.
    #[wasm_bindgen(js_name = cancelComposition)]
    pub fn cancel_composition(&mut self) -> Result<String> {
        let d = self.inner()?;
        let Some(at) = d.uncompose() else {
            return Ok("null".into());
        };
        d.content_epoch += 1;
        d.relayout();
        Ok(json(&d.receipt(Some(Address::of(&at)))))
    }

    /// Copies `[from, to)` (`CopyRequest`) as `Copied` text and HTML for
    /// the system clipboard; the engine keeps the fragment for `paste`.
    pub fn copy(&mut self, request: &str) -> Result<String> {
        let req: CopyRequest = parse(request)?;
        let d = self.inner()?;
        let frag = clip::copy(&d.editor.doc, d.instance, &req.from.pos(), &req.to.pos())
            .map_err(|e| Inner::edit_error(0, e))?;
        let out = Copied {
            text: frag.text(),
            html: frag.html(),
        };
        *d.clipboard.borrow_mut() = Some(frag);
        Ok(json(&out))
    }

    /// Pastes at `at` (`PasteRequest`) as one undo step: the engine's
    /// fragment when `text` is absent or is the fragment's text, else
    /// `text` as typed. A `Receipt` with the caret after the paste.
    pub fn paste(&mut self, request: &str) -> Result<String> {
        let req: PasteRequest = parse(request)?;
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
        let at = req.at.pos();
        d.check(&at)?;
        let frag = d.clipboard.borrow().clone();
        let text = req
            .text
            .map(|t| t.replace("\r\n", "\n").replace('\r', "\n"));
        let frag = frag.filter(|f| text.as_ref().is_none_or(|t| *t == f.text()));
        let (ops, end) = match (frag, text) {
            (Some(f), _) => clip::paste(&mut d.editor.doc, d.instance, &f, &at)
                .map_err(|e| Inner::edit_error(0, e))?,
            (None, Some(t)) => {
                let (mut ops, caret) = d.ops(vec![EditOp::InsertText {
                    at: Address::of(&at),
                    text: t,
                }])?;
                (std::mem::take(&mut ops), caret.unwrap_or(at))
            }
            (None, None) => return Err(EngineError::new("BAD_REQUEST", "nothing to paste")),
        };
        d.editor
            .transact(ops)
            .map_err(|(i, e)| Inner::edit_error(i, e))?;
        d.content_epoch += 1;
        d.relayout();
        Ok(json(&d.receipt(Some(Address::of(&end)))))
    }

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
        Ok(json(&d.receipt(None)))
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
        d.not_composing()?;
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
            Format::Hwp3 => {
                return Err(EngineError::new(
                    "UNSUPPORTED_FORMAT",
                    "HWP 3 documents cannot be saved yet",
                ));
            }
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

/// Wire units text takes once inserted: a tab is an 8-unit control.
fn wire_len(text: &str) -> u32 {
    text.chars()
        .map(|c| if c == '\t' { 8 } else { c.len_utf16() as u32 })
        .sum()
}
