//! The document root.

use std::sync::Arc;

use crate::{DocInfo, Paragraph, SharedBytes};

/// Reads a BinData payload (a picture's file bytes, an OLE storage) by its
/// 1-based BinData id, as pictures and fills refer to it.
pub type BinReader = dyn Fn(u16) -> Option<SharedBytes> + Send + Sync;

/// Reads what an OLE storage shows, by BinData id.
pub type OleReader = dyn Fn(u16) -> Option<Arc<crate::chart::OleView>> + Send + Sync;

/// The document's embedded binary data, read on demand from the container.
#[derive(Clone)]
pub struct BinStore {
    read: Arc<BinReader>,
    ole: Option<Arc<OleReader>>,
}

impl BinStore {
    pub fn new(read: impl Fn(u16) -> Option<SharedBytes> + Send + Sync + 'static) -> Self {
        Self {
            read: Arc::new(read),
            ole: None,
        }
    }

    /// Adds the reader of what OLE objects show.
    pub fn with_ole(
        mut self,
        ole: impl Fn(u16) -> Option<Arc<crate::chart::OleView>> + Send + Sync + 'static,
    ) -> Self {
        self.ole = Some(Arc::new(ole));
        self
    }

    pub fn get(&self, id: u16) -> Option<SharedBytes> {
        (self.read)(id)
    }

    /// What an OLE storage shows: the picture its application saved for
    /// display, or the chart its data describes.
    pub fn ole(&self, id: u16) -> Option<Arc<crate::chart::OleView>> {
        self.ole.as_ref().and_then(|p| p(id))
    }
}

impl std::fmt::Debug for BinStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BinStore")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Format {
    Hwp5,
    Hwpx,
    /// Single-file HWPML 2.x XML.
    Hml,
    /// Hangul 97 files, read into the model the way Hancom converts them
    /// to HWP 5.
    Hwp3,
}

/// A format version `major.minor.micro.build` (HWP 5 stores it as one DWORD
/// `0xMMnnPPbb`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Version(pub u32);

impl Version {
    pub fn parts(self) -> [u8; 4] {
        self.0.to_be_bytes()
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let [a, b, c, d] = self.parts();
        write!(f, "{a}.{b}.{c}.{d}")
    }
}

/// File-level facts.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DocMeta {
    pub format: Format,
    pub version: Version,
    /// HWP 5 FileHeader property bits: 0 compressed, 1 password, 2 distribution, ...
    pub flags: u32,
}

impl DocMeta {
    pub fn compressed(&self) -> bool {
        self.flags & 1 != 0
    }
    pub fn password(&self) -> bool {
        self.flags & 2 != 0
    }
    pub fn distribution(&self) -> bool {
        self.flags & 4 != 0
    }
    /// The body Hancom opens is the protected `ViewText`: a distribution
    /// document, or a document with bit 14, which Hancom cannot open without
    /// that stream (its `BodyText` then has no stored lines).
    pub fn keeps_body_in_view_text(&self) -> bool {
        self.flags & (4 | 1 << 14) != 0
    }
}

/// A section: a body paragraph list with its own page setup (carried by the
/// section definition control in its first paragraph).
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Section {
    pub paras: Vec<Paragraph>,
    /// The decompressed stream as read; `None` once the paragraph list
    /// itself changed (paragraphs inserted, removed or reordered).
    #[cfg_attr(feature = "serde", serde(skip))]
    pub src: Option<SharedBytes>,
    /// Index of the source stream (`BodyText/Section{n}`), so an untouched
    /// section can be copied as stored.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub source_index: Option<u32>,
    /// The master page of the section's last page, which replaces the
    /// even, odd or both master page there. Readers fill it from the
    /// records or markup that keep it; writers encode the typed list.
    pub last_page_master: Option<crate::ParaList>,
    /// Master pages for individual, 1-based pages of the section.
    pub page_masters: Vec<(u32, crate::ParaList)>,
}

impl Section {
    pub fn is_clean(&self) -> bool {
        self.src.is_some() && self.paras.iter().all(Paragraph::is_clean)
    }

