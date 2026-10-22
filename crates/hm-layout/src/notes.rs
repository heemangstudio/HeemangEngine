//! Footnotes at the foot of their column.
//!
//! A footnote goes on the page of the line that refers to it. The notes of
//! a column stack up from the body's bottom edge, the last note's last line
//! ending there, with the note shape's gap between notes. Above the first
//! note the separator line sits by the gap below it, and body text keeps the
//! gap above it clear. Every note taken into a column lowers the limit the
//! column's lines must stay under.

use hm_model::{BorderLine, Note, NoteShape, Table};

use crate::furniture::{CellStep, Frame, Rule};
use crate::paginate::{Page, Rect};
use crate::table::TableLayout;

/// The separator length Hancom draws for a stored length of -1: 5 cm.
const DEFAULT_SEPARATOR: i32 = 14173;

/// Notes waiting at the foot of one column.
#[derive(Default)]
struct Bucket {
    x: i32,
    w: i32,
    frames: Vec<Frame>,
}

/// The footnote areas of the page being filled.
#[derive(Default)]
pub(crate) struct NoteArea {
    shape: NoteShape,
    buckets: Vec<Bucket>,
}

impl NoteArea {
    pub(crate) fn new(shape: Option<&NoteShape>) -> Self {
        Self {
            shape: shape.cloned().unwrap_or_else(|| NoteShape {
                separator_length: -1,
                space_above: 850,
                space_below: 567,
                space_between: 283,
                line_kind: 1,
                line_width: 1,
                ..Default::default()
            }),
            buckets: Vec::new(),
        }
    }

    /// Height the notes of `frames` take, with the separator gaps.
    fn height_of(&self, frames: &[&Frame]) -> i32 {
        if frames.is_empty() {
            return 0;
        }
        let s = &self.shape;
        let notes: i32 = frames.iter().map(|f| f.content).sum();
        s.space_above as i32
            + s.space_below as i32
            + notes
            + s.space_between as i32 * (frames.len() as i32 - 1)
    }

    fn bucket(&self, x: i32) -> Option<&Bucket> {
        self.buckets.iter().find(|b| b.x == x)
    }

    /// Height reserved at the foot of the column at page x `x`.
    pub(crate) fn reserve(&self, x: i32) -> i32 {
        self.bucket(x)
            .map_or(0, |b| self.height_of(&b.frames.iter().collect::<Vec<_>>()))
    }

    /// How much more the column at `x` reserves once `more` join it.
    pub(crate) fn extra(&self, x: i32, more: &[Frame]) -> i32 {
        if more.is_empty() {
            return 0;
        }
        let mut all: Vec<&Frame> = self
            .bucket(x)
            .map_or(Vec::new(), |b| b.frames.iter().collect());
        let before = self.height_of(&all);
        all.extend(more.iter());
        self.height_of(&all) - before
    }

    pub(crate) fn add(&mut self, x: i32, w: i32, more: Vec<Frame>) {
        if more.is_empty() {
            return;
        }
        match self.buckets.iter_mut().find(|b| b.x == x) {
            Some(b) => b.frames.extend(more),
            None => self.buckets.push(Bucket { x, w, frames: more }),
        }
    }

    /// Places the waiting notes on `page`, whose body is `body`, and empties
    /// the area.
    pub(crate) fn flush(&mut self, page: &mut Page, body: Rect) {
        let s = self.shape.clone();
        for b in self.buckets.drain(..) {
            if b.frames.is_empty() {
                continue;
            }
            let notes: i32 = b.frames.iter().map(|f| f.content).sum::<i32>()
                + s.space_between as i32 * (b.frames.len() as i32 - 1);
            let first = body.y + body.h - notes;
            if s.line_kind != 0 {
                let len = match s.separator_length {
                    l if l < 0 => DEFAULT_SEPARATOR,
                    l => l.min(b.w),
                };
                let y = first - s.space_below as i32;
                page.rules.push(Rule {
                    x1: b.x,
                    y1: y,
                    x2: b.x + len.min(b.w),
                    y2: y,
                    line: BorderLine {
                        kind: s.line_kind,
                        width: s.line_width,
                        color: s.line_color,
                    },
                });
            }
            let mut top = first;
            for mut f in b.frames {
                let h = f.content;
                f.shift(top - f.rect.y);
                // Notes laid out before their column was known move to it.
                f.shift_x(b.x - f.rect.x);
                f.rect.h = h;
                top += h + s.space_between as i32;
                page.frames.push(f);
            }
        }
    }
}

/// A footnote referred to inside a table: the path to its control and the
/// table y of the line that refers to it.
pub(crate) struct CellNote<'a> {
    pub y: i32,
    pub steps: Vec<CellStep>,
    pub note: &'a Note,
}

/// The footnotes referred to in the cells of table `t` laid out as `tl`, in
/// cell order. A note inside a nested table counts at the line of the outer
/// cell that holds that table.
pub(crate) fn in_table<'a>(t: &'a Table, tl: &TableLayout) -> Vec<CellNote<'a>> {
    let mut out = Vec::new();
    collect(t, tl, &[], None, &mut out);
    out
}

