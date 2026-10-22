//! Text that controls generate in a line: automatic numbers and note
//! reference marks.

use hm_model::{Control, ControlKind, NumberKind};

use crate::heads::number;

/// Generated text of a control, and whether it is drawn as superscript.
/// `page` is the page being laid out (0 when unknown).
pub(crate) fn generated(c: &Control, page: u32) -> Option<(String, bool)> {
    let ch = |c: u16| char::from_u32(c as u32).filter(|c| *c != '\0');
    match &c.kind {
        ControlKind::AutoNumber(a) => {
            let n = match a.kind() {
                NumberKind::Page if page > 0 => page,
                _ => a.number as u32,
            };
            let mut s = String::new();
            s.extend(ch(a.prefix));
            // Shape 0x81 repeats the user's character n times.
            match a.shape() {
                0x81 => {
                    if let Some(u) = ch(a.user_char) {
                        s.extend(std::iter::repeat_n(u, n.clamp(1, 8) as usize));
                    }
                }
                shape => s.push_str(&number(n, shape)),
            }
            s.extend(ch(a.suffix));
            Some((s, a.superscript()))
        }
        // The reference mark of a note: its number with the note's
        // decoration, raised.
        ControlKind::Note(n) => {
            let mut s = String::new();
            s.extend(ch(n.prefix));
            s.push_str(&n.number.to_string());
            s.extend(ch(n.suffix));
            Some((s, true))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_numbers_follow_the_page() {
        let c = Control {
            kind: ControlKind::AutoNumber(hm_model::AutoNumber {
                attr: 0,
                number: 7,
                ..Default::default()
            }),
            src: None,
        };
        assert_eq!(generated(&c, 3), Some(("3".to_string(), false)));
        assert_eq!(generated(&c, 0), Some(("7".to_string(), false)));
        let note = Control {
            kind: ControlKind::Note(Box::new(hm_model::Note {
                number: 2,
                suffix: ')' as u16,
                ..Default::default()
            })),
            src: None,
        };
        assert_eq!(generated(&note, 3), Some(("2)".to_string(), true)));
    }
}
