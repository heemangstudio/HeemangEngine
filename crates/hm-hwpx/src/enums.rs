//! OWPML enumeration names and the HWP 5 codes they stand for.
//!
//! Each table lists `(name, code)` pairs; the first name of a code is the
//! one writers emit. Lookups ignore ASCII case so older spellings
//! (`DoubleSlim`) map too.

pub type Table = &'static [(&'static str, u32)];

pub fn code(t: Table, name: &str) -> Option<u32> {
    let squash = |s: &str| -> String {
        s.chars()
            .filter(|c| *c != '_')
            .map(|c| c.to_ascii_uppercase())
            .collect()
    };
    t.iter()
        .find(|(n, _)| *n == name)
        .or_else(|| {
            let want = squash(name);
            t.iter().find(|(n, _)| squash(n) == want)
        })
        .map(|&(_, c)| c)
}

pub fn name(t: Table, code: u32) -> Option<&'static str> {
    t.iter().find(|&&(_, c)| c == code).map(|&(n, _)| n)
}

/// Border, note separator and column line kinds as HWP 5 files store them
/// (0 is no line).
pub const LINE_KIND: Table = &[
    ("NONE", 0),
    ("SOLID", 1),
    ("DOT", 2),
    ("DASH", 3),
    ("DASH_DOT", 4),
    ("DASH_DOT_DOT", 5),
    ("LONG_DASH", 6),
    ("CIRCLE", 7),
    ("DOUBLE_SLIM", 8),
    ("SLIM_THICK", 9),
    ("THICK_SLIM", 10),
    ("SLIM_THICK_SLIM", 11),
    ("WAVE", 12),
    ("DOUBLE_WAVE", 13),
    ("DOUBLEWAVE", 13),
    ("THICK_3D", 14),
    ("THICK_3D_REVERSE_LIGHTING", 15),
    ("3D", 16),
    ("3D_REVERSE_LIGHTING", 17),
];

/// Line shapes inside character shape bits (underline, strikeout), where
/// 0 is a solid line.
pub const LINE_SHAPE: Table = &[
    ("SOLID", 0),
    ("DOT", 1),
    ("DASH", 2),
    ("DASH_DOT", 3),
    ("DASH_DOT_DOT", 4),
    ("LONG_DASH", 5),
    ("CIRCLE", 6),
    ("DOUBLE_SLIM", 7),
    ("SLIM_THICK", 8),
    ("THICK_SLIM", 9),
    ("SLIM_THICK_SLIM", 10),
    ("WAVE", 11),
    ("DOUBLE_WAVE", 12),
    ("DOUBLEWAVE", 12),
    ("THICK_3D", 13),
    ("THICK_3D_REVERSE_LIGHTING", 14),
    ("3D", 15),
    ("3D_REVERSE_LIGHTING", 16),
];

/// Border widths in millimetres, by index.
pub const LINE_WIDTHS_MM: [f64; 16] = [
    0.1, 0.12, 0.15, 0.2, 0.25, 0.3, 0.4, 0.5, 0.6, 0.7, 1.0, 1.5, 2.0, 3.0, 4.0, 5.0,
];

pub fn width_index(s: &str) -> Option<u8> {
    let v: f64 = s.trim().trim_end_matches("mm").trim().parse().ok()?;
    LINE_WIDTHS_MM
        .iter()
        .enumerate()
        .min_by(|a, b| (a.1 - v).abs().total_cmp(&(b.1 - v).abs()))
        .map(|(i, _)| i as u8)
}

pub fn width_name(i: u8) -> String {
    let v = LINE_WIDTHS_MM[(i as usize).min(15)];
    if v.fract() == 0.0 {
        format!("{v:.1} mm")
    } else {
        format!("{v} mm")
    }
}

pub const LANG: [&str; 7] = [
    "HANGUL", "LATIN", "HANJA", "JAPANESE", "OTHER", "SYMBOL", "USER",
];

pub const FONT_ATTR_SLOT: [&str; 7] = [
    "hangul", "latin", "hanja", "japanese", "other", "symbol", "user",
];

pub const FONT_TYPE: Table = &[("REP", 0), ("TTF", 1), ("HFT", 2)];

