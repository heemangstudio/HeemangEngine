//! Edit operations on the model and their inverses.
//!
//! Paragraph operations change a contiguous run in one list; their inverse
//! restores that run as it was ([`Op::Replace`]). Definition edits instead
//! replace document tables and keep their previous tables as the inverse.
//! Changed paragraphs lose their source bytes and stored line segments, so
//! writers re-encode them and layout breaks their lines afresh; the lists
//! and controls holding them lose their source bytes too.

use hm_model::{
    CharRun, Control, ControlKind, Document, ObjectPart, PARA_END, Paragraph, RangeTag,
    SectionChild, Shape, ShapePart, TrackMark, Unit, control_width,
};

/// One step from a paragraph into a paragraph list one of its controls
/// holds: the paragraph, the control's index among its controls, and the
/// list's index among the control's lists (see [`control_lists`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    pub para: u32,
    pub control: u32,
    pub index: u32,
}

/// A paragraph list: a section's body, or a list reached from it through
/// controls (table cells and captions, text boxes, headers, footers,
/// notes, master pages), nested to any depth.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListPath {
    pub section: u32,
    pub steps: Vec<Step>,
}

/// A wire position in a paragraph of a list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pos {
    pub list: ListPath,
    pub para: u32,
    pub offset: u32,
}

#[derive(Clone, Debug)]
pub enum Op {
    /// Replaces document definitions as an undoable operation.
    SetDocInfo {
        info: Box<hm_model::DocInfo>,
    },
    SetProperties {
        properties: hm_model::DocProperties,
    },
    /// Inserts characters; `'\n'` splits the paragraph, `'\t'` is a tab,
    /// U+2028 a line break, U+00A0 and U+2007 the two fixed spaces.
    InsertText {
        at: Pos,
        text: String,
    },
    /// Deletes `[from, to)`, joining paragraphs when the range spans them.
    Delete {
        from: Pos,
        to: Pos,
    },
    /// Splits a paragraph; the new one starts on a new page when asked.
    Split {
        at: Pos,
        page_break: bool,
    },
    /// Applies a char shape to `[from, to)`.
    SetCharShape {
        from: Pos,
        to: Pos,
        shape: u32,
    },
    /// Applies a para shape to paragraphs `first..=last` of a list.
    SetParaShape {
        list: ListPath,
        first: u32,
        last: u32,
        shape: u16,
    },
    /// Gives paragraphs `first..=last` of a list a style with its para
    /// shape and, when set, one char shape for all their text.
    SetStyle {
        list: ListPath,
        first: u32,
        last: u32,
        style: u8,
        para_shape: u16,
        char_shape: Option<u32>,
    },
    /// Sets the page break before a paragraph, or clears the page or column break it
    /// starts with.
    SetPageBreak {
        list: ListPath,
        para: u32,
        on: bool,
    },
    /// Replaces `count` paragraphs at `start` with `paras` (inverse of the
    /// others).
    Replace {
        list: ListPath,
        start: u32,
        count: u32,
        paras: Vec<Paragraph>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditError {
    NoSuchList,
    NoSuchParagraph,
    /// An offset past the paragraph end or inside a control's units.
    BadOffset,
    /// A range whose ends are in different lists or reversed.
    BadRange,
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            EditError::NoSuchList => "no such paragraph list",
            EditError::NoSuchParagraph => "no such paragraph",
            EditError::BadOffset => "offset outside the text or inside a control",
            EditError::BadRange => "range ends in different lists or reversed",
        })
    }
}

impl std::error::Error for EditError {}

