//! Font catalog, face resolution, SFNT metrics and outlines.
//!
//! The host registers font bytes ([`FontSystem::add`]); this crate never
//! touches the file system, so the same code runs natively over `mmap` and
//! in wasm over JavaScript buffers. Faces parse on first use. Hosts short on
//! memory register compact copies ([`FontSystem::add_compact`]).

mod compact;
mod face;
mod fontmap;
pub mod hchar;
pub mod hft;
mod resolve;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use skrifa::MetadataProvider;
use skrifa::raw::{FileRef, FontRef, TableProvider};
use skrifa::string::StringId;

pub use compact::FontSource;
use face::HftFamily;
pub use face::{Face, GlyphInfo, Outline, PathOp};
pub use hchar::CodeTables;
use hft::HftFile;
pub use resolve::{FaceQuery, Resolved, ScriptClass, class_defaults, classify};

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
    /// The family name Hancom for Mac lists the face under: its Korean
    /// family name when it has one, else its English one.
    pub local: String,
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
    font_map: fontmap::FontMap,
    /// Bold and italic of HFT files by upper-case file name, from Hancom's
    /// HFT catalog.
    hft_styles: HashMap<String, (bool, bool)>,
    /// Sources that are compact copies, by source index.
    compact: HashMap<usize, Compact>,
    /// A layout is running: outlines it misses call for laying out again.
    in_layout: Arc<AtomicBool>,
}

/// A compact copy of one face of a registered file.
struct Compact {
    label: String,
    /// The face's index in the file the host registered.
    index: u32,
    residency: Arc<compact::Residency>,
}

