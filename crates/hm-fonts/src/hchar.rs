//! Hancom's legacy character code (HChar), which HFT fonts index.
//!
//! ASCII is literal and Hangul is Johab: `0x8000 | initial << 10 | vowel << 5
//! | final` with Johab's component values. Everything else goes through
//! Hancom's conversion tables (`u300_*.dat` in the Hancom application),
//! which the host may register; without them only ASCII and Hangul convert.

/// The tables' marker for a character without an HChar.
pub const MISSING: u16 = 0x3441;

/// Johab component values of the modern initials, vowels and finals in
/// Unicode order. Final 1 means no final.
const INITIAL: [u8; 19] = [
    2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20,
];
const VOWEL: [u8; 21] = [
    3, 4, 5, 6, 7, 10, 11, 12, 13, 14, 15, 18, 19, 20, 21, 22, 23, 26, 27, 28, 29,
];
const FINAL: [u8; 28] = [
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 19, 20, 21, 22, 23, 24, 25, 26, 27,
    28, 29,
];

/// Compatibility consonants U+3131..U+314E as Johab: initial value when the
/// consonant can start a syllable, else its final value.
const COMPAT_CONSONANT: [(u8, u8); 30] = [
    (2, 0),
    (3, 0),
    (0, 4),
    (4, 0),
    (0, 6),
    (0, 7),
    (5, 0),
    (6, 0),
    (7, 0),
    (0, 10),
    (0, 11),
    (0, 12),
    (0, 13),
    (0, 14),
    (0, 15),
    (0, 16),
    (8, 0),
    (9, 0),
    (10, 0),
    (0, 20),
    (11, 0),
    (12, 0),
    (13, 0),
    (14, 0),
    (15, 0),
    (16, 0),
    (17, 0),
    (18, 0),
    (19, 0),
    (20, 0),
];

/// The 2,350 KS X 1001 syllables as a bit set over U+AC00..U+D7A3.
static KS_SYLLABLES: &[u8; 1397] = include_bytes!("../data/ks_syllables.bin");

fn johab(initial: u8, vowel: u8, fin: u8) -> u16 {
    0x8000 | (initial as u16) << 10 | (vowel as u16) << 5 | fin as u16
}

/// The Johab code of a modern Hangul syllable.
pub fn syllable(c: char) -> Option<u16> {
    let i = (c as u32).checked_sub(0xAC00).filter(|&i| i < 11172)? as usize;
    Some(johab(
        INITIAL[i / 588],
        VOWEL[(i % 588) / 28],
        FINAL[i % 28],
    ))
}

/// The modern syllable of a Johab code, if it is one.
pub fn syllable_of(code: u16) -> Option<char> {
    if code & 0x8000 == 0 {
        return None;
    }
    let pos = |table: &[u8], v: u16| table.iter().position(|&t| t as u16 == v);
    let i = pos(&INITIAL, (code >> 10) & 31)?;
    let v = pos(&VOWEL, (code >> 5) & 31)?;
    let f = pos(&FINAL, code & 31)?;
    char::from_u32(0xAC00 + (i * 588 + v * 28 + f) as u32)
}

/// A compatibility jamo (U+3131..U+3163) as a stand-alone Johab code: the
/// missing components take their fill values (initial 1, vowel 2, final 1).
fn compat_jamo(c: char) -> Option<u16> {
    let u = c as u32;
    match u {
        0x3131..=0x314E => {
            let (initial, fin) = COMPAT_CONSONANT[(u - 0x3131) as usize];
            Some(if initial != 0 {
                johab(initial, 2, 1)
            } else {
                johab(1, 2, fin)
            })
        }
        0x314F..=0x3163 => Some(johab(1, VOWEL[(u - 0x314F) as usize], 1)),
        _ => None,
    }
}

