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

mod agent_api;
mod clip;
mod clip_import;
mod clipboard;
mod connectors;
mod count;
mod definitions;
mod dialogs;
mod editing;
mod engine;
mod format;
mod grouping;
mod history;
pub mod index;
mod notes;
mod object_api;
mod objects;
mod operations;
pub mod packet;
mod queries;
mod range;
mod rendering;
mod saving;
mod selection;
mod studio_bridge;
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
            r#"{{"contract":{},"packet":{},"open":["hwp5","hwpx","hwp3","hml"],"serialize":["same-format","cross-format"],"#,
            r#""scopes":["body","cell","tableCaption","textBox","objectCaption","header","footer","note","masterPage"],"#,
            r#""ops":["insertText","delete","split","formatChars","formatParas","pageBreak","applyStyle","setList","setCharShape","setParaShape","createStyle","updateStyle","deleteStyle","ensureList","insertTableRows","deleteTableRows","insertTableColumns","deleteTableColumns","mergeTableCells","splitTableCell","setPageSettings","setSectionSettings","setColumns","setNoteShape","setPageBorder","setPageHide","createHeaderFooter","insertNote","renumberNotes","insertField","updateField","insertBookmark","renameBookmark","deleteControl","setFormValue","setDocumentProperties","columnBreak","insertNewNumber","moveObject","resizeObject","resizeTableCells","createTable","insertPicture","assignPictureImage","createShape","deleteObject","relocateObject","setObjectProperties","setTableProperties","setCellProperties","setCellZone","insertEquation","promoteOleEquation","setEquation","setTableCaption","setObjectCaption","setLineEnds","groupObjects","ungroupObject"],"objects":true,"objectTransforms":true,"connectors":true,"paragraphCaptures":true,"tableTranspose":true,"adopt":true,"furniturePreview":true,"paste":["at","replace"],"#,
            r#""locate":["point","caret"],"motions":["left","right","lineStart","lineEnd","up","down"],"#,
            r#""pageInfo":true,"equationScripts":true,"query":["paragraphs","formatContext","styles","style","lists","list","table","count","formatRuns","lines","fonts","fontCandidates","charShape","paraShape","sectionSettings","documentProperties","noteShape","controls","fields","objectProperties","tableProperties","cellProperties","pictureData","equation","equationPreview"],"search":true,"select":["crossList","cells"],"export":["svg","pdf","hwpx"],"fonts":["fontFile","outlineRequests","fallbacks","unregister"],"#,
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

/// Decompresses a Brotli stream, such as a WOFF2 font's tables, into at
/// most `max` bytes. Browsers without a Brotli `DecompressionStream` read
/// web fonts through this.
#[wasm_bindgen(js_name = brotliDecompress)]
pub fn brotli_decompress(bytes: &[u8], max: usize) -> Result<Vec<u8>> {
    use std::io::Read;
    let mut out = Vec::new();
    let reader = brotli_decompressor::Decompressor::new(bytes, 4096);
    reader
        .take(max as u64 + 1)
        .read_to_end(&mut out)
        .map_err(|e| EngineError::new("MALFORMED_INPUT", format!("brotli: {e}")))?;
    if out.len() > max {
        return Err(EngineError::new(
            "MALFORMED_INPUT",
            "brotli: output too large",
        ));
    }
    Ok(out)
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
    /// Object transforms hosts keep to swap back in, by handle.
    transforms: std::collections::HashMap<u32, object_api::Transform>,
    paragraph_captures: std::collections::HashMap<u32, hm_model::Paragraph>,
    transpose_clipboard: Option<hm_model::Table>,
    cell_clipboard: Option<hm_model::Table>,
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

#[wasm_bindgen]
impl Doc {
    /// Releases the document; later calls fail with `CLOSED`.
    pub fn close(&mut self) {
        self.inner = None;
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
                Format::Hml => "hml",
                Format::Hwp3 => "hwp3",
            },
            page_count: d.layout.pages.len(),
            dirty: d.editor.history_id() != d.saved_history || d.composition.is_some(),
            undo,
            redo,
        }))
    }
}

/// Whether a list lies in a header, footer, note or master page.
fn furniture_list(doc: &hm_model::Document, list: &hm_edit::ListPath) -> bool {
    let mut path = hm_edit::ListPath {
        section: list.section,
        steps: Vec::new(),
    };
    for step in &list.steps {
        let Some(c) = hm_edit::ops::list(doc, &path)
            .and_then(|paras| paras.get(step.para as usize))
            .and_then(|p| p.controls.get(step.control as usize))
        else {
            return false;
        };
        if matches!(
            c.kind,
            hm_model::ControlKind::HeaderFooter(_)
                | hm_model::ControlKind::Note(_)
                | hm_model::ControlKind::Section(_)
        ) {
            return true;
        }
        path.steps.push(step.clone());
    }
    false
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
