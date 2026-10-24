//! Character counts as Hancom's status bar shows them.
//!
//! Every character of the text counts once per code point, spaces, tabs
//! and the two fixed spaces included; line breaks, paragraph ends, numbers
//! and objects do not. A field adds its end mark. The tables, text boxes,
//! captions, headers, footers and notes a paragraph holds add their text;
//! equations and master pages add nothing.

use hm_model::{Control, ControlKind, Paragraph, Unit};

fn counts_inside(c: &Control) -> bool {
    match &c.kind {
        ControlKind::Section(_) => false,
        ControlKind::Object(o) => o.equation().is_none(),
        _ => true,
    }
}

fn control(c: &Control) -> usize {
    if !counts_inside(c) {
        return 0;
    }
    hm_edit::ops::control_lists(c)
        .into_iter()
        .map(|l| paragraphs(l))
        .sum()
}

/// The count of `[from, to)` of a paragraph, with what its controls there hold.
pub fn paragraph(p: &Paragraph, from: u32, to: u32) -> usize {
    let mut n = 0;
    for (pos, u) in p.units() {
        if pos < from || pos >= to {
            continue;
        }
        n += match u {
            Unit::Char(_) => 1,
            // Tab, field end; no-break and fixed-width spaces.
            Unit::Inline(9 | 4) | Unit::CharControl(30 | 31) => 1,
            Unit::Extended { index, .. } => p.controls.get(index as usize).map_or(0, control),
            _ => 0,
        };
    }
    n
}

/// Every paragraph of a list.
pub fn paragraphs(list: &[Paragraph]) -> usize {
    list.iter().map(|p| paragraph(p, 0, p.len())).sum()
}

/// Paragraphs `a..=b` of a list from offset `ao` of `a` to `bo` of `b`.
pub fn range(list: &[Paragraph], a: u32, ao: u32, b: u32, bo: u32) -> usize {
    (a..=b)
        .filter_map(|i| list.get(i as usize).map(|p| (i, p)))
        .map(|(i, p)| {
            let s = if i == a { ao } else { 0 };
            let e = if i == b { bo } else { p.len() };
            paragraph(p, s, e)
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_characters_and_tabs_but_not_breaks() {
        let mut p = Paragraph::default();
        p.text = "a b\tc"
            .encode_utf16()
            .flat_map(|u| {
                if u == 9 {
                    vec![9, 0, 0, 0, 0, 0, 0, 9]
                } else {
                    vec![u]
                }
            })
            .chain([10, 0x41, 13])
            .collect();
        // "a", " ", "b", tab, "c", line break, "A", end.
        assert_eq!(paragraph(&p, 0, p.len()), 6);
        assert_eq!(paragraph(&p, 0, 2), 2);
    }
}
