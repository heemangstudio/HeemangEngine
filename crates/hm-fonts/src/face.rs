//! A loaded face: metrics, cached character lookups and glyph outlines.
//!
//! A face is either one SFNT font or one HFT family, whose script ranges
//! live in separate files (members) that may use different design ems.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::raw::{FontRef, TableProvider};
use skrifa::{GlyphId, MetadataProvider};

use crate::FontData;
use crate::compact::Residency;
use crate::hchar::CodeTables;
use crate::hft::{GlyphRef, HftFile};

/// A character's glyph and unhinted advance in font units.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlyphInfo {
    pub glyph: u16,
    pub advance: i32,
    /// The design em the advance and outline are in.
    pub em: u16,
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
    /// The design em of the coordinates.
    pub units_per_em: u16,
}

/// The files of an HFT family, in script-slot order.
pub(crate) struct HftFamily {
    pub members: Vec<Arc<HftFile>>,
    pub codes: Arc<CodeTables>,
}

enum Backing {
    Sfnt {
        data: FontData,
        index: u32,
        /// For a compact copy: the glyphs it has outlines for, and whether
        /// a layout is running (set by the font system).
        compact: Option<(Arc<Residency>, Arc<AtomicBool>)>,
    },
    Hft(HftFamily),
}

pub struct Face {
    backing: Backing,
    pub family: String,
    pub units_per_em: u16,
    /// Ascent and descent in font units, both positive.
    pub ascent: i32,
    pub descent: i32,
    glyphs: Mutex<HashMap<u32, Option<GlyphInfo>>>,
    outlines: Mutex<HashMap<u16, Arc<Outline>>>,
    /// HFT glyph ids: member and glyph per dense id.
    hft_ids: Mutex<Vec<(u8, GlyphRef)>>,
}

impl std::fmt::Debug for Face {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Face")
            .field("family", &self.family)
            .finish()
    }
}