pub const FAMILY_TYPE: Table = &[
    ("FCAT_UNKNOWN", 0),
    ("FCAT_MYUNGJO", 1),
    ("FCAT_GOTHIC", 2),
    ("FCAT_SSERIF", 3),
    ("FCAT_BRUSHSCRIPT", 4),
    ("FCAT_DECORATIVE", 5),
    ("FCAT_NONRECTMJ", 6),
    ("FCAT_NONRECTGT", 7),
];

pub const UNDERLINE_TYPE: Table = &[("NONE", 0), ("BOTTOM", 1), ("CENTER", 2), ("TOP", 3)];

pub const OUTLINE_TYPE: Table = &[
    ("NONE", 0),
    ("SOLID", 1),
    ("DOT", 2),
    ("THICK", 3),
    ("DASH", 4),
    ("DASH_DOT", 5),
    ("DASH_DOT_DOT", 6),
];

pub const SHADOW_TYPE: Table = &[("NONE", 0), ("DROP", 1), ("CONTINUOUS", 2)];

pub const SYM_MARK: Table = &[
    ("NONE", 0),
    ("DOT_ABOVE", 1),
    ("RING_ABOVE", 2),
    ("CARON", 3),
    ("TILDE", 4),
    ("DOT_MIDDLE", 5),
    ("COLON", 6),
    ("GRAVE_ACCENT", 7),
    ("ACUTE_ACCENT", 8),
    ("CIRCUMFLEX", 9),
    ("MACRON", 10),
    ("HOOK_ABOVE", 11),
    ("DOT_BELOW", 12),
];

pub const H_ALIGN: Table = &[
    ("JUSTIFY", 0),
    ("LEFT", 1),
    ("RIGHT", 2),
    ("CENTER", 3),
    ("DISTRIBUTE", 4),
    ("DISTRIBUTE_SPACE", 5),
];

pub const V_ALIGN: Table = &[
    ("BASELINE", 0),
    ("BASEONLINE", 0),
    ("TOP", 1),
    ("CENTER", 2),
    ("BOTTOM", 3),
];

pub const HEADING: Table = &[("NONE", 0), ("OUTLINE", 1), ("NUMBER", 2), ("BULLET", 3)];

pub const BREAK_LATIN: Table = &[("KEEP_WORD", 0), ("HYPHENATION", 1), ("BREAK_WORD", 2)];

/// Hancom stores "keep word" for Korean as bit 7 set.
pub const BREAK_NON_LATIN: Table = &[("BREAK_WORD", 0), ("KEEP_WORD", 1)];

pub const LINE_WRAP: Table = &[("BREAK", 0), ("SQUEEZE", 1), ("KEEP", 2)];

pub const LINE_SPACING: Table = &[
    ("PERCENT", 0),
    ("FIXED", 1),
    ("BETWEEN_LINES", 2),
    ("AT_LEAST", 3),
];

pub const TAB_TYPE: Table = &[("LEFT", 0), ("RIGHT", 1), ("CENTER", 2), ("DECIMAL", 3)];

pub const HEAD_ALIGN: Table = &[("LEFT", 0), ("CENTER", 1), ("RIGHT", 2)];

pub const NUMBER_FORMAT: Table = &[
    ("DIGIT", 0),
    ("CIRCLED_DIGIT", 1),
    ("ROMAN_CAPITAL", 2),
    ("ROMAN_SMALL", 3),
    ("LATIN_CAPITAL", 4),
    ("LATIN_SMALL", 5),
    ("CIRCLED_LATIN_CAPITAL", 6),
    ("CIRCLED_LATIN_SMALL", 7),
    ("HANGUL_SYLLABLE", 8),
    ("CIRCLED_HANGUL_SYLLABLE", 9),
    ("HANGUL_JAMO", 10),
    ("CIRCLED_HANGUL_JAMO", 11),
    ("HANGUL_PHONETIC", 12),
    ("IDEOGRAPH", 13),
    ("CIRCLED_IDEOGRAPH", 14),
    ("DECAGON_CIRCLE", 15),
    ("DECAGON_CIRCLE_HANJA", 16),
    ("SYMBOL", 0x80),
    ("USER_CHAR", 0x81),
];

