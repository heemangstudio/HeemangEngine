//! Paragraph heads: bullets, numbers and outline numbers drawn before the
//! first line.

use std::collections::HashMap;

use hm_model::{DocInfo, ListRestart, ParaHead, ParaShape};

/// The head of one paragraph, before measurement.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Head {
    pub text: String,
    pub info: ParaHead,
    pub image: Option<ImageHead>,
}

/// A resolved picture used as a paragraph head.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ImageHead {
    pub bin: u16,
    pub contrast: i8,
    pub brightness: i8,
    pub effect: u8,
}

impl Head {
    /// The char shape the head is drawn with, when the reference is valid.
    pub fn char_shape(&self, doc: &DocInfo) -> Option<u32> {
        let id = self.info.char_shape;
        doc.char_shape(id).filter(|c| c.base_size > 0).map(|_| id)
    }

    /// Distance between the head and the text: a percent of `size` unless
    /// head attribute bit 4 makes it absolute HU.
    pub fn distance(&self, size: i32) -> i32 {
        let d = self.info.text_distance as i32;
        if self.info.attr & (1 << 4) != 0 {
            d
        } else {
            hm_model::units::mul_div(size, d, 100)
        }
    }
}

/// Numbering counters, carried from paragraph to paragraph in document order.
/// Each definition counts on its own, whatever paragraphs of other
/// definitions come between: Hancom numbers definitions A, B, A at level 1
/// `1. 1. 2.`, and B at level 2 between two A paragraphs at level 1 leaves
/// them `1-1 1-2` and itself `1-1-1`, never taking A's numbers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Heads {
    /// Per numbering id: the current number of each level, 0 when unused.
    counters: HashMap<u16, [u32; 10]>,
}

impl Heads {
    /// A resolved head, including a valid empty format. Missing or unsupported
    /// definitions return None. Advances the counters it uses. `outline` is
    /// the section's outline numbering id (1-based, 0 for none).
    pub fn next(&mut self, shape: &ParaShape, info: &DocInfo, outline: u16) -> Option<Head> {
        self.next_with_restart(shape, info, outline, None)
    }

    pub fn next_with_restart(
        &mut self,
        shape: &ParaShape,
        info: &DocInfo,
        outline: u16,
        restart: Option<ListRestart>,
    ) -> Option<Head> {
        let level = (shape.level() as usize).min(6);
        match shape.head_kind() {
            3 => {
                let b = info
                    .bullets
                    .get((shape.numbering as usize).checked_sub(1)?)?;
                let image = if let Some(id) = b.image.filter(|&id| id != 0) {
                    let bin = u16::try_from(id).ok().filter(|&id| id > 0)?;
                    info.bin_data.get(bin as usize - 1)?;
                    let [contrast, brightness, effect, _] = b.image_info.unwrap_or_default();
                    Some(ImageHead {
                        bin,
                        contrast: contrast as i8,
                        brightness: brightness as i8,
                        effect,
                    })
                } else {
                    None
                };
                let text = if image.is_some() {
                    String::new()
                } else {
                    char::from_u32(b.bullet_char as u32)
                        .filter(|c| !c.is_control())?
                        .to_string()
                };
                Some(Head {
                    text,
                    info: b.head,
                    image,
                })
            }
            kind @ (1 | 2) => {
                let id = if kind == 1 { outline } else { shape.numbering };
                let n = info.numberings.get((id as usize).checked_sub(1)?)?;
                let lvl = n.levels.get(level)?;
                let starts: Vec<u32> = (0..7)
                    .map(|l| n.level_starts.map_or(n.start.unwrap_or(1) as u32, |s| s[l]))
                    .collect();
                // Counters are kept per definition, so continuing one is
                // what every paragraph does.
                let c = self.counters.entry(id).or_insert([0; 10]);
                c[level] = if let Some(ListRestart::Start(start)) = restart {
                    start.max(1)
                } else if c[level] == 0 {
                    starts[level].max(1)
                } else {
                    c[level].saturating_add(1)
                };
                // Levels above the first one used count as started: Hancom
                // numbers levels 3, 2, 1, 1 `1) 나. 2. 3.`.
                for parent in c.iter_mut().zip(&starts).take(level) {
                    if *parent.0 == 0 {
                        *parent.0 = (*parent.1).max(1);
                    }
                }
                for deeper in c.iter_mut().skip(level + 1) {
                    *deeper = 0;
                }
                let counters = *c;
                let text = expand(&lvl.format.to_string_lossy(), |l| {
                    let value = if counters[l] == 0 {
                        starts.get(l).copied().unwrap_or(1).max(1)
                    } else {
                        counters[l]
                    };
                    // HWPX stores shapes past 15 (甲乙丙 16, symbols 0x80) in
                    // the bits above the four of HWP 5.
                    let format = n.levels.get(l).map_or(0, |x| (x.head.attr >> 5) & 0xFF);
                    number(value, format)
                });
                Some(Head {
                    text,
                    info: lvl.head,
                    image: None,
                })
            }
            _ => None,
        }
    }
}