/// The paragraph lists a control holds, in order: a table's cells then
/// its caption; an object's text boxes (group members depth first) then
/// its caption; a header's, footer's or note's list; a section
/// definition's master pages.
pub fn control_lists(c: &Control) -> Vec<&Vec<Paragraph>> {
    let mut out = Vec::new();
    match &c.kind {
        ControlKind::Table(t) => {
            out.extend(t.cells.iter().map(|c| &c.paras));
            out.extend(t.caption.iter().map(|c| &c.paras));
        }
        ControlKind::Object(o) => {
            fn walk<'a>(s: &'a Shape, out: &mut Vec<&'a Vec<Paragraph>>) {
                for p in &s.parts {
                    match p {
                        ShapePart::Text(t) => out.push(&t.paras),
                        ShapePart::Child(c) => walk(c, out),
                        _ => {}
                    }
                }
            }
            if let Some(s) = o.shape() {
                walk(s, &mut out);
            }
            out.extend(o.caption().map(|c| &c.paras));
        }
        ControlKind::HeaderFooter(h) => out.push(&h.list.paras),
        ControlKind::Note(n) => out.push(&n.list.paras),
        ControlKind::Section(s) => out.extend(s.children.iter().filter_map(|c| match c {
            SectionChild::MasterPage(l) => Some(&l.paras),
            _ => None,
        })),
        _ => {}
    }
    out
}

/// List `k` of [`control_lists`], writable.
fn control_list_mut(c: &mut Control, k: usize) -> Option<&mut Vec<Paragraph>> {
    let mut lists: Vec<&mut Vec<Paragraph>> = Vec::new();
    match &mut c.kind {
        ControlKind::Table(t) => {
            lists.extend(t.cells.iter_mut().map(|c| &mut c.paras));
            lists.extend(t.caption.iter_mut().map(|c| &mut c.paras));
        }
        ControlKind::Object(o) => {
            fn walk<'a>(s: &'a mut Shape, out: &mut Vec<&'a mut Vec<Paragraph>>) {
                for p in &mut s.parts {
                    match p {
                        ShapePart::Text(t) => out.push(&mut t.paras),
                        ShapePart::Child(c) => walk(c, out),
                        _ => {}
                    }
                }
            }
            let (mut shape, mut caption) = (false, None);
            for part in &mut o.parts {
                match part {
                    ObjectPart::Shape(s) if !shape => {
                        shape = true;
                        walk(s, &mut lists);
                    }
                    ObjectPart::Caption(c) if caption.is_none() => caption = Some(&mut c.paras),
                    _ => {}
                }
            }
            lists.extend(caption);
        }
        ControlKind::HeaderFooter(h) => lists.push(&mut h.list.paras),
        ControlKind::Note(n) => lists.push(&mut n.list.paras),
        ControlKind::Section(s) => lists.extend(s.children.iter_mut().filter_map(|c| match c {
            SectionChild::MasterPage(l) => Some(&mut l.paras),
            _ => None,
        })),
        _ => {}
    }
    lists.into_iter().nth(k)
}

/// The paragraph list at `path`, marking the containers on the way dirty.
pub fn list_mut<'a>(
    doc: &'a mut Document,
    path: &ListPath,
) -> Result<&'a mut Vec<Paragraph>, EditError> {
    let sec = doc
        .sections
        .get_mut(path.section as usize)
        .ok_or(EditError::NoSuchList)?;
    let mut list = &mut sec.paras;
    for step in &path.steps {
        let control = list
            .get_mut(step.para as usize)
            .and_then(|p| p.controls.get_mut(step.control as usize))
            .ok_or(EditError::NoSuchList)?;
        control.src = None;
        list = control_list_mut(control, step.index as usize).ok_or(EditError::NoSuchList)?;
    }
    Ok(list)
}

/// The paragraph list at `path`, read only.
pub fn list<'a>(doc: &'a Document, path: &ListPath) -> Option<&'a Vec<Paragraph>> {
    let mut list = &doc.sections.get(path.section as usize)?.paras;
    for step in &path.steps {
        let c = list
            .get(step.para as usize)?
            .controls
            .get(step.control as usize)?;
        list = control_lists(c).into_iter().nth(step.index as usize)?;
    }
    Some(list)
}

/// Source spans outside an operation's replaced paragraphs. Restoring its
/// inverse alone would leave these containers dirty after a failed batch.
#[derive(Clone)]
pub(crate) struct Sources {
    path: ListPath,
    info_only: bool,
    section: Option<hm_model::SharedBytes>,
    controls: Vec<Option<hm_model::SharedBytes>>,
}

