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
pub use furniture::{CellStep, Frame, FrameKind, FrameLine, ListRef, Mark, PageBorder, Rule};
pub use measure::{InlineBox, Kind, Measured};
pub use paginate::{Page, PlacedLine, Rect, SectionLayout};
pub use para::{HeadLayout, Line, ParaLayout, Placed, place};
pub use style::Style;
pub use table::{CaptionLayout, CellLayout, CellLine, PlacedTable, TableLayout};

/// A laid-out document.
#[derive(Clone, Debug, Default)]
pub struct DocLayout {
    pub sections: Vec<SectionLayout>,
    pub pages: Vec<Page>,
    /// Styles referenced by [`Measured::styles`].
    pub styles: Vec<Style>,
    /// Face names that resolved to another family: `(requested, used)`.
    pub substitutions: Vec<(String, String)>,
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

/// Lays out a whole document.
pub fn layout(doc: &Document, fonts: &FontSystem) -> DocLayout {
    layout_with(doc, fonts, Options::default())
}

/// Lays out a whole document with explicit [`Options`].
pub fn layout_with(doc: &Document, fonts: &FontSystem, options: Options) -> DocLayout {
    let mut styles = style::Styles::new(&doc.info, fonts);
    styles.stored_lines = options.stored_lines;
    let mut heads = heads::Heads::default();
    let mut pages = Vec::new();
    let mut sections = Vec::with_capacity(doc.sections.len());
    let mut furniture = furniture::State::default();
    for (i, sec) in doc.sections.iter().enumerate() {
        let first = pages.len();
        let laid = paginate::section(
            &mut heads,
            doc,
            i,
            sec,
            &mut styles,
            &mut pages,
            furniture.last_number(),
        );
        furniture::decorate(
            doc,
            i as u32,
            &laid.paras,
            &mut pages[first..],
            &mut styles,
            &mut furniture,
            paginate::default_tab(sec),
        );
        sections.push(laid);
    }
    DocLayout {
        sections,
        pages,
        styles: styles.list().to_vec(),
        substitutions: styles.substituted.clone(),
    }
}
