//! The document root.

use std::sync::Arc;

use crate::{DocInfo, Paragraph, SharedBytes};

/// Reads a BinData payload (a picture's file bytes, an OLE storage) by its
/// 1-based BinData id, as pictures and fills refer to it.
pub type BinReader = dyn Fn(u16) -> Option<SharedBytes> + Send + Sync;

/// The document's embedded binary data, read on demand from the container.
#[derive(Clone)]
pub struct BinStore {
    read: Arc<BinReader>,
    previews: Option<Arc<BinReader>>,
}

impl BinStore {
    pub fn new(read: impl Fn(u16) -> Option<SharedBytes> + Send + Sync + 'static) -> Self {
        Self {
            read: Arc::new(read),
            previews: None,
        }
    }

    /// Adds the reader of OLE objects' display pictures.
    pub fn with_previews(
        mut self,
        previews: impl Fn(u16) -> Option<SharedBytes> + Send + Sync + 'static,
    ) -> Self {
        self.previews = Some(Arc::new(previews));
        self
    }

    pub fn get(&self, id: u16) -> Option<SharedBytes> {
        (self.read)(id)
    }

    /// The picture an OLE storage carries for display, a Windows metafile
    /// or a device-independent bitmap, as its application saved it.
    pub fn preview(&self, id: u16) -> Option<SharedBytes> {
        self.previews.as_ref().and_then(|p| p(id))
    }
}

impl std::fmt::Debug for BinStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BinStore")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Hwp5,
    Hwpx,
}

/// A format version `major.minor.micro.build` (HWP 5 stores it as one DWORD
/// `0xMMnnPPbb`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
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
}

/// A section: a body paragraph list with its own page setup (carried by the
/// section definition control in its first paragraph).
#[derive(Clone, Debug, Default)]
pub struct Section {
    pub paras: Vec<Paragraph>,
    /// The decompressed stream as read; `None` once the paragraph list
    /// itself changed (paragraphs inserted, removed or reordered).
    pub src: Option<SharedBytes>,
    /// Index of the source stream (`BodyText/Section{n}`), so an untouched
    /// section can be copied as stored.
    pub source_index: Option<u32>,
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
pub struct Diagnostic {
    pub message: String,
}

#[derive(Clone, Debug)]
pub struct Document {
    pub meta: DocMeta,
    pub info: DocInfo,
    pub sections: Vec<Section>,
    /// The original file, so writers can copy container parts the model
    /// does not hold (BinData, previews, summary, scripts).
    pub source: Option<SharedBytes>,
    /// Embedded pictures and storages, when the container provides them.
    pub bins: Option<BinStore>,
    pub diagnostics: Vec<Diagnostic>,
}

impl Document {
    /// True when nothing changed since reading.
    pub fn is_clean(&self) -> bool {
        self.source.is_some() && self.info.is_clean() && self.sections.iter().all(Section::is_clean)
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
