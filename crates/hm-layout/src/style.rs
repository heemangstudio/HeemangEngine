//! Resolving character shapes into measured styles.

use std::collections::HashMap;
use std::sync::Arc;

use hm_fonts::{Face, FaceId, FaceQuery, FontSystem, ScriptClass, classify};
use hm_model::units::mul_div;
use hm_model::{CharShape, DocInfo, Script};

/// What measuring and drawing a run needs, resolved once per
/// `(char shape, script, face)`.
#[derive(Clone, Debug)]
pub struct Style {
    pub char_shape: u32,
    pub script: Script,
    pub face: Option<FaceId>,
    /// The face is the one the document names.
    pub exact: bool,
    /// Logical glyph size in HU: base size times relative size, times 16/25
    /// for superscript and subscript.
    pub size: i32,
    /// Size for line metrics: the base size.
    pub line_size: i32,
    /// Width ratio percent.
    pub ratio: i32,
    /// Character spacing percent.
    pub spacing: i32,
    /// Vertical offset in HU, positive down.
    pub offset: i32,
    pub color: u32,
    /// Bold requested but the face is regular.
    pub fake_bold: bool,
    /// Italic requested but the resolved face has upright outlines.
    pub italic: bool,
    /// 0 none, 1 below, 3 above.
    pub underline: u8,
    pub underline_color: u32,
    pub strike: bool,
    /// Background shade color, when set (COLORREF).
    pub shade: Option<u32>,
    pub units_per_em: u16,
}

#[derive(Clone, Copy)]
struct Primary {
    face: Option<FaceId>,
    exact: bool,
    fake_bold: bool,
}

/// The face Hancom draws a non-Latin character an HFT face lacks with.
const HFT_FALLBACK: &str = "Haansoft Dotum";

/// How a style draws its char shape's glyphs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Variant {
    Plain,
    /// As superscript whatever the char shape says.
    Superscript,
    /// As a note's reference mark.
    NoteMark,
}

/// The char shape id of styles made by [`Styles::builtin_style`]; no
/// document char shape has it.
pub const BUILTIN: u32 = u32::MAX;

/// The style table and face resolutions of a layout, kept for the next
/// layout of the same document.
#[derive(Default)]
pub struct StyleCache {
    list: Vec<Style>,
    faces: Vec<Option<Arc<Face>>>,
    by_key: HashMap<(u32, Script, Option<FaceId>, Variant), u32>,
    primary: HashMap<(u32, Script), Primary>,
    fallback: HashMap<(char, bool), Option<FaceId>>,
    mapped: HashMap<(String, char), Option<FaceId>>,
    substituted: Vec<(String, String)>,
    misses: Vec<Miss>,
    requested: Vec<String>,
}

/// A face that lacked a character: its name, and whether the character is
/// Latin, the face HFT and the name's class serif, which pick the chain of
/// families tried for the character; and the face that had it.
type Miss = (String, bool, bool, bool, Option<FaceId>);

/// The style table of one layout, built on demand.
pub struct Styles<'a> {
    info: &'a DocInfo,
    fonts: &'a FontSystem,
    list: Vec<Style>,
    faces: Vec<Option<Arc<Face>>>,
    by_key: HashMap<(u32, Script, Option<FaceId>, Variant), u32>,
    primary: HashMap<(u32, Script), Primary>,
    fallback: HashMap<(char, bool), Option<FaceId>>,
    /// Font-map stand-ins by face name and character.
    mapped: HashMap<(String, char), Option<FaceId>>,
    /// Requested face names that did not resolve exactly: `(requested, used)`.
    pub substituted: Vec<(String, String)>,
    misses: Vec<Miss>,
    /// Face names the layout asked for, found or not.
    pub requested: Vec<String>,
    /// The page being laid out, for automatic page numbers; 0 when not
    /// known, which shows the number Hancom stored.
    pub page_number: u32,
    pub total_pages: u32,
    /// The current section's outline numbering definition for nested lists.
    pub(crate) outline_numbering: u16,
    /// The body paragraph (by address) whose own automatic page numbers
    /// take the width of the number Hancom stored: Hancom breaks such a
    /// line with the stored number and fits the shown one into its room.
    pub(crate) stored_number_room: usize,
    /// The body paragraph (by address) that snaps to the section's
    /// character grid, and the grid.
    pub(crate) char_grid: (usize, CharGrid),
    /// Paragraph layouts kept from earlier layouts of the same document.
    pub(crate) memo: Option<crate::incr::Memo>,
    /// Replay the line membership Hancom stored for unedited paragraphs
    /// (on by default); off lays out every paragraph fresh.
    pub stored_lines: bool,
}

