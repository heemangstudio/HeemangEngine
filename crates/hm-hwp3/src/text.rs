//! Character codes: the 2-byte `hchar` of paragraph text and the byte
//! strings (`kchar`) of names.
//!
//! Hangul syllables use the 5-5-5 bit Johab layout. Symbols and Hanja sit
//! in the Johab rows that map one to one onto KS X 1001, so they decode
//! through EUC-KR. Codes below 0x8000 past ASCII are Hancom's own set (see
//! [`hnc`]).

/// Initial consonant per 5-bit code; `FILL` for the filler, `NONE` invalid.
const FILL: u8 = 0xFE;
const NONE: u8 = 0xFF;

const fn initial(code: u32) -> u8 {
    match code {
        1 => FILL,
        2..=20 => (code - 2) as u8,
        _ => NONE,
    }
}

const fn medial(code: u32) -> u8 {
    match code {
        2 => FILL,
        3..=7 => (code - 3) as u8,
        10..=15 => (code - 5) as u8,
        18..=23 => (code - 7) as u8,
        26..=29 => (code - 9) as u8,
        _ => NONE,
    }
}

const fn final_(code: u32) -> u8 {
    match code {
        1 => 0,
        2..=17 => (code - 1) as u8,
        19..=29 => (code - 2) as u8,
        _ => NONE,
    }
}

/// Compatibility jamo of the 19 initials and the 27 finals.
const INITIAL_JAMO: [u16; 19] = [
    0x3131, 0x3132, 0x3134, 0x3137, 0x3138, 0x3139, 0x3141, 0x3142, 0x3143, 0x3145, 0x3146, 0x3147,
    0x3148, 0x3149, 0x314A, 0x314B, 0x314C, 0x314D, 0x314E,
];
const FINAL_JAMO: [u16; 27] = [
    0x3131, 0x3132, 0x3133, 0x3134, 0x3135, 0x3136, 0x3137, 0x3139, 0x313A, 0x313B, 0x313C, 0x313D,
    0x313E, 0x313F, 0x3140, 0x3141, 0x3142, 0x3144, 0x3145, 0x3146, 0x3147, 0x3148, 0x314A, 0x314B,
    0x314C, 0x314D, 0x314E,
];

fn hangul(c: u16) -> Option<char> {
    let c = c as u32;
    let (i, m, f) = (
        initial((c >> 10) & 31),
        medial((c >> 5) & 31),
        final_(c & 31),
    );
    if i == NONE || m == NONE || f == NONE {
        return None;
    }
    let code = match (i, m, f) {
        (FILL, FILL, 0) => 0x3000,
        (FILL, FILL, f) => FINAL_JAMO[f as usize - 1] as u32,
        (i, FILL, 0) => INITIAL_JAMO[i as usize] as u32,
        (FILL, m, 0) => 0x314F + m as u32,
        (FILL, _, _) | (_, FILL, _) => return None,
        (i, m, f) => 0xAC00 + (i as u32 * 21 + m as u32) * 28 + f as u32,
    };
    char::from_u32(code)
}

/// A Johab symbol or Hanja code through its KS X 1001 row and cell.
fn ksx1001(c: u16) -> Option<char> {
    let (lead, trail) = ((c >> 8) as u8, c as u8);
    let row = match lead {
        0xD9..=0xDE => (lead - 0xD9) * 2 + 0x21,
        0xE0..=0xF9 => (lead - 0xE0) * 2 + 0x4A,
        _ => return None,
    };
    let (row, cell) = match trail {
        0x31..=0x7E => (row, trail - 0x31 + 0x21),
        0x91..=0xA0 => (row, trail - 0x91 + 0x6F),
        0xA1..=0xFE => (row + 1, trail),
        _ => return None,
    };
    euc_kr(row, cell)
}