/// What [`FontSystem::add_compact`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Added {
    /// Faces new to the catalog.
    pub faces: usize,
    /// The file was registered before; its copies now hold the outlines
    /// asked for since.
    pub refilled: bool,
    /// A layout measured an outline the copy lacked, so laying out again
    /// can change the result.
    pub relayout: bool,
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
            // Windows records first: Mac Korean records use an encoding
            // the name table reader does not decode.
            let names = || font.localized_strings(StringId::FAMILY_NAME);
            let local = names()
                .find(|s| s.language() == Some("ko-KR"))
                .or_else(|| names().english_or_first())
                .map_or_else(|| families[0].clone(), |s| s.chars().collect());
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
                local,
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

    /// Registers a font file like [`FontSystem::add`], but keeps for each
    /// TrueType face only its metric tables and the outlines of `chars`,
    /// reading just those parts of the file. Other outlines come empty
    /// until the host registers the same label again
    /// ([`FontSystem::outline_requests`]), which refills the copies with
    /// every glyph asked for since. An HFT file is read whole once and kept
    /// the same way; CFF files are kept whole.
    pub fn add_compact<S: FontSource + ?Sized>(
        &mut self,
        label: impl Into<String>,
        src: &S,
        chars: &[char],
    ) -> Added {
        let label = label.into();
        if src.read(0, 25).is_some_and(|b| hft::is_hft(&b)) {
            return self.add_compact_hft(label, src, chars);
        }
        let mut mine: Vec<usize> = self
            .compact
            .iter()
            .filter(|(_, c)| c.label == label)
            .map(|(&s, _)| s)
            .collect();
        if !mine.is_empty() {
            mine.sort_unstable();
            let mut added = Added {
                refilled: true,
                ..Added::default()
            };
            for s in mine {
                let c = &self.compact[&s];
                added.relayout |= c.residency.missed_in_layout();
                let mut glyphs: Vec<u16> = c.residency.glyphs().collect();
                glyphs.extend(c.residency.wanted());
                let index = c.index;
                let Some((data, residency)) = compact::build(src, index, chars, &glyphs) else {
                    continue;
                };
                self.sources[s] = Arc::new(data);
                self.compact.insert(
                    s,
                    Compact {
                        label: label.clone(),
                        index,
                        residency: Arc::new(residency),
                    },
                );
                for (id, e) in self.entries.iter().enumerate() {
                    if e.source == s {
                        self.faces[id] = OnceLock::new();
                    }
                }
            }
            return added;
        }
        let whole = |fs: &mut FontSystem| {
            let faces = src
                .read(0, src.size())
                .map_or(0, |b| fs.add(label.clone(), Arc::new(b)));
            Added {
                faces,
                ..Added::default()
            }
        };
        if !compact::is_truetype(src) {
            return whole(self);
        }
        let copies: Option<Vec<_>> = (0..compact::face_count(src).unwrap_or(0))
            .map(|index| compact::build(src, index, chars, &[]).map(|c| (index, c)))
            .collect();
        let Some(copies) = copies.filter(|c| !c.is_empty()) else {
            return whole(self);
        };
        let mut faces = 0;
        for (index, (data, residency)) in copies {
            let source = self.sources.len();
            let n = self.add(label.clone(), Arc::new(data));
            if n > 0 {
                self.compact.insert(
                    source,
                    Compact {
                        label: label.clone(),
                        index,
                        residency: Arc::new(residency),
                    },
                );
            }
            faces += n;
        }
        Added {
            faces,
            ..Added::default()
        }
    }

    /// Unregisters the files registered under `label`: their faces leave
    /// the catalog (their ids stay, resolving to nothing) and an HFT family
    /// keeps its other files. Returns the faces removed.
    pub fn remove(&mut self, label: &str) -> usize {
        let mut removed = 0;
        let members: Vec<usize> = (0..self.hft.len())
            .filter(|&m| self.hft[m].0 == label)
            .collect();
        for id in 0..self.entries.len() {
            let e = &self.entries[id];
            let dead = match e.kind {
                FontKind::Sfnt => e.label == label && e.source != usize::MAX,
                FontKind::Hft => {
                    let list = self.hft_members.entry(id as u32).or_default();
                    let before = list.len();
                    list.retain(|m| !members.contains(m));
                    if list.len() != before {
                        self.faces[id] = OnceLock::new();
                    }
                    before > 0 && list.is_empty()
                }
            };
            if !dead {
                continue;
            }
            let source = e.source;
            for ids in self.by_family.values_mut() {
                ids.retain(|&i| i as usize != id);
            }
            if source != usize::MAX {
                self.sources[source] = Arc::new(Vec::<u8>::new());
                self.compact.remove(&source);
            }
            self.entries[id].source = usize::MAX;
            self.faces[id] = OnceLock::from(None);
            removed += 1;
        }
        self.by_family.retain(|_, ids| !ids.is_empty());
        for m in members {
            self.hft[m].0.clear();
            self.hft[m].1 = Arc::new(HftFile::empty());
        }
        removed
    }

    /// Labels of compact files a request found an outline missing in:
    /// register each again to fill those outlines in.
    pub fn outline_requests(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for c in self.compact.values() {
            if !c.residency.wanted().is_empty() && !out.contains(&c.label) {
                out.push(c.label.clone());
            }
        }
        for (label, file) in &self.hft {
            if file.residency().is_some_and(|r| r.has_wanted()) && !out.contains(label) {
                out.push(label.clone());
            }
        }
        out.sort();
        out
    }

    /// Marks a layout as running, so outlines it finds missing ask for a
    /// new layout once refilled.
    pub fn set_layout_running(&self, running: bool) {
        self.in_layout.store(running, Ordering::Relaxed);
    }

    /// Bytes of font data the catalog holds.
    pub fn resident_bytes(&self) -> usize {
        let sources: usize = self.sources.iter().map(|d| (**d).as_ref().len()).sum();
        let hft: usize = self.hft.iter().map(|h| h.1.size()).sum();
        let bits: usize = self.compact.values().map(|c| c.residency.size()).sum();
        sources + hft + bits
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

    /// [`FontSystem::add_compact`] for an HFT file: the file is read whole
    /// once and kept as a copy with the outlines of `chars` only; adding the
    /// same label again rebuilds the copy with the outlines asked for since.
    fn add_compact_hft<S: FontSource + ?Sized>(
        &mut self,
        label: String,
        src: &S,
        chars: &[char],
    ) -> Added {
        let Some(file) = src
            .read(0, src.size())
            .and_then(|b| HftFile::parse(Arc::new(b)).ok())
        else {
            return Added::default();
        };
        let codes: Vec<u16> = chars.iter().filter_map(|&c| self.codes.hchar(c)).collect();
        let mine: Vec<usize> = (0..self.hft.len())
            .filter(|&m| self.hft[m].0 == label && self.hft[m].1.residency().is_some())
            .collect();
        if mine.is_empty() {
            let copy = file.compact(&codes, &[], self.in_layout.clone());
            return Added {
                faces: self.add_hft_file(label, copy),
                ..Added::default()
            };
        }
        let mut added = Added {
            refilled: true,
            ..Added::default()
        };
        for m in mine {
            let Some(r) = self.hft[m].1.residency().cloned() else {
                continue;
            };
            added.relayout |= r.missed_in_layout();
            self.hft[m].1 = Arc::new(file.compact(&codes, &r.all(), self.in_layout.clone()));
            for (&id, members) in &self.hft_members {
                if members.contains(&m) {
                    self.faces[id as usize] = OnceLock::new();
                }
            }
        }
        added
    }

    /// An HFT file covers some script slots of a family; files with the
    /// same family name and slant form one face.
    fn add_hft(&mut self, label: String, data: FontData) -> usize {
        let Ok(file) = HftFile::parse(data) else {
            return 0;
        };
        self.add_hft_file(label, file)
    }

    fn add_hft_file(&mut self, label: String, file: HftFile) -> usize {
        let (family, mut bold, mut italic) = hft_family_name(&file);
        let file_name = label
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or("")
            .to_ascii_uppercase();
        if let Some(&(b, i)) = self.hft_styles.get(&file_name) {
            (bold, italic) = (b, i);
        }
        let weight = if bold { 700 } else { 400 };
        let member = self.hft.len();
        self.hft.push((label.clone(), Arc::new(file)));
        let existing = self.family(&family).iter().copied().find(|&id| {
            let e = &self.entries[id as usize];
            e.kind == FontKind::Hft && e.italic == italic && e.weight == weight
        });
        if let Some(id) = existing {
            self.hft_members.entry(id).or_default().push(member);
            return 0;
        }
        let id = self.push_entry(FaceEntry {
            label,
            kind: FontKind::Hft,
            index: 0,
            local: family.clone(),
            families: vec![family],
            postscript: None,
            weight,
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

    /// Registers Hancom's HFT catalog (`hftinfo.dat`), which tells the bold
    /// and italic files of a family apart where the files themselves do not.
    /// Register it before the HFT files. Returns false when the bytes list
    /// no HFT file.
    pub fn add_hft_info(&mut self, bytes: &[u8]) -> bool {
        let styles = fontmap::hft_styles(bytes);
        let any = !styles.is_empty();
        for (file, bold, italic) in styles {
            self.hft_styles.insert(file, (bold, italic));
        }
        any
    }

    /// Registers Hancom's font mapping table (`FontMap.dat`), which names
    /// the faces that stand in for a face. Returns false when the bytes hold
    /// no mapping.
    pub fn add_font_map(&mut self, bytes: &[u8]) -> bool {
        self.font_map.add(bytes)
    }

    /// The first face standing in for face name `name` in the font map that
    /// has a glyph for `ch`.
    pub fn mapped_fallback(&self, name: &str, ch: char) -> Option<FaceId> {
        self.font_map
            .stand_ins(name)
            .iter()
            .find_map(|n| self.family_glyph(n, ch))
    }

    /// A face of family `name` that has a glyph for `ch`.
    pub fn family_glyph(&self, name: &str, ch: char) -> Option<FaceId> {
        self.family(name)
            .iter()
            .map(|&id| FaceId(id))
            .find(|&id| self.face(id).is_some_and(|f| f.has(ch)))
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

    /// Whether text set in face name `name` draws with a registered face:
    /// one of that family, or of a stand-in the font map gives it.
    pub fn resolves(&self, name: &str) -> bool {
        !self.family(name).is_empty()
            || self
                .font_map
                .stand_ins(name)
                .iter()
                .any(|n| !self.family(n).is_empty())
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
                        self.compact
                            .get(&e.source)
                            .map(|c| (c.residency.clone(), self.in_layout.clone())),
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

/// An HFT file's family name, weight and slant: a face name ending in
/// "Bold", "Italic", "Oblique" or "Bold Italic" is that style of the family
/// without the suffix.
fn hft_family_name(file: &HftFile) -> (String, bool, bool) {
    // Style words end the name: `HCI Poppy Bold Italic`, `HCI Acacia Demi
    // Oblique`, `HCI Bellflower Light`.
    fn strip(name: &str) -> (&str, bool, bool) {
        let mut base = name;
        let (mut bold, mut italic) = (false, false);
        loop {
            if let Some(b) = base
                .strip_suffix(" Italic")
                .or(base.strip_suffix(" Oblique"))
                .or(base.strip_suffix(" Obliqu"))
            {
                italic = true;
                base = b.trim_end();
            } else if let Some(b) = base.strip_suffix(" Bold").or(base.strip_suffix(" Demi")) {
                bold = true;
                base = b.trim_end();
            } else if let Some(b) = base.strip_suffix(" Light") {
                base = b.trim_end();
            } else {
                break;
            }
        }
        (base, bold, italic)
    }
    let face = file.face_name.trim();
    let (_, family_bold, family_italic) = strip(file.family.trim());
    let (base, bold, italic) = strip(if face.is_empty() {
        file.family.trim()
    } else {
        face
    });
    (
        base.to_string(),
        bold || family_bold,
        italic || family_italic,
    )
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
