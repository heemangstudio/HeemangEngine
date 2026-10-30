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
#[derive(Clone, Debug, Default)]
pub struct Heads {
    /// Per numbering id: the current number of each level, 0 when unused.
    counters: HashMap<u16, [u32; 10]>,
    /// The numbering and level of the last numbered paragraph.
    last: Option<(u16, usize)>,
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
                // A paragraph switching to another numbering continues the
                // list at a deeper level than the last numbered paragraph and
                // starts it anew otherwise.
                if kind == 2
                    && let Some((prev, prev_level)) = self.last
                    && prev != id
                {
                    let carried = if level > prev_level {
                        self.counters.get(&prev).copied().unwrap_or([0; 10])
                    } else {
                        [0; 10]
                    };
                    if restart == Some(ListRestart::Continue) {
                        self.counters.entry(id).or_insert(carried);
                    } else {
                        self.counters.insert(id, carried);
                    }
                }
                if kind == 2 {
                    self.last = Some((id, level));
                }
                let c = self.counters.entry(id).or_insert([0; 10]);
                c[level] = if let Some(ListRestart::Start(start)) = restart {
                    start.max(1)
                } else if c[level] == 0 {
                    starts[level].max(1)
                } else {
                    c[level].saturating_add(1)
                };
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
                    let format = n.levels.get(l).map_or(0, |x| (x.head.attr >> 5) & 0xF);
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
pub fn number(n: u32, format: u32) -> String {
    const CIRCLED: [char; 20] = [
        '①', '②', '③', '④', '⑤', '⑥', '⑦', '⑧', '⑨', '⑩', '⑪', '⑫', '⑬', '⑭', '⑮', '⑯', '⑰', '⑱',
        '⑲', '⑳',
    ];
    const GANADA: [char; 14] = [
        '가', '나', '다', '라', '마', '바', '사', '아', '자', '차', '카', '타', '파', '하',
    ];
    const GIYEOK: [char; 14] = [
        'ㄱ', 'ㄴ', 'ㄷ', 'ㄹ', 'ㅁ', 'ㅂ', 'ㅅ', 'ㅇ', 'ㅈ', 'ㅊ', 'ㅋ', 'ㅌ', 'ㅍ', 'ㅎ',
    ];
    const IL: [&str; 10] = ["일", "이", "삼", "사", "오", "육", "칠", "팔", "구", "십"];
    const HANJA: [&str; 10] = ["一", "二", "三", "四", "五", "六", "七", "八", "九", "十"];
    // The ten heavenly stems, in Hangul and in Hanja.
    const GAPEUL: [char; 10] = ['갑', '을', '병', '정', '무', '기', '경', '신', '임', '계'];
    const JIAYI: [char; 10] = ['甲', '乙', '丙', '丁', '戊', '己', '庚', '辛', '壬', '癸'];
    // Four symbols taken in turn (HWP 5 table 41); Hancom 2022 draws the
    // first three as * † ‡.
    const SYMBOLS: [char; 4] = ['*', '†', '‡', '§'];
    let idx = n.saturating_sub(1) as usize;
    let cycle = |set: &[char]| set[idx % set.len()].to_string();
    match format {
        1 if idx < CIRCLED.len() => CIRCLED[idx].to_string(),
        2 => roman(n).to_uppercase(),
        3 => roman(n),
        4 => alpha(n).to_uppercase(),
        5 => alpha(n),
        6 if idx < 26 => char::from_u32(0x24B6 + idx as u32).unwrap().to_string(),
        7 if idx < 26 => char::from_u32(0x24D0 + idx as u32).unwrap().to_string(),
        8 => cycle(&GANADA),
        9 if idx < 14 => char::from_u32(0x326E + idx as u32).unwrap().to_string(),
        10 => cycle(&GIYEOK),
        11 if idx < 14 => char::from_u32(0x3260 + idx as u32).unwrap().to_string(),
        12 if idx < 10 => IL[idx].to_string(),
        13 if idx < 10 => HANJA[idx].to_string(),
        14 if idx < 10 => char::from_u32(0x3280 + idx as u32).unwrap().to_string(),
        15 => cycle(&GAPEUL),
        16 => cycle(&JIAYI),
        0x80 => cycle(&SYMBOLS),
        _ => n.to_string(),
    }
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

fn alpha(n: u32) -> String {
    let i = n.saturating_sub(1);
    let c = (b'a' + (i % 26) as u8) as char;
    c.to_string().repeat(i as usize / 26 + 1)
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
        assert_eq!(number(27, 5), "aa");
        assert_eq!(expand("^1.^2)", |l| (l + 1).to_string()), "1.2)");
    }
}