/// Position of a Johab code among the KS X 1001 syllables, as Hancom's
/// binary search reports it: the index when present, else the last index
/// probed.
pub fn ks_search(code: u16) -> (usize, bool) {
    let Some(c) = syllable_of(code) else {
        // Codes outside the syllable range still binary-search the table.
        return probe(code);
    };
    let i = c as usize - 0xAC00;
    let present = KS_SYLLABLES[i >> 3] & (1 << (i & 7)) != 0;
    if present {
        return (ks_rank(i), true);
    }
    probe(code)
}

fn ks_rank(syllable_index: usize) -> usize {
    let whole = syllable_index >> 3;
    let mut n: usize = KS_SYLLABLES[..whole]
        .iter()
        .map(|b| b.count_ones() as usize)
        .sum();
    n += (KS_SYLLABLES[whole] & ((1u8 << (syllable_index & 7)) - 1)).count_ones() as usize;
    n
}

/// The `n`th KS syllable's Johab code.
fn ks_code(n: usize) -> u16 {
    let mut seen = 0;
    for (byte_index, &b) in KS_SYLLABLES.iter().enumerate() {
        let ones = b.count_ones() as usize;
        if seen + ones > n {
            let mut k = n - seen;
            for bit in 0..8 {
                if b & (1 << bit) != 0 {
                    if k == 0 {
                        let c = char::from_u32(0xAC00 + (byte_index * 8 + bit) as u32).unwrap();
                        return syllable(c).unwrap();
                    }
                    k -= 1;
                }
            }
        }
        seen += ones;
    }
    0
}

/// Hancom's KS binary search over codes it does not contain.
fn probe(code: u16) -> (usize, bool) {
    let (mut lo, mut hi) = (0i32, 2349i32);
    let mut mid = 0;
    while lo <= hi {
        mid = (lo + hi) >> 1;
        let at = ks_code(mid as usize);
        if at == code {
            return (mid as usize, true);
        }
        if at > code {
            hi = mid - 1;
        } else {
            lo = mid + 1;
        }
    }
    (mid as usize, false)
}

/// The 42 stand-alone jamo codes Hancom's Jaso search knows: 21 vowels,
/// 19 initials and two special codes.
const JASO: [u16; 42] = [
    0x8461, 0x8481, 0x84a1, 0x84c1, 0x84e1, 0x8541, 0x8561, 0x8581, 0x85a1, 0x85c1, 0x85e1, 0x8641,
    0x8661, 0x8681, 0x86a1, 0x86c1, 0x86e1, 0x8741, 0x8761, 0x8781, 0x87a1, 0x8841, 0x8c41, 0x9041,
    0x9441, 0x9841, 0x9c41, 0xa041, 0xa441, 0xa841, 0xac41, 0xb041, 0xb441, 0xb841, 0xbc41, 0xc041,
    0xc441, 0xc841, 0xcc41, 0xd041, 0xd3c1, 0xd3c5,
];

/// The Jaso index of a code.
pub fn jaso_search(code: u16) -> Option<usize> {
    JASO.binary_search(&code).ok()
}

/// Hancom's membership test: a KS syllable, or the special code 0xD3C5.
pub fn is_in_ks(code: u16) -> bool {
    code == 0xD3C5 || ks_search(code).1
}

/// Hancom's conversion tables, registered by the host.
#[derive(Clone, Debug, Default)]
pub struct CodeTables {
    /// U+0000..U+33FF, indexed by scalar (the table spans U+3FFF, but
    /// Hanja take U+3400 on).
    general: Vec<u16>,
    /// U+3400..U+9FFF, U+A700..U+A7FF and U+F900..U+FAFF.
    hanja: Vec<u16>,
    /// U+A000..U+A6FF, U+A800..U+A8FF and U+FB00..U+FFFD.
    compatibility: Vec<u16>,
    /// `(unicode, hchar)` pairs consulted when a table has no entry.
    pairs: Vec<(u16, u16)>,
    /// HChar to Unicode private-use glyph, from Hancom's PUA table.
    pua: Vec<(u16, char)>,
}

