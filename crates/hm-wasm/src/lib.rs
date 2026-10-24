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
mod format;
pub mod index;
pub mod packet;
mod range;
mod tables;
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
    CharPatchJs, ComposeRequest, Copied, CopyRequest, FaceRequest, FontRequests, Match,
    ObjectTarget, PasteRequest, SearchRequest, SearchResult,
};
use wasm_bindgen::prelude::*;

use index::{Line, PageIndex, Paths, caret_range};
use types::{
    Address, Affinity, Caret, ControlInfo, DocState, EditOp, EngineError, LocateRequest, Located,
    Motion, NavigateRequest, PageInfo, ParagraphInfo, QueryRequest, Receipt, Rect, Result,
    SelectRequest, TransactRequest, json, parse,
};

impl From<(usize, hm_edit::EditError)> for EngineError {
    fn from((i, e): (usize, hm_edit::EditError)) -> Self {
        Inner::edit_error(i, e)
    }
}

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
            r#"{{"contract":{},"packet":{},"open":["hwp5","hwpx","hwp3"],"serialize":["same-format","cross-format"],"#,
            r#""scopes":["body","cell","tableCaption","textBox","objectCaption","header","footer","note","masterPage"],"#,
            r#""ops":["insertText","delete","split","formatChars","formatParas","pageBreak","applyStyle","setList","insertTableRows","deleteTableRows","insertTableColumns","deleteTableColumns","mergeTableCells","splitTableCell"],"paste":["at","replace"],"#,
            r#""locate":["point","caret"],"motions":["left","right","lineStart","lineEnd","up","down"],"#,
            r#""pageInfo":true,"equationScripts":true,"query":["paragraphs","formatContext","styles","style","lists","list","table","lines"],"search":true,"select":["crossList","cells"],"export":["svg","pdf"],"fonts":["fontFile","outlineRequests","fallbacks","unregister"],"#,
            r#""missing":["compare","merge"]}}"#
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

/// BLAKE3 of bytes fed in pieces, for the host's document digests.
#[wasm_bindgen]
pub struct Blake3Hasher(blake3::Hasher);

#[wasm_bindgen]
impl Blake3Hasher {
    #[wasm_bindgen(constructor)]
    #[allow(clippy::new_without_default)]
    pub fn new() -> Blake3Hasher {
        Blake3Hasher(blake3::Hasher::new())
    }