impl Sources {
    pub(crate) fn capture(doc: &Document, op: &Op) -> Result<Self, EditError> {
        if matches!(op, Op::SetDocInfo { .. }) {
            return Ok(Self {
                path: ListPath::default(),
                info_only: true,
                section: None,
                controls: Vec::new(),
            });
        }
        let (path, _, _) = span(doc, op)?;
        let section = doc
            .sections
            .get(path.section as usize)
            .ok_or(EditError::NoSuchList)?;
        let mut list = &section.paras;
        let mut controls = Vec::new();
        for step in &path.steps {
            let c = list
                .get(step.para as usize)
                .and_then(|p| p.controls.get(step.control as usize))
                .ok_or(EditError::NoSuchList)?;
            controls.push(c.src.clone());
            list = control_lists(c)
                .into_iter()
                .nth(step.index as usize)
                .ok_or(EditError::NoSuchList)?;
        }
        Ok(Self {
            info_only: false,
            path,
            section: section.src.clone(),
            controls,
        })
    }

    pub(crate) fn restore(self, doc: &mut Document) {
        if self.info_only {
            return;
        }
        let section = &mut doc.sections[self.path.section as usize];
        section.src = self.section;
        let mut list = &mut section.paras;
        for (step, src) in self.path.steps.iter().zip(self.controls) {
            let c = &mut list[step.para as usize].controls[step.control as usize];
            c.src = src;
            list = control_list_mut(c, step.index as usize).expect("rolled back list");
        }
    }
}

/// Applies `op` and returns its inverse.
pub fn apply(doc: &mut Document, op: &Op) -> Result<Op, EditError> {
    if let Op::SetDocInfo { info } = op {
        return Ok(Op::SetDocInfo {
            info: Box::new(std::mem::replace(&mut doc.info, *info.clone())),
        });
    }
    if let Op::SetProperties { properties } = op {
        return Ok(Op::SetProperties {
            properties: std::mem::replace(&mut doc.info.properties, properties.clone()),
        });
    }
    let (list_path, start, count) = span(doc, op)?;
    let section_changes = list_path.steps.is_empty();
    let old: Vec<Paragraph> = {
        let l = list(doc, &list_path).ok_or(EditError::NoSuchList)?;
        let end = start as usize + count as usize;
        if end > l.len() {
            return Err(EditError::NoSuchParagraph);
        }
        l[start as usize..end].to_vec()
    };
    let new = edited(&old, op, start)?;
    let new_count = new.len() as u32;
    let l = list_mut(doc, &list_path)?;
    l.splice(start as usize..(start + count) as usize, new);
    if section_changes && (new_count != count || matches!(op, Op::Replace { .. })) {
        doc.sections[list_path.section as usize].src = None;
    }
    Ok(Op::Replace {
        list: list_path,
        start,
        count: new_count,
        paras: old,
    })
}

/// A field marker's identity that survives edits around it: the begin's instance id or the
/// end's field id (its first two information words).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldMark {
    Begin(u32),
    End(u32),
}

fn field_mark(p: &Paragraph, offset: u32, unit: Unit) -> Option<FieldMark> {
    match unit {
        Unit::Extended { code: 3, index } => match &p.controls.get(index as usize)?.kind {
            ControlKind::Field(f) => Some(FieldMark::Begin(f.instance)),
            _ => None,
        },
        Unit::Inline(4) => {
            let w = |k: u32| p.text.get((offset + k) as usize).copied().unwrap_or(0) as u32;
            Some(FieldMark::End(w(1) | w(2) << 16))
        }
        _ => None,
    }
}

/// The field markers of `list` with their positions, and for each its partner's index
/// when it has one. Pairs nest across the list's paragraphs.
fn field_pairs(list: &[Paragraph]) -> Vec<((u32, u32), FieldMark, Option<usize>)> {
    let mut out: Vec<((u32, u32), FieldMark, Option<usize>)> = Vec::new();
    let mut open = Vec::new();
    for (pi, p) in list.iter().enumerate() {
        for (offset, unit) in p.units() {
            let Some(mark) = field_mark(p, offset, unit) else {
                continue;
            };
            let k = out.len();
            out.push(((pi as u32, offset), mark, None));
            match mark {
                FieldMark::Begin(_) => open.push(k),
                FieldMark::End(_) => {
                    if let Some(b) = open.pop() {
                        out[b].2 = Some(k);
                        out[k].2 = Some(b);
                    }
                }
            }
        }
    }
    out
}