/// Hancom codes below 0x8000 that do not follow the KS X 1001 layout, with
/// the character Hancom's own conversion to HWP 5 gives them. A range maps
/// in order from its first character.
const HNC: &[(u16, u16, u32)] = &[
    (0x0081, 0x0081, 0x201C),
    (0x0082, 0x0082, 0x201D),
    (0x0083, 0x0083, 0x2018),
    (0x0084, 0x0084, 0x2019),
    (0x2014, 0x2014, 0x2014),
    (0x2022, 0x2022, 0x2022),
    (0x205A, 0x205A, 0x25CB),
    (0x2024, 0x2024, 0x30FB),
    (0x21B3, 0x21B3, 0x21B3),
    (0x2219, 0x2219, 0x2219),
    (0x2C21, 0x2C3A, 0x24D0),
    (0x2E00, 0x2E00, 0xF0288),
    (0x2E01, 0x2E09, 0x2460),
    (0x2E0A, 0x2E12, 0xF0289),
    (0x2F06, 0x2F06, 0x25A0),
    (0x2F11, 0x2F11, 0x25E6),
    (0x2F17, 0x2F17, 0x2022),
    (0x3013, 0x3027, 0xF0806),
    (0x303D, 0x303D, 0xF0827),
    (0x3062, 0x3062, 0x201C),
    (0x3063, 0x3063, 0x201D),
    (0x3066, 0x3066, 0x2027),
    (0x3067, 0x3067, 0x2018),
    (0x3068, 0x3068, 0x2019),
    (0x309B, 0x309B, 0xFF62),
    (0x309D, 0x309D, 0xFF63),
    (0x30BB, 0x30BB, 0xF0854),
    (0x30BD, 0x30BD, 0xF0855),
    (0x3366, 0x3366, 0xF03C5),
    (0x3404, 0x3404, 0x2024),
    (0x340D, 0x340D, 0xFF5E),
    (0x37ED, 0x37ED, 0x2550),
];

/// Hancom's own codes: general symbols at their Unicode values, symbol
/// rows of KS X 1001 from 0x3400 (96 cells a row) and its Hanja in KS X
/// 1001 order from 0x4000.
fn hnc(c: u16) -> Option<char> {
    if let Some(&(a, _, u)) = HNC.iter().find(|&&(a, b, _)| (a..=b).contains(&c)) {
        return char::from_u32(u + (c - a) as u32);
    }
    let (row, cell) = match c {
        // Punctuation, letterlike and arrow symbols keep their Unicode
        // values.
        0x2000..0x2300 => return char::from_u32(c as u32),
        0x3400..0x3760 => {
            let i = c - 0x3400;
            (0x21 + i / 96, 0x20 + i % 96)
        }
        0x4000..0x5318 => {
            let i = c - 0x4000;
            (0x4A + i / 94, 0x21 + i % 94)
        }
        _ => return None,
    };
    if !(0x21..=0x7E).contains(&cell) {
        return None;
    }
    euc_kr(row as u8, cell as u8)
}

fn euc_kr(row: u8, cell: u8) -> Option<char> {
    let euc = [row | 0x80, cell | 0x80];
    let (s, bad) = encoding_rs::EUC_KR.decode_without_bom_handling(&euc);
    if bad {
        return None;
    }
    let mut it = s.chars();
    match (it.next(), it.next()) {
        (Some(ch), None) => Some(ch),
        _ => None,
    }
}

/// The character an `hchar` (32 and up) stands for, if known.
pub fn hchar(c: u16) -> Option<char> {
    match c {
        0..=0x7F => char::from_u32(c as u32),
        // A middle dot Hancom keeps among the jamo fillers.
        0x87C1 => Some('\u{00B7}'),
        0x8000.. => hangul(c).or_else(|| ksx1001(c)),
        _ => hnc(c),
    }
}

/// A NUL-terminated byte string: ASCII bytes and 2-byte big-endian Johab
/// codes.
pub fn kchars(b: &[u8]) -> String {
    let mut s = String::new();
    let mut i = 0;
    while i < b.len() && b[i] != 0 {
        if b[i] < 0x80 {
            s.push(b[i] as char);
            i += 1;
        } else if i + 1 < b.len() {
            let c = u16::from_be_bytes([b[i], b[i + 1]]);
            s.push(hchar(c).unwrap_or('\u{FFFD}'));
            i += 2;
        } else {
            break;
        }
    }
    s
}

/// A NUL-terminated `hchar` string.
pub fn hchars(b: &[u8]) -> String {
    b.chunks_exact(2)
        .map(|w| u16::from_le_bytes([w[0], w[1]]))
        .take_while(|&c| c != 0)
        .map(|c| hchar(c).unwrap_or('\u{FFFD}'))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn johab_codes_decode() {
        assert_eq!(hchar(0x8861), Some('가'));
        assert_eq!(hchar(0xD3BD), Some('힣'));
        assert_eq!(hchar(0x8441), Some('\u{3000}'));
        assert_eq!(hchar(0x8841), Some('ㄱ'));
        assert_eq!(hchar(0x8461), Some('ㅏ'));
        assert_eq!(hchar(0x8442), Some('ㄱ'));
        // A syllable with a filler final code that does not exist.
        assert_eq!(hchar(0x8860), None);
        // KS X 1001 row 0x21 cell 0x21 (ideographic space) and the first Hanja.
        assert_eq!(hchar(0xD931), Some('\u{3000}'));
        assert_eq!(hchar(0xE031), Some('伽'));
        assert_eq!(kchars(b"A\x88\x61\0junk"), "A가");
    }
}