impl Face {
    fn new(backing: Backing, family: String, units_per_em: u16, ascent: i32, descent: i32) -> Face {
        Face {
            backing,
            family,
            units_per_em,
            ascent,
            descent,
            glyphs: Mutex::new(HashMap::new()),
            outlines: Mutex::new(HashMap::new()),
            hft_ids: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn load(
        data: FontData,
        index: u32,
        family: String,
        compact: Option<(Arc<Residency>, Arc<AtomicBool>)>,
    ) -> Option<Face> {
        let font = FontRef::from_index((*data).as_ref(), index).ok()?;
        let units_per_em = font.head().ok()?.units_per_em().max(1);
        let (ascent, descent) = crate::vertical_metrics(&font);
        Some(Face::new(
            Backing::Sfnt {
                data,
                index,
                compact,
            },
            family,
            units_per_em,
            ascent,
            descent,
        ))
    }

    /// An HFT family. Hancom's HFT metric producer gives 80 percent of the
    /// em as ascent and 20 percent as descent.
    pub(crate) fn load_hft(family: HftFamily, name: String) -> Option<Face> {
        let em = family.members.first()?.em.clamp(1, u16::MAX as i32);
        Some(Face::new(
            Backing::Hft(family),
            name,
            em as u16,
            em * 8 / 10,
            em * 2 / 10,
        ))
    }

    /// True for an HFT family.
    pub fn is_hft(&self) -> bool {
        matches!(self.backing, Backing::Hft(_))
    }

    fn font(&self) -> Option<FontRef<'_>> {
        match &self.backing {
            Backing::Sfnt { data, index, .. } => {
                FontRef::from_index((**data).as_ref(), *index).ok()
            }
            Backing::Hft(_) => None,
        }
    }

    /// The glyph and advance for `ch`, or `None` when the face lacks it.
    pub fn glyph(&self, ch: char) -> Option<GlyphInfo> {
        let key = ch as u32;
        if let Some(hit) = self.glyphs.lock().unwrap().get(&key) {
            return *hit;
        }
        let info = match &self.backing {
            Backing::Sfnt { .. } => self.sfnt_glyph(ch),
            Backing::Hft(family) => self.hft_glyph(family, ch),
        };
        self.glyphs.lock().unwrap().insert(key, info);
        info
    }

    fn sfnt_glyph(&self, ch: char) -> Option<GlyphInfo> {
        let font = self.font()?;
        font.charmap().map(ch).filter(|g| g.to_u32() != 0).map(|g| {
            let advance = font
                .hmtx()
                .ok()
                .and_then(|h| h.advance(g))
                .unwrap_or(self.units_per_em);
            GlyphInfo {
                glyph: g.to_u32() as u16,
                advance: advance as i32,
                em: self.units_per_em,
            }
        })
    }

    /// The first member with an outline for the character's HChar decides;
    /// its advance table gives the width in its header em.
    fn hft_glyph(&self, family: &HftFamily, ch: char) -> Option<GlyphInfo> {
        let code = family.codes.hchar(ch)?;
        for (mi, m) in family.members.iter().enumerate() {
            let Some(g) = m.glyph(code) else { continue };
            let em = m.em.clamp(1, u16::MAX as i32);
            let advance = m.advance(code).unwrap_or(em);
            let mut ids = self.hft_ids.lock().unwrap();
            let id = u16::try_from(ids.len()).ok()?;
            ids.push((mi as u8, g));
            return Some(GlyphInfo {
                glyph: id,
                advance,
                em: em as u16,
            });
        }
        None
    }

    pub fn has(&self, ch: char) -> bool {
        self.glyph(ch).is_some()
    }

    /// Advance Hancom adds to an HFT glyph drawn with synthetic bold:
    /// `(em + 10) / 20` before scaling. SFNT faces add none.
    pub fn synthetic_bold_extra(&self, info: &GlyphInfo) -> i32 {
        if self.is_hft() {
            (info.em as i32 + 10) / 20
        } else {
            0
        }
    }

    /// False for a glyph a compact copy has no outline for yet: its
    /// outline comes empty and the font system lists the face's file as
    /// wanted again ([`crate::FontSystem::outline_requests`]).
    pub fn resident(&self, glyph: u16) -> bool {
        match &self.backing {
            Backing::Sfnt {
                compact: Some((r, _)),
                ..
            } => r.has(glyph),
            _ => true,
        }
    }

    /// The unhinted outline of a glyph in font units, cached.
    pub fn outline(&self, glyph: u16) -> Arc<Outline> {
        if let Some(o) = self.outlines.lock().unwrap().get(&glyph) {
            return o.clone();
        }
        if let Backing::Sfnt {
            compact: Some((r, in_layout)),
            ..
        } = &self.backing
            && !r.has(glyph)
        {
            r.want(glyph, in_layout.load(Ordering::Relaxed));
            return Arc::new(Outline::default());
        }
        let o = Arc::new(match &self.backing {
            Backing::Sfnt { .. } => self.sfnt_outline(glyph),
            Backing::Hft(family) => self.hft_outline(family, glyph),
        });
        self.outlines.lock().unwrap().insert(glyph, o.clone());
        o
    }

    fn sfnt_outline(&self, glyph: u16) -> Outline {
        let mut pen = Pen(Outline {
            ops: Vec::new(),
            units_per_em: self.units_per_em,
        });
        if let Some(font) = self.font()
            && let Some(g) = font.outline_glyphs().get(GlyphId::new(glyph as u32))
        {
            let _ = g.draw(
                DrawSettings::unhinted(Size::unscaled(), LocationRef::default()),
                &mut pen,
            );
        }
        pen.0
    }

    fn hft_outline(&self, family: &HftFamily, glyph: u16) -> Outline {
        let Some(&(member, g)) = self.hft_ids.lock().unwrap().get(glyph as usize) else {
            return Outline::default();
        };
        let Some(m) = family.members.get(member as usize) else {
            return Outline::default();
        };
        let Some(o) = m.outline(g) else {
            return Outline::default();
        };
        let em = o.em.clamp(1, u16::MAX as i32);
        let shift = m.baseline(g) as f32;
        let ops = o
            .ops
            .iter()
            .map(|op| match *op {
                PathOp::Move(x, y) => PathOp::Move(x, y - shift),
                PathOp::Line(x, y) => PathOp::Line(x, y - shift),
                PathOp::Quad(a, b, x, y) => PathOp::Quad(a, b - shift, x, y - shift),
                PathOp::Cubic(a, b, c, d, x, y) => {
                    PathOp::Cubic(a, b - shift, c, d - shift, x, y - shift)
                }
                PathOp::Close => PathOp::Close,
            })
            .collect();
        Outline {
            ops,
            units_per_em: em as u16,
        }
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
