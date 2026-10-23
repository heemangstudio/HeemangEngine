//! Paragraph text with its wire offsets: caret stops between graphemes,
//! and search.

use hm_edit::{ListPath, Pos, Step};
use hm_model::{Document, Paragraph, Unit};
use unicode_segmentation::UnicodeSegmentation;

/// A control's place in the text: one grapheme of its own.
const CONTROL: char = '\u{FFFC}';

/// The paragraph as text, controls as U+FFFC, with the wire offset of each
/// char and the paragraph end's offset last.
pub fn text(p: &Paragraph) -> (String, Vec<u32>) {
    let mut s = String::new();
    let mut at = Vec::new();
    for (pos, u) in p.units() {
        match u {
            Unit::Char(c) => s.push(c),
            Unit::CharControl(13) => continue,
            Unit::CharControl(10) => s.push('\n'),
            Unit::Inline(9) => s.push('\t'),
            _ => s.push(CONTROL),
        }
        at.push(pos);
    }
    at.push(p.len().saturating_sub(1));
    (s, at)
}

/// Where the caret may stop in a paragraph: grapheme starts from after the
/// leading section and column controls, and the end mark.
pub fn stops(p: &Paragraph) -> Vec<u32> {
    let lo = hm_edit::ops::lead(p);
    let (s, at) = text(p);
    let chars: Vec<usize> = s.char_indices().map(|(b, _)| b).collect();
    let mut out: Vec<u32> = s
        .grapheme_indices(true)
        .filter_map(|(b, _)| chars.binary_search(&b).ok().map(|k| at[k]))
        .filter(|&q| q >= lo)
        .collect();
    out.push(*at.last().unwrap_or(&0));
    out.dedup();
    out
}

/// Every list in document order: each paragraph, then the lists its
/// controls hold.
pub fn lists(doc: &Document) -> Vec<ListPath> {
    fn walk(paras: &[Paragraph], list: &ListPath, out: &mut Vec<ListPath>) {
        out.push(list.clone());
        for (pi, p) in paras.iter().enumerate() {
            for (ci, c) in p.controls.iter().enumerate() {
                for (k, inner) in hm_edit::ops::control_lists(c).into_iter().enumerate() {
                    let mut path = list.clone();
                    path.steps.push(Step {
                        para: pi as u32,
                        control: ci as u32,
                        index: k as u32,
                    });
                    walk(inner, &path, out);
                }
            }
        }
    }
    let mut out = Vec::new();
    for (si, s) in doc.sections.iter().enumerate() {
        let list = ListPath {
            section: si as u32,
            steps: Vec::new(),
        };
        walk(&s.paras, &list, &mut out);
    }
    out
}

fn fold(c: char, case: bool) -> char {
    if case {
        c
    } else {
        c.to_lowercase().next().unwrap_or(c)
    }
}

/// Matches of `needle` within single paragraphs, as `[from, to)`, in list
/// order, at most `limit`.
pub fn search(
    doc: &Document,
    needle: &str,
    case: bool,
    scope: Option<&ListPath>,
    limit: usize,
) -> Vec<(Pos, Pos)> {
    let needle: Vec<char> = needle.chars().map(|c| fold(c, case)).collect();
    let mut out = Vec::new();
    if needle.is_empty() {
        return out;
    }
    let all = lists(doc);
    let chosen = all.iter().filter(|l| scope.is_none_or(|s| *l == s));
    for list in chosen {
        let Some(paras) = hm_edit::ops::list(doc, list) else {
            continue;
        };
        for (pi, p) in paras.iter().enumerate() {
            let (s, at) = text(p);
            let hay: Vec<char> = s.chars().map(|c| fold(c, case)).collect();
            let mut k = 0;
            while k + needle.len() <= hay.len() {
                if hay[k..k + needle.len()] == needle[..] {
                    let pos = |i: usize| Pos {
                        list: list.clone(),
                        para: pi as u32,
                        offset: at[i],
                    };
                    let end = k + needle.len();
                    // The end is where the next unit starts.
                    let to = if end < at.len() - 1 {
                        pos(end)
                    } else {
                        Pos {
                            offset: *at.last().unwrap_or(&0),
                            ..pos(k)
                        }
                    };
                    out.push((pos(k), to));
                    if out.len() >= limit {
                        return out;
                    }
                    k = end;
                } else {
                    k += 1;
                }
            }
        }
    }
    out
}