/// The markers outside `[from, to)` of fields the range cuts in two: deleting the range
/// leaves them without a partner. Hancom drops such a field whole and keeps its text.
pub fn cut_field_partners(list: &[Paragraph], from: &Pos, to: &Pos) -> Vec<FieldMark> {
    let inside = |q: (u32, u32)| q >= (from.para, from.offset) && q < (to.para, to.offset);
    let pairs = field_pairs(list);
    pairs
        .iter()
        .filter_map(|(at, mark, partner)| {
            let other = pairs[(*partner)?].0;
            (!inside(*at) && inside(other)).then_some(*mark)
        })
        .collect()
}

/// Positions (paragraph, offset) of the markers in `list` that have no partner and whose
/// identity is one of `marks`.
pub fn unmatched_field_marks(list: &[Paragraph], marks: &[FieldMark]) -> Vec<(u32, u32)> {
    field_pairs(list)
        .into_iter()
        .filter(|(_, mark, partner)| partner.is_none() && marks.contains(mark))
        .map(|(at, _, _)| at)
        .collect()
}

/// The list and paragraph range an operation touches.
fn span(doc: &Document, op: &Op) -> Result<(ListPath, u32, u32), EditError> {
    let one = |p: &Pos| (p.list.clone(), p.para, 1);
    Ok(match op {
        Op::SetDocInfo { .. } => unreachable!("definitions do not touch paragraphs"),
        Op::SetProperties { .. } => (ListPath::default(), 0, 0),
        Op::InsertText { at, .. } | Op::Split { at, .. } => one(at),
        Op::Delete { from, to } | Op::SetCharShape { from, to, .. } => {
            if from.list != to.list || (to.para, to.offset) < (from.para, from.offset) {
                return Err(EditError::BadRange);
            }
            (from.list.clone(), from.para, to.para - from.para + 1)
        }
        Op::SetParaShape {
            list, first, last, ..
        }
        | Op::SetStyle {
            list, first, last, ..
        } => {
            if last < first {
                return Err(EditError::BadRange);
            }
            (list.clone(), *first, last - first + 1)
        }
        Op::SetPageBreak { list, para, .. } => (list.clone(), *para, 1),
        Op::Replace {
            list: l,
            start,
            count,
            ..
        } => {
            let _ = doc;
            (l.clone(), *start, *count)
        }
    })
}