impl CodeTables {
    /// Registers one `u300_*.dat` file. Returns false when it is not a
    /// table this conversion uses.
    pub fn add(&mut self, bytes: &[u8]) -> bool {
        let Some((name, data)) = parse_table(bytes) else {
            return false;
        };
        let words: Vec<u16> = data
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        if name.contains("Private Use Area") {
            self.pua = words
                .chunks_exact(4)
                .take(0x1900)
                .enumerate()
                .filter_map(|(i, row)| {
                    (row[0] != MISSING).then_some((row[0], char::from_u32(0xE000 + i as u32)?))
                })
                .collect();
            self.pua.sort_unstable_by_key(|&(code, _)| code);
        } else if name.contains("General symbols") {
            self.general = words;
        } else if name.starts_with("Hwpcode-Table") && name.contains("Hanja") {
            self.hanja = words;
        } else if name.contains("Compatibility") {
            self.compatibility = words;
        } else if name.starts_with("Unicode->Hwpcode") {
            self.pairs = words.chunks_exact(2).map(|p| (p[0], p[1])).collect();
        } else {
            return false;
        }
        true
    }

    pub fn is_empty(&self) -> bool {
        self.general.is_empty()
            && self.hanja.is_empty()
            && self.compatibility.is_empty()
            && self.pua.is_empty()
    }

    /// A representable legacy Araea/Araeai syllable, without changing text.
    /// Modern lead/final indices use the same Johab fields as HFT syllables;
    /// the two old vowel slots are 30 and 31. The registered table supplies
    /// the private-use character, never a spelling-specific substitution.
    pub fn hangul_cluster(&self, chars: &[char]) -> Option<char> {
        let lead = (*chars.first()? as u32).checked_sub(0x1100)? as usize;
        let initial = *INITIAL.get(lead)?;
        let vowel = match chars.get(1)? {
            '\u{119e}' => 30,
            '\u{11a1}' => 31,
            _ => return None,
        };
        let fin = match chars {
            [_, _] => 1,
            [_, _, c @ '\u{11a8}'..='\u{11c2}'] => FINAL[*c as usize - 0x11A7],
            _ => return None,
        };
        let code = johab(initial, vowel, fin);
        let i = self.pua.binary_search_by_key(&code, |&(h, _)| h).ok()?;
        if i > 0 && self.pua[i - 1].0 == code || self.pua.get(i + 1).is_some_and(|p| p.0 == code) {
            return None;
        }
        Some(self.pua[i].1)
    }

    fn table(&self, u: u32) -> Option<u16> {
        let (table, index) = match u {
            0x3400..=0x9FFF => (&self.hanja, u - 0x3400),
            0xA700..=0xA7FF => (&self.hanja, u - 0x3B00),
            0xF900..=0xFAFF => (&self.hanja, u - 0x8C00),
            0..=0x33FF => (&self.general, u),
            0xA000..=0xA6FF => (&self.compatibility, u - 0xA000),
            0xA800..=0xA8FF => (&self.compatibility, u - 0xA100),
            0xFB00..=0xFFFD => (&self.compatibility, u - 0xF300),
            _ => return None,
        };
        table.get(index as usize).copied().filter(|&h| h != MISSING)
    }

    /// The HChar of a character, if it has one.
    pub fn hchar(&self, c: char) -> Option<u16> {
        let u = c as u32;
        if let Some(h) = syllable(c).or_else(|| compat_jamo(c)) {
            return Some(h);
        }
        if (0x20..0x7F).contains(&u) {
            return Some(u as u16);
        }
        if let Some(h) = self.table(u) {
            return Some(h);
        }
        let u16_value = u16::try_from(u).ok()?;
        self.pairs
            .iter()
            .find(|p| p.0 == u16_value)
            .map(|p| p.1)
            .filter(|&h| h != MISSING)
    }
}