pub const STYLE_TYPE: Table = &[("PARA", 0), ("CHAR", 1)];

pub const TARGET_PROGRAM: Table = &[("HWP201X", 0), ("HWP200X", 1), ("MS_WORD", 2)];

pub const LANDSCAPE: Table = &[("WIDELY", 0), ("NARROWLY", 1)];

pub const GUTTER: Table = &[("LEFT_ONLY", 0), ("LEFT_RIGHT", 1), ("TOP_BOTTOM", 2)];

pub const NOTE_NUMBERING: Table = &[("CONTINUOUS", 0), ("ON_SECTION", 1), ("ON_PAGE", 2)];

pub const FOOTNOTE_PLACE: Table = &[
    ("EACH_COLUMN", 0),
    ("MERGED_COLUMN", 1),
    ("RIGHT_MOST_COLUMN", 2),
];

pub const ENDNOTE_PLACE: Table = &[("END_OF_DOCUMENT", 0), ("END_OF_SECTION", 1)];

pub const PAGE_BORDER_TYPE: Table = &[("BOTH", 0), ("EVEN", 1), ("ODD", 2)];

pub const TEXT_BORDER: Table = &[("CONTENT", 0), ("PAPER", 1)];

pub const FILL_AREA: Table = &[("PAPER", 0), ("PAGE", 1), ("BORDER", 2)];

pub const COLUMN_TYPE: Table = &[("NEWSPAPER", 0), ("BALANCED_NEWSPAPER", 1), ("PARALLEL", 2)];

pub const COLUMN_LAYOUT: Table = &[("LEFT", 0), ("RIGHT", 1), ("MIRROR", 2)];

pub const TEXT_DIRECTION: Table = &[("HORIZONTAL", 0), ("VERTICAL", 1), ("VERTICALALL", 2)];

pub const PAGE_STARTS_ON: Table = &[("BOTH", 0), ("EVEN", 1), ("ODD", 2)];

pub const VISIBILITY: Table = &[("SHOW_ALL", 0), ("HIDE_FIRST", 1), ("SHOW_FIRST", 2)];

pub const SLASH: Table = &[
    ("NONE", 0),
    ("CENTER", 0b010),
    ("CENTER_BELOW", 0b011),
    ("CENTER_ABOVE", 0b110),
    ("ALL", 0b111),
];

pub const CENTER_LINE: Table = &[
    ("NONE", 0),
    ("VERTICAL", 1),
    ("HORIZONTAL", 2),
    ("CROSS", 3),
];

pub const HATCH: Table = &[
    ("HORIZONTAL", 1),
    ("VERTICAL", 2),
    ("BACK_SLASH", 3),
    ("SLASH", 4),
    ("CROSS", 5),
    ("CROSS_DIAGONAL", 6),
];

pub const GRADATION: Table = &[("LINEAR", 1), ("RADIAL", 2), ("CONICAL", 3), ("SQUARE", 4)];

pub const IMG_MODE: Table = &[
    ("TILE", 0),
    ("TILE_HORZ_TOP", 1),
    ("TILE_HORZ_BOTTOM", 2),
    ("TILE_VERT_LEFT", 3),
    ("TILE_VERT_RIGHT", 4),
    ("TOTAL", 5),
    ("CENTER", 6),
    ("CENTER_TOP", 7),
    ("CENTER_BOTTOM", 8),
    ("LEFT_CENTER", 9),
    ("LEFT_TOP", 10),
    ("LEFT_BOTTOM", 11),
    ("RIGHT_CENTER", 12),
    ("RIGHT_TOP", 13),
    ("RIGHT_BOTTOM", 14),
    ("ZOOM", 15),
];

pub const IMG_EFFECT: Table = &[
    ("REAL_PIC", 0),
    ("GRAY_SCALE", 1),
    ("BLACK_WHITE", 2),
    ("PATTERN8X8", 4),
];

/// Object placement (common object attributes).
pub const VERT_REL_TO: Table = &[("PAPER", 0), ("PAGE", 1), ("PARA", 2)];