    /// The first section definition in the section's first paragraph.
    pub fn definition(&self) -> Option<&crate::SectionDef> {
        self.paras
            .first()?
            .controls
            .iter()
            .find_map(|c| c.as_section())
    }
}

/// Something worth reporting about the input that did not stop reading.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Diagnostic {
    pub message: String,
}

#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Document {
    pub meta: DocMeta,
    pub info: DocInfo,
    pub sections: Vec<Section>,
    /// The original file, so writers can copy container parts the model
    /// does not hold (BinData, previews, summary, scripts).
    #[cfg_attr(feature = "serde", serde(skip))]
    pub source: Option<SharedBytes>,
    /// Embedded pictures and storages, when the container provides them.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub bins: Option<BinStore>,
    pub diagnostics: Vec<Diagnostic>,
}

impl Document {
    /// True when nothing changed since reading.
    pub fn is_clean(&self) -> bool {
        self.source.is_some() && self.info.is_clean() && self.sections.iter().all(Section::is_clean)
    }

    /// Gives back the room the readers' vectors grew beyond what they hold:
    /// paragraph lists, cells, controls and each paragraph's text, runs,
    /// lines and records. Nothing they hold changes.
    pub fn shrink_to_fit(&mut self) {
        self.sections.shrink_to_fit();
        self.for_each_list_mut(&mut |paras| {
            paras.shrink_to_fit();
            for p in paras {
                p.text.shrink_to_fit();
                p.runs.shrink_to_fit();
                p.line_segs.shrink_to_fit();
                p.range_tags.shrink_to_fit();
                p.controls.shrink_to_fit();
                p.extra.shrink_to_fit();
                p.tracks.shrink_to_fit();
                for c in &mut p.controls {
                    if let crate::ControlKind::Table(t) = &mut c.kind {
                        t.cells.shrink_to_fit();
                    }
                }
            }
        });
    }

    /// Calls `f` with every paragraph list of the document, each before the
    /// lists inside its paragraphs: section bodies and master pages, then
    /// cells, captions, text boxes, headers, footers and notes.
    pub fn for_each_list_mut(&mut self, f: &mut dyn FnMut(&mut Vec<Paragraph>)) {
        for s in &mut self.sections {
            each_list(&mut s.paras, f);
            for l in s
                .last_page_master
                .iter_mut()
                .chain(s.page_masters.iter_mut().map(|(_, l)| l))
            {
                each_list(&mut l.paras, f);
            }
        }
    }

    /// Drops every source span so a save re-encodes all records.
    pub fn mark_all_dirty(&mut self) {
        self.info.mark_all_dirty();
        for s in &mut self.sections {
            s.src = None;
            s.paras.iter_mut().for_each(Paragraph::mark_all_dirty);
        }
    }
}

fn each_list(paras: &mut Vec<Paragraph>, f: &mut dyn FnMut(&mut Vec<Paragraph>)) {
    use crate::{ControlKind, ObjectPart, SectionChild};
    f(paras);
    for p in paras {
        for c in &mut p.controls {
            match &mut c.kind {
                ControlKind::Table(t) => {
                    for cell in &mut t.cells {
                        each_list(&mut cell.paras, f);
                    }
                    if let Some(cap) = &mut t.caption {
                        each_list(&mut cap.paras, f);
                    }
                }
                ControlKind::Object(o) => {
                    for part in &mut o.parts {
                        match part {
                            ObjectPart::Caption(cap) => each_list(&mut cap.paras, f),
                            ObjectPart::Shape(s) => each_shape_list(s, f),
                            _ => {}
                        }
                    }
                }
                ControlKind::HeaderFooter(h) => each_list(&mut h.list.paras, f),
                ControlKind::Note(n) => each_list(&mut n.list.paras, f),
                ControlKind::Section(s) => {
                    for child in &mut s.children {
                        if let SectionChild::MasterPage(l) = child {
                            each_list(&mut l.paras, f);
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

fn each_shape_list(s: &mut crate::Shape, f: &mut dyn FnMut(&mut Vec<Paragraph>)) {
    for part in &mut s.parts {
        match part {
            crate::ShapePart::Text(t) => each_list(&mut t.paras, f),
            crate::ShapePart::Child(c) => each_shape_list(c, f),
            _ => {}
        }
    }
}