/// Replaces `^1`..`^7` with level numbers.
fn expand(format: &str, value: impl Fn(usize) -> String) -> String {
    let mut out = String::new();
    let mut chars = format.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '^' {
            if let Some(d) = chars
                .peek()
                .and_then(|d| d.to_digit(10))
                .filter(|d| (1..=7).contains(d))
            {
                chars.next();
                out.push_str(&value(d as usize - 1));
                continue;
            }
        }
        out.push(c);
    }
    out
}

/// Formats a number in one of Hancom's number shapes (HWP 5 table 41).
/// A number past the end of its shape's table starts the table over, as
/// Hancom 2022 does: ⑳ ① at 20, 21; a z a at 26, 27; ㉮ again at 15. The
/// 가 series runs through six vowel rows (가 … 하, 거 … 허, 고, 구, 그, 기)
/// before it starts over at 85; Roman numerals and the composed 일 / 一
/// numbers (십일, 二十一) start over after 999 and 99.
pub fn number(n: u32, format: u32) -> String {
    const CIRCLED: [char; 20] = [
        '①', '②', '③', '④', '⑤', '⑥', '⑦', '⑧', '⑨', '⑩', '⑪', '⑫', '⑬', '⑭', '⑮', '⑯', '⑰', '⑱',
        '⑲', '⑳',
    ];
    const GIYEOK: [char; 14] = [
        'ㄱ', 'ㄴ', 'ㄷ', 'ㄹ', 'ㅁ', 'ㅂ', 'ㅅ', 'ㅇ', 'ㅈ', 'ㅊ', 'ㅋ', 'ㅌ', 'ㅍ', 'ㅎ',
    ];
    // Initial consonant indexes of ㄱ ㄴ ㄷ … ㅎ and the vowel rows ㅏ ㅓ ㅗ ㅜ ㅡ ㅣ
    // in the Hangul syllable block (U+AC00 + (initial * 21 + vowel) * 28).
    const INITIALS: [u32; 14] = [0, 2, 3, 5, 6, 7, 9, 11, 12, 14, 15, 16, 17, 18];
    const VOWELS: [u32; 6] = [0, 4, 8, 13, 18, 20];
    const IL: [&str; 10] = ["일", "이", "삼", "사", "오", "육", "칠", "팔", "구", "십"];
    const HANJA: [&str; 10] = ["一", "二", "三", "四", "五", "六", "七", "八", "九", "十"];
    // The ten heavenly stems, in Hangul and in Hanja.
    const GAPEUL: [char; 10] = ['갑', '을', '병', '정', '무', '기', '경', '신', '임', '계'];
    const JIAYI: [char; 10] = ['甲', '乙', '丙', '丁', '戊', '己', '庚', '辛', '壬', '癸'];
    // Four symbols taken in turn (HWP 5 table 41); Hancom 2022 draws the
    // first three as * † ‡.
    const SYMBOLS: [char; 4] = ['*', '†', '‡', '§'];
    let idx = n.saturating_sub(1);
    let cycle = |set: &[char]| set[idx as usize % set.len()].to_string();
    let from = |first: u32, len: u32| char::from_u32(first + idx % len).unwrap().to_string();
    match format {
        1 => cycle(&CIRCLED),
        2 => roman(idx % 999 + 1).to_uppercase(),
        3 => roman(idx % 999 + 1),
        4 => from('A' as u32, 26),
        5 => from('a' as u32, 26),
        6 => from(0x24B6, 26),
        7 => from(0x24D0, 26),
        8 => {
            let (row, col) = ((idx / 14) % 6, idx % 14);
            let code = 0xAC00 + (INITIALS[col as usize] * 21 + VOWELS[row as usize]) * 28;
            char::from_u32(code).unwrap().to_string()
        }
        9 => from(0x326E, 14),
        10 => cycle(&GIYEOK),
        11 => from(0x3260, 14),
        12 => tens(idx % 99 + 1, &IL),
        13 => tens(idx % 99 + 1, &HANJA),
        14 => from(0x3280, 10),
        15 => cycle(&GAPEUL),
        16 => cycle(&JIAYI),
        0x80 => cycle(&SYMBOLS),
        _ => n.to_string(),
    }
}