/// Splits a `u300` file into its table name and binary data.
fn parse_table(bytes: &[u8]) -> Option<(String, &[u8])> {
    let head = &bytes[..bytes.len().min(512)];
    let text = String::from_utf8_lossy(head);
    let name = text
        .split("Table name : ")
        .nth(1)?
        .split(['\r', '\n'])
        .next()?
        .trim()
        .to_string();
    let count_at = find(head, b"Data count")?;
    let line_end = count_at + find(&head[count_at..], b"\r\n")? + 2;
    Some((name, &bytes[line_end..]))
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Decodes a Johab byte string from an HFT header: ASCII bytes and
/// two-byte Hangul codes, stopping at the first zero.
pub fn decode_johab_bytes(raw: &[u8]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < raw.len() && raw[i] != 0 {
        let b = raw[i];
        if b < 0x80 {
            out.push(b as char);
            i += 1;
            continue;
        }
        let Some(&next) = raw.get(i + 1) else { break };
        let code = (b as u16) << 8 | next as u16;
        let c = syllable_of(code)
            .or_else(|| jamo_of(code))
            .unwrap_or('\u{fffd}');
        out.push(c);
        i += 2;
    }
    out
}

/// A stand-alone Johab jamo as a compatibility jamo.
fn jamo_of(code: u16) -> Option<char> {
    (0x3131u32..=0x3163)
        .filter_map(char::from_u32)
        .find(|&c| compat_jamo(c) == Some(code))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_clusters_use_registered_mapping_and_reject_ambiguous_outputs() {
        let table = |codes: &[u16]| {
            let mut out =
                b"Table name : Hwpcode-Table (UCS Private Use Area order)\r\nData count : 3\r\n"
                    .to_vec();
            for &code in codes {
                for word in [code, 0, 0, 0] {
                    out.extend_from_slice(&word.to_le_bytes());
                }
            }
            out
        };
        let mut codes = CodeTables::default();
        assert!(codes.add(&table(&[
            johab(2, 30, 1),
            johab(2, 31, 1),
            johab(20, 30, 5)
        ])));
        assert_eq!(
            codes.hangul_cluster(&['\u{1100}', '\u{119e}']),
            Some('\u{e000}')
        );
        assert_eq!(
            codes.hangul_cluster(&['\u{1100}', '\u{11a1}']),
            Some('\u{e001}')
        );
        assert_eq!(
            codes.hangul_cluster(&['\u{1112}', '\u{119e}', '\u{11ab}']),
            Some('\u{e002}')
        );
        assert_eq!(
            codes.hangul_cluster(&['\u{1112}', '\u{1161}', '\u{11ab}']),
            None
        );
        assert!(codes.add(&table(&[johab(20, 30, 5), johab(20, 30, 5)])));
        assert_eq!(
            codes.hangul_cluster(&['\u{1112}', '\u{119e}', '\u{11ab}']),
            None
        );
    }

    #[test]
    fn hangul_is_johab() {
        assert_eq!(syllable('가'), Some(0x8861));
        assert_eq!(syllable('각'), Some(0x8862));
        assert_eq!(syllable('한'), Some(0xD065));
        assert_eq!(syllable('힣'), Some(0xD3BD));
        assert_eq!(syllable_of(0xD065), Some('한'));
        assert_eq!(compat_jamo('ㄱ'), Some(0x8841));
        assert_eq!(compat_jamo('ㅏ'), Some(0x8461));
        assert_eq!(decode_johab_bytes(&[0xaf, 0xa5, 0x20, 0x41, 0]), "신 A");
    }

    #[test]
    fn ks_search_follows_hancom() {
        assert_eq!(ks_search(0x8861), (0, true));
        assert_eq!(ks_search(0xD3B7), (2349, true));
        // Absent codes report the last probed index, not an insertion point.
        assert_eq!(ks_search(0x8863), (2, false));
        assert_eq!(ks_search(0x8866), (2, false));
        assert!(is_in_ks(0xD3C5));
        assert_eq!(jaso_search(0xD3C5), Some(41));
        assert_eq!(jaso_search(0x8862), None);
    }
}