    pub fn update(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    /// The hash as lowercase hex; the hasher can go on taking bytes.
    #[wasm_bindgen(js_name = finalizeHex)]
    pub fn finalize_hex(&self) -> String {
        self.0.finalize().to_hex().to_string()
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
    /// The history position last saved: undoing back to it is clean.
    saved_history: u64,
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
    /// Pixel sizes of pictures hosts decode, by their bytes.
    picture_sizes: std::collections::HashMap<(usize, usize), Option<(u32, u32)>>,
    /// The picture decoded last, kept for the request that follows its
    /// size.
    last_picture: Option<((usize, usize), Rc<hm_render::image::Raster>)>,
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
    /// A picture decoded at its own size, the last one kept.
    fn decoded(
        &mut self,
        pic: &hm_render::draw::ImageItem,
    ) -> Option<Rc<hm_render::image::Raster>> {
        let key = (pic.data.as_ptr() as usize, pic.data.len());
        if let Some((k, r)) = &self.last_picture
            && *k == key
        {
            return Some(r.clone());
        }
        let r = Rc::new(hm_render::image::decode(&pic.data)?);
        self.last_picture = Some((key, r.clone()));
        Some(r)
    }

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

    /// The laid-out lines of the paragraph at `pos`, in order. A
    /// paragraph split over pages lists the lines of each page it reaches.
    fn lines(&mut self, pos: &Pos) -> Result<Vec<types::LineInfo>> {
        let mut out: Vec<types::LineInfo> = Vec::new();
        let s = pos.list.section;
        let top = pos.list.steps.first().map_or(pos.para, |c| c.para);
        let furniture = !pos.list.steps.is_empty();
        for i in 0..self.layout.pages.len() {
            let page = &self.layout.pages[i];
            if page.section != s {
                continue;
            }
            // Body text only shows on pages that hold its top paragraph.
            if !furniture {
                let holds = page.lines.iter().any(|l| l.para == top)
                    || page.tables.iter().any(|t| t.para == top)
                    || page.objects.iter().any(|o| o.para == top);
                if !holds {
                    continue;
                }
            }
            let index = self.index(i)?;
            for l in &index.lines {
                if l.list != pos.list || l.para != pos.para {
                    continue;
                }
                if out.iter().any(|o| o.line == l.line) {
                    continue;
                }
                out.push(types::LineInfo {
                    line: l.line,
                    page: i as u32,
                    y: l.y,
                    height: l.height,
                    baseline: l.baseline,
                    start: l.start(),
                    end: l.end(),
                    left: l.left(),
                    right: l.right(),
                });
            }
            // Furniture repeats on every page: the first page showing it
            // is enough.
            if furniture && !out.is_empty() {
                break;
            }
        }
        out.sort_by_key(|l| l.line);
        Ok(out)
    }

    fn receipt(&self, caret: Option<Address>) -> Receipt {
        self.receipt_after(caret, &self.page_sigs(), Some(&[]), None)
    }

    /// A fingerprint of each laid-out page: what sits where on it.
    fn page_sigs(&self) -> Vec<u64> {
        use std::hash::{Hash, Hasher};
        if !self.laid {
            return Vec::new();
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
    fn changed_pages(
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
    fn receipt_after(
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

    fn page_info(&self, index: usize, p: &hm_layout::Page) -> PageInfo {
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

    /// The selection collapsed at `caret`.
    fn collapsed(caret: &Address) -> serde_json::Value {
        serde_json::json!({
            "anchor": caret, "focus": caret,
            "anchorAffinity": "downstream", "focusAffinity": "downstream",
        })
    }

    /// Typed operations as model operations, with the caret after them.
    fn ops(
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
                    let p = Self::char_patch(doc, fonts, &patch)?;
                    for (from, to) in Self::spans_in(doc, &from.pos(), &to.pos())? {
                        ops.extend(hm_edit::script::char_ops(doc, &p, &from, &to).map_err(bad)?);
                    }
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
                    for (from, to) in Self::spans_in(doc, &from.pos(), &to.pos())? {
                        ops.extend(
                            hm_edit::script::para_ops(doc, &p, &from.list, from.para, to.para)
                                .map_err(bad)?,
                        );
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
                EditOp::SetList { from, to, list } => {
                    let set = Self::list_patch(doc, list.as_ref())?;
                    for (from, to) in Self::spans_in(doc, &from.pos(), &to.pos())? {
                        let bases: Vec<u16> = {
                            let paras = hm_edit::ops::list(doc, &from.list)
                                .ok_or_else(|| bad(hm_edit::EditError::NoSuchList))?;
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
                other => {
                    let (table, action) = tables::request(&other)
                        .ok_or_else(|| EngineError::new("BAD_REQUEST", "unknown edit operation"))?;
                    ops.push(tables::edit(doc, fonts, table, action)?.0);
                }
            }
        }
        Ok((ops, caret))
    }

    /// A host character patch for the editor: the face found or added in
    /// every slot (it must name a registered family), spacing checked.
    fn char_patch(
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
        let faces = match p.font_face.as_deref() {
            Some(name) => {
                if !fonts.resolves(name) {
                    return Err(EngineError::new(
                        "BAD_REQUEST",
                        format!("no registered font family {name:?}"),
                    ));
                }
                let f = clip::face_named(doc, name);
                Some(std::array::from_fn(|k| clip::face_id(doc, k, &f)))
            }
            None => None,
        };
        Ok(hm_edit::script::CharPatch {
            bold: p.bold,
            italic: p.italic,
            underline: p.underline,
            size: p.size,
            color: p.color,
            strike: p.strike,
            faces,
            spacing,
        })
    }

    /// The head kind, definition id and level a list request sets (none to
    /// leave lists); a restart adds a copy of the numbering starting there.
    fn list_patch(
        doc: &mut hm_model::Document,
        list: Option<&types::ListSpec>,
    ) -> Result<Option<(u32, u16, u32)>> {
        let Some(l) = list else { return Ok(None) };
        let info = &mut doc.info;
        let (kind, count) = match l.kind.as_str() {
            "numbering" => (2, info.numberings.len()),
            "bullet" => (3, info.bullets.len()),
            k => {
                return Err(EngineError::new(
                    "BAD_REQUEST",
                    format!("unknown list kind {k:?}"),
                ));
            }
        };
        if l.id == 0 || l.id as usize > count {
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

    /// The table control at `para`, `control` of a list.
    fn table_at(
        &self,
        list: &hm_edit::ListPath,
        para: u32,
        control: u32,
    ) -> Result<&hm_model::Table> {
        match hm_edit::ops::list(&self.editor.doc, list)
            .and_then(|l| l.get(para as usize))
            .and_then(|p| p.controls.get(control as usize))
            .map(|c| &c.kind)
        {
            Some(hm_model::ControlKind::Table(t)) => Ok(t),
            _ => Err(EngineError::new("BAD_ADDRESS", "no such table")),
        }
    }

    /// Cell origins and list addresses with their visible fragments on each page.
    fn table_info(&mut self, table: types::TableAddress, page: Option<u32>) -> Result<String> {
        if let Some(page) = page {
            if page as usize >= self.layout.pages.len() {
                return Err(EngineError::new("NO_SUCH_PAGE", "no such table query page"));
            }
        }
        let path = tables::path(&table);
        let t = self.table_at(&path, table.para, table.control)?;
        let (rows, columns) = (t.rows, t.cols);
        let cells: Vec<_> = t
            .cells
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let mut steps = table.steps.clone();
                steps.push(types::StepJs {
                    para: table.para,
                    control: table.control,
                    index: i as u32,
                });
                (
                    types::CellRef {
                        row: c.row,
                        column: c.col,
                    },
                    c.row_span,
                    c.col_span,
                    steps,
                    c as *const hm_model::Cell as usize,
                )
            })
            .collect();
        let furniture = table.steps.first().is_some_and(|step| {
            self.editor
                .doc
                .sections
                .get(table.section as usize)
                .and_then(|s| s.paras.get(step.para as usize))
                .and_then(|p| p.controls.get(step.control as usize))
                .is_some_and(|c| {
                    matches!(
                        c.kind,
                        hm_model::ControlKind::Section(_)
                            | hm_model::ControlKind::HeaderFooter(_)
                            | hm_model::ControlKind::Note(_)
                    )
                })
        });
        let pages = match page {
            Some(p) => vec![p as usize],
            None if furniture => self
                .layout
                .pages
                .iter()
                .enumerate()
                .filter(|(_, p)| p.section == table.section)
                .map(|(i, _)| i)
                .collect(),
            None => self.pages_of(
                &Pos {
                    list: path,
                    para: table.para,
                    offset: 0,
                },
                None,
            ),
        };
        let mut fragments = std::collections::HashMap::<usize, Vec<Rect>>::new();
        let wanted: std::collections::HashSet<_> = cells.iter().map(|c| c.4).collect();
        for page in pages {
            for c in
                hm_render::page_cells(&self.editor.doc, &self.layout, &self.fonts.borrow(), page)
            {
                if wanted.contains(&c.cell) {
                    fragments.entry(c.cell).or_default().push(Rect {
                        page: page as u32,
                        x: c.x,
                        y: c.y,
                        width: c.width,
                        height: c.height,
                    });
                }
            }
        }
        let mut out = Vec::new();
        let mut boxes = std::collections::BTreeMap::<u32, [i32; 4]>::new();
        for (at, row_span, column_span, steps, key) in cells {
            let mut rects = fragments.remove(&key).unwrap_or_default();
            rects.sort_by_key(|r| (r.page, r.y, r.x));
            for r in &rects {
                let b = boxes
                    .entry(r.page)
                    .or_insert([r.x, r.y, r.x + r.width, r.y + r.height]);
                b[0] = b[0].min(r.x);
                b[1] = b[1].min(r.y);
                b[2] = b[2].max(r.x + r.width);
                b[3] = b[3].max(r.y + r.height);
            }
            out.push(serde_json::json!({ "row": at.row, "column": at.column,
                "rowSpan": row_span, "columnSpan": column_span,
                "list": { "section": table.section, "steps": steps }, "rects": rects }));
        }
        let rects: Vec<_> = boxes
            .into_iter()
            .map(|(page, b)| Rect {
                page,
                x: b[0],
                y: b[1],
                width: b[2] - b[0],
                height: b[3] - b[1],
            })
            .collect();
        Ok(json(
            &serde_json::json!({ "table": table, "rows": rows, "columns": columns, "cells": out, "rects": rects }),
        ))
    }

    /// A selection request normalized, with its page rectangles.
    fn selection(&mut self, req: SelectRequest) -> Result<(range::Range, Vec<Rect>)> {
        let (range, page) = match req {
            SelectRequest::Text(t) => (self.range(&t.from.pos(), &t.to.pos())?, t.page),
            SelectRequest::Cells(c) => {
                if c.kind != "cells" {
                    return Err(EngineError::new(
                        "BAD_REQUEST",
                        format!("unknown kind {:?}", c.kind),
                    ));
                }
                let list = Address {
                    section: c.table.section,
                    steps: c.table.steps.clone(),
                    para: 0,
                    offset: 0,
                }
                .pos()
                .list;
                let t = self.table_at(&list, c.table.para, c.table.control)?;
                let rows = (c.from.row.min(c.to.row), c.from.row.max(c.to.row));
                let cols = (
                    c.from.column.min(c.to.column),
                    c.from.column.max(c.to.column),
                );
                let (rows, cols) = range::grow(t, rows, cols);
                (
                    range::Range::Cells {
                        list,
                        para: c.table.para,
                        control: c.table.control,
                        rows,
                        cols,
                    },
                    c.page,
                )
            }
        };
        let mut out = Vec::new();
        let rect = |page: usize, [x, y, w, h]: [i32; 4]| Rect {
            page: page as u32,
            x,
            y,
            width: w,
            height: h,
        };
        match &range {
            range::Range::Text(from, to) => {
                let pages: Vec<usize> = match page {
                    Some(p) => vec![p as usize],
                    None => {
                        let a = self.caret(from, Affinity::Downstream, None)?;
                        let b = self.caret(to, Affinity::Upstream, None)?;
                        match (a, b) {
                            (Some(a), Some(b)) => (a.page as usize..=b.page as usize).collect(),
                            (Some(a), None) => vec![a.page as usize],
                            (None, Some(b)) => vec![b.page as usize],
                            (None, None) => Vec::new(),
                        }
                    }
                };
                for &p in &pages {
                    for r in self.index(p)?.selection(from, to) {
                        out.push(rect(p, r));
                    }
                }
                let doc = &self.editor.doc;
                let paras = hm_edit::ops::list(doc, &from.list);
                // Tables and objects of the body inside the range show whole.
                let inside = |para: u32, control: u32| -> bool {
                    let Some(p) = paras.and_then(|l| l.get(para as usize)) else {
                        return false;
                    };
                    p.units().any(|(pos, u)| {
                        matches!(u, Unit::Extended { index, .. } if index == control)
                            && (from.para, from.offset) <= (para, pos)
                            && (para, pos) < (to.para, to.offset)
                    })
                };
                for p in pages {
                    let Some(page) = self.layout.pages.get(p) else {
                        continue;
                    };
                    if !from.list.steps.is_empty() || page.section != from.list.section {
                        continue;
                    }
                    for t in &page.tables {
                        if inside(t.para, t.control) {
                            let w = self
                                .layout
                                .para(page.section, t.para)
                                .table(t.control)
                                .map_or(0, |tl| tl.width);
                            out.push(rect(p, [t.x, t.y, w, t.height()]));
                        }
                    }
                    for o in &page.objects {
                        if inside(o.para, o.control) {
                            out.push(rect(p, [o.rect.x, o.rect.y, o.rect.w, o.rect.h]));
                        }
                    }
                }
            }
            range::Range::Cells {
                list,
                para,
                control,
                rows,
                cols,
            } => {
                let t = self.table_at(list, *para, *control)?;
                let cells = range::block_cells(t, *rows, *cols);
                if list.steps.is_empty() {
                    // Body tables: each cell box on each fragment showing it.
                    let section = list.section;
                    let tl = self.layout.para(section, *para).table(*control);
                    for (i, pg) in self.layout.pages.iter().enumerate() {
                        if pg.section != section || page.is_some_and(|q| q as usize != i) {
                            continue;
                        }
                        for f in pg
                            .tables
                            .iter()
                            .filter(|f| f.para == *para && f.control == *control)
                        {
                            let Some(tl) = tl else { continue };
                            let carried = f.carried.as_ref().map_or(0, |c| c.height);
                            for &k in &cells {
                                let Some(c) = tl.cells.get(k) else { continue };
                                let (y0, y1) = (c.y.max(f.top), (c.y + c.h).min(f.bottom));
                                if y1 <= y0 {
                                    continue;
                                }
                                let y = f.y + f.header + carried + (y0 - f.top);
                                out.push(rect(i, [f.x + c.x, y, c.w, y1 - y0]));
                            }
                        }
                    }
                }
                // A table set as a character sits on its line, its bottom on
                // the baseline when the line has room above it.
                if out.is_empty()
                    && list.steps.is_empty()
                    && let Some(tl) = self.layout.para(list.section, *para).table(*control)
                {
                    let height = tl.height;
                    let boxes: Vec<[i32; 4]> = cells
                        .iter()
                        .filter_map(|&k| tl.cells.get(k).map(|c| [c.x, c.y, c.w, c.h]))
                        .collect();
                    let at = hm_edit::ops::list(&self.editor.doc, list)
                        .and_then(|l| l.get(*para as usize))
                        .and_then(|p| range::control_span(p, *control))
                        .map_or(0, |s| s.0);
                    let pages: Vec<usize> = match page {
                        Some(p) => vec![p as usize],
                        None => self.pages_of(
                            &Pos {
                                list: list.clone(),
                                para: *para,
                                offset: at,
                            },
                            None,
                        ),
                    };
                    for p in pages {
                        let index = self.index(p)?;
                        let Some(l) = index.lines.iter().find(|l| {
                            l.list == *list && l.para == *para && l.start() <= at && at < l.end()
                        }) else {
                            continue;
                        };
                        // Never above its line's top.
                        let (x, y) = (l.x_of(at), l.y + (l.baseline - height).max(0));
                        for [cx, cy, w, h] in &boxes {
                            out.push(rect(p, [x + cx, y + cy, *w, *h]));
                        }
                    }
                }
                if out.is_empty() {
                    // Nested tables: the lines of each cell, boxed per page.
                    let pages: Vec<usize> = match page {
                        Some(p) => vec![p as usize],
                        None => self.pages_of(
                            &Pos {
                                list: list.clone(),
                                para: *para,
                                offset: 0,
                            },
                            None,
                        ),
                    };
                    for p in pages {
                        let index = self.index(p)?;
                        for &k in &cells {
                            let mut steps = list.steps.clone();
                            steps.push(hm_edit::Step {
                                para: *para,
                                control: *control,
                                index: k as u32,
                            });
                            let cell = hm_edit::ListPath {
                                section: list.section,
                                steps,
                            };
                            let mut b = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
                            for l in index.lines.iter().filter(|l| l.list == cell) {
                                b = (
                                    b.0.min(l.left()),
                                    b.1.min(l.y),
                                    b.2.max(l.right()),
                                    b.3.max(l.y + l.height),
                                );
                            }
                            if b.0 <= b.2 {
                                out.push(rect(p, [b.0, b.1, b.2 - b.0, b.3 - b.1]));
                            }
                        }
                    }
                }
            }
        }
        Ok((range, out))
    }

    /// A selection from `a` to `b` normalized ([`range::normalize`]).
    fn range(&self, a: &Pos, b: &Pos) -> Result<range::Range> {
        self.check(a)?;
        self.check(b)?;
        range::normalize(&self.editor.doc, a, b).map_err(|e| EngineError::new("BAD_ADDRESS", e))
    }

    /// The single-list spans a selection covers: the range itself, or the
    /// whole content of each cell of a block.
    fn spans(&self, a: &Pos, b: &Pos) -> Result<Vec<(Pos, Pos)>> {
        Self::spans_in(&self.editor.doc, a, b)
    }

    fn spans_in(doc: &hm_model::Document, a: &Pos, b: &Pos) -> Result<Vec<(Pos, Pos)>> {
        for pos in [a, b] {
            if caret_range(doc, &pos.list, pos.para).is_none() {
                return Err(EngineError::new(
                    "BAD_ADDRESS",
                    "no such paragraph list or paragraph",
                ));
            }
        }
        let range = range::normalize(doc, a, b).map_err(|e| EngineError::new("BAD_ADDRESS", e))?;
        Ok(match range {
            range::Range::Text(from, to) => vec![(from, to)],
            range::Range::Cells {
                list,
                para,
                control,
                rows,
                cols,
            } => {
                let Some(hm_model::ControlKind::Table(t)) = hm_edit::ops::list(doc, &list)
                    .and_then(|l| l.get(para as usize))
                    .and_then(|p| p.controls.get(control as usize))
                    .map(|c| &c.kind)
                else {
                    return Err(EngineError::new("BAD_ADDRESS", "no such table"));
                };
                range::block_cells(t, rows, cols)
                    .into_iter()
                    .map(|i| {
                        let mut steps = list.steps.clone();
                        steps.push(hm_edit::Step {
                            para,
                            control,
                            index: i as u32,
                        });
                        let cell = hm_edit::ListPath {
                            section: list.section,
                            steps,
                        };
                        let paras = &t.cells[i].paras;
                        let last = paras.len().saturating_sub(1);
                        let from = Pos {
                            list: cell.clone(),
                            para: 0,
                            offset: paras.first().map_or(0, hm_edit::ops::lead),
                        };
                        let to = Pos {
                            list: cell,
                            para: last as u32,
                            offset: paras.last().map_or(0, |p| p.len().saturating_sub(1)),
                        };
                        (from, to)
                    })
                    .collect()
            }
        })
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
            dirty: d.editor.history_id() != d.saved_history || d.composition.is_some(),
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
                let key = (pic.data.as_ptr() as usize, pic.data.len());
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
                "{}-{:x}-{:x}",
                d.instance,
                pic.data.as_ptr() as usize,
                pic.data.len()
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
        let (_, rects) = self.inner()?.selection(req)?;
        Ok(json(&rects))
    }

    /// Like [`Doc::select`], with the selection as normalized
    /// (`Selected`): the text range in the common list, or the block of
    /// cells with each cell's list.
    #[wasm_bindgen(js_name = selectRange)]
    pub fn select_range(&mut self, request: &str) -> Result<String> {
        let req: SelectRequest = parse(request)?;
        let d = self.inner()?;
        let (range, rects) = d.selection(req)?;
        let range = match range {
            range::Range::Text(from, to) => types::SelectedRange::Text {
                from: Address::of(&from),
                to: Address::of(&to),
            },
            range::Range::Cells {
                list,
                para,
                control,
                rows,
                cols,
            } => {
                let t = d.table_at(&list, para, control)?;
                let step = |p: &hm_edit::Step| types::StepJs {
                    para: p.para,
                    control: p.control,
                    index: p.index,
                };
                let cells = range::block_cells(t, rows, cols)
                    .into_iter()
                    .map(|i| {
                        let c = &t.cells[i];
                        let mut steps: Vec<types::StepJs> = list.steps.iter().map(step).collect();
                        steps.push(types::StepJs {
                            para,
                            control,
                            index: i as u32,
                        });
                        types::SelectedCell {
                            row: c.row,
                            column: c.col,
                            row_span: c.row_span.max(1),
                            column_span: c.col_span.max(1),
                            list: types::ListAddressOut {
                                section: list.section,
                                steps,
                            },
                        }
                    })
                    .collect();
                types::SelectedRange::Cells {
                    table: types::TableAddress {
                        section: list.section,
                        steps: list.steps.iter().map(step).collect(),
                        para,
                        control,
                    },
                    from: types::CellRef {
                        row: rows.0,
                        column: cols.0,
                    },
                    to: types::CellRef {
                        row: rows.1,
                        column: cols.1,
                    },
                    cells,
                }
            }
        };
        Ok(json(&types::Selected { range, rects }))
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
        let before = d.page_sigs();
        let mut caret = None;
        let explicit_after = req.after_selection.clone();
        let mut after = req.after_selection.or_else(|| req.selection.clone());
        let mut table_changes = Vec::new();
        let fonts = d.fonts.clone();
        let changed = d.editor.transact_mapped(
            req.ops,
            req.selection.as_ref().map(|v| v.to_string()),
            |doc, op| {
                let (ops, next) = match tables::request(&op) {
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
                if let Some(next) = next {
                    if explicit_after.is_none() {
                        after = Some(Inner::collapsed(&Address::of(&next)));
                    }
                    caret = Some(next);
                }
                Ok::<_, EngineError>((ops, after.as_ref().map(|v| v.to_string())))
            },
        )?;
        if !changed {
            return Ok(json(&d.receipt(None)));
        }
        d.content_epoch += 1;
        d.relayout();
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
        if !text.is_empty() {
            let after = Some(Inner::collapsed(&Address::of(&end)).to_string());
            d.editor
                .transact_with(vec![Op::InsertText { at, text }], None, after)
                .map_err(|(i, e)| Inner::edit_error(i, e))?;
        }
        d.content_epoch += 1;
        d.relayout();
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

    /// Copies `[from, to)` (`CopyRequest`) as `Copied` text and HTML for
    /// the system clipboard. The fragment is shared by this Engine's
    /// documents until the next copy, survives source closure, and carries
    /// face names for paste into another document.
    pub fn copy(&mut self, request: &str) -> Result<String> {
        let req: CopyRequest = parse(request)?;
        let d = self.inner()?;
        let (from, to) = match d.range(&req.from.pos(), &req.to.pos())? {
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
                return Ok(json(&Copied { text, html }));
            }
        };
        let frag = clip::copy(&d.editor.doc, d.instance, &from, &to)
            .map_err(|e| Inner::edit_error(0, e))?;
        let out = Copied {
            text: frag.text(),
            html: frag.html(),
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
        let frag = d.clipboard.borrow().clone();
        let text = req
            .text
            .map(|t| t.replace("\r\n", "\n").replace('\r', "\n"));
        let frag = frag.filter(|f| text.as_ref().is_none_or(|t| *t == f.text()));
        // Pasting may add shapes and faces to the document tables; a
        // failure anywhere puts them back.
        let tables = d.editor.doc.info.clone();
        let made = match (frag, text) {
            (Some(f), _) => clip::paste(&mut d.editor.doc, d.instance, &f, &at)
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
                return Err(e);
            }
        };
        // The paste's operations hold after the cut: it only removes text
        // after `at`, and the shape pasted over is the one before it.
        if let Some(cut) = cut {
            ops.insert(0, cut);
        }
        let before = d.page_sigs();
        let after = req
            .after_selection
            .unwrap_or_else(|| Inner::collapsed(&Address::of(&end)));
        if let Err((i, e)) =
            d.editor
                .transact_with(ops, kept.map(|v| v.to_string()), Some(after.to_string()))
        {
            d.editor.doc.info = tables;
            return Err(Inner::edit_error(i, e));
        }
        d.content_epoch += 1;
        d.relayout();
        let scopes = d.editor.changed().map(<[_]>::to_vec);
        Ok(json(&d.receipt_after(
            Some(Address::of(&end)),
            &before,
            scopes.as_deref(),
            Some(after),
        )))
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
        Ok(json(&d.receipt_after(None, &[], None, None)))
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

    /// Bounded reads of the document (`QueryRequest`), as JSON:
    /// `paragraphs` gives `ParagraphInfo[]` for a list, `formatContext` a
    /// `FormatContext`, `styles` `StyleInfo[]` and `style` one (or null),
    /// `lists` `ListInfo[]` and `list` one (or null).
    pub fn query(&mut self, request: &str) -> Result<String> {
        let req: QueryRequest = parse(request)?;
        let (list, from, count) = match req {
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

    /// The document written as `format` (`"hwp5"` or `"hwpx"`): in its
    /// own format as [`Doc::serialize`] writes it, in the other as a new
    /// file encoded from the model alone. The open document keeps its
    /// format; `markSaved` clears the dirty state.
    #[wasm_bindgen(js_name = serializeAs)]
    pub fn serialize_as(&mut self, format: &str, mark_saved: bool) -> Result<Vec<u8>> {
        let target = match format {
            "hwp5" => Format::Hwp5,
            "hwpx" => Format::Hwpx,
            _ => {
                return Err(EngineError::new(
                    "UNSUPPORTED_FORMAT",
                    format!("cannot write {format}"),
                ));
            }
        };
        let d = self.inner()?;
        if d.editor.doc.meta.format == target {
            return self.serialize(mark_saved);
        }
        if d.editor.doc.meta.format == Format::Hwp3 {
            return Err(EngineError::new(
                "UNSUPPORTED_FORMAT",
                "HWP 3 documents cannot be saved yet",
            ));
        }
        let doc = hm_edit::lines::prepare_conversion(&d.editor.doc, &d.layout, target)
            .map_err(|e| EngineError::new("SAVE_FAILED", e))?;
        let bytes = match target {
            Format::Hwpx => hm_hwpx::write_new(&doc).map_err(|e| e.to_string()),
            _ => hm_hwp5::write_new(&doc).map_err(|e| e.to_string()),
        }
        .map_err(|e| EngineError::new("SAVE_FAILED", e))?;
        if mark_saved {
            d.saved_epoch = d.content_epoch;
        }
        Ok(bytes)
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
            d.saved_history = d.editor.history_id();
        }
        Ok(bytes)
    }

    /// Marks the document saved as it was at `expectedEpoch` (after the
    /// host wrote what `serialize(false)` gave it); `STALE_EPOCH` when it
    /// changed since. Changes nothing else.
    #[wasm_bindgen(js_name = markSaved)]
    pub fn mark_saved(&mut self, expected_epoch: u32) -> Result<()> {
        let d = self.raw()?;
        if expected_epoch != d.content_epoch {
            return Err(EngineError::new(
                "STALE_EPOCH",
                format!(
                    "expected epoch {expected_epoch}, the document is at {}",
                    d.content_epoch
                ),
            ));
        }
        d.saved_epoch = d.content_epoch;
        d.saved_history = d.editor.history_id();
        Ok(())
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

/// The wire offset of each UTF-16 unit of a paragraph's plain text, then
/// the end mark's.
fn text_offsets(p: &hm_model::Paragraph) -> Vec<u32> {
    let mut out = Vec::new();
    for (pos, u) in p.units() {
        match u {
            Unit::Char(c) => out.extend((0..c.len_utf16() as u32).map(|k| pos + k)),
            Unit::Inline(9) | Unit::CharControl(10 | 30 | 31) => out.push(pos),
            _ => {}
        }
    }
    out.push(p.len().saturating_sub(1));
    out
}

/// Wire units text takes once inserted: a tab is an 8-unit control.
fn wire_len(text: &str) -> u32 {
    text.chars()
        .map(|c| if c == '\t' { 8 } else { c.len_utf16() as u32 })
        .sum()
}
