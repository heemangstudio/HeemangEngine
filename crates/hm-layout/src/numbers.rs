//! Text that controls generate in a line: automatic numbers and note
//! reference marks.

use hm_model::{Control, ControlKind, NumberKind};

use crate::heads::number;

/// How generated text is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Look {
    Plain,
    Superscript,
    /// A note's reference mark.
    NoteMark,
}

/// Generated text of a control, and how it is drawn. `page` is the page
/// being laid out (0 when unknown).
pub(crate) fn generated(c: &Control, page: u32) -> Option<(String, Look)> {
    generated_with_total(c, page, 0)
}

pub(crate) fn generated_with_total(c: &Control, page: u32, total: u32) -> Option<(String, Look)> {
    let ch = |c: u16| char::from_u32(c as u32).filter(|c| *c != '\0');
    match &c.kind {
        ControlKind::AutoNumber(a) => {
            let n = match a.kind() {
                NumberKind::Page if page > 0 => page,
                NumberKind::Other(6) if total > 0 => total,
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
            let look = if a.superscript() {
                Look::Superscript
            } else {
                Look::Plain
            };
            Some((s, look))
        }
        // The reference mark of a note: its number with the note's
        // decoration.
        ControlKind::Note(n) => {
            let mut s = String::new();
            s.extend(ch(n.prefix));
            s.push_str(&n.number.to_string());
            s.extend(ch(n.suffix));
            Some((s, Look::NoteMark))
        }
        _ => None,
    }
}

/// The text Hancom stored for an automatic page number, which it lays its
/// line out with whatever page shows it.
pub(crate) fn stored_page_number(c: &Control) -> Option<(String, Look)> {
    match &c.kind {
        ControlKind::AutoNumber(a) if a.kind() == NumberKind::Page => generated(c, 0),
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
        assert_eq!(generated(&c, 3), Some(("3".to_string(), Look::Plain)));
        assert_eq!(generated(&c, 0), Some(("7".to_string(), Look::Plain)));
        let note = Control {
            kind: ControlKind::Note(Box::new(hm_model::Note {
                number: 2,
                suffix: ')' as u16,
                ..Default::default()
            })),
            src: None,
        };
        assert_eq!(
            generated(&note, 3),
            Some(("2)".to_string(), Look::NoteMark))
        );
    }
}

/// Total-page counters need a second pass after the body page count is known.
pub(crate) fn has_total(doc: &hm_model::Document) -> bool {
    fn paras(ps: &[hm_model::Paragraph]) -> bool {
        ps.iter().flat_map(|p| &p.controls).any(|c| match &c.kind {
            ControlKind::AutoNumber(n) => n.kind() == NumberKind::Other(6),
            ControlKind::Table(t) => {
                t.cells.iter().any(|c| paras(&c.paras))
                    || t.caption.as_ref().is_some_and(|c| paras(&c.paras))
            }
            ControlKind::HeaderFooter(h) => paras(&h.list.paras),
            ControlKind::Note(n) => paras(&n.list.paras),
            ControlKind::Object(o) => o.parts.iter().any(|p| match p {
                hm_model::ObjectPart::Caption(c) => paras(&c.paras),
                hm_model::ObjectPart::Shape(s) => shape(s),
                _ => false,
            }),
            _ => false,
        })
    }
    fn shape(s: &hm_model::Shape) -> bool {
        s.parts.iter().any(|p| match p {
            hm_model::ShapePart::Text(t) => paras(&t.paras),
            hm_model::ShapePart::Child(s) => shape(s),
            _ => false,
        })
    }
    doc.sections.iter().any(|s| paras(&s.paras))
}
