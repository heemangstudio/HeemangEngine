//! The formatter: style resolution, measurement, line breaking, line
//! metrics, paragraph placement, columns and pages, and the audit against
//! Hancom's stored line segments.
//!
//! All values are integer HWPUNIT. Our own arithmetic decides every line;
//! stored line segments are only compared ([`audit`]).

pub mod audit;
pub mod heads;
pub mod measure;
pub mod objects;
pub mod paginate;
pub mod para;
pub mod script;
pub mod style;
pub mod table;

use hm_fonts::FontSystem;
use hm_model::Document;

pub use audit::{Audit, audit};
pub use measure::{InlineBox, Kind, Measured};
pub use paginate::{Page, PlacedLine, Rect, SectionLayout};
pub use para::{HeadLayout, Line, ParaLayout, Placed, place};
pub use style::Style;
pub use table::{CellLayout, CellLine, PlacedTable, TableLayout};

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

/// Lays out a whole document.
pub fn layout(doc: &Document, fonts: &FontSystem) -> DocLayout {
    let mut styles = style::Styles::new(&doc.info, fonts);
    let mut heads = heads::Heads::default();
    let mut pages = Vec::new();
    let mut sections = Vec::with_capacity(doc.sections.len());
    for (i, sec) in doc.sections.iter().enumerate() {
        sections.push(paginate::section(
            &mut heads,
            doc,
            i,
            sec,
            &mut styles,
            &mut pages,
        ));
    }
    DocLayout {
        sections,
        pages,
        styles: styles.list().to_vec(),
        substitutions: styles.substituted.clone(),
    }
}
