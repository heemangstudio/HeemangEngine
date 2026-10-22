//! Paragraph heads: bullets, numbers and outline numbers drawn before the
//! first line.

use std::collections::HashMap;

use hm_model::{DocInfo, ParaHead, ParaShape};

/// The head of one paragraph, before measurement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Head {
    pub text: String,
    pub info: ParaHead,
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
}

impl Heads {
    /// The head of a paragraph, advancing the counters it uses. `outline` is
    /// the section's outline numbering id (1-based, 0 for none).
    pub fn next(&mut self, shape: &ParaShape, info: &DocInfo, outline: u16) -> Option<Head> {
        let level = (shape.level() as usize).min(6);
        match shape.head_kind() {
            3 => {
                let b = info
                    .bullets
                    .get((shape.numbering as usize).checked_sub(1)?)?;
                if b.image.is_some_and(|i| i != 0) {
                    return None;
                }
                let c = char::from_u32(b.bullet_char as u32).filter(|c| !c.is_control())?;
                Some(Head {
                    text: c.to_string(),
                    info: b.head,
                })
            }
            kind @ (1 | 2) => {
                let id = if kind == 1 { outline } else { shape.numbering };
                let n = info.numberings.get((id as usize).checked_sub(1)?)?;
                let lvl = n.levels.get(level)?;
                let starts: Vec<u32> = (0..7)
                    .map(|l| n.level_starts.map_or(n.start.unwrap_or(1) as u32, |s| s[l]))
                    .collect();
                let c = self.counters.entry(id).or_insert([0; 10]);
                c[level] = if c[level] == 0 {
                    starts[level].max(1)
                } else {
                    c[level] + 1
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
                (!text.is_empty()).then_some(Head {
                    text,
                    info: lvl.head,
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
    fn formats_numbers() {
        assert_eq!(number(3, 0), "3");
        assert_eq!(number(3, 1), "③");
        assert_eq!(number(4, 2), "IV");
        assert_eq!(number(2, 8), "나");
        assert_eq!(number(27, 5), "aa");
        assert_eq!(expand("^1.^2)", |l| (l + 1).to_string()), "1.2)");
    }
}
