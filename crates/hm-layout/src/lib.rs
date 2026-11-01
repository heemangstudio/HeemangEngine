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
    /// Paragraphs whose stored lines are usable replayed them
    /// ([`Options::stored_lines`] on, and the document does not ask for its
    /// lines to be composed again); off, every line was broken fresh.
    pub stored_lines: bool,
    /// Names the layout among those a [`LayoutCache`] made, which a later
    /// layout through the same cache can take pages from.
    pub(crate) token: u64,
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
    /// Lay the document out for Hancom's editing screen, which shows the
    /// guide text of unfilled click-here fields; off lays it out for print
    /// and PDF, which leave that text out and close up the line.
    pub guides: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            stored_lines: true,
            guides: false,
        }
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
    run(doc, &mut styles, options, None)
}

/// Whether a later layout can go on from a page of the section: not when
/// it numbers lines or numbers its footnotes anew on each page, which take
/// the whole section into account.
fn resumable(sec: &hm_model::Section) -> bool {
    sec.definition().is_none_or(|d| {
        d.line_numbers.is_none()
            && !d
                .note_shapes()
                .next()
                .is_some_and(|s| (s.attr >> 10) & 3 == 2)
    })
}

pub(crate) fn run(
    doc: &Document,
    styles: &mut style::Styles<'_>,
    options: Options,
    mut reuse: Option<&mut incr::Reuse>,
) -> DocLayout {
    styles.stored_lines = options.stored_lines;
    styles.guides = options.guides;
    styles.note_numbers.clear();
    if let Some(r) = reuse.as_deref_mut() {
        r.begin(doc.sections.len());
    }
    let mut heads = heads::Heads::default();
    let mut pages = Vec::new();
    let mut sections = Vec::with_capacity(doc.sections.len());
    let mut furniture = furniture::State::default();
    for (i, sec) in doc.sections.iter().enumerate() {
        // A section no edit reached, starting as it did, stays as it was.
        if let Some(r) = reuse.as_deref_mut()
            && let Some(laid) = r.whole(
                doc,
                i,
                &mut heads,
                &mut furniture,
                &mut pages,
                styles.total_pages,
            )
        {
            sections.push(laid);
            continue;
        }
        let first = pages.len();
        let heads_before = heads.clone();
        let start = (heads.clone(), furniture.clone(), first, styles.total_pages);
        let resumable = reuse.is_some() && resumable(sec);
        // Otherwise pagination goes on from the last page start the edits
        // leave as it was.
        let mut resumed = match reuse.as_deref_mut() {
            Some(r) if resumable => {
                r.resume(doc, i, &heads, &furniture, &mut pages, styles.total_pages)
            }
            _ => None,
        };
        let mut snaps = resumable.then(|| {
            resumed
                .as_mut()
                .map(|r| std::mem::take(&mut r.snaps))
                .unwrap_or_default()
        });
        let mut resume = resumed.as_mut().map(|r| paginate::Resume {
            snap: &r.snap,
            paras: std::mem::take(&mut r.paras),
            later: &r.later,
            same_from: r.same_from,
            met: None,
        });
        let mut laid = paginate::section(
            &mut heads,
            doc,
            i,
            sec,
            styles,
            &mut pages,
            furniture.last_number(),
            &[],
            resume.as_mut(),
            snaps.as_mut(),
        );
        let met = resume.and_then(|r| r.met);
        // Pagination that met the earlier layout ends as that one did.
        let mut finished = None;
        if let (Some(met), Some(r), Some(reuse), Some(kept)) =
            (met, resumed.as_mut(), reuse.as_deref_mut(), snaps.as_mut())
        {
            finished = reuse.finish(
                doc, i, r, met, first, &mut pages, &mut laid, kept, &mut heads,
            );
            if finished.is_none() {
                // Cannot happen with a consistent record; lay out whole.
                pages.truncate(first);
                heads = heads_before.clone();
                resumed = None;
                laid = paginate::section(
                    &mut heads,
                    doc,
                    i,
                    sec,
                    styles,
                    &mut pages,
                    furniture.last_number(),
                    &[],
                    None,
                    None,
                );
                snaps = None;
            }
        }
        // Notes under all columns take room from columns filled before
        // them: lay the section out again keeping that room from the start.
        let mut known = Vec::new();
        let mut again = false;
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
                None,
                None,
            );
            again = true;
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
                None,
                None,
            );
            again = true;
        }
        // Numbers still moving after that report what the last layout drew.
        drawn_numbers(doc, &mut laid, styles);
        // Pages before the one pagination went on from keep their
        // decoration, and so do the pages from where it met the earlier
        // layout when decoration arrives there in the state it did.
        let finished = finished.filter(|_| !again);
        let mut resumed = resumed.filter(|_| !again);
        let section_end = pages.len() - first;
        let until = finished.as_ref().map_or(section_end, |f| f.0);
        let (from, mut decorations) = match resumed.as_mut() {
            Some(r) => {
                furniture = r.state.clone();
                (r.from, std::mem::take(&mut r.decorations))
            }
            None => (0, Vec::new()),
        };
        let tab = paginate::default_tab(sec);
        furniture::decorate(
            doc,
            i as u32,
            &laid.paras,
            &mut pages[first..],
            styles,
            &mut furniture,
            tab,
            (from, until),
            reuse.is_some().then_some(&mut decorations),
        );
        if let Some((page, earlier)) = finished {
            if earlier.first().is_some_and(|d| d.state.same(&furniture)) {
                furniture = incr::Reuse::ended(resumed.as_ref().expect("finished")).clone();
                decorations.extend(earlier);
            } else {
                for (d, p) in earlier.iter().zip(&mut pages[first + page..]) {
                    d.strip(p);
                }
                furniture::decorate(
                    doc,
                    i as u32,
                    &laid.paras,
                    &mut pages[first..],
                    styles,
                    &mut furniture,
                    tab,
                    (page, section_end),
                    reuse.is_some().then_some(&mut decorations),
                );
            }
        }
        let known_paras = resumed.map_or_else(Vec::new, |r| r.known);
        if let Some(r) = reuse.as_deref_mut() {
            r.keep(
                doc,
                i,
                start,
                known_paras,
                &heads,
                &furniture,
                pages.len() - first,
                snaps.filter(|_| !again),
                decorations,
            );
        }
        sections.push(laid);
    }
    // The total page count shows the pages a first layout gave. Shown only
    // by headers, footers and master pages, it moves no line, so the pages
    // are decorated again with the count instead of laid out again.
    if let Some(body) = numbers::total_in(doc) {
        let count = pages.len().max(1) as u32;
        if styles.total_pages == 0 || (!body && styles.total_pages != count) {
            styles.total_pages = count;
            let redecorated = !body
                && reuse
                    .as_deref_mut()
                    .is_some_and(|r| r.redecorate(doc, styles, &mut pages, &sections));
            if !redecorated {
                if let Some(r) = reuse.as_deref_mut() {
                    r.forget();
                }
                return run(doc, styles, options, reuse);
            }
        }
    }
    DocLayout {
        sections,
        pages,
        token: 0,
        styles: styles.list().to_vec(),
        substitutions: styles.substituted.clone(),
        fallback_families: styles.fallback_families(),
        requested_faces: styles.requested.clone(),
        stored_lines: styles.replay_imported_lines(),
    }
}
