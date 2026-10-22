//! Font catalog, face resolution, SFNT metrics and outlines.
//!
//! The host registers font bytes ([`FontSystem::add`]); this crate never
//! touches the file system, so the same code runs natively over `mmap` and
//! in wasm over JavaScript buffers. Faces parse on first use.

mod face;
pub mod hchar;
pub mod hft;
mod resolve;

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use skrifa::MetadataProvider;
use skrifa::raw::{FileRef, FontRef, TableProvider};
use skrifa::string::StringId;

pub use face::{Face, GlyphInfo, Outline, PathOp};
pub use hchar::CodeTables;
use face::HftFamily;
use hft::HftFile;
pub use resolve::{FaceQuery, Resolved, ScriptClass, classify};

/// Font file bytes shared with the host.
pub type FontData = Arc<dyn AsRef<[u8]> + Send + Sync>;

/// Index of a catalog entry (one face of one file).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FaceId(pub u32);

/// The outline technology of a catalog entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontKind {
    Sfnt,
    Hft,
}

/// What the catalog knows about a face without loading it.
#[derive(Clone, Debug)]
pub struct FaceEntry {
    /// The label the host gave the file (usually its path).
    pub label: String,
    pub kind: FontKind,
    pub index: u32,
    /// Every family name in the `name` table (ids 1 and 16), all languages.
    pub families: Vec<String>,
    pub postscript: Option<String>,
    pub weight: u16,
    pub italic: bool,
    source: usize,
}

impl FaceEntry {
    pub fn bold(&self) -> bool {
        self.weight >= 600
    }
}

/// The catalog plus lazily loaded faces.
#[derive(Default)]
pub struct FontSystem {
    sources: Vec<FontData>,
    entries: Vec<FaceEntry>,
    faces: Vec<OnceLock<Option<Arc<Face>>>>,
    /// Normalized family name to entries.
    by_family: HashMap<String, Vec<u32>>,
    /// Parsed HFT files; an HFT entry lists its members here.
    hft: Vec<(String, Arc<HftFile>)>,
    hft_members: HashMap<u32, Vec<usize>>,
    codes: CodeTables,
    codes_shared: OnceLock<Arc<CodeTables>>,
}