fn collect<'a>(
    t: &'a Table,
    tl: &TableLayout,
    prefix: &[CellStep],
    at: Option<i32>,
    out: &mut Vec<CellNote<'a>>,
) {
    for (ci, (cell, cl)) in t.cells.iter().zip(&tl.cells).enumerate() {
        for cline in &cl.lines {
            let (Some(para), Some(pl)) = (
                cell.paras.get(cline.para as usize),
                cl.paras.get(cline.para as usize),
            ) else {
                continue;
            };
            let Some(line) = pl.lines.get(cline.line as usize) else {
                continue;
            };
            let y = at.unwrap_or(cl.content_y() + cline.y);
            for (pos, unit) in para.units() {
                let hm_model::Unit::Extended { index, .. } = unit else {
                    continue;
                };
                if pos < line.start || pos >= line.end {
                    continue;
                }
                let Some(c) = para.controls.get(index as usize) else {
                    continue;
                };
                let mut steps = prefix.to_vec();
                steps.push(CellStep {
                    cell: ci as u32,
                    para: cline.para,
                    control: index,
                });
                if let Some(note) = c.as_note().filter(|n| !n.endnote) {
                    out.push(CellNote { y, steps, note });
                } else if let (Some(nt), Some(ntl)) = (c.as_table(), pl.table(index)) {
                    collect(nt, ntl, &steps, Some(y), out);
                }
            }
        }
    }
}

/// True when a table holds a footnote anywhere in its cells.
pub(crate) fn table_has_notes(t: &Table) -> bool {
    t.cells.iter().flat_map(|c| &c.paras).any(|p| {
        p.controls.iter().any(|c| {
            c.as_note().is_some_and(|n| !n.endnote) || c.as_table().is_some_and(table_has_notes)
        })
    })
}

/// An endnote to place: where its control lives and the note itself.
pub(crate) struct Endnote<'a> {
    pub section: u32,
    pub para: u32,
    pub control: u32,
    pub note: &'a hm_model::Note,
}

/// Flows endnotes after the body: the separator line with its gaps, then
/// each note's lines through the columns and pages like body text, notes
/// apart by the shape's gap.
#[allow(clippy::too_many_arguments)]
pub(crate) fn place_endnotes(
    flow: &mut crate::paginate::Flow,
    notes: &[Endnote<'_>],
    shape: Option<&NoteShape>,
    info: &hm_model::DocInfo,
    styles: &mut crate::style::Styles<'_>,
    default_tab: i32,
) {
    use crate::furniture::{FrameKind, FrameLine, ListRef};
    if notes.is_empty() {
        return;
    }
    let s = shape.cloned().unwrap_or_default();
    let (above, below, between) = (
        s.space_above as i32,
        s.space_below as i32,
        s.space_between as i32,
    );
    // The separator and its gaps open the area where the body ends.
    if flow.has_content && flow.fill[flow.col] + above + below >= flow.region_height() {
        flow.next_column();
    }
    let at = flow.fill[flow.col] + if flow.has_content { above } else { 0 };
    let (cx, cw) = flow.column();
    if s.line_kind != 0 {
        let len = match s.separator_length {
            l if l < 0 => DEFAULT_SEPARATOR,
            0 => 0,
            l => l.min(cw),
        };
        if len > 0 {
            let y = flow.page_y(at);
            let rule = Rule {
                x1: cx,
                y1: y,
                x2: cx + len,
                y2: y,
                line: BorderLine {
                    kind: s.line_kind,
                    width: s.line_width,
                    color: s.line_color,
                },
            };
            if let Some(page) = flow.pages.last_mut() {
                page.rules.push(rule);
            }
        }
    }
    let col = flow.col;
    flow.fill[col] = at + below;
    flow.has_content = true;
    for (k, n) in notes.iter().enumerate() {
        let list = ListRef::Control {
            section: n.section,
            para: n.para,
            control: n.control,
        };
        let env = crate::para::ParaEnv {
            column_width: flow.column().1,
            default_tab,
        };
        let (paras, lines, _) = crate::table::stack(
            &n.note.list.paras,
            info,
            styles,
            env,
            &mut crate::heads::Heads::default(),
        );
        if k > 0 {
            let col = flow.col;
            flow.fill[col] += between;
        }
        let mut frame: Option<crate::furniture::Frame> = None;
        let mut prev_y = None;
        let mut start = flow.fill[flow.col];
        for l in &lines {
            let line = &paras[l.para as usize].lines[l.line as usize];
            // Lines keep their spacing from the list: the gap from the
            // previous line, or the note's start.
            let offset = prev_y.map_or(l.y, |p: i32| l.y - p);
            let mut y = start + offset;
            if flow.has_content && y + line.height >= flow.region_height() {
                if let (Some(f), Some(page)) = (frame.take(), flow.pages.last_mut()) {
                    page.frames.push(f);
                }
                flow.next_column();
                flow.has_content = false;
                y = flow.fill[flow.col];
            }
            let (cx, cw) = flow.column();
            let page_y = flow.page_y(y);
            let f = frame.get_or_insert_with(|| crate::furniture::Frame {
                kind: FrameKind::Endnotes,
                list: list.clone(),
                rect: Rect {
                    x: cx,
                    y: page_y,
                    w: cw,
                    h: 0,
                },
                paras: paras.clone(),
                lines: Vec::new(),
                objects: Vec::new(),
                tables: Vec::new(),
                content: 0,
            });
            f.lines.push(FrameLine {
                para: l.para,
                line: l.line,
                x: cx,
                y: page_y,
            });
            f.rect.h = page_y + line.height - f.rect.y;
            f.content = f.rect.h;
            let col = flow.col;
            flow.fill[col] = y + line.height;
            flow.has_content = true;
            start = y;
            prev_y = Some(l.y);
        }
        if let (Some(f), Some(page)) = (frame, flow.pages.last_mut()) {
            page.frames.push(f);
        }
    }
}