/// 1..99 spelled with digit names and a name for ten: 십, 십일, 이십일.
fn tens(n: u32, names: &[&str; 10]) -> String {
    let (t, o) = (n / 10, n % 10);
    let mut s = String::new();
    if t > 1 {
        s.push_str(names[t as usize - 1]);
    }
    if t > 0 {
        s.push_str(names[9]);
    }
    if o > 0 {
        s.push_str(names[o as usize - 1]);
    }
    s
}

fn roman(mut n: u32) -> String {
    const T: [(u32, &str); 13] = [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let mut s = String::new();
    for &(v, r) in &T {
        while n >= v {
            s.push_str(r);
            n -= v;
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_restart_and_continue_keep_numbering_local_to_the_paragraph() {
        let mut info = DocInfo::default();
        let mut numbering = hm_model::Numbering::default();
        numbering.levels.push(hm_model::NumberingLevel {
            format: hm_model::HwpString("^1.".encode_utf16().collect()),
            ..Default::default()
        });
        info.numberings.push(numbering.clone());
        info.numberings.push(numbering);
        let shape = ParaShape {
            attr1: 2 << 23,
            numbering: 1,
            ..Default::default()
        };
        let other = ParaShape {
            numbering: 2,
            ..shape.clone()
        };
        let mut heads = Heads::default();
        assert_eq!(heads.next(&shape, &info, 0).unwrap().text, "1.");
        assert_eq!(heads.next(&other, &info, 0).unwrap().text, "1.");
        assert_eq!(
            heads
                .next_with_restart(&shape, &info, 0, Some(ListRestart::Continue))
                .unwrap()
                .text,
            "2."
        );
        for kind in [1, 2] {
            let shape = ParaShape {
                attr1: kind << 23,
                numbering: 1,
                ..Default::default()
            };
            let mut heads = Heads::default();
            assert_eq!(heads.next(&shape, &info, 1).unwrap().text, "1.");
            assert_eq!(
                heads
                    .next_with_restart(&shape, &info, 1, Some(ListRestart::Start(7)))
                    .unwrap()
                    .text,
                "7."
            );
            assert_eq!(heads.next(&shape, &info, 1).unwrap().text, "8.");
            let plain = ParaShape::default();
            assert!(
                heads
                    .next_with_restart(&plain, &info, 1, Some(ListRestart::Start(3)))
                    .is_none()
            );
            assert_eq!(
                heads
                    .next_with_restart(&shape, &info, 1, Some(ListRestart::Start(0)))
                    .unwrap()
                    .text,
                "1."
            );
        }
    }

    #[test]
    fn empty_numbering_resolves_without_turning_missing_definitions_into_heads() {
        let mut info = DocInfo::default();
        info.numberings.push(hm_model::Numbering {
            levels: vec![hm_model::NumberingLevel::default()],
            ..Default::default()
        });
        let mut heads = Heads::default();
        let shape = ParaShape {
            attr1: 2 << 23,
            numbering: 1,
            ..Default::default()
        };
        assert_eq!(heads.next(&shape, &info, 0).unwrap().text, "");
        let missing = ParaShape {
            numbering: 2,
            ..shape.clone()
        };
        assert!(heads.next(&missing, &info, 0).is_none());
        info.bullets.push(hm_model::Bullet {
            bullet_char: 0x25cf,
            image: Some(1),
            ..Default::default()
        });
        let image = ParaShape {
            attr1: 3 << 23,
            numbering: 1,
            ..Default::default()
        };
        assert!(heads.next(&image, &info, 0).is_none());
        info.bullets[0].image = Some(0);
        assert_eq!(heads.next(&image, &info, 0).unwrap().text, "●");
    }

    #[test]
    fn image_bullets_resolve_their_binary_reference_independently_of_text() {
        let mut info = DocInfo::default();
        info.bin_data = vec![Default::default(), Default::default()];
        info.bullets.push(hm_model::Bullet {
            // An image head does not need a usable fallback character.
            bullet_char: 0,
            image: Some(2),
            image_info: Some([20, 246, 1, 2]),
            ..Default::default()
        });
        let shape = ParaShape {
            attr1: 3 << 23,
            numbering: 1,
            ..Default::default()
        };
        let mut heads = Heads::default();
        assert!(heads.next(&shape, &info, 0).is_some());
        for missing in [-1, 3, 65536] {
            info.bullets[0].image = Some(missing);
            assert!(heads.next(&shape, &info, 0).is_none());
        }
        info.bullets[0].image = Some(0);
        assert!(heads.next(&shape, &info, 0).is_none());
    }

    #[test]
    fn formats_numbers() {
        assert_eq!(number(3, 0), "3");
        assert_eq!(number(3, 1), "③");
        assert_eq!(number(4, 2), "IV");
        assert_eq!(number(2, 8), "나");
        assert_eq!(expand("^1.^2)", |l| (l + 1).to_string()), "1.2)");
    }

    /// Hancom 2022's heads for numbers past the end of each shape's table
    /// (its PDF of one paragraph per number and shape).
    #[test]
    fn numbers_past_a_table_start_it_over_as_hancom_does() {
        let at = [10, 11, 20, 21, 27, 43, 57, 71, 85, 99, 100, 101, 110, 1000];
        let hancom = [
            (1, "⑩ ⑪ ⑳ ① ⑦ ③ ⑰ ⑪ ⑤ ⑲ ⑳ ① ⑩ ⑳"),
            (2, "X XI XX XXI XXVII XLIII LVII LXXI LXXXV XCIX C CI CX I"),
            (3, "x xi xx xxi xxvii xliii lvii lxxi lxxxv xcix c ci cx i"),
            (4, "J K T U A Q E S G U V W F L"),
            (5, "j k t u a q e s g u v w f l"),
            (6, "Ⓙ Ⓚ Ⓣ Ⓤ Ⓐ Ⓠ Ⓔ Ⓢ Ⓖ Ⓤ Ⓥ Ⓦ Ⓕ Ⓛ"),
            (7, "ⓙ ⓚ ⓣ ⓤ ⓐ ⓠ ⓔ ⓢ ⓖ ⓤ ⓥ ⓦ ⓕ ⓛ"),
            (8, "차 카 버 서 퍼 구 그 기 가 거 너 더 터 비"),
            (9, "㉷ ㉸ ㉳ ㉴ ㉺ ㉮ ㉮ ㉮ ㉮ ㉮ ㉯ ㉰ ㉹ ㉳"),
            (10, "ㅊ ㅋ ㅂ ㅅ ㅍ ㄱ ㄱ ㄱ ㄱ ㄱ ㄴ ㄷ ㅌ ㅂ"),
            (11, "㉩ ㉪ ㉥ ㉦ ㉬ ㉠ ㉠ ㉠ ㉠ ㉠ ㉡ ㉢ ㉫ ㉥"),
            (
                12,
                "십 십일 이십 이십일 이십칠 사십삼 오십칠 칠십일 팔십오 구십구 일 이 십일 십",
            ),
            (
                13,
                "十 十一 二十 二十一 二十七 四十三 五十七 七十一 八十五 九十九 一 二 十一 十",
            ),
            (14, "㊉ ㊀ ㊉ ㊀ ㊆ ㊂ ㊆ ㊀ ㊄ ㊈ ㊉ ㊀ ㊉ ㊉"),
            (15, "계 갑 계 갑 경 병 경 갑 무 임 계 갑 계 계"),
            (16, "癸 甲 癸 甲 庚 丙 庚 甲 戊 壬 癸 甲 癸 癸"),
        ];
        for (format, heads) in hancom {
            let ours: Vec<String> = at.iter().map(|&n| number(n, format)).collect();
            assert_eq!(ours.join(" "), heads, "format {format}");
        }
        // The 가 series at the ends of its rows; Roman numerals around 999.
        let ours: Vec<String> = [14, 15, 28, 29, 30].iter().map(|&n| number(n, 8)).collect();
        assert_eq!(ours.join(" "), "하 거 허 고 노");
        let ours: Vec<String> = [999, 1001, 1998, 4000]
            .iter()
            .map(|&n| number(n, 2))
            .collect();
        assert_eq!(ours.join(" "), "CMXCIX II CMXCIX IV");
    }

    /// Shapes stored past HWP 5's four bits head their paragraphs too:
    /// 甲乙丙 (16), which Hancom's PDF draws, and the symbols (0x80).
    #[test]
    fn number_shapes_past_fifteen_head_paragraphs() {
        let mut info = DocInfo::default();
        for format in [16, 0x80] {
            info.numberings.push(hm_model::Numbering {
                levels: vec![hm_model::NumberingLevel {
                    head: ParaHead {
                        attr: format << 5,
                        ..Default::default()
                    },
                    format: hm_model::HwpString::from_text("^1."),
                }],
                ..Default::default()
            });
        }
        let mut heads = Heads::default();
        let mut text = |numbering| {
            let shape = ParaShape {
                attr1: 2 << 23,
                numbering,
                ..Default::default()
            };
            heads.next(&shape, &info, 0).unwrap().text
        };
        assert_eq!([text(1), text(1), text(2)], ["甲.", "乙.", "*."]);
    }

    /// Hancom's heads for paragraphs of two definitions A and B whose
    /// levels show their parents (`A^1.`, `A^1-^2`, `A^1-^2-^3`), each
    /// sequence in a document of its own.
    #[test]
    fn each_definition_counts_on_its_own_and_starts_unused_parents() {
        let mut info = DocInfo::default();
        for name in ["A", "B"] {
            let mut n = hm_model::Numbering {
                start: Some(1),
                level_starts: Some([1; 7]),
                ..Default::default()
            };
            for format in ["^1.", "^1-^2", "^1-^2-^3"] {
                n.levels.push(hm_model::NumberingLevel {
                    format: hm_model::HwpString::from_text(&format!("{name}{format}")),
                    ..Default::default()
                });
            }
            info.numberings.push(n);
        }
        let hancom = [
            ("A0 B0 A0", "A1. B1. A2."),
            ("A1 B0 A1", "A1-1 B1. A1-2"),
            ("A0 A0 B1 A0", "A1. A2. B1-1 A3."),
            ("A0 A1 B0 A1", "A1. A1-1 B1. A1-2"),
            ("A1 B2 B2 A1 B0 A1", "A1-1 B1-1-1 B1-1-2 A1-2 B2. A1-3"),
            ("A0 A0 B0 B1 A1", "A1. A2. B1. B1-1 A2-1"),
            ("A0 B1 B1 A0 A1", "A1. B1-1 B1-2 A2. A2-1"),
            ("A1 A1 B1 A1", "A1-1 A1-2 B1-1 A1-3"),
            ("A2 A1 A0 A0 A1", "A1-1-1 A1-2 A2. A3. A3-1"),
        ];
        for (sequence, expected) in hancom {
            let mut heads = Heads::default();
            let got: Vec<String> = sequence
                .split(' ')
                .map(|p| {
                    let shape = ParaShape {
                        attr1: 2 << 23 | (p[1..].parse::<u32>().unwrap() << 25),
                        numbering: if p.starts_with('A') { 1 } else { 2 },
                        ..Default::default()
                    };
                    heads.next(&shape, &info, 0).unwrap().text
                })
                .collect();
            assert_eq!(got.join(" "), expected, "{sequence}");
        }
    }
}
