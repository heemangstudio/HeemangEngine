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

/// The style table of one layout, built on demand.
pub struct Styles<'a> {
    info: &'a DocInfo,
    fonts: &'a FontSystem,
    list: Vec<Style>,
    faces: Vec<Option<Arc<Face>>>,
    by_key: HashMap<(u32, Script, Option<FaceId>), u32>,
    primary: HashMap<(u32, Script), Primary>,
    fallback: HashMap<(char, bool), Option<FaceId>>,
    /// Requested face names that did not resolve exactly: `(requested, used)`.
    pub substituted: Vec<(String, String)>,
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
            substituted: Vec::new(),
        }
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
        let p = self.primary_face(cs_id, script);
        let (mut face, mut exact) = (p.face, p.exact);
        if let (Some(c), Some(f)) = (ch, face)
            && !self.fonts.face(f).is_some_and(|f| f.has(c))
        {
            let cs = self.char_shape(cs_id);
            let class = classify(
                &self
                    .face_name(&cs, script)
                    .map(|f| f.name.to_string_lossy())
                    .unwrap_or_default(),
            );
            let fonts = self.fonts;
            let found = *self
                .fallback
                .entry((c, class == ScriptClass::Serif))
                .or_insert_with(|| fonts.fallback_for(c, class));
            if found.is_some() {
                face = found;
                exact = false;
            }
        }
        if let Some(&i) = self.by_key.get(&(cs_id, script, face)) {
            return i;
        }
        let cs = self.char_shape(cs_id);
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
            offset: mul_div(cs.base_size, cs.offset(script), 100),
            color: cs.color,
            fake_bold: p.fake_bold,
            italic: cs.italic(),
            underline: cs.underline_kind() as u8,
            underline_color: cs.underline_color,
            strike: cs.strike() != 0,
            shade: Some(cs.shade_color).filter(|&c| c >> 24 == 0 && c != 0x00FF_FFFF),
            units_per_em: loaded.as_ref().map_or(1000, |f| f.units_per_em),
        };
        let i = self.list.len() as u32;
        self.list.push(style);
        self.faces.push(loaded);
        self.by_key.insert((cs_id, script, face), i);
        i
    }
}
