//! Footnotes at the foot of their column.
//!
//! A footnote goes on the page of the line that refers to it. When that
//! line fits its column only without all of its notes, Hancom keeps the
//! line there, fills the column's foot with as many note lines as fit and
//! sets the rest at the foot of the next column or page. The notes of
//! a column stack up from the body's bottom edge, the last note's last line
//! ending there, with the note shape's gap between notes. Above the first
//! note the separator line sits by the gap below it, and body text keeps the
//! gap above it clear. Every note taken into a column lowers the limit the
//! column's lines must stay under.

use hm_model::{BorderLine, Note, NoteShape, Table};

use crate::furniture::{CellStep, Frame, ListRef, Rule};
use crate::paginate::{Page, Rect};
use crate::table::TableLayout;

/// The separator length Hancom draws for a stored length of -1, about
/// 5 cm: its PDFs show 14184 HU in every note placement (each column,
/// merged, rightmost column).
const DEFAULT_SEPARATOR: i32 = 14184;

/// Notes waiting at the foot of one column.
#[derive(Default)]
struct Bucket {
    x: i32,
    w: i32,
    frames: Vec<Frame>,
    /// The part of the column's last line's notes that did not fit under
    /// it: it goes on in the next column.
    carried: Vec<Frame>,
}

/// The footnote areas of the page being filled.
#[derive(Default)]
pub(crate) struct NoteArea {
    shape: NoteShape,
    buckets: Vec<Bucket>,
    /// Carried notes of a page already placed, for the next column.
    next: Vec<Frame>,
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
            next: Vec::new(),
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
        self.bucket(x).map_or(0, |b| {
            self.height_of(&b.frames.iter().chain(&b.carried).collect::<Vec<_>>())
        })
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

    /// Where the notes of a multi-column page go (shape bits 8-9): 0 the
    /// foot of each column, 1 the full width under all columns, 2 the
    /// rightmost column.
    pub(crate) fn placement(&self) -> u32 {
        (self.shape.attr >> 8) & 3
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.buckets.is_empty()
    }

    pub(crate) fn add(&mut self, x: i32, w: i32, more: Vec<Frame>) {
        if more.is_empty() {
            return;
        }
        self.bucket_mut(x, w).frames.extend(more);
    }

    fn bucket_mut(&mut self, x: i32, w: i32) -> &mut Bucket {
        match self.buckets.iter().position(|b| b.x == x) {
            Some(i) => &mut self.buckets[i],
            None => {
                self.buckets.push(Bucket {
                    x,
                    w,
                    ..Default::default()
                });
                self.buckets.last_mut().unwrap()
            }
        }
    }

    /// Takes the notes, or the rest of them, that go on at the foot of the
    /// next column or page. They fill this column meanwhile.
    pub(crate) fn carry(&mut self, x: i32, w: i32, more: Vec<Frame>) {
        if more.is_empty() {
            return;
        }
        self.bucket_mut(x, w).carried.extend(more);
    }

    /// Splits the notes of a line that fits its column while they do not:
    /// the column takes as many of their lines as fit while its notes stay
    /// within `room` more height; the rest go on in the next column or page.
    pub(crate) fn split_to_fit(
        &self,
        x: i32,
        more: Vec<Frame>,
        room: i32,
    ) -> (Vec<Frame>, Vec<Frame>) {
        let s = &self.shape;
        let mut taken = self.bucket(x).map_or(0, |b| b.frames.len());
        let mut used = 0;
        let mut fit = Vec::new();
        let mut rest = Vec::new();
        for f in more {
            if !rest.is_empty() {
                rest.push(f);
                continue;
            }
            let gap = if taken == 0 {
                s.space_above as i32 + s.space_below as i32
            } else {
                s.space_between as i32
            };
            let left = room - used - gap;
            if f.content <= left {
                used += gap + f.content;
                taken += 1;
                fit.push(f);
                continue;
            }
            match split_frame(f, left) {
                (Some(head), tail) => {
                    fit.push(head);
                    rest.push(tail);
                }
                (None, whole) => rest.push(whole),
            }
        }
        (fit, rest)
    }