pub const HORZ_REL_TO: Table = &[("PAPER", 0), ("PAGE", 1), ("COLUMN", 2), ("PARA", 3)];

pub const VERT_ALIGN: Table = &[
    ("TOP", 0),
    ("CENTER", 1),
    ("BOTTOM", 2),
    ("INSIDE", 3),
    ("OUTSIDE", 4),
];

pub const HORZ_ALIGN: Table = &[
    ("LEFT", 0),
    ("CENTER", 1),
    ("RIGHT", 2),
    ("INSIDE", 3),
    ("OUTSIDE", 4),
];

pub const WIDTH_REL_TO: Table = &[
    ("PAPER", 0),
    ("PAGE", 1),
    ("COLUMN", 2),
    ("PARA", 3),
    ("ABSOLUTE", 4),
];

pub const HEIGHT_REL_TO: Table = &[("PAPER", 0), ("PAGE", 1), ("ABSOLUTE", 2)];

pub const TEXT_WRAP: Table = &[
    ("SQUARE", 0),
    ("TOP_AND_BOTTOM", 1),
    ("BEHIND_TEXT", 2),
    ("IN_FRONT_OF_TEXT", 3),
    ("TIGHT", 4),
    ("THROUGH", 5),
];

pub const TEXT_FLOW: Table = &[
    ("BOTH_SIDES", 0),
    ("LEFT_ONLY", 1),
    ("RIGHT_ONLY", 2),
    ("LARGEST_ONLY", 3),
];

pub const NUMBERING_TYPE: Table = &[("NONE", 0), ("PICTURE", 1), ("TABLE", 2), ("EQUATION", 3)];

pub const AUTO_NUM_TYPE: Table = &[
    ("PAGE", 0),
    ("FOOTNOTE", 1),
    ("ENDNOTE", 2),
    ("PICTURE", 3),
    ("TABLE", 4),
    ("EQUATION", 5),
    ("TOTAL_PAGE", 6),
];

pub const PAGE_TYPE: Table = &[("BOTH", 0), ("EVEN", 1), ("ODD", 2)];

pub const PAGE_NUM_POS: Table = &[
    ("NONE", 0),
    ("TOP_LEFT", 1),
    ("TOP_CENTER", 2),
    ("TOP_RIGHT", 3),
    ("BOTTOM_LEFT", 4),
    ("BOTTOM_CENTER", 5),
    ("BOTTOM_RIGHT", 6),
    ("OUTSIDE_TOP", 7),
    ("OUTSIDE_BOTTOM", 8),
    ("INSIDE_TOP", 9),
    ("INSIDE_BOTTOM", 10),
];

pub const FIELD_TYPE: Table = &[
    ("CLICK_HERE", 0x2563_6c6b),
    ("HYPERLINK", 0x2568_6c6b),
    ("BOOKMARK", 0x2562_6d6b),
    ("FORMULA", 0x2566_6d75),
    ("SUMMARY", 0x2573_6d72),
    ("USER_INFO", 0x2575_7372),
    ("DATE", 0x2564_7465),
    ("DOC_DATE", 0x2564_6474),
    ("PATH", 0x2570_6174),
    ("CROSSREF", 0x2578_7266),
    ("MAILMERGE", 0x256d_6d67),
    ("MEMO", 0x2525_6d65),
    ("PRIVATE_INFO", 0x2563_7072),
    ("UNKNOWN", 0x2575_6e6b),
    ("TABLE_OF_CONTENTS", 0x2574_6f63),
    ("REVISION_SIGN", 0x2573_6967),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookups_ignore_spelling() {
        assert_eq!(code(LINE_KIND, "DoubleSlim"), Some(8));
        assert_eq!(code(LINE_KIND, "DOUBLEWAVE"), Some(13));
        assert_eq!(name(LINE_KIND, 13), Some("DOUBLE_WAVE"));
        assert_eq!(width_index("0.12 mm"), Some(1));
        assert_eq!(width_index("4 mm"), Some(14));
        assert_eq!(width_name(10), "1.0 mm");
        assert_eq!(width_name(1), "0.12 mm");
    }
}