/// The new paragraphs replacing `old` (which starts at index `start`).
fn edited(old: &[Paragraph], op: &Op, start: u32) -> Result<Vec<Paragraph>, EditError> {
    let rel = |p: &Pos| (p.para - start) as usize;
    match op {
        Op::SetDocInfo { .. } => unreachable!("definitions do not touch paragraphs"),
        Op::SetProperties { .. } => unreachable!("properties are edited at document level"),
        Op::Replace { paras, .. } => Ok(paras.clone()),
        Op::InsertText { at, text } => {
            let mut p = old[0].clone();
            check(&p, at.offset)?;
            let mut out = Vec::new();
            let mut offset = at.offset;
            for (i, piece) in text.split('\n').enumerate() {
                if i > 0 {
                    let (a, b) = split(&p, offset);
                    out.push(a);
                    p = b;
                    offset = 0;
                }
                let units = encode(piece);
                offset = insert_units(&mut p, offset, &units);
            }
            out.push(p);
            Ok(out.into_iter().map(touched).collect())
        }
        Op::Split { at, page_break } => {
            let p = &old[0];
            check(p, at.offset)?;
            let (a, mut b) = split(p, at.offset);
            if *page_break {
                b.header.break_kind |= hm_model::break_kind::PAGE;
            }
            Ok(vec![touched(a), touched(b)])
        }
        Op::Delete { from, to } => {
            let (a, b) = (rel(from), rel(to));
            check(&old[a], from.offset)?;
            check(&old[b], to.offset)?;
            if a == b {
                let mut p = old[a].clone();
                remove_units(&mut p, from.offset, to.offset);
                return Ok(vec![touched(p)]);
            }
            // Keep the head of the first paragraph and the tail of the last.
            let mut head = old[a].clone();
            let head_len = head.len() - 1;
            remove_units(&mut head, from.offset, head_len);
            let mut tail = old[b].clone();
            remove_units(&mut tail, 0, to.offset);
            if from.offset > lead(&old[a]) {
                return Ok(vec![touched(join(head, tail))]);
            }
            // A head keeping no characters (only section and column definitions) gives way
            // to the last paragraph, as in Hancom: its style, shapes and numbering stay, in the
            // head's place in the flow (its breaks), and the definitions take its first
            // character shape.
            head.runs = vec![CharRun {
                pos: 0,
                shape: tail.shape_at(0),
            }];
            let (header, restart) = (tail.header.clone(), tail.list_restart);
            let mut p = join(head, tail);
            p.header = hm_model::ParaHeader {
                break_kind: p.header.break_kind,
                ..header
            };
            p.list_restart = restart;
            Ok(vec![touched(p)])
        }
        Op::SetCharShape { from, to, shape } => {
            let (a, b) = (rel(from), rel(to));
            let mut out = old.to_vec();
            for (k, p) in out.iter_mut().enumerate() {
                let s = if k == a { from.offset } else { 0 };
                let e = if k == b { to.offset } else { p.len() };
                if k < a || k > b || s >= e {
                    continue;
                }
                check(p, s)?;
                if e > p.len() {
                    return Err(EditError::BadOffset);
                }
                if e < p.len() {
                    check(p, e)?;
                }
                set_shape(p, s, e, *shape);
                *p = touched(std::mem::take(p));
            }
            Ok(out)
        }
        Op::SetParaShape { shape, .. } => Ok(old
            .iter()
            .map(|p| {
                let mut p = p.clone();
                p.header.para_shape = *shape;
                touched(p)
            })
            .collect()),
        Op::SetStyle {
            style,
            para_shape,
            char_shape,
            ..
        } => Ok(old
            .iter()
            .map(|p| {
                let mut p = p.clone();
                p.header.style = *style;
                p.header.para_shape = *para_shape;
                if let Some(shape) = char_shape {
                    let len = p.len();
                    set_shape(&mut p, 0, len, *shape);
                }
                touched(p)
            })
            .collect()),
        Op::SetPageBreak { on, .. } => {
            let mut p = old[0].clone();
            if *on {
                // A paragraph starts a page or a column, not both: Hancom
                // keeps the column break when both are set.
                p.header.break_kind |= hm_model::break_kind::PAGE;
                p.header.break_kind &= !hm_model::break_kind::COLUMN;
            } else {
                // Taking the break away, as Backspace does in Hancom, removes either kind.
                p.header.break_kind &= !(hm_model::break_kind::PAGE | hm_model::break_kind::COLUMN);
            }
            Ok(vec![touched(p)])
        }
    }
}

/// A changed paragraph: no source bytes, no stored lines.
fn touched(mut p: Paragraph) -> Paragraph {
    p.src = None;
    p.line_segs.clear();
    p
}

/// Offsets must fall on unit boundaries inside the text, not before the
/// section and column definitions a paragraph may start with.
pub fn check(p: &Paragraph, offset: u32) -> Result<(), EditError> {
    let last = p.len().saturating_sub(1);
    if offset > last || offset < lead(p) {
        return Err(EditError::BadOffset);
    }
    if offset == last {
        return Ok(());
    }
    if p.units().any(|(pos, _)| pos == offset) {
        Ok(())
    } else {
        Err(EditError::BadOffset)
    }
}

