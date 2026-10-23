//! Equations: Hancom's equation script, parsed and laid out by our own
//! engine.
//!
//! Hancom recomputes an equation's width from its script when it opens a
//! document, so the stored object width is not used for layout. The
//! script is parsed ([`parse`]) into nodes, laid out into boxes in HU
//! ([`layout`]) and painted as glyphs and rules relative to the equation's
//! baseline.

mod font;
mod lex;
mod parse;
mod table;
mod typeset;

use std::sync::Arc;

use hm_fonts::{Face, FaceId, FontSystem};
use hm_model::Equation;

pub use font::{EQUATION_FAMILIES, MathFonts, TEXT_FAMILIES, grid_px, pitch, pua};
pub use parse::{Node, parse};

/// One painted element of an equation, relative to its left edge and
/// baseline (y grows downward).
#[derive(Clone, Debug, PartialEq)]
pub enum MathItem {
    Glyph {
        face: FaceId,
        glyph: u16,
        /// The character painted (private use code points included).
        ch: char,
        /// Em size in HU.
        size: i32,
        x: i32,
        /// Baseline.
        y: i32,
        /// Horizontal scale percent (stretched delimiters).
        x_scale: i32,
        bold: bool,
        /// Synthetic slant for faces without italics.
        italic: bool,
        color: u32,
    },
    /// A filled rectangle (fraction bars, radical tops, overlines).
    Rule {
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        color: u32,
    },
}

/// A laid-out equation.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MathLayout {
    pub width: i32,
    /// Extent above and below the baseline.
    pub ascent: i32,
    pub descent: i32,
    pub items: Vec<MathItem>,
}

impl MathLayout {
    pub fn height(&self) -> i32 {
        self.ascent + self.descent
    }

    /// The baseline as Hancom stores it: a percentage of the height.
    pub fn baseline_percent(&self) -> i32 {
        let h = self.height();
        if h <= 0 {
            return 85;
        }
        self.ascent * 100 / h + 1
    }
}

/// Lays out an equation's script with its stored size and color.
pub fn layout(eq: &Equation, fonts: &FontSystem) -> MathLayout {
    let name = eq.font.as_ref().map(|f| f.to_string_lossy());
    let mf = MathFonts::new(fonts, name.as_deref());
    let script = eq.script.to_string_lossy();
    layout_script(&script, eq.size as i32, eq.color & 0x00FF_FFFF, &mf)
}

/// Lays out a script at `size` HU.
pub fn layout_script(script: &str, size: i32, color: u32, fonts: &MathFonts) -> MathLayout {
    let node = parse(script);
    typeset::typeset(&node, size.max(100), color, fonts)
}

/// Faces the painter needs, by id.
pub fn face_of(fonts: &MathFonts, id: FaceId) -> Option<Arc<Face>> {
    if let Some((i, f)) = &fonts.eq
        && *i == id
    {
        return Some(f.clone());
    }
    fonts
        .text
        .iter()
        .find(|(i, _)| *i == id)
        .map(|(_, f)| f.clone())
}
