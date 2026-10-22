//! Which of the seven script slots measures a character.

use hm_model::Script;

/// The slot a character is measured and drawn with, or `None` for spaces,
/// which take the slot of the character before them.
///
/// Hancom for Mac draws ASCII and Latin-1 punctuation and the curly quotes
/// with the Latin face, CJK punctuation, arrows, shapes, letterlike
/// symbols, enclosed alphanumerics and Greek with the Symbol face, and
/// characters outside the listed blocks with the Hangul face.
pub fn script_of(ch: char) -> Option<Script> {
    let c = ch as u32;
    Some(match c {
        0x20 | 0xA0 | 0x2002..=0x200B | 0x3000 => return None,
        // U+0387 (Greek ano teleia) goes with its canonical U+00B7.
        0x21..=0x7E | 0xA1..=0x24F | 0x387 | 0x1E00..=0x1EFF | 0x2018..=0x201F => Script::Latin,
        0x1100..=0x11FF | 0x3130..=0x318F | 0xA960..=0xA97F | 0xAC00..=0xD7AF | 0xD7B0..=0xD7FF => {
            Script::Hangul
        }
        0x2024 | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x2FFFF => {
            Script::Hanja
        }
        0x3040..=0x30FF | 0x31F0..=0x31FF => Script::Japanese,
        0x370..=0x3FF
        | 0x1F00..=0x1FFF
        | 0x203B
        | 0x2100..=0x214F
        | 0x2190..=0x23FF
        | 0x2460..=0x24FF
        | 0x2500..=0x27BF
        | 0x3001..=0x303F
        | 0x3260..=0x327F => Script::Symbol,
        0xE000..=0xF8FF => Script::User,
        _ => Script::Hangul,
    })
}

/// How a character takes part in line breaking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BreakClass {
    Space,
    /// Hangul syllables and jamo.
    Hangul,
    /// Hanja and kana: breakable between any two.
    Ideograph,
    /// Latin letters and digits.
    Alnum,
    /// Cannot start a line (Hancom's default list: closing brackets,
    /// punctuation, small kana, units).
    Close,
    /// Cannot end a line (opening brackets and currency signs).
    Open,
    Other,
}

/// Hancom's default characters that may not start a line, sorted.
const NO_LINE_START: [u16; 89] = [
    0x0021, 0x0025, 0x0029, 0x002C, 0x002E, 0x003A, 0x003B, 0x003E, 0x003F, 0x005D, 0x005E, 0x0060,
    0x007D, 0x007E, 0x00B0, 0x00B7, 0x2019, 0x201D, 0x2024, 0x2025, 0x2026, 0x2030, 0x2031, 0x2032,
    0x2033, 0x2034, 0x2035, 0x2036, 0x2037, 0x2103, 0x2109, 0x3001, 0x3002, 0x3003, 0x3005, 0x3009,
    0x300B, 0x300D, 0x300F, 0x3011, 0x3015, 0x3017, 0x3019, 0x301B, 0x3041, 0x3043, 0x3045, 0x3047,
    0x3049, 0x3063, 0x3083, 0x3085, 0x3087, 0x308E, 0x309B, 0x309C, 0x309D, 0x309E, 0x30A1, 0x30A3,
    0x30A5, 0x30A7, 0x30A9, 0x30C3, 0x30E3, 0x30E5, 0x30E7, 0x30EE, 0x30F5, 0x30F6, 0x30FB, 0x30FC,
    0x30FD, 0x30FE, 0xFF01, 0xFF05, 0xFF09, 0xFF0C, 0xFF0E, 0xFF1A, 0xFF1B, 0xFF1E, 0xFF1F, 0xFF3D,
    0xFF3E, 0xFF40, 0xFF5D, 0xFF5E, 0xFF65,
];

/// Hancom's default characters that may not end a line, sorted.
const NO_LINE_END: [u16; 48] = [
    0x0024, 0x0028, 0x003C, 0x005B, 0x005C, 0x007B, 0x00A2, 0x00A3, 0x00A4, 0x00A5, 0x00A7, 0x09F2,
    0x09F3, 0x0E3F, 0x2018, 0x201C, 0x20A0, 0x20A1, 0x20A2, 0x20A3, 0x20A4, 0x20A5, 0x20A6, 0x20A7,
    0x20A8, 0x20A9, 0x20AA, 0x20AB, 0x20AC, 0x3008, 0x300A, 0x300C, 0x300E, 0x3010, 0x3014, 0x3016,
    0x3018, 0x301A, 0xFF04, 0xFF08, 0xFF1C, 0xFF3B, 0xFF3C, 0xFF5B, 0xFFE0, 0xFFE1, 0xFFE5, 0xFFE6,
];

pub fn break_class(ch: char) -> BreakClass {
    let u = ch as u32;
    if u <= 0xFFFF {
        let u = u as u16;
        if NO_LINE_START.binary_search(&u).is_ok() {
            return BreakClass::Close;
        }
        if NO_LINE_END.binary_search(&u).is_ok() {
            return BreakClass::Open;
        }
    }
    match ch {
        ' ' | '\u{3000}' => BreakClass::Space,
        _ => match script_of(ch) {
            Some(Script::Hangul) if (ch as u32) >= 0x1100 => BreakClass::Hangul,
            Some(Script::Hanja | Script::Japanese) => BreakClass::Ideograph,
            _ if ch.is_alphanumeric() => BreakClass::Alnum,
            _ => BreakClass::Other,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_follow_hancom_mac() {
        assert_eq!(script_of('가'), Some(Script::Hangul));
        assert_eq!(script_of('A'), Some(Script::Latin));
        assert_eq!(script_of(','), Some(Script::Latin));
        assert_eq!(script_of('“'), Some(Script::Latin));
        assert_eq!(script_of('漢'), Some(Script::Hanja));
        assert_eq!(script_of('①'), Some(Script::Symbol));
        assert_eq!(script_of('「'), Some(Script::Symbol));
        assert_eq!(script_of('ㆍ'), Some(Script::Hangul));
        assert_eq!(script_of('\u{387}'), Some(Script::Latin));
        assert_eq!(script_of('·'), Some(Script::Latin));
        assert_eq!(script_of(' '), None);
    }

    #[test]
    fn break_classes() {
        assert_eq!(break_class('가'), BreakClass::Hangul);
        assert_eq!(break_class('a'), BreakClass::Alnum);
        assert_eq!(break_class(')'), BreakClass::Close);
        assert_eq!(break_class('('), BreakClass::Open);
        assert_eq!(break_class(' '), BreakClass::Space);
    }
}