/// A section's character grid: each character takes whole cells of
/// `pitch` HU, centred in them. On manuscript paper (`manuscript`) a
/// pair of narrow lowercase letters or digits shares a cell.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct CharGrid {
    pub pitch: i32,
    pub manuscript: bool,
}

/// Effective glyph size of a slot (HU): base size times relative size,
/// times 16/25 for superscript and subscript.
pub fn glyph_size(cs: &CharShape, script: Script) -> i32 {
    let base = line_size(cs);
    let rel = cs.rel_size(script);
    let size = if rel == 100 {
        base
    } else {
        mul_div(base, rel, 100)
    };
    if cs.superscript() || cs.subscript() {
        mul_div(size, 16, 25)
    } else {
        size
    }
}

/// Vertical offset of a slot's glyphs from the line's baseline (HU,
/// positive down): the relative position, plus Hancom's superscript raise
/// of 44 percent or subscript drop of 12 percent of the base size.
pub fn position_offset(cs: &CharShape, script: Script) -> i32 {
    let base = line_size(cs);
    let mut y = mul_div(cs.base_size, cs.offset(script), 100);
    if cs.superscript() {
        y -= mul_div(base, 44, 100);
    } else if cs.subscript() {
        y += mul_div(base, 12, 100);
    }
    y
}

/// Size a run contributes to line metrics (HU): the base size. Relative
/// size changes glyphs, not the paragraph's line metrics.
pub fn line_size(cs: &CharShape) -> i32 {
    if cs.base_size > 0 { cs.base_size } else { 1000 }
}

impl<'a> Styles<'a> {
    /// The source authority is immutable for this layout's DocInfo. An
    /// explicit fresh request always wins. Use the same effective value
    /// for formatting and the generic paragraph memo key.
    pub(crate) fn replay_imported_lines(&self) -> bool {
        self.stored_lines
            && self.info.imported_line_membership != hm_model::ImportedLineMembership::Recompose
    }

    /// The character grid `para` snaps to; pitch 0 for none.
    pub(crate) fn char_grid_of(&self, para: &hm_model::Paragraph) -> CharGrid {
        if std::ptr::addr_eq(para, self.char_grid.0 as *const hm_model::Paragraph) {
            self.char_grid.1
        } else {
            CharGrid::default()
        }
    }

    pub fn new(info: &'a DocInfo, fonts: &'a FontSystem) -> Self {
        Self {
            info,
            fonts,
            list: Vec::new(),
            faces: Vec::new(),
            by_key: HashMap::new(),
            primary: HashMap::new(),
            fallback: HashMap::new(),
            mapped: HashMap::new(),
            substituted: Vec::new(),
            misses: Vec::new(),
            requested: Vec::new(),
            page_number: 0,
            total_pages: 0,
            outline_numbering: 0,
            stored_number_room: 0,
            char_grid: (0, CharGrid::default()),
            stored_lines: true,
            memo: None,
        }
    }