/// Lowercase without spaces: Hancom documents write `맑은 고딕` and `맑은고딕`.
pub(crate) fn normalize(name: &str) -> String {
    name.chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

impl FontSystem {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a font file: TTF, OTF, TTC or HFT. Returns the number of
    /// faces added; an HFT file joins its family's face and adds one only
    /// when it starts a family.
    pub fn add(&mut self, label: impl Into<String>, data: FontData) -> usize {
        let label = label.into();
        let bytes = (*data).as_ref();
        if hft::is_hft(bytes) || label.to_ascii_lowercase().ends_with(".hft") {
            return self.add_hft(label, data);
        }
        let fonts: Vec<FontRef<'_>> = match FileRef::new(bytes) {
            Ok(FileRef::Font(f)) => vec![f],
            Ok(FileRef::Collection(c)) => c.iter().filter_map(Result::ok).collect(),
            Err(_) => return 0,
        };
        let source = self.sources.len();
        let mut added = 0;
        for (index, font) in fonts.iter().enumerate() {
            let mut families = Vec::new();
            for id in [StringId::FAMILY_NAME, StringId::TYPOGRAPHIC_FAMILY_NAME] {
                for s in font.localized_strings(id) {
                    let name: String = s.chars().collect();
                    if !name.is_empty() && !families.contains(&name) {
                        families.push(name);
                    }
                }
            }
            if families.is_empty() {
                continue;
            }
            let postscript = font
                .localized_strings(StringId::POSTSCRIPT_NAME)
                .english_or_first()
                .map(|s| s.chars().collect());
            let attrs = font.attributes();
            let weight = attrs.weight.value().round() as u16;
            let italic = attrs.style != skrifa::attribute::Style::Normal;
            self.push_entry(FaceEntry {
                label: label.clone(),
                kind: FontKind::Sfnt,
                index: index as u32,
                families,
                postscript,
                weight,
                italic,
                source,
            });
            added += 1;
        }
        if added > 0 {
            self.sources.push(data);
        }
        added
    }

    fn push_entry(&mut self, entry: FaceEntry) -> u32 {
        let id = self.entries.len() as u32;
        for f in &entry.families {
            self.by_family.entry(normalize(f)).or_default().push(id);
        }
        self.entries.push(entry);
        self.faces.push(OnceLock::new());
        id
    }

    /// An HFT file covers some script slots of a family; files with the
    /// same family name and slant form one face.
    fn add_hft(&mut self, label: String, data: FontData) -> usize {
        let Ok(file) = HftFile::parse(data) else {
            return 0;
        };
        let (family, italic) = hft_family_name(&file);
        let member = self.hft.len();
        self.hft.push((label.clone(), Arc::new(file)));
        let existing = self.family(&family).iter().copied().find(|&id| {
            let e = &self.entries[id as usize];
            e.kind == FontKind::Hft && e.italic == italic
        });
        if let Some(id) = existing {
            self.hft_members.entry(id).or_default().push(member);
            return 0;
        }
        let id = self.push_entry(FaceEntry {
            label,
            kind: FontKind::Hft,
            index: 0,
            families: vec![family],
            postscript: None,
            weight: 400,
            italic,
            source: usize::MAX,
        });
        self.hft_members.insert(id, vec![member]);
        1
    }

    /// Registers one of Hancom's code conversion tables (`u300_*.dat`), which
    /// HFT faces need for characters other than ASCII and Hangul. Returns
    /// false when the bytes are not such a table.
    pub fn add_code_table(&mut self, bytes: &[u8]) -> bool {
        self.codes.add(bytes)
    }

    pub fn entries(&self) -> &[FaceEntry] {
        &self.entries
    }

    pub fn entry(&self, id: FaceId) -> &FaceEntry {
        &self.entries[id.0 as usize]
    }

    /// HFT files registered so far, with their labels.
    pub fn hft_files(&self) -> &[(String, Arc<HftFile>)] {
        &self.hft
    }

    /// Entries whose family matches `name` (case and space insensitive).
    pub fn family(&self, name: &str) -> &[u32] {
        self.by_family
            .get(&normalize(name))
            .map_or(&[], Vec::as_slice)
    }

    /// Loads (once) and returns a face.
    pub fn face(&self, id: FaceId) -> Option<Arc<Face>> {
        self.faces
            .get(id.0 as usize)?
            .get_or_init(|| {
                let e = &self.entries[id.0 as usize];
                match e.kind {
                    FontKind::Sfnt => Face::load(
                        self.sources[e.source].clone(),
                        e.index,
                        e.families[0].clone(),
                    ),
                    FontKind::Hft => {
                        let mut members: Vec<Arc<HftFile>> = self
                            .hft_members
                            .get(&id.0)
                            .map(|m| m.iter().map(|&i| self.hft[i].1.clone()).collect())
                            .unwrap_or_default();
                        // Script-slot order (the low bits of the style mask:
                        // Hangul, Latin, Hanja, Japanese, Other, Symbol, User).
                        members.sort_by_key(|m| (m.styles & 0xFFFF).trailing_zeros());
                        let codes = self
                            .codes_shared
                            .get_or_init(|| Arc::new(self.codes.clone()))
                            .clone();
                        Face::load_hft(HftFamily { members, codes }, e.families[0].clone())
                    }
                }
                .map(Arc::new)
            })
            .clone()
    }
}

/// An HFT file's family name and slant: names ending in "Italic" or
/// "Oblique" are the slanted face of the family without the suffix.
fn hft_family_name(file: &HftFile) -> (String, bool) {
    for name in [&file.face_name, &file.family] {
        let trimmed = name.trim();
        for suffix in [" Italic", " Oblique"] {
            if let Some(base) = trimmed.strip_suffix(suffix) {
                return (base.trim().to_string(), true);
            }
        }
    }
    let name = if file.face_name.trim().is_empty() {
        &file.family
    } else {
        &file.face_name
    };
    (name.trim().to_string(), false)
}

/// Font-unit metrics of a face.
pub(crate) fn vertical_metrics(font: &FontRef<'_>) -> (i32, i32) {
    if let Ok(os2) = font.os2()
        && (os2.us_win_ascent() != 0 || os2.us_win_descent() != 0)
    {
        return (os2.us_win_ascent() as i32, os2.us_win_descent() as i32);
    }
    if let Ok(hhea) = font.hhea() {
        return (
            hhea.ascender().to_i16() as i32,
            -(hhea.descender().to_i16() as i32),
        );
    }
    let upem = font.head().map(|h| h.units_per_em()).unwrap_or(1000) as i32;
    (upem * 8 / 10, upem * 2 / 10)
}
