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
        0x21..=0x7E | 0xA1..=0x24F | 0x1E00..=0x1EFF | 0x2018..=0x201F => Script::Latin,
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
    /// Cannot start a line: closing brackets and punctuation.
    Close,
    /// Cannot end a line: opening brackets.
    Open,
    Other,
}

pub fn break_class(ch: char) -> BreakClass {
    match ch {
        ' ' | '\u{3000}' => BreakClass::Space,
        ')' | ']' | '}' | ',' | '.' | '!' | '?' | ':' | ';' | '%' | '\u{2019}' | '\u{201D}'
        | '\u{3001}' | '\u{3002}' | '\u{3009}' | '\u{300B}' | '\u{300D}' | '\u{300F}'
        | '\u{3011}' | '\u{3015}' | '\u{FF09}' | '\u{FF0C}' | '\u{FF0E}' | '\u{FF1A}'
        | '\u{FF1B}' | '\u{FF1F}' | '\u{FF01}' | '\u{00B7}' | '\u{2026}' | '\u{2025}' => {
            BreakClass::Close
        }
        '(' | '[' | '{' | '\u{2018}' | '\u{201C}' | '\u{3008}' | '\u{300A}' | '\u{300C}'
        | '\u{300E}' | '\u{3010}' | '\u{3014}' | '\u{FF08}' => BreakClass::Open,
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
