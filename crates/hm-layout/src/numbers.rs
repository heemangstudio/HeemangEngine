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

/// How a section draws the numbers of its notes' reference marks: the
/// note shape's number shape and user character.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct MarkFormat {
    pub shape: u32,
    pub user_char: u16,
}

impl MarkFormat {
    pub(crate) fn of(shape: &hm_model::NoteShape) -> Self {
        Self {
            shape: shape.attr & 0xFF,
            user_char: shape.user_char,
        }
    }
}

/// What generated text depends on besides its control.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Context<'a> {
    /// The page being laid out (0 when unknown).
    pub page: u32,
    pub total: u32,
    /// Mark formats of the section's footnotes and endnotes.
    pub marks: [MarkFormat; 2],
    /// Numbers notes show instead of the stored one, by [`key`].
    pub numbers: Option<&'a std::collections::HashMap<usize, u32>>,
    /// The number of the note whose list is being laid out, when it shows
    /// another than the stored one.
    pub note: Option<u32>,
}

/// What a note is known by within one layout: its address.
pub(crate) fn key(note: &hm_model::Note) -> usize {
    note as *const hm_model::Note as usize
}

/// `n` in number shape `shape`: shape 0x81 repeats the user's character
/// `n` times.
fn formatted(n: u32, shape: u32, user_char: u16) -> String {
    let ch = |c: u16| char::from_u32(c as u32).filter(|c| *c != '\0');
    match shape {
        0x81 => ch(user_char)
            .map(|u| std::iter::repeat_n(u, n.clamp(1, 8) as usize).collect())
            .unwrap_or_default(),
        shape => number(n, shape),
    }
}

/// Generated text of a control, and how it is drawn. `page` is the page
/// being laid out (0 when unknown).
pub(crate) fn generated(c: &Control, page: u32) -> Option<(String, Look)> {
    generated_in(
        c,
        &Context {
            page,
            ..Default::default()
        },
    )
}

pub(crate) fn generated_in(c: &Control, cx: &Context<'_>) -> Option<(String, Look)> {
    let ch = |c: u16| char::from_u32(c as u32).filter(|c| *c != '\0');
    match &c.kind {
        ControlKind::AutoNumber(a) => {
            let n = match a.kind() {
                NumberKind::Page if cx.page > 0 => cx.page,
                NumberKind::Other(6) if cx.total > 0 => cx.total,
                NumberKind::Footnote if cx.note.is_some() => cx.note.unwrap_or(0),
                _ => a.number as u32,
            };
            let mut s = String::new();
            s.extend(ch(a.prefix));
            s.push_str(&formatted(n, a.shape(), a.user_char));
            s.extend(ch(a.suffix));
            // A note's own number set as superscript takes the size and
            // raise of the note's reference mark.
            let look = match a.kind() {
                _ if !a.superscript() => Look::Plain,
                NumberKind::Footnote | NumberKind::Endnote => Look::NoteMark,
                _ => Look::Superscript,
            };
            Some((s, look))
        }
        // The reference mark of a note: its number in the section's note
        // shape, with the note's decoration.
        ControlKind::Note(n) => {
            let f = cx.marks[n.endnote as usize];
            let number = cx
                .numbers
                .and_then(|m| m.get(&key(n)).copied())
                .unwrap_or(n.number);
            let mut s = String::new();
            s.extend(ch(n.prefix));
            s.push_str(&formatted(number, f.shape, f.user_char));
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
            ControlKind::Section(s) => s.children.iter().any(|c| match c {
                hm_model::SectionChild::MasterPage(list) => paras(&list.paras),
                _ => false,
            }),
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
    doc.sections.iter().any(|s| {
        paras(&s.paras)
            || s.last_page_master
                .as_ref()
                .is_some_and(|list| paras(&list.paras))
            || s.page_masters.iter().any(|(_, list)| paras(&list.paras))
    })
}

#[cfg(test)]
mod master_counter_tests {
    use super::*;
    use hm_model::*;
    #[test]
    fn total_page_fields_are_detected_in_each_master_source_list() {
        let counter = Control {
            kind: ControlKind::AutoNumber(AutoNumber {
                attr: 6,
                number: 1,
                ..Default::default()
            }),
            src: None,
        };
        let cx = Context {
            page: 3,
            total: 17,
            ..Default::default()
        };
        assert_eq!(
            generated_in(&counter, &cx),
            Some(("17".into(), Look::Plain))
        );
        let list = ParaList {
            paras: vec![Paragraph {
                controls: vec![counter],
                ..Default::default()
            }],
            ..Default::default()
        };
        for kind in 0..3 {
            let mut section = Section::default();
            match kind {
                0 => section.paras.push(Paragraph {
                    controls: vec![Control {
                        kind: ControlKind::Section(Box::new(SectionDef {
                            children: vec![SectionChild::MasterPage(list.clone())],
                            ..Default::default()
                        })),
                        src: None,
                    }],
                    ..Default::default()
                }),
                1 => section.page_masters.push((1, list.clone())),
                _ => section.last_page_master = Some(list.clone()),
            }
            let doc = Document {
                meta: DocMeta {
                    format: Format::Hwp5,
                    version: Version(0x05000300),
                    flags: 0,
                },
                info: DocInfo::default(),
                sections: vec![section],
                source: None,
                bins: None,
                diagnostics: vec![],
            };
            assert!(has_total(&doc), "master source {kind}");
        }
    }
}
