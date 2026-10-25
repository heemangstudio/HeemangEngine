//! Footnote and endnote numbers in document order.
//!
//! Layout draws a note's reference mark and its own automatic number from
//! the numbers stored in the model, so an edit that adds, removes or
//! restarts notes renumbers the ones after it the way Hancom shows them.

use hm_edit::{ListPath, Op, Pos, Step, ops};
use hm_model::*;

/// The first number of each kind (footnote, endnote) in a section, and
/// whether the section counts per page (then its stored numbers stay).
fn section_start(doc: &Document, section: &Section, kind: usize) -> (u32, bool) {
    let shape = section.definition().and_then(|d| d.note_shapes().nth(kind));
    let fallback = if kind == 1 {
        doc.info.properties.endnote_start
    } else {
        doc.info.properties.footnote_start
    };
    let start = shape
        .map(|s| s.start)
        .filter(|n| *n != 0)
        .unwrap_or(fallback.max(1));
    let mode = shape.map_or(0, |s| (s.attr >> 10) & 3);
    (start as u32, mode == 2)
}

/// Whether the note and its own automatic number already show `n`.
fn shows(note: &Note, n: u32) -> bool {
    note.number == n
        && note.list.paras.first().is_none_or(|p| {
            p.controls.iter().all(|c| match &c.kind {
                ControlKind::AutoNumber(a)
                    if matches!(a.kind(), NumberKind::Footnote | NumberKind::Endnote) =>
                {
                    a.number as u32 == n
                }
                _ => true,
            })
        })
}

fn number(note: &mut Note, n: u32) {
    note.number = n;
    if let Some(p) = note.list.paras.first_mut() {
        for c in &mut p.controls {
            if let ControlKind::AutoNumber(a) = &mut c.kind
                && matches!(a.kind(), NumberKind::Footnote | NumberKind::Endnote)
            {
                a.number = n.min(u16::MAX as u32) as u16;
            }
        }
    }
}

struct Counter {
    /// The next number of each kind; `None` while a section counts per page.
    next: [Option<u32>; 2],
}

/// Walks a list in document order, nested lists at their control, and
/// collects replacements: a paragraph before the lists inside it, so a later
/// inner replacement lands on the replaced outer paragraph.
fn walk(doc: &Document, path: &ListPath, counter: &mut Counter, out: &mut Vec<Op>) {
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
                ControlKind::NewNumber(n) => match n.kind() {
                    NumberKind::Footnote => counter.next[0] = Some(n.number as u32),
                    NumberKind::Endnote => counter.next[1] = Some(n.number as u32),
                    _ => {}
                },
                ControlKind::Note(note) => {
                    let kind = usize::from(note.endnote);
                    if let Some(n) = counter.next[kind] {
                        counter.next[kind] = Some(n + 1);
                        if !shows(note, n) {
                            let q = fixed.get_or_insert_with(|| p.clone());
                            if let ControlKind::Note(m) = &mut q.controls[index as usize].kind {
                                number(m, n);
                            }
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
                walk(doc, &nested, counter, &mut inner);
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

/// Replacements that give every note the number Hancom would show: each
/// kind counts on across sections from the first section's start, restarts
/// where a section restarts numbering or a new-number control sets it.
/// Sections numbered per page keep their stored numbers.
pub fn renumber(doc: &Document) -> Vec<Op> {
    let mut counter = Counter { next: [None; 2] };
    let mut out = Vec::new();
    for (si, section) in doc.sections.iter().enumerate() {
        let restarts = section
            .definition()
            .map(|d| {
                d.note_shapes()
                    .map(|s| (s.attr >> 10) & 3 == 1)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for kind in 0..2 {
            let (start, per_page) = section_start(doc, section, kind);
            counter.next[kind] = if per_page {
                None
            } else if si == 0 || restarts.get(kind).copied().unwrap_or(false) {
                Some(start)
            } else {
                counter.next[kind].or(Some(start))
            };
        }
        let path = ListPath {
            section: si as u32,
            steps: vec![],
        };
        walk(doc, &path, &mut counter, &mut out);
    }
    out
}

fn counts(c: &Control) -> bool {
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