/// Where text may start: after the leading section and column definitions.
pub fn lead(p: &Paragraph) -> u32 {
    let mut end = 0;
    for (pos, u) in p.units() {
        match u {
            Unit::Extended { index, .. }
                if p.controls.get(index as usize).is_some_and(|c| {
                    matches!(c.kind, ControlKind::Section(_) | ControlKind::Columns(_))
                }) =>
            {
                end = pos + 8;
            }
            _ => break,
        }
    }
    end
}

/// Characters as wire units: tabs become 8-unit tab controls, U+2028 a
/// line break, U+00A0 a nonbreaking space and U+2007 a fixed-width space.
fn encode(s: &str) -> Vec<u16> {
    let mut out = Vec::with_capacity(s.len());
    for c in s.chars() {
        if c == '\t' {
            out.extend_from_slice(&[9, 0, 0, 0, 0, 0, 0, 9]);
        } else if c == '\u{2028}' {
            out.push(10);
        } else if c == '\u{a0}' {
            out.push(30);
        } else if c == '\u{2007}' {
            out.push(31);
        } else if (c as u32) >= 32 {
            let mut buf = [0u16; 2];
            out.extend_from_slice(c.encode_utf16(&mut buf));
        }
    }
    out
}

/// Inserts units at `at`; returns the position after them. The inserted
/// text takes the char shape of the character before it (or after it at
/// the paragraph start).
pub fn insert_units(p: &mut Paragraph, at: u32, units: &[u16]) -> u32 {
    let n = units.len() as u32;
    if n == 0 {
        return at;
    }
    let shape = p.shape_at(at.saturating_sub(1));
    p.text
        .splice(at as usize..at as usize, units.iter().copied());
    for r in &mut p.runs {
        if r.pos > at || (r.pos == at && at > 0) {
            r.pos += n;
        }
    }
    let _ = shape;
    normalize_runs(p);
    shift_marks(p, at, n as i64);
    at + n
}

/// Inserts an anchored control through a paragraph replacement, preserving
/// character runs and tracked ranges in the same way as text insertion.
pub fn insert_control(doc: &Document, at: &Pos, control: Control) -> Result<Op, EditError> {
    let mut p = list(doc, &at.list)
        .and_then(|l| l.get(at.para as usize))
        .cloned()
        .ok_or(EditError::NoSuchParagraph)?;
    check(&p, at.offset)?;
    let index = p
        .units()
        .filter(|(offset, u)| *offset < at.offset && matches!(u, Unit::Extended { .. }))
        .count();
    let id = control.id().0;
    // Notes and automatic numbers have their own control characters.
    let code = match control.kind {
        ControlKind::Note(_) => 17,
        ControlKind::AutoNumber(_) => 18,
        _ => 11,
    };
    insert_units(
        &mut p,
        at.offset,
        &[code, id as u16, (id >> 16) as u16, 0, 0, 0, 0, code],
    );
    p.controls.insert(index, control);
    Ok(Op::Replace {
        list: at.list.clone(),
        start: at.para,
        count: 1,
        paras: vec![touched(p)],
    })
}

/// Removes units `[s, e)` with the controls they hold.
pub fn remove_units(p: &mut Paragraph, s: u32, e: u32) {
    if e <= s {
        return;
    }
    // Extended controls in the range leave the control list.
    let gone: Vec<u32> = p
        .units()
        .filter_map(|(pos, u)| match u {
            Unit::Extended { index, .. } if pos >= s && pos < e => Some(index),
            _ => None,
        })
        .collect();
    for &i in gone.iter().rev() {
        p.controls.remove(i as usize);
    }
    p.text.drain(s as usize..e as usize);
    let n = e - s;
    for r in &mut p.runs {
        if r.pos >= e {
            r.pos -= n;
        } else if r.pos > s {
            r.pos = s;
        }
    }
    normalize_runs(p);
    p.range_tags.retain_mut(|t| {
        t.start = clip(t.start, s, e);
        t.end = clip(t.end, s, e);
        t.end > t.start
    });
    for m in &mut p.tracks {
        m.pos = clip(m.pos, s, e);
    }
}

fn clip(v: u32, s: u32, e: u32) -> u32 {
    if v >= e {
        v - (e - s)
    } else if v > s {
        s
    } else {
        v
    }
}