    /// Styles continuing a previous layout's table, so its style indices
    /// stay valid and its face resolution is not repeated.
    pub fn resume(info: &'a DocInfo, fonts: &'a FontSystem, kept: StyleCache) -> Self {
        let mut s = Self::new(info, fonts);
        s.list = kept.list;
        s.faces = kept.faces;
        s.by_key = kept.by_key;
        s.primary = kept.primary;
        s.fallback = kept.fallback;
        s.mapped = kept.mapped;
        s.substituted = kept.substituted;
        s.misses = kept.misses;
        s.requested = kept.requested;
        s
    }

    /// What a later layout of the same document can continue from.
    pub fn keep(self) -> StyleCache {
        StyleCache {
            list: self.list,
            faces: self.faces,
            by_key: self.by_key,
            primary: self.primary,
            fallback: self.fallback,
            mapped: self.mapped,
            substituted: self.substituted,
            misses: self.misses,
            requested: self.requested,
        }
    }

    /// Families the layout looked in, in order, for characters a face
    /// lacked, up to the one that had them, whether or not they are
    /// registered: a host registering fonts on demand provides the ones it
    /// has.
    pub fn fallback_families(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let [defaults, serif, sans] = hm_fonts::class_defaults();
        for (name, latin, hft, is_serif, found) in &self.misses {
            let (aliases, stand_ins) = self.fonts.face_candidates(name, &[]);
            let mut chain: Vec<&str> = Vec::new();
            if *latin {
                chain.extend(stand_ins.iter().map(String::as_str));
            } else if *hft {
                chain.extend(aliases.iter().skip(1).map(String::as_str));
                chain.push(HFT_FALLBACK);
            }
            let (first, second) = if *is_serif {
                (serif, sans)
            } else {
                (sans, serif)
            };
            chain.extend(defaults.iter().chain(first).chain(second));
            let had = found.map(|f| &self.fonts.entry(f).families);
            for n in chain {
                if !out.iter().any(|o| o == n) {
                    out.push(n.to_string());
                }
                let key = |s: &str| s.to_lowercase().replace(' ', "");
                if had.is_some_and(|fs| fs.iter().any(|f| key(f) == key(n))) {
                    break;
                }
            }
        }
        out
    }

