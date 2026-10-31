//! Footnote and endnote numbers in document order, and the note shape
//! Hancom mirrors into every note.
//!
//! Hancom numbers footnotes and endnotes separately, in document order
//! (body, table cells, text boxes), continuing across sections unless a
//! section restarts from its shape's start number. A note stores what it
//! shows: its number (page-local when footnotes restart on every page),
//! the mark characters, and a flag holding the number format code plus 256
//! for a per-page restart. The note's own automatic number carries the
//! shape's format, superscript bit and characters.

use crate::{
    AutoNumber, Control, ControlKind, Document, NewNumber, Note, NoteShape, NumberKind, ObjectPart,
    Paragraph, SectionChild, ShapePart,
};

/// Numbering restarts with every section (`attr` bits 10-11).
pub const RESTART_SECTION: u32 = 1;
/// Footnote numbering restarts on every page.
pub const RESTART_PAGE: u32 = 2;
/// The control flag bit marking a footnote numbered per page.
pub const FLAG_PAGE: u32 = 256;

/// The parts of a note shape a note mirrors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mirror {
    /// Number format code (shape `attr` bits 0-7).
    pub format: u32,
    pub superscript: bool,
    /// Footnotes numbered per page.
    pub per_page: bool,
    pub user_char: u16,
    pub prefix: u16,
    pub suffix: u16,
}

impl Mirror {
    pub fn of(shape: &NoteShape, endnote: bool) -> Mirror {
        Mirror {
            format: shape.attr & 0xFF,
            superscript: shape.attr & (1 << 12) != 0,
            per_page: !endnote && (shape.attr >> 10) & 3 == RESTART_PAGE,
            user_char: shape.user_char,
            prefix: shape.prefix,
            suffix: shape.suffix,
        }
    }

    /// The control flag's shape bits.
    pub fn flag(&self) -> u32 {
        self.format | if self.per_page { FLAG_PAGE } else { 0 }
    }
}

/// What a note shows: its number (`None` while footnotes are numbered per
/// page, which layout decides) and its section's shape (`None` when the
/// section has none).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoteLook {
    pub number: Option<u32>,
    pub mirror: Option<Mirror>,
}

fn is_note_number(a: &AutoNumber) -> bool {
    matches!(a.kind(), NumberKind::Footnote | NumberKind::Endnote)
}

impl NoteLook {
    fn number_shows(&self, a: &AutoNumber, n: u32) -> bool {
        let shape_bits = (0xFF << 4) | (1 << 12);
        a.number as u32 == n.min(u16::MAX as u32)
            && self.mirror.is_none_or(|m| {
                a.attr & shape_bits == (m.format << 4) | (u32::from(m.superscript) << 12)
                    && (a.user_char, a.prefix, a.suffix) == (m.user_char, m.prefix, m.suffix)
            })
    }

    /// Whether `note` and the automatic numbers in its list already show
    /// this.
    pub fn shown_by(&self, note: &Note) -> bool {
        let n = self.number.unwrap_or(note.number);
        note.number == n
            && self.mirror.is_none_or(|m| {
                note.flag & 0x1FF == m.flag()
                    && (note.user_char, note.prefix, note.suffix)
                        == (m.user_char, m.prefix, m.suffix)
            })
            && note.list.paras.iter().all(|p| {
                p.controls.iter().all(|c| match &c.kind {
                    ControlKind::AutoNumber(a) if is_note_number(a) => self.number_shows(a, n),
                    _ => true,
                })
            })
    }

    /// Makes `note` show this. Flag bits other than the shape's are kept.
    /// Paragraphs whose numbers change lose their source and stored lines.
    pub fn apply(&self, note: &mut Note) {
        let n = self.number.unwrap_or(note.number);
        note.number = n;
        if let Some(m) = self.mirror {
            note.flag = (note.flag & !0x1FF) | m.flag();
            note.user_char = m.user_char;
            note.prefix = m.prefix;
            note.suffix = m.suffix;
        }
        for p in &mut note.list.paras {
            let mut changed = false;
            for c in &mut p.controls {
                let ControlKind::AutoNumber(a) = &mut c.kind else {
                    continue;
                };
                if !is_note_number(a) || self.number_shows(a, n) {
                    continue;
                }
                a.number = n.min(u16::MAX as u32) as u16;
                if let Some(m) = self.mirror {
                    a.attr = (a.attr & !((0xFF << 4) | (1 << 12)))
                        | (m.format << 4)
                        | (u32::from(m.superscript) << 12);
                    a.user_char = m.user_char;
                    a.prefix = m.prefix;
                    a.suffix = m.suffix;
                }
                c.src = None;
                changed = true;
            }
            if changed {
                p.src = None;
                p.line_segs.clear();
            }
        }
    }
}

/// The number the note's own automatic number shows.
pub fn own_number(note: &Note) -> Option<u32> {
    note.list.paras.iter().find_map(|p| {
        p.controls.iter().find_map(|c| match &c.kind {
            ControlKind::AutoNumber(a) if is_note_number(a) => Some(a.number as u32),
            _ => None,
        })
    })
}

/// Counts notes in document order.
#[derive(Clone, Debug, Default)]
pub struct NoteCounter {
    /// The next number of each kind (footnote, endnote); `None` while the
    /// section numbers footnotes per page.
    next: [Option<u32>; 2],
    mirrors: [Option<Mirror>; 2],
}

impl NoteCounter {
    pub fn new() -> NoteCounter {
        NoteCounter::default()
    }

