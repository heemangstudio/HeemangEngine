//! Footnote and endnote numbers in document order.
//!
//! Layout draws a note's reference mark and its own automatic number from
//! the numbers stored in the model, so an edit that adds, removes or
//! restarts notes renumbers the ones after it the way Hancom shows them.

use hm_edit::{ListPath, Op, Pos, Step, ops};
use hm_model::notes;
use hm_model::*;

/// Where a note control sits, for page-local numbering. Layout (which
/// knows the pages) fills these numbers in through [`renumber_with`].
pub struct NoteAt<'a> {
    pub list: &'a ListPath,
    pub para: u32,
    pub control: u32,
}

/// Walks a list in document order, nested lists at their control, and
/// collects replacements: a paragraph before the lists inside it, so a later
/// inner replacement lands on the replaced outer paragraph.
fn walk(
    doc: &Document,
    path: &ListPath,
    counter: &mut notes::NoteCounter,
    page_local: &dyn Fn(NoteAt<'_>) -> Option<u32>,
    out: &mut Vec<Op>,
) {
    let Some(paras) = ops::list(doc, path) else {
        return;
    };
    for (pi, p) in paras.iter().enumerate() {
        let mut fixed: Option<Paragraph> = None;
        let mut inner = Vec::new();
        for (_, u) in p.units() {
            let Unit::Extended { index, .. } = u else {
                continue;
            };
            let Some(c) = p.controls.get(index as usize) else {
                continue;
            };
            match &c.kind {
                ControlKind::NewNumber(n) => counter.new_number(n),
                ControlKind::Note(note) => {
                    let mut look = counter.next(note.endnote);
                    if look.number.is_none() {
                        look.number = page_local(NoteAt {
                            list: path,
                            para: pi as u32,
                            control: index,
                        });
                    }
                    if !look.shown_by(note) {
                        let q = fixed.get_or_insert_with(|| p.clone());
                        let c = &mut q.controls[index as usize];
                        if let ControlKind::Note(m) = &mut c.kind {
                            look.apply(m);
                            c.src = None;
                        }
                    }
                    // A note's own list holds no notes.
                    continue;
                }
                _ => {}
            }
            for i in 0..ops::control_lists(c).len() {
                let mut nested = path.clone();
                nested.steps.push(Step {
                    para: pi as u32,
                    control: index,
                    index: i as u32,
                });
                walk(doc, &nested, counter, page_local, &mut inner);
            }
        }
        if let Some(mut q) = fixed {
            q.src = None;
            q.line_segs.clear();
            out.push(Op::Replace {
                list: path.clone(),
                start: pi as u32,
                count: 1,
                paras: vec![q],
            });
        }
        out.append(&mut inner);
    }
}

/// Replacements that give every note the number and mirrored note shape
/// Hancom shows: each kind counts on across sections, restarts where a
/// section restarts numbering or a new-number control sets it. Footnotes
/// numbered per page keep their stored numbers.
pub fn renumber(doc: &Document) -> Vec<Op> {
    renumber_with(doc, &|_| None)
}

/// [`renumber`] with page-local numbers for footnotes numbered per page:
/// `page_local` gives the number layout shows for the note control at a
/// place, or `None` to keep the stored one.
pub fn renumber_with(doc: &Document, page_local: &dyn Fn(NoteAt<'_>) -> Option<u32>) -> Vec<Op> {
    let mut counter = notes::NoteCounter::new();
    let mut out = Vec::new();
    for si in 0..doc.sections.len() {
        counter.section(doc, si);
        let path = ListPath {
            section: si as u32,
            steps: vec![],
        };
        walk(doc, &path, &mut counter, page_local, &mut out);
    }
    out
}

/// Whether `c` is, or holds, a note or a note number restart.
pub fn counts(c: &Control) -> bool {
    match &c.kind {
        ControlKind::Note(_) => true,
        ControlKind::NewNumber(n) => matches!(n.kind(), NumberKind::Footnote | NumberKind::Endnote),
        _ => ops::control_lists(c)
            .iter()
            .any(|l| l.iter().any(|p| p.controls.iter().any(counts))),
    }
}

/// Whether `[from, to)` holds a note, a note number restart, or a control
/// with one inside.
pub fn in_range(doc: &Document, from: &Pos, to: &Pos) -> bool {
    let Some(paras) = ops::list(doc, &from.list) else {
        return false;
    };
    (from.para..=to.para).any(|pi| {
        let Some(p) = paras.get(pi as usize) else {
            return false;
        };
        p.units().any(|(offset, u)| {
            let Unit::Extended { index, .. } = u else {
                return false;
            };
            let after_start = pi > from.para || offset >= from.offset;
            let before_end = pi < to.para || offset < to.offset;
            after_start && before_end && p.controls.get(index as usize).is_some_and(counts)
        })
    })
}

/// Whether the control at `target` is one whose removal changes numbers.
pub fn is_counted(doc: &Document, list: &ListPath, para: u32, control: u32) -> bool {
    ops::list(doc, list)
        .and_then(|l| l.get(para as usize))
        .and_then(|p| p.controls.get(control as usize))
        .is_some_and(counts)
}

/// Whether `list` lies inside a footnote or endnote.
pub fn in_note(doc: &Document, list: &ListPath) -> bool {
    (0..list.steps.len()).any(|k| {
        let outer = ListPath {
            section: list.section,
            steps: list.steps[..k].to_vec(),
        };
        let step = &list.steps[k];
        ops::list(doc, &outer)
            .and_then(|l| l.get(step.para as usize))
            .and_then(|p| p.controls.get(step.control as usize))
            .is_some_and(|c| matches!(c.kind, ControlKind::Note(_)))
    })
}

/// The style a new note's paragraph takes, Hancom's 각주 (Footnote) or
/// 미주 (Endnote), found by name: `(style, paragraph shape, character
/// shape)`.
pub fn note_style(doc: &Document, endnote: bool) -> Option<(u8, u16, u32)> {
    let (local, english) = if endnote {
        ("미주", "Endnote")
    } else {
        ("각주", "Footnote")
    };
    let find = |f: &dyn Fn(&Style) -> bool| doc.info.styles.iter().position(f);
    let id = find(&|s| s.local_name.to_string_lossy() == local).or_else(|| {
        find(&|s| {
            s.english_name
                .to_string_lossy()
                .eq_ignore_ascii_case(english)
        })
    })?;
    let style = &doc.info.styles[id];
    Some((
        u8::try_from(id).ok()?,
        style.para_shape,
        style.char_shape as u32,
    ))
}

/// Layout's name for the list of the note control at `at`.
fn list_ref(at: &NoteAt<'_>) -> hm_layout::ListRef {
    let section = at.list.section;
    let Some((first, rest)) = at.list.steps.split_first() else {
        return hm_layout::ListRef::Control {
            section,
            para: at.para,
            control: at.control,
        };
    };
    // Each step's list index pairs with the paragraph and control the next
    // step (or the note itself) names inside it.
    let mut steps = Vec::with_capacity(at.list.steps.len());
    let mut cell = first.index;
    for s in rest {
        steps.push(hm_layout::CellStep {
            cell,
            para: s.para,
            control: s.control,
        });
        cell = s.index;
    }
    steps.push(hm_layout::CellStep {
        cell,
        para: at.para,
        control: at.control,
    });
    hm_layout::ListRef::Cell {
        section,
        para: first.para,
        control: first.control,
        steps: steps.into(),
    }
}

/// Replacements storing the page-local numbers `layout` shows for footnotes
/// numbered per page (empty when no section numbers them so, or they
/// already show them).
pub fn page_local(doc: &Document, layout: &hm_layout::DocLayout) -> Vec<Op> {
    if layout.sections.iter().all(|s| s.note_numbers.is_empty()) {
        return Vec::new();
    }
    renumber_with(doc, &|at| {
        let want = list_ref(&at);
        layout
            .sections
            .get(at.list.section as usize)?
            .note_numbers
            .iter()
            .find(|(l, _)| *l == want)
            .map(|(_, n)| *n)
    })
}
