//! The formatter: style resolution, measurement, line breaking, line
//! metrics, paragraph placement, columns and pages, and the audit against
//! Hancom's stored line segments.
//!
//! All values are integer HWPUNIT. Our own arithmetic decides every line;
//! stored line segments are only compared ([`audit`]).

pub mod annot;
pub mod audit;
mod columns;
pub mod compat;
pub mod forms;
pub mod furniture;
pub mod heads;
pub mod incr;
mod linenum;
pub mod math;
pub mod measure;
mod notes;
mod numbers;
pub mod objects;
pub mod paginate;
pub mod para;
pub mod script;
pub mod style;
pub mod table;

use hm_fonts::FontSystem;
use hm_model::Document;

pub use audit::{Audit, audit};
pub use furniture::{
    CellStep, Frame, FrameKind, FrameLine, LAST_PAGE_MASTER, ListRef, Mark, PageBorder, Rule,
};
pub use incr::{LayoutCache, layout_cached};
pub use measure::{InlineBox, Kind, Measured};
pub use paginate::{Page, PlacedLine, Rect, SectionLayout};
pub use para::{HeadLayout, Line, ParaLayout, Placed, place};
pub use style::Style;
pub use table::{
    CUT_ROOM, CaptionLayout, Carried, CellLayout, CellLine, CellTable, PlacedTable, TableLayout,
};

/// A laid-out document.
#[derive(Clone, Debug, Default)]
pub struct DocLayout {
    pub sections: Vec<SectionLayout>,
    pub pages: Vec<Page>,
    /// Styles referenced by [`Measured::styles`].
    pub styles: Vec<Style>,
    /// Face names that resolved to another family: `(requested, used)`.
    pub substitutions: Vec<(String, String)>,
    /// Families looked in for characters a face lacked
    /// ([`style::Styles::fallback_families`]).
    pub fallback_families: Vec<String>,
    /// Face names the document's text asked for, found or not.
    pub requested_faces: Vec<String>,
}

impl DocLayout {
    pub fn para(&self, section: u32, para: u32) -> &ParaLayout {
        &self.sections[section as usize].paras[para as usize]
    }
}

/// Choices for [`layout_with`].
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Keep the line membership Hancom stored for unedited paragraphs, as
    /// Hancom does when it opens a file; off breaks every line fresh.
    pub stored_lines: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { stored_lines: true }
    }
}

/// Takes the page-local numbers of a section's footnotes as the numbers
/// they show; true when one changed.
fn renumber(doc: &Document, laid: &SectionLayout, styles: &mut style::Styles<'_>) -> bool {
    let mut changed = false;
    for (list, n) in &laid.note_numbers {
        let Some(note) = furniture::note_of(doc, list) else {
            continue;
        };
        let key = numbers::key(note);
        let shown = styles
            .note_numbers
            .get(&key)
            .copied()
            .unwrap_or(note.number);
        if shown != *n {
            styles.note_numbers.insert(key, *n);
            changed = true;
        }
    }
    changed
}

/// Makes a section's page-local footnote numbers those its notes were
/// drawn with: the number shown instead of the stored one, or the stored
/// one.
fn drawn_numbers(doc: &Document, laid: &mut SectionLayout, styles: &style::Styles<'_>) {
    for (list, n) in &mut laid.note_numbers {
        if let Some(note) = furniture::note_of(doc, list) {
            *n = styles
                .note_numbers
                .get(&numbers::key(note))
                .copied()
                .unwrap_or(note.number);
        }
    }
}

/// Lays out a whole document.
pub fn layout(doc: &Document, fonts: &FontSystem) -> DocLayout {
    layout_with(doc, fonts, Options::default())
}

/// Lays out a whole document with explicit [`Options`].
pub fn layout_with(doc: &Document, fonts: &FontSystem, options: Options) -> DocLayout {
    let mut styles = style::Styles::new(&doc.info, fonts);
    run(doc, &mut styles, options)
}

pub(crate) fn run(doc: &Document, styles: &mut style::Styles<'_>, options: Options) -> DocLayout {
    styles.stored_lines = options.stored_lines;
    styles.note_numbers.clear();
    let mut heads = heads::Heads::default();
    let mut pages = Vec::new();
    let mut sections = Vec::with_capacity(doc.sections.len());
    let mut furniture = furniture::State::default();
    for (i, sec) in doc.sections.iter().enumerate() {
        let first = pages.len();
        let heads_before = heads.clone();
        let mut laid = paginate::section(
            &mut heads,
            doc,
            i,
            sec,
            styles,
            &mut pages,
            furniture.last_number(),
            &[],
        );
        // Notes under all columns take room from columns filled before
        // them: lay the section out again keeping that room from the start.
        let mut known = Vec::new();
        if laid.merged_notes.iter().any(|&h| h > 0) {
            known = std::mem::take(&mut laid.merged_notes);
            pages.truncate(first);
            heads = heads_before.clone();
            laid = paginate::section(
                &mut heads,
                doc,
                i,
                sec,
                styles,
                &mut pages,
                furniture.last_number(),
                &known,
            );
        }
        // Footnotes numbered anew on each page show the number of their
        // mark's page: lay the section out again while a note shows
        // another, as a changed mark can move the lines after it.
        for _ in 0..3 {
            if !renumber(doc, &laid, styles) {
                break;
            }
            pages.truncate(first);
            heads = heads_before.clone();
            laid = paginate::section(
                &mut heads,
                doc,
                i,
                sec,
                styles,
                &mut pages,
                furniture.last_number(),
                &known,
            );
        }
        // Numbers still moving after that report what the last layout drew.
        drawn_numbers(doc, &mut laid, styles);
        furniture::decorate(
            doc,
            i as u32,
            &laid.paras,
            &mut pages[first..],
            styles,
            &mut furniture,
            paginate::default_tab(sec),
        );
        sections.push(laid);
    }
    if styles.total_pages == 0 && numbers::has_total(doc) {
        styles.total_pages = pages.len().max(1) as u32;
        return run(doc, styles, options);
    }
    DocLayout {
        sections,
        pages,
        styles: styles.list().to_vec(),
        substitutions: styles.substituted.clone(),
        fallback_families: styles.fallback_families(),
        requested_faces: styles.requested.clone(),
    }
}
