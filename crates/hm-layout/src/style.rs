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

/// The style table of one layout, built on demand.
pub struct Styles<'a> {
    info: &'a DocInfo,
    fonts: &'a FontSystem,
    list: Vec<Style>,
    faces: Vec<Option<Arc<Face>>>,
    by_key: HashMap<(u32, Script, Option<FaceId>, bool), u32>,
    primary: HashMap<(u32, Script), Primary>,
    fallback: HashMap<(char, bool), Option<FaceId>>,
    /// Font-map stand-ins by face name and character.
    mapped: HashMap<(String, char), Option<FaceId>>,
    /// Requested face names that did not resolve exactly: `(requested, used)`.
    pub substituted: Vec<(String, String)>,
    /// The page being laid out, for automatic page numbers; 0 when not
    /// known, which shows the number Hancom stored.
    pub page_number: u32,
    /// Replay the line membership Hancom stored for unedited paragraphs
    /// (on by default); off lays out every paragraph fresh.
    pub stored_lines: bool,
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
            page_number: 0,
            stored_lines: true,
        }
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

    fn char_shape(&self, id: u32) -> CharShape {
        self.info
            .char_shape(id)
            .cloned()
            .unwrap_or_else(|| CharShape {
                base_size: 1000,
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
        let resolved = match self.face_name(&cs, script) {
            Some(f) => {
                let name = f.name.to_string_lossy();
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
        self.style_variant(cs_id, script, ch, false)
    }

    /// Like [`Styles::style_for`], drawn as superscript whatever the char
    /// shape says: note reference marks and superscript numbers.
    pub fn superscript_style(&mut self, cs_id: u32, script: Script, ch: Option<char>) -> u32 {
        self.style_variant(cs_id, script, ch, true)
    }

    fn style_variant(&mut self, cs_id: u32, script: Script, ch: Option<char>, raised: bool) -> u32 {
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
            let fonts = self.fonts;
            // A Latin character takes the face Hancom's font map stands in
            // for this one (Palatino Linotype for HCI Poppy); any other
            // character an HFT face lacks takes Haansoft Dotum, as Hancom's
            // PDFs show for symbols; the rest take a face of the name's class
            // that has them.
            let preferred = if script == Script::Latin {
                *self
                    .mapped
                    .entry((name.clone(), c))
                    .or_insert_with(|| fonts.mapped_fallback(&name, c))
            } else if primary.is_hft() {
                fonts.family_glyph(HFT_FALLBACK, c)
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
            if found.is_some() {
                face = found;
                exact = false;
            }
        }
        if let Some(&i) = self.by_key.get(&(cs_id, script, face, raised)) {
            return i;
        }
        let mut cs = self.char_shape(cs_id);
        if raised {
            cs.attr = (cs.attr | 1 << 15) & !(1 << 16);
        }
        let loaded = face.and_then(|f| self.fonts.face(f));
        let style = Style {
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
            italic: cs.italic(),
            underline: cs.underline_kind() as u8,
            underline_color: cs.underline_color,
            // Hancom stores "no strikeout" as kind 1 with shape 15 in most
            // character shapes and draws nothing for it.
            strike: cs.strike() != 0 && (cs.attr >> 26) & 15 != 15,
            shade: Some(cs.shade_color).filter(|&c| c >> 24 == 0 && c != 0x00FF_FFFF),
            units_per_em: loaded.as_ref().map_or(1000, |f| f.units_per_em),
        };
        let i = self.list.len() as u32;
        self.list.push(style);
        self.faces.push(loaded);
        self.by_key.insert((cs_id, script, face, raised), i);
        i
    }
}
