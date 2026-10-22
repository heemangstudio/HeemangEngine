//! Equation glyphs and their metrics.
//!
//! Hancom sets equations in HYhwpEQ. Italic Latin and Greek letters,
//! digits and the common operators live in the font's private use area;
//! upright Latin letters use its ASCII glyphs and other symbols their
//! Unicode code points. Glyph pitches are measured the way Hancom's
//! equation editor does: the font is sized to an even pixel height at 96
//! dpi, the advance is rounded to whole pixels and glyphs are packed at 90
//! percent of it.

use std::sync::Arc;

use hm_fonts::{Face, FaceId, FontSystem};

/// The equation face and a fallback for characters it lacks.
pub struct MathFonts {
    pub eq: Option<(FaceId, Arc<Face>)>,
    /// Faces tried in order for characters HYhwpEQ does not have
    /// (Hangul, primes).
    pub text: Vec<(FaceId, Arc<Face>)>,
}

/// Families Hancom falls back to inside equations.
const TEXT_FAMILIES: &[&str] = &[
    "HCR Batang",
    "함초롬바탕",
    "Haansoft Batang",
    "한컴바탕",
    "바탕",
    "AppleMyungjo",
];

impl MathFonts {
    pub fn new(fonts: &FontSystem, name: Option<&str>) -> MathFonts {
        let pick = |family: &str| {
            fonts.family(family).first().and_then(|&id| {
                let id = FaceId(id);
                fonts.face(id).map(|f| (id, f))
            })
        };
        let eq = name
            .and_then(&pick)
            .or_else(|| pick("HyhwpEQ"))
            .or_else(|| pick("HancomEQN"));
        let text = TEXT_FAMILIES.iter().filter_map(|f| pick(f)).collect();
        MathFonts { eq, text }
    }
}

/// Italic Greek letters in HYhwpEQ's private use area, capitals then
/// small letters.
const GREEK: &str = "ΑΒΓΔΕΖΗΘΙΚΛΜΝΞΟΠΡΣΤΥΦΧΨΩαβγδεζηθικλμνξοπρστυφχψω";

/// The private use glyph Hancom paints for a character in an italic or
/// upright context, when there is one.
pub fn pua(c: char, italic: bool) -> Option<char> {
    let code = match c {
        'A'..='Z' if italic => 0xE000 + (c as u32 - 'A' as u32),
        'a'..='z' if italic => 0xE0E5 + (c as u32 - 'a' as u32),
        '1'..='9' => 0xE034 + (c as u32 - '1' as u32),
        '0' => 0xE03D,
        '!' => 0xE03E,
        '%' => 0xE042,
        '*' => 0xE043,
        '(' => 0xE044,
        ')' => 0xE045,
        '-' | '−' => 0xE046,
        '=' => 0xE047,
        '+' => 0xE048,
        '[' => 0xE049,
        ']' => 0xE04A,
        '{' => 0xE04B,
        '}' => 0xE04C,
        '|' => 0xE04D,
        ';' => 0xE04E,
        ':' => 0xE04F,
        ',' => 0xE052,
        '.' => 0xE053,
        '/' => 0xE054,
        '<' => 0xE055,
        '>' => 0xE056,
        '?' => 0xE057,
        '∑' => 0xE067,
        '∫' => 0xE05B,
        _ => {
            let i = GREEK.chars().position(|g| g == c)?;
            0xE085 + i as u32
        }
    };
    char::from_u32(code)
}

/// Pixel em of the even-pixel font Hancom measures with, for a size in HU.
pub fn grid_px(size: i32) -> i32 {
    // Half the 96-dpi pixel size, rounded half up, doubled.
    2 * ((size.max(0) + 75) / 150)
}

/// The packed pitch of a glyph advance (font units of `upem`) in HU.
pub fn pitch(advance: i32, upem: i32, size: i32) -> i32 {
    let px = (advance * grid_px(size) * 2 + upem) / (2 * upem.max(1));
    px * 75 * 9 / 10
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eleven_point_glyphs_pack_at_ninety_percent() {
        assert_eq!(grid_px(1100), 14);
        // A digit (512 of 1024) is 7 px; packed it takes 472 HU.
        assert_eq!(pitch(512, 1024, 1100), 472);
        // Italic x (585) rounds to 8 px.
        assert_eq!(pitch(585, 1024, 1100), 540);
        assert_eq!(pua('x', true), Some('\u{e0fc}'));
        assert_eq!(pua('θ', false), Some('\u{e0a4}'));
        assert_eq!(pua('π', true), Some('\u{e0ac}'));
    }
}