    pub fn fonts(&self) -> &'a FontSystem {
        self.fonts
    }

    pub fn info(&self) -> &'a DocInfo {
        self.info
    }

    pub fn get(&self, i: u32) -> &Style {
        &self.list[i as usize]
    }

    pub fn list(&self) -> &[Style] {
        &self.list
    }

    /// The loaded face of a style.
    pub fn face(&self, i: u32) -> Option<&Arc<Face>> {
        self.faces[i as usize].as_ref()
    }

    /// The document's char shape `id`, if it exists.
    pub fn char_shape_ref(&self, id: u32) -> Option<&'a CharShape> {
        self.info.char_shape(id)
    }

    pub(crate) fn char_shape(&self, id: u32) -> CharShape {
        self.info
            .char_shape(id)
            .cloned()
            .unwrap_or_else(|| CharShape {
                base_size: 1000,
                shade_color: 0xFFFF_FFFF,
                ..Default::default()
            })
    }

    fn face_name(&self, cs: &CharShape, script: Script) -> Option<&'a hm_model::FaceName> {
        self.info
            .face(script, cs.face(script))
            .or_else(|| self.info.face(Script::Hangul, cs.face(Script::Hangul)))
    }

    fn primary_face(&mut self, cs_id: u32, script: Script) -> Primary {
        if let Some(&hit) = self.primary.get(&(cs_id, script)) {
            return hit;
        }
        let cs = self.char_shape(cs_id);
        let resolved = match self
            .info
            .char_shape(cs_id)
            .and_then(|cs| self.face_name(cs, script))
        {
            Some(f) => {
                let name = f.name.to_string_lossy();
                if !self.requested.contains(&name) {
                    self.requested.push(name.clone());
                }
                let sub = f.substitute.as_ref().map(|(_, n)| n.to_string_lossy());
                let def = f.default_name.as_ref().map(|n| n.to_string_lossy());
                let alternates: Vec<&str> =
                    sub.iter().chain(def.iter()).map(String::as_str).collect();
                let r = self.fonts.resolve(&FaceQuery {
                    name: &name,
                    alternates,
                    bold: cs.bold(),
                    italic: cs.italic(),
                });
                if let Some(r) = r.filter(|r| !r.exact)
                    && !self.substituted.iter().any(|(a, _)| *a == name)
                {
                    self.substituted
                        .push((name, self.fonts.entry(r.face).families[0].clone()));
                }
                r
            }
            None if self.info.char_shape(cs_id).is_none() => {
                self.fonts.resolve(&FaceQuery::default())
            }
            None => self
                .fonts
                .class_default(ScriptClass::Serif, cs.bold(), cs.italic()),
        };
        let p = Primary {
            face: resolved.map(|r| r.face),
            exact: resolved.is_some_and(|r| r.exact),
            fake_bold: resolved.is_some_and(|r| r.synthetic_bold),
        };
        self.primary.insert((cs_id, script), p);
        p
    }

    /// The style for a character of `script` in char shape `cs_id`; when the
    /// shape's face lacks `ch`, a fallback face that has it.
    pub fn style_for(&mut self, cs_id: u32, script: Script, ch: Option<char>) -> u32 {
        self.style_variant(cs_id, script, ch, Variant::Plain)
    }

    /// Like [`Styles::style_for`], drawn as superscript whatever the char
    /// shape says: note reference marks and superscript numbers.
    pub fn superscript_style(&mut self, cs_id: u32, script: Script, ch: Option<char>) -> u32 {
        self.style_variant(cs_id, script, ch, Variant::Superscript)
    }

    /// The style of a note's reference mark in char shape `cs_id`.
    pub fn note_mark_style(&mut self, cs_id: u32, script: Script, ch: Option<char>) -> u32 {
        self.style_variant(cs_id, script, ch, Variant::NoteMark)
    }

    /// A copy of style `base` changed by `f`, as a new style.
    pub fn derived(&mut self, base: u32, f: impl FnOnce(&mut Style)) -> u32 {
        let mut style = self.list[base as usize].clone();
        f(&mut style);
        let face = self.faces[base as usize].clone();
        let i = self.list.len() as u32;
        self.list.push(style);
        self.faces.push(face);
        i
    }

    /// A style outside the document's char shapes: `face` at `size` HU,
    /// full width, no spacing, black, for text Hancom draws with one of its
    /// built-in styles. A character the face lacks takes a sans face that
    /// has it.
    pub fn builtin_style(&mut self, face: &str, script: Script, ch: char, size: i32) -> u32 {
        let fonts = self.fonts;
        let named = fonts
            .resolve(&FaceQuery {
                name: face,
                ..Default::default()
            })
            .map(|r| r.face);
        let face = match named {
            Some(f) if fonts.face(f).is_some_and(|l| l.has(ch)) => Some(f),
            other => fonts.fallback_for(ch, ScriptClass::Sans).or(other),
        };
        if let Some(&i) = self.by_key.get(&(BUILTIN, script, face, Variant::Plain)) {
            return i;
        }
        let loaded = face.and_then(|f| fonts.face(f));
        let style = Style {
            char_shape: BUILTIN,
            script,
            face,
            exact: named == face,
            size,
            line_size: size,
            ratio: 100,
            spacing: 0,
            offset: 0,
            color: 0,
            fake_bold: false,
            italic: false,
            underline: 0,
            underline_color: 0,
            strike: false,
            shade: None,
            units_per_em: loaded.as_ref().map_or(1000, |f| f.units_per_em),
        };
        let i = self.list.len() as u32;
        self.list.push(style);
        self.faces.push(loaded);
        self.by_key
            .insert((BUILTIN, script, face, Variant::Plain), i);
        i
    }

    fn style_variant(
        &mut self,
        cs_id: u32,
        script: Script,
        ch: Option<char>,
        variant: Variant,
    ) -> u32 {
        let p = self.primary_face(cs_id, script);
        let (mut face, mut exact) = (p.face, p.exact);
        if let (Some(c), Some(f)) = (ch, face)
            && let Some(primary) = self.fonts.face(f)
            && !primary.has(c)
        {
            let cs = self.char_shape(cs_id);
            let name = self
                .face_name(&cs, script)
                .map(|f| f.name.to_string_lossy())
                .unwrap_or_default();
            let class = classify(&name);
            let flags = (
                name.clone(),
                script == Script::Latin,
                primary.is_hft(),
                class == ScriptClass::Serif,
            );
            let fonts = self.fonts;
            // A Latin character takes the face Hancom's font map stands in
            // for this one (Palatino Linotype for HCI Poppy); any other
            // character an HFT face lacks takes the face Hancom draws for
            // the name when it is missing (함초롬바탕 for 한양신명조, 한컴바탕
            // for 명조), else Haansoft Dotum, as Hancom's PDFs show for
            // symbols; the rest take a face of the name's class that has them.
            let preferred = if script == Script::Latin {
                *self
                    .mapped
                    .entry((name.clone(), c))
                    .or_insert_with(|| fonts.mapped_fallback(&name, c))
            } else if primary.is_hft() {
                fonts
                    .substitute_glyph(&name, c)
                    .or_else(|| fonts.family_glyph(HFT_FALLBACK, c))
            } else {
                None
            };
            let found = match preferred {
                Some(f) => Some(f),
                None => *self
                    .fallback
                    .entry((c, class == ScriptClass::Serif))
                    .or_insert_with(|| fonts.fallback_for(c, class)),
            };
            let miss = (flags.0, flags.1, flags.2, flags.3, found);
            if !self.misses.contains(&miss) {
                self.misses.push(miss);
            }
            if found.is_some() {
                face = found;
                exact = false;
            }
        }
        if let Some(&i) = self.by_key.get(&(cs_id, script, face, variant)) {
            return i;
        }
        let mut cs = self.char_shape(cs_id);
        if variant == Variant::Superscript {
            cs.attr = (cs.attr | 1 << 15) & !(1 << 16);
        }
        let loaded = face.and_then(|f| self.fonts.face(f));
        let mut style = Style {
            char_shape: cs_id,
            script,
            face,
            exact,
            size: glyph_size(&cs, script),
            line_size: line_size(&cs),
            ratio: cs.ratio(script),
            spacing: cs.spacing(script),
            offset: position_offset(&cs, script),
            color: cs.color,
            fake_bold: p.fake_bold,
            italic: cs.italic() && face.is_none_or(|id| !self.fonts.entry(id).italic),
            underline: cs.underline_kind() as u8,
            underline_color: cs.underline_color,
            // Hancom stores "no strikeout" as kind 1 with shape 15 in most
            // character shapes and draws nothing for it.
            strike: cs.strike() != 0 && (cs.attr >> 26) & 15 != 15,
            shade: Some(cs.shade_color).filter(|&c| c >> 24 == 0 && c != 0x00FF_FFFF),
            units_per_em: loaded.as_ref().map_or(1000, |f| f.units_per_em),
        };
        if variant == Variant::NoteMark {
            // Hancom sets a note's reference mark at three quarters of the
            // run's glyph size, raised 21 percent of the base size above
            // the run's baseline, or 10 percent in a superscript run.
            style.size = mul_div(style.size, 75, 100);
            let raise = if cs.superscript() { 10 } else { 21 };
            style.offset -= mul_div(line_size(&cs), raise, 100);
        }
        let i = self.list.len() as u32;
        self.list.push(style);
        self.faces.push(loaded);
        self.by_key.insert((cs_id, script, face, variant), i);
        i
    }
}
