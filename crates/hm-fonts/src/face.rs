//! A loaded face: metrics, cached character lookups and glyph outlines.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::raw::{FontRef, TableProvider};
use skrifa::{GlyphId, MetadataProvider};

use crate::FontData;

/// A character's glyph and unhinted advance in font units.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlyphInfo {
    pub glyph: u16,
    pub advance: u16,
}

/// One outline command in font units, y up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PathOp {
    Move(f32, f32),
    Line(f32, f32),
    Quad(f32, f32, f32, f32),
    Cubic(f32, f32, f32, f32, f32, f32),
    Close,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outline {
    pub ops: Vec<PathOp>,
}

pub struct Face {
    data: FontData,
    index: u32,
    pub family: String,
    pub units_per_em: u16,
    /// Ascent and descent in font units, both positive.
    pub ascent: i32,
    pub descent: i32,
    glyphs: Mutex<HashMap<u32, Option<GlyphInfo>>>,
    outlines: Mutex<HashMap<u16, Arc<Outline>>>,
}

impl std::fmt::Debug for Face {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Face")
            .field("family", &self.family)
            .field("index", &self.index)
            .finish()
    }
}

impl Face {
    pub(crate) fn load(data: FontData, index: u32, family: String) -> Option<Face> {
        let font = FontRef::from_index((*data).as_ref(), index).ok()?;
        let units_per_em = font.head().ok()?.units_per_em().max(1);
        let (ascent, descent) = crate::vertical_metrics(&font);
        Some(Face {
            data,
            index,
            family,
            units_per_em,
            ascent,
            descent,
            glyphs: Mutex::new(HashMap::new()),
            outlines: Mutex::new(HashMap::new()),
        })
    }

    fn font(&self) -> FontRef<'_> {
        FontRef::from_index((*self.data).as_ref(), self.index).expect("face validated at load")
    }

    /// The glyph and advance for `ch`, or `None` when the face lacks it.
    pub fn glyph(&self, ch: char) -> Option<GlyphInfo> {
        let key = ch as u32;
        if let Some(hit) = self.glyphs.lock().unwrap().get(&key) {
            return *hit;
        }
        let font = self.font();
        let info = font.charmap().map(ch).filter(|g| g.to_u32() != 0).map(|g| {
            let advance = font
                .hmtx()
                .ok()
                .and_then(|h| h.advance(g))
                .unwrap_or(self.units_per_em);
            GlyphInfo {
                glyph: g.to_u32() as u16,
                advance,
            }
        });
        self.glyphs.lock().unwrap().insert(key, info);
        info
    }

    pub fn has(&self, ch: char) -> bool {
        self.glyph(ch).is_some()
    }

    /// The unhinted outline of a glyph in font units, cached.
    pub fn outline(&self, glyph: u16) -> Arc<Outline> {
        if let Some(o) = self.outlines.lock().unwrap().get(&glyph) {
            return o.clone();
        }
        let font = self.font();
        let mut pen = Pen(Outline::default());
        if let Some(g) = font.outline_glyphs().get(GlyphId::new(glyph as u32)) {
            let _ = g.draw(
                DrawSettings::unhinted(Size::unscaled(), LocationRef::default()),
                &mut pen,
            );
        }
        let o = Arc::new(pen.0);
        self.outlines.lock().unwrap().insert(glyph, o.clone());
        o
    }
}

struct Pen(Outline);

impl OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.ops.push(PathOp::Move(x, y));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.ops.push(PathOp::Line(x, y));
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.0.ops.push(PathOp::Quad(cx0, cy0, x, y));
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.ops.push(PathOp::Cubic(cx0, cy0, cx1, cy1, x, y));
    }
    fn close(&mut self) {
        self.0.ops.push(PathOp::Close);
    }
}