    /// Enters section `index`: the first section and a section restarting
    /// its numbering start from the start number (the shape's for a
    /// restarting section; Hancom ignores it otherwise), the others
    /// continue.
    pub fn section(&mut self, doc: &Document, index: usize) {
        let shapes: Vec<&NoteShape> = doc
            .sections
            .get(index)
            .and_then(|s| s.definition())
            .map(|d| d.note_shapes().collect())
            .unwrap_or_default();
        for kind in 0..2 {
            let shape = shapes.get(kind).copied();
            let endnote = kind == 1;
            self.mirrors[kind] = shape.map(|s| Mirror::of(s, endnote));
            let mode = shape.map_or(0, |s| (s.attr >> 10) & 3);
            let first = if endnote {
                doc.info.properties.endnote_start
            } else {
                doc.info.properties.footnote_start
            }
            .max(1) as u32;
            self.next[kind] = if !endnote && mode == RESTART_PAGE {
                None
            } else if mode == RESTART_SECTION {
                Some(shape.map_or(1, |s| s.start.max(1) as u32))
            } else if index == 0 {
                Some(first)
            } else {
                self.next[kind].or(Some(first))
            };
        }
    }

    /// A new-number control restarting footnotes or endnotes.
    pub fn new_number(&mut self, n: &NewNumber) {
        let kind = match n.kind() {
            NumberKind::Footnote => 0,
            NumberKind::Endnote => 1,
            _ => return,
        };
        if self.next[kind].is_some() {
            self.next[kind] = Some(n.number as u32);
        }
    }

    /// The look of the next note of its kind.
    pub fn next(&mut self, endnote: bool) -> NoteLook {
        let kind = usize::from(endnote);
        let number = self.next[kind];
        if let Some(n) = number {
            self.next[kind] = Some(n + 1);
        }
        NoteLook {
            number,
            mirror: self.mirrors[kind],
        }
    }
}

/// The paragraph lists of a control in document order, writable: a table's
/// cells then its caption, an object's text boxes then its caption, a
/// header's, footer's or note's list, a section's master pages.
pub fn lists_mut(c: &mut Control) -> Vec<&mut Vec<Paragraph>> {
    fn shape<'a>(s: &'a mut crate::Shape, out: &mut Vec<&'a mut Vec<Paragraph>>) {
        for p in &mut s.parts {
            match p {
                ShapePart::Text(t) => out.push(&mut t.paras),
                ShapePart::Child(c) => shape(c, out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    match &mut c.kind {
        ControlKind::Table(t) => {
            out.extend(t.cells.iter_mut().map(|c| &mut c.paras));
            out.extend(t.caption.iter_mut().map(|c| &mut c.paras));
        }
        ControlKind::Object(o) => {
            let mut caption = None;
            let mut seen = false;
            for part in &mut o.parts {
                match part {
                    ObjectPart::Shape(s) if !seen => {
                        seen = true;
                        shape(s, &mut out);
                    }
                    ObjectPart::Caption(c) if caption.is_none() => caption = Some(&mut c.paras),
                    _ => {}
                }
            }
            out.extend(caption);
        }
        ControlKind::HeaderFooter(h) => out.push(&mut h.list.paras),
        ControlKind::Section(s) => out.extend(s.children.iter_mut().filter_map(|c| match c {
            SectionChild::MasterPage(l) => Some(&mut l.paras),
            _ => None,
        })),
        _ => {}
    }
    out
}

fn sync_list(paras: &mut [Paragraph], counter: &mut NoteCounter) {
    for p in paras {
        let mut changed = false;
        for c in &mut p.controls {
            match &mut c.kind {
                ControlKind::NewNumber(n) => counter.new_number(n),
                ControlKind::Note(note) => {
                    let mut look = counter.next(note.endnote);
                    // A page-local number is the one the note's own
                    // automatic number shows.
                    look.number = look.number.or_else(|| own_number(note));
                    if !look.shown_by(note) {
                        look.apply(note);
                        c.src = None;
                        changed = true;
                    }
                }
                _ => {
                    for list in lists_mut(c) {
                        sync_list(list, counter);
                    }
                }
            }
        }
        if changed {
            p.src = None;
        }
    }
}

/// Gives every note the number and mirrored shape Hancom shows, in place.
/// Footnotes numbered per page keep their stored numbers. Readers of files
/// that store no note attributes (HWPML) call this.
pub fn sync(doc: &mut Document) {
    let mut counter = NoteCounter::new();
    for si in 0..doc.sections.len() {
        counter.section(doc, si);
        let mut paras = std::mem::take(&mut doc.sections[si].paras);
        sync_list(&mut paras, &mut counter);
        doc.sections[si].paras = paras;
    }
}

fn each_note(paras: &mut [Paragraph], f: &mut dyn FnMut(&mut Control)) {
    for p in paras {
        for c in &mut p.controls {
            if matches!(c.kind, ControlKind::Note(_)) {
                f(c);
            } else {
                for list in lists_mut(c) {
                    each_note(list, f);
                }
            }
        }
    }
}

/// Gives every note without an instance id one no other note has, as Hancom
/// does when it opens a file that stores none (HWPML).
pub fn fill_inst_ids(doc: &mut Document) {
    let mut next = 0u32;
    for s in &mut doc.sections {
        each_note(&mut s.paras, &mut |c| {
            if let ControlKind::Note(n) = &c.kind {
                next = next.max(n.inst_id.unwrap_or(0));
            }
        });
    }
    for s in &mut doc.sections {
        each_note(&mut s.paras, &mut |c| {
            if let ControlKind::Note(n) = &mut c.kind
                && n.inst_id.is_none()
            {
                next = next.wrapping_add(1).max(1);
                n.inst_id = Some(next);
                c.src = None;
            }
        });
    }
}
