//! Footnote and endnote numbers in document order.
//!
//! Layout draws a note's reference mark and its own automatic number from
//! the numbers stored in the model, so an edit that adds, removes or
//! restarts notes renumbers the ones after it the way Hancom shows them.

use hm_edit::{ListPath, Op, Pos, Step, ops};
use hm_model::notes;
use hm_model::*;

/// Walks a list in document order, nested lists at their control, and
/// collects replacements: a paragraph before the lists inside it, so a later
/// inner replacement lands on the replaced outer paragraph.
fn walk(doc: &Document, path: &ListPath, counter: &mut notes::NoteCounter, out: &mut Vec<Op>) {
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
                    let look = counter.next(note.endnote);
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

/// Replacements that give every note the number and mirrored note shape
/// Hancom shows: each kind counts on across sections, restarts where a
/// section restarts numbering or a new-number control sets it. Footnotes
/// numbered per page keep their stored numbers ([`page_local`] stores what
/// layout shows).
pub fn renumber(doc: &Document) -> Vec<Op> {
    let mut counter = notes::NoteCounter::new();
    let mut out = Vec::new();
    for si in 0..doc.sections.len() {
        counter.section(doc, si);
        let path = ListPath {
            section: si as u32,
            steps: vec![],
        };
        walk(doc, &path, &mut counter, &mut out);
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

/// Whether `list` lies inside a footnote or endnote.
pub fn in_note(doc: &Document, list: &ListPath) -> bool {
    inside(doc, list, &|c| matches!(c.kind, ControlKind::Note(_)))
}

/// Whether `list` lies inside a note, a header or footer, or a master page,
/// where Hancom inserts no notes.
pub fn in_furniture(doc: &Document, list: &ListPath) -> bool {
    inside(doc, list, &|c| {
        matches!(
            c.kind,
            ControlKind::Note(_) | ControlKind::HeaderFooter(_) | ControlKind::Section(_)
        )
    })
}

fn inside(doc: &Document, list: &ListPath, is: &dyn Fn(&Control) -> bool) -> bool {
    (0..list.steps.len()).any(|k| {
        let outer = ListPath {
            section: list.section,
            steps: list.steps[..k].to_vec(),
        };
        let step = &list.steps[k];
        ops::list(doc, &outer)
            .and_then(|l| l.get(step.para as usize))
            .and_then(|p| p.controls.get(step.control as usize))
            .is_some_and(is)
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
    named_style(doc, local, english)
}

/// The style Hancom gives the paragraph of a new header or footer: 머리말
/// (Header), which footers take too.
pub fn header_style(doc: &Document) -> Option<(u8, u16, u32)> {
    named_style(doc, "머리말", "Header")
}

/// A style by its local or English name: its ID and shapes.
fn named_style(doc: &Document, local: &str, english: &str) -> Option<(u8, u16, u32)> {
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

/// Where layout's list of a note lives: the list holding the note control,
/// its paragraph and control.
fn note_place(r: &hm_layout::ListRef) -> Option<(ListPath, u32, u32)> {
    match r {
        hm_layout::ListRef::Control {
            section,
            para,
            control,
        } => Some((
            ListPath {
                section: *section,
                steps: vec![],
            },
            *para,
            *control,
        )),
        hm_layout::ListRef::Cell {
            section,
            para,
            control,
            steps,
        } => {
            let (last, _) = steps.split_last()?;
            let mut path = ListPath {
                section: *section,
                steps: vec![],
            };
            let (mut p, mut c) = (*para, *control);
            for s in steps.iter() {
                path.steps.push(Step {
                    para: p,
                    control: c,
                    index: s.cell,
                });
                (p, c) = (s.para, s.control);
            }
            Some((path, last.para, last.control))
        }
        _ => None,
    }
}

/// Replacements storing the page-local numbers `layout` shows for footnotes
/// numbered per page, for just the notes whose stored number differs.
/// Nothing to do, at no cost, when no section numbers footnotes per page.
pub fn page_local(doc: &Document, layout: &hm_layout::DocLayout) -> Vec<Op> {
    // Host paragraphs being fixed, each with whether a number changed.
    let mut fixed: Vec<(ListPath, u32, Paragraph, bool)> = Vec::new();
    for section in &layout.sections {
        for (r, n) in &section.note_numbers {
            let Some((list, para, control)) = note_place(r) else {
                continue;
            };
            let k = match fixed.iter().position(|(l, p, ..)| *l == list && *p == para) {
                Some(k) => k,
                None => {
                    let Some(p) = ops::list(doc, &list).and_then(|l| l.get(para as usize)) else {
                        continue;
                    };
                    fixed.push((list, para, p.clone(), false));
                    fixed.len() - 1
                }
            };
            let (_, _, host, changed) = &mut fixed[k];
            let Some(c) = host.controls.get_mut(control as usize) else {
                continue;
            };
            let ControlKind::Note(note) = &mut c.kind else {
                continue;
            };
            // The number only; the mirrored shape stays as stored.
            let look = notes::NoteLook {
                number: Some(*n),
                mirror: None,
            };
            if !look.shown_by(note) {
                look.apply(note);
                c.src = None;
                *changed = true;
            }
        }
    }
    fixed
        .into_iter()
        .filter(|f| f.3)
        .map(|(list, para, mut p, _)| {
            p.src = None;
            p.line_segs.clear();
            Op::Replace {
                list,
                start: para,
                count: 1,
                paras: vec![p],
            }
        })
        .collect()
}

fn max_inst_id(paras: &[Paragraph], max: &mut u32) {
    for p in paras {
        for c in &p.controls {
            if let ControlKind::Note(n) = &c.kind {
                *max = (*max).max(n.inst_id.unwrap_or(0));
            }
            for list in ops::control_lists(c) {
                max_inst_id(list, max);
            }
        }
    }
}

/// An instance id no note in `doc` has.
pub fn fresh_inst_id(doc: &Document) -> u32 {
    let mut max = 0;
    for s in &doc.sections {
        max_inst_id(&s.paras, &mut max);
    }
    max.wrapping_add(1).max(1)
}

/// Readies a pasted control: every note in it, its own or nested, becomes a
/// note of its own with an instance id from `next`; pasted into a note
/// (`into_note`), the nested note marks are dropped and their text kept.
pub fn renew_pasted(c: &mut Control, next: &mut u32, into_note: bool) {
    if let ControlKind::Note(n) = &mut c.kind {
        n.inst_id = Some(*next);
        *next = next.wrapping_add(1).max(1);
        c.src = None;
        return;
    }
    let mut touched = false;
    for list in notes::lists_mut(c) {
        for p in list.iter_mut() {
            if into_note {
                let marks: Vec<u32> = p
                    .units()
                    .filter_map(|(offset, u)| match u {
                        Unit::Extended { index, .. }
                            if matches!(
                                p.controls.get(index as usize).map(|c| &c.kind),
                                Some(ControlKind::Note(_))
                            ) =>
                        {
                            Some(offset)
                        }
                        _ => None,
                    })
                    .collect();
                for &offset in marks.iter().rev() {
                    ops::remove_units(p, offset, offset + 8);
                }
                if !marks.is_empty() {
                    p.src = None;
                    p.line_segs.clear();
                    touched = true;
                }
            }
            for inner in &mut p.controls {
                if counts(inner) {
                    renew_pasted(inner, next, into_note);
                    p.src = None;
                    touched = true;
                }
            }
        }
    }
    if touched {
        c.src = None;
    }
}