    /// Moves the carried notes to the column at page x `x`, `w` wide.
    pub(crate) fn adopt_carried(&mut self, x: i32, w: i32) {
        let mut moved = std::mem::take(&mut self.next);
        for b in &mut self.buckets {
            moved.append(&mut b.carried);
        }
        self.buckets.retain(|b| !b.frames.is_empty());
        self.add(x, w, moved);
    }

    /// Places the waiting notes on `page`, whose body is `body`, and empties
    /// the area.
    pub(crate) fn flush(&mut self, page: &mut Page, body: Rect) {
        let s = self.shape.clone();
        for mut b in std::mem::take(&mut self.buckets) {
            self.next.append(&mut b.carried);
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

/// Cuts a note frame between lines so that its first part is at most
/// `height` tall: the first part (none when not even one line fits) and
/// the rest, moved up to start at the frame's top.
fn split_frame(f: Frame, height: i32) -> (Option<Frame>, Frame) {
    let top = f.rect.y;
    let bottom = |l: &crate::furniture::FrameLine| {
        let h = f
            .paras
            .get(l.para as usize)
            .and_then(|p| p.lines.get(l.line as usize))
            .map_or(0, |pl| pl.height);
        l.y + h - top
    };
    let k = f.lines.iter().take_while(|l| bottom(l) <= height).count();
    if k == 0 || k == f.lines.len() {
        return (None, f);
    }
    let cut = f.lines[k].y;
    let mut head = f.clone();
    head.lines.truncate(k);
    head.content = bottom(&f.lines[k - 1]);
    head.objects.retain(|o| o.rect.y < cut);
    head.tables.retain(|t| t.y < cut);
    let mut tail = f;
    tail.lines.drain(..k);
    tail.objects.retain(|o| o.rect.y >= cut);
    tail.tables.retain(|t| t.y >= cut);
    tail.content -= cut - top;
    let dy = top - cut;
    tail.lines.iter_mut().for_each(|l| l.y += dy);
    tail.objects.iter_mut().for_each(|o| o.rect.y += dy);
    tail.tables.iter_mut().for_each(|t| t.y += dy);
    (Some(head), tail)
}

/// The text boxes of a drawing object, its own first, then those of its
/// group members in stored order: the cells a [`CellStep`] indexes when it
/// steps into an object.
pub(crate) fn text_boxes(o: &hm_model::ObjectControl) -> Vec<&hm_model::TextBox> {
    fn walk<'a>(s: &'a hm_model::Shape, out: &mut Vec<&'a hm_model::TextBox>) {
        out.extend(s.text());
        for c in s.children() {
            walk(c, out);
        }
    }
    let mut out = Vec::new();
    if let Some(s) = o.shape() {
        walk(s, &mut out);
    }
    out
}

/// The footnotes in the text boxes of drawing object `o`: the step to each
/// note's control and the note.
pub(crate) fn in_object(o: &hm_model::ObjectControl) -> Vec<(CellStep, &Note)> {
    let mut out = Vec::new();
    for (k, tb) in text_boxes(o).into_iter().enumerate() {
        for (pi, p) in tb.paras.iter().enumerate() {
            for (ci, c) in p.controls.iter().enumerate() {
                if let Some(n) = c.as_note().filter(|n| !n.endnote) {
                    let step = CellStep {
                        cell: k as u32,
                        para: pi as u32,
                        control: ci as u32,
                    };
                    out.push((step, n));
                }
            }
        }
    }
    out
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
    pub list: ListRef,
    pub note: &'a hm_model::Note,
}

/// Endnotes in body order, including the cells and text boxes anchored there.
pub(crate) fn endnotes(section: u32, paras: &[hm_model::Paragraph]) -> Vec<Endnote<'_>> {
    fn collect<'a>(
        c: &'a hm_model::Control,
        [section, para, control]: [u32; 3],
        steps: &mut Vec<CellStep>,
        out: &mut Vec<Endnote<'a>>,
    ) {
        if let Some(note) = c.as_note().filter(|n| n.endnote) {
            let list = if steps.is_empty() {
                ListRef::Control {
                    section,
                    para,
                    control,
                }
            } else {
                ListRef::Cell {
                    section,
                    para,
                    control,
                    steps: steps.clone().into(),
                }
            };
            out.push(Endnote { list, note });
            return;
        }
        let lists: Vec<&[hm_model::Paragraph]> = if let Some(t) = c.as_table() {
            t.cells.iter().map(|c| c.paras.as_slice()).collect()
        } else if let Some(o) = c.as_object() {
            text_boxes(o)
                .into_iter()
                .map(|t| t.paras.as_slice())
                .collect()
        } else {
            return;
        };
        for (ci, paras) in lists.into_iter().enumerate() {
            for (pi, p) in paras.iter().enumerate() {
                for (ni, c) in p.controls.iter().enumerate() {
                    steps.push(CellStep {
                        cell: ci as u32,
                        para: pi as u32,
                        control: ni as u32,
                    });
                    collect(c, [section, para, control], steps, out);
                    steps.pop();
                }
            }
        }
    }
    let mut out = Vec::new();
    let mut steps = Vec::new();
    for (pi, p) in paras.iter().enumerate() {
        for (ci, c) in p.controls.iter().enumerate() {
            collect(c, [section, pi as u32, ci as u32], &mut steps, &mut out);
        }
    }
    out
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
    use crate::furniture::{FrameKind, FrameLine};
    if notes.is_empty() {
        return;
    }
    let s = shape.cloned().unwrap_or_default();
    let (above, below, between) = (
        s.space_above as i32,
        s.space_below as i32,
        s.space_between as i32,
    );
    // The separator and its gaps open the area where the body's last line
    // ends, without the spacing below that line.
    let end = |flow: &crate::paginate::Flow| match flow.line_end {
        Some((col, bottom)) if col == flow.col => bottom,
        _ => flow.fill[flow.col],
    };
    if flow.has_content && end(flow) + above + below >= flow.region_height() {
        flow.next_column();
    }
    let at = if flow.has_content {
        end(flow) + above
    } else {
        flow.fill[flow.col]
    };
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
    let mut carried = Vec::new();
    for (k, n) in notes.iter().enumerate() {
        let list = n.list.clone();
        let env = crate::para::ParaEnv {
            column_width: flow.column().1,
            default_tab,
        };
        let (paras, lines, _) = crate::table::stack_endnote(
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
        let mut prev_para = None;
        let mut start = flow.fill[flow.col];
        let mut omitted = 0;
        for l in &lines {
            let line = &paras[l.para as usize].lines[l.line as usize];
            // Lines keep their spacing from the list: the gap from the
            // previous line, or the note's start.
            let offset = prev_y.map_or(l.y, |p: i32| l.y - p);
            let removed = if prev_para.is_some_and(|pi| pi != l.para) {
                std::mem::take(&mut omitted)
            } else {
                0
            };
            let mut y = start + offset - removed;
            while flow.has_content && y + line.height >= flow.region_height() {
                if let (Some(f), Some(page)) = (frame.take(), flow.pages.last_mut()) {
                    page.frames.push(f);
                }
                flow.next_column();
                place_carried_endnote_objects(flow, &mut carried);
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
            // Floating objects hang from their paragraph in the column its
            // first line lands in.
            if l.line == 0
                && let Some(p) = n.note.list.paras.get(l.para as usize)
            {
                let pl = &paras[l.para as usize];
                let anchor = crate::objects::Anchor {
                    paper: flow.paper,
                    paper_x: None,
                    paper_y: None,
                    flow_top: None,
                    body: flow.body,
                    column: (cx, cw),
                    para_top: page_y - pl.before,
                    para_left: cx + pl.lines.first().map_or(0, |l| l.seg_x),
                    para_right: pl.right,
                };
                let placed = crate::objects::place(p, l.para, &anchor);
                let mut deferred = Vec::new();
                for object in placed {
                    let common = p.controls[object.control as usize].object_common();
                    if common.is_some_and(|o| {
                        o.text_wrap() == 1
                            && o.height + i32::from(o.margins[2]) + i32::from(o.margins[3])
                                <= flow.region_height()
                    }) && crate::objects::spills(p, &object, flow.body).is_some()
                    {
                        let bands = crate::objects::bands_of(p, pl, &anchor, &|k| {
                            k == object.control as usize
                        });
                        if let Some(band) = bands.own.first().or(bands.after.first()) {
                            deferred.push(object.control);
                            carried.push(Frame {
                                kind: FrameKind::Endnotes,
                                list: list.clone(),
                                rect: Rect {
                                    x: cx,
                                    y: band.top,
                                    w: cw,
                                    h: band.bottom - band.top,
                                },
                                paras: paras.clone(),
                                lines: Vec::new(),
                                objects: vec![object],
                                tables: Vec::new(),
                                content: band.bottom - band.top,
                            });
                        } else {
                            f.objects.push(object);
                        }
                    } else {
                        f.objects.push(object);
                    }
                }
                if !deferred.is_empty() {
                    // A drawing may continue in the next column while its
                    // anchor and following text finish this one. Remove only
                    // the room its box took in the unpaginated list.
                    let bands = crate::objects::bands(p, pl, &anchor);
                    let floor = bands
                        .own
                        .iter()
                        .chain(&bands.after)
                        .filter(|b| !b.square)
                        .map(|b| b.bottom)
                        .max()
                        .unwrap_or(page_y);
                    let kept = crate::objects::bands_of(p, pl, &anchor, &|k| {
                        !deferred.contains(&(k as u32))
                    });
                    let kept_floor = kept
                        .own
                        .iter()
                        .chain(&kept.after)
                        .filter(|b| !b.square)
                        .map(|b| b.bottom)
                        .max()
                        .unwrap_or(page_y);
                    omitted =
                        (floor - kept_floor.max(page_y + pl.lines_height() + pl.after)).max(0);
                }
            }
            f.rect.h = page_y + line.height - f.rect.y;
            f.content = f.rect.h;
            let col = flow.col;
            flow.fill[col] = y + line.height;
            flow.ink = flow.ink.max(y + line.height);
            flow.has_content = true;
            start = y;
            prev_y = Some(l.y);
            prev_para = Some(l.para);
        }
        if let (Some(f), Some(page)) = (frame, flow.pages.last_mut()) {
            page.frames.push(f);
        }
    }
    if !carried.is_empty() {
        flow.next_column();
        place_carried_endnote_objects(flow, &mut carried);
    }
}

/// Carried drawings open the next note column before its text resumes.
fn place_carried_endnote_objects(flow: &mut crate::paginate::Flow, carried: &mut Vec<Frame>) {
    for mut frame in carried.drain(..) {
        if flow.fill[flow.col] > 0 && flow.fill[flow.col] + frame.content > flow.region_height() {
            flow.next_column();
        }
        let (x, _) = flow.column();
        let top = flow.fill[flow.col];
        frame.shift(flow.page_y(top) - frame.rect.y);
        frame.shift_x(x - frame.rect.x);
        flow.fill[flow.col] = top + frame.content;
        flow.ink = flow.ink.max(flow.fill[flow.col]);
        flow.has_content = true;
        if let Some(page) = flow.pages.last_mut() {
            page.frames.push(frame);
        }
    }
}