/// Moves range tags and marks after an insertion of `n` units at `at`.
fn shift_marks(p: &mut Paragraph, at: u32, n: i64) {
    let mv = |v: &mut u32| {
        if *v > at {
            *v = (*v as i64 + n) as u32;
        }
    };
    for t in &mut p.range_tags {
        mv(&mut t.start);
        mv(&mut t.end);
    }
    for m in &mut p.tracks {
        mv(&mut m.pos);
    }
}

/// Merges runs with equal shapes, drops empty ones, keeps one at 0.
fn normalize_runs(p: &mut Paragraph) {
    let len = p.len();
    let mut out: Vec<CharRun> = Vec::with_capacity(p.runs.len());
    let mut runs = std::mem::take(&mut p.runs);
    runs.sort_by_key(|r| r.pos);
    for r in runs {
        if r.pos >= len.max(1) && !out.is_empty() {
            continue;
        }
        match out.last_mut() {
            Some(last) if last.pos == r.pos => last.shape = r.shape,
            Some(last) if last.shape == r.shape => {}
            _ => out.push(r),
        }
    }
    if let Some(first) = out.first_mut() {
        first.pos = 0;
    }
    // A merge at 0 can leave two runs with one shape next to each other.
    out.dedup_by(|b, a| a.shape == b.shape);
    p.runs = out;
}

/// Applies `shape` to `[s, e)`.
fn set_shape(p: &mut Paragraph, s: u32, e: u32, shape: u32) {
    let after = p.shape_at(e);
    let mut runs: Vec<CharRun> = p
        .runs
        .iter()
        .copied()
        .filter(|r| r.pos < s || r.pos > e)
        .collect();
    runs.push(CharRun { pos: s, shape });
    if e < p.len() {
        runs.push(CharRun {
            pos: e,
            shape: after,
        });
    }
    p.runs = runs;
    normalize_runs(p);
}

/// Splits a paragraph at `at`: the head ends there with a paragraph end,
/// the tail takes the rest with the same para shape.
fn split(p: &Paragraph, at: u32) -> (Paragraph, Paragraph) {
    let mut head = p.clone();
    let mut tail = p.clone();
    let len = p.len();
    // Head: text up to `at`, then the paragraph end.
    remove_units(&mut head, at, len - 1);
    // Tail: drop everything before `at`.
    remove_units(&mut tail, 0, at);
    tail.header.break_kind &= !(hm_model::break_kind::PAGE | hm_model::break_kind::COLUMN);
    // Section and column definitions stay in the first paragraph.
    tail.header.break_kind &= !(hm_model::break_kind::SECTION | hm_model::break_kind::MULTI_COLUMN);
    (head, tail)
}

/// Joins two paragraphs: `a`'s text without its end, then `b`'s.
fn join(mut a: Paragraph, b: Paragraph) -> Paragraph {
    let at = a.len() - 1;
    a.text.truncate(at as usize);
    let n = a.len();
    a.text.extend_from_slice(&b.text);
    a.runs.extend(b.runs.iter().map(|r| CharRun {
        pos: r.pos + n,
        shape: r.shape,
    }));
    a.controls.extend(b.controls);
    a.range_tags.extend(b.range_tags.iter().map(|t| RangeTag {
        start: t.start + n,
        end: t.end + n,
        tag: t.tag,
    }));
    a.tracks.extend(b.tracks.into_iter().map(|m| TrackMark {
        pos: m.pos + n,
        ..m
    }));
    normalize_runs(&mut a);
    a
}

/// The units of a paragraph a control at `pos` spans.
pub fn unit_width(code: u16) -> u32 {
    control_width(code)
}

/// A paragraph holding just the paragraph end, in char shape `shape`.
pub fn empty_paragraph(template: &Paragraph, shape: u32) -> Paragraph {
    let mut p = Paragraph {
        header: template.header.clone(),
        text: vec![PARA_END],
        runs: vec![CharRun { pos: 0, shape }],
        ..Default::default()
    };
    p.header.break_kind = 0;
    p
}

/// Keeps a compiler-visible use of `Control` for callers matching kinds.
pub fn is_table(c: &Control) -> bool {
    c.as_table().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hm_model::{DocInfo, DocMeta, Format, Section, SharedBytes, Version};

    fn doc(texts: &[&str]) -> Document {
        let paras = texts
            .iter()
            .map(|t| Paragraph {
                text: t.encode_utf16().chain([PARA_END]).collect(),
                runs: vec![CharRun { pos: 0, shape: 1 }],
                src: Some(SharedBytes::from(vec![1u8])),
                ..Default::default()
            })
            .collect();
        Document {
            meta: DocMeta {
                format: Format::Hwp5,
                version: Version(0),
                flags: 0,
            },
            info: DocInfo::default(),
            sections: vec![Section {
                paras,
                src: Some(SharedBytes::from(vec![1u8])),
                source_index: Some(0),
                last_page_master: None,
                page_masters: Vec::new(),
            }],
            source: None,
            bins: None,
            diagnostics: Vec::new(),
        }
    }

    fn at(para: u32, offset: u32) -> Pos {
        Pos {
            list: ListPath::default(),
            para,
            offset,
        }
    }

    fn texts(d: &Document) -> Vec<String> {
        d.sections[0].paras.iter().map(|p| p.plain_text()).collect()
    }

    #[test]
    fn insert_split_delete_and_undo() {
        let mut d = doc(&["가나다", "라마"]);
        let undo1 = apply(
            &mut d,
            &Op::InsertText {
                at: at(0, 1),
                text: "X\nY".into(),
            },
        )
        .unwrap();
        assert_eq!(texts(&d), ["가X", "Y나다", "라마"]);
        assert!(d.sections[0].src.is_none());
        assert!(d.sections[0].paras[0].src.is_none());
        assert!(d.sections[0].paras[2].src.is_some());
        let undo2 = apply(
            &mut d,
            &Op::Delete {
                from: at(0, 1),
                to: at(2, 1),
            },
        )
        .unwrap();
        assert_eq!(texts(&d), ["가마"]);
        apply(&mut d, &undo2).unwrap();
        assert_eq!(texts(&d), ["가X", "Y나다", "라마"]);
        apply(&mut d, &undo1).unwrap();
        assert_eq!(texts(&d), ["가나다", "라마"]);
        assert!(d.sections[0].paras[0].src.is_some());
    }

    #[test]
    fn char_shapes_follow_edits() {
        let mut d = doc(&["abcdef"]);
        apply(
            &mut d,
            &Op::SetCharShape {
                from: at(0, 2),
                to: at(0, 4),
                shape: 7,
            },
        )
        .unwrap();
        let runs = |d: &Document| {
            d.sections[0].paras[0]
                .runs
                .iter()
                .map(|r| (r.pos, r.shape))
                .collect::<Vec<_>>()
        };
        assert_eq!(runs(&d), [(0, 1), (2, 7), (4, 1)]);
        apply(
            &mut d,
            &Op::InsertText {
                at: at(0, 3),
                text: "ZZ".into(),
            },
        )
        .unwrap();
        assert_eq!(runs(&d), [(0, 1), (2, 7), (6, 1)]);
        apply(
            &mut d,
            &Op::Delete {
                from: at(0, 1),
                to: at(0, 5),
            },
        )
        .unwrap();
        // The inserted text took the shape around it; d keeps its shape.
        assert_eq!(d.sections[0].paras[0].plain_text(), "adef");
        assert_eq!(runs(&d), [(0, 1), (1, 7), (2, 1)]);
    }

    #[test]
    fn offsets_inside_controls_are_refused() {
        let mut d = doc(&["a"]);
        let p = &mut d.sections[0].paras[0];
        p.text = vec![0x61, 9, 0, 0, 0, 0, 0, 0, 9, PARA_END];
        let r = apply(
            &mut d,
            &Op::InsertText {
                at: at(0, 3),
                text: "x".into(),
            },
        );
        assert_eq!(r.unwrap_err(), EditError::BadOffset);
    }
}
