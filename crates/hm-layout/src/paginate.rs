//! Pages and columns.

use hm_model::units::unpack_distance;
use hm_model::{BorderLine, ColumnDef, Document, Paragraph, Section, break_kind};

use crate::furniture::{Frame, ListRef, Mark, PageBorder, Rule};
use crate::heads::Heads;
use crate::objects::{self, Anchor, Band};
use crate::para::{ParaEnv, ParaLayout, format};
use crate::style::Styles;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// A line placed on a page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlacedLine {
    /// Paragraph index in the section.
    pub para: u32,
    pub line: u32,
    /// Left edge of the column, page coordinates.
    pub x: i32,
    /// Top of the line, page coordinates.
    pub y: i32,
    /// Top of the line from the top of its column region, as Hancom stores `vpos`.
    pub vpos: i32,
}

#[derive(Clone, Debug, Default)]
pub struct Page {
    pub section: u32,
    /// Paper size.
    pub width: i32,
    pub height: i32,
    pub body: Rect,
    /// Column boxes of every region on the page.
    pub columns: Vec<Rect>,
    pub lines: Vec<PlacedLine>,
    /// Floating objects of body paragraphs anchored on this page.
    pub objects: Vec<objects::Placement>,
    /// Table fragments not set as characters.
    pub tables: Vec<crate::table::PlacedTable>,
    /// The page number as displayed: section starts and new numbers applied.
    pub number: u32,
    /// Paragraph lists placed outside the body flow: master pages, header,
    /// footer and notes.
    pub frames: Vec<Frame>,
    /// Column and note separators.
    pub rules: Vec<Rule>,
    /// Generated text such as the page number.
    pub marks: Vec<Mark>,
    /// The page border and background, when drawn.
    pub border: Option<PageBorder>,
}

/// The formatted paragraphs of one section.
#[derive(Clone, Debug, Default)]
pub struct SectionLayout {
    pub paras: Vec<ParaLayout>,
}

/// Default tab interval when a section carries none (40 pt).
const DEFAULT_TAB: i32 = 4000;

/// A section's default tab interval in HU.
pub(crate) fn default_tab(sec: &Section) -> i32 {
    sec.definition()
        .map(|d| unpack_distance(d.default_tab, || 1000))
        .filter(|t| *t > 0)
        .unwrap_or(DEFAULT_TAB)
}

pub(crate) struct Flow {
    pub(crate) pages: Vec<Page>,
    section: u32,
    pub(crate) paper: (i32, i32),
    pub(crate) body: Rect,
    /// Columns of the current region: `(x, width)` relative to the body.
    cols: Vec<(i32, i32)>,
    pub(crate) col: usize,
    /// Top of the current region from the body top.
    region_top: i32,
    /// Fill of each column of the region, from the region top.
    pub(crate) fill: Vec<i32>,
    pub(crate) has_content: bool,
    /// Bands blocked by objects on the current page (page coordinates).
    pub(crate) bands: Vec<Band>,
    /// Separator of the current column region, when it draws one.
    separator: Option<BorderLine>,
    /// Footnotes waiting at the foot of the page's columns.
    pub(crate) notes: crate::notes::NoteArea,
    /// The current region balances its columns when it ends.
    balanced: bool,
    /// Index of the region's first line among the current page's lines.
    region_first_line: usize,
    /// Empty lines hidden at the top of the current page.
    hidden_top: u8,
    /// Body x on odd and even pages when the binding faces pages.
    mirror: Option<(i32, i32)>,
    /// The number the current page takes, as the furniture will number it.
    number: u32,
    /// Footnotes in the cells of the current paragraph's tables that are
    /// not set as characters: control index, table y of the referring line
    /// and the laid-out note.
    table_notes: Vec<(u32, i32, Frame)>,
    /// Bottom of the lowest line set in the current region on this page,
    /// from the region top, without the spacing below it.
    pub(crate) ink: i32,
    /// The current region's separators are drawn.
    region_closed: bool,
    /// Flowing tables of the paragraph being placed that sit below its
    /// lines so far.
    pub(crate) deferred: crate::table::Deferred,
}

impl Flow {
    /// Draws the current region's column separators down to `bottom`
    /// (relative to the body top): one between each pair of columns, in the
    /// middle of the gap.
    fn close_region(&mut self, bottom: i32) {
        if std::mem::replace(&mut self.region_closed, true) {
            return;
        }
        let Some(line) = self.separator else { return };
        let Some(page) = self.pages.last_mut() else {
            return;
        };
        if bottom <= self.region_top {
            return;
        }
        // Separators sit between neighbours on the page, whichever way the
        // columns fill.
        let mut by_x = self.cols.clone();
        by_x.sort_by_key(|c| c.0);
        for pair in by_x.windows(2) {
            let x = self.body.x + (pair[0].0 + pair[0].1 + pair[1].0) / 2;
            page.rules.push(Rule {
                x1: x,
                y1: self.body.y + self.region_top,
                x2: x,
                y2: self.body.y + bottom,
                line,
            });
        }
    }

    /// Closes the region where its page ends. Hancom draws its separators
    /// down to the lowest line on the page, or to the body bottom under
    /// footnotes.
    fn close_page_region(&mut self) {
        let bottom = if self.notes.is_empty() {
            self.region_top + self.ink
        } else {
            self.body.h
        };
        self.close_region(bottom);
    }

    fn new_page(&mut self) {
        if !self.pages.is_empty() {
            self.close_page_region();
            self.flush_notes();
        }
        // Facing pages mirror the body on even pages.
        if let Some((odd, even)) = self.mirror {
            self.body.x = if self.pages.len() % 2 == 1 { even } else { odd };
        }
        let page = Page {
            section: self.section,
            width: self.paper.0,
            height: self.paper.1,
            body: self.body,
            ..Default::default()
        };
        self.pages.push(page);
        self.number += 1;
        self.region_first_line = 0;
        self.hidden_top = 0;
        self.region_top = 0;
        self.region_closed = false;
        self.ink = 0;
        self.col = 0;
        self.fill = vec![0; self.cols.len()];
        self.has_content = false;
        self.bands.clear();
        self.push_columns();
    }

    /// Page y of a region-relative y.
    pub(crate) fn page_y(&self, y: i32) -> i32 {
        self.body.y + self.region_top + y
    }

    /// The current column as page `(x, width)`.
    pub(crate) fn column(&self) -> (i32, i32) {
        let (x, w) = self.cols[self.col];
        (self.body.x + x, w)
    }

    fn push_columns(&mut self) {
        let (body, top) = (self.body, self.region_top);
        let page = self.pages.last_mut().unwrap();
        for &(x, w) in &self.cols {
            page.columns.push(Rect {
                x: body.x + x,
                y: body.y + top,
                w,
                h: body.h - top,
            });
        }
    }

    /// Room from the region top down to the column's limit: the body bottom,
    /// raised by the footnotes the column carries.
    pub(crate) fn region_height(&self) -> i32 {
        self.body.h - self.region_top - self.notes.reserve(self.column().0)
    }

    /// Moves the footnotes of the current paragraph's table `control` whose
    /// referring lines start in the table band `[top, bottom)` to the foot
    /// of the current column.
    pub(crate) fn take_table_notes(&mut self, control: u32, top: i32, bottom: i32) {
        if self.table_notes.is_empty() {
            return;
        }
        let (taken, kept) = std::mem::take(&mut self.table_notes)
            .into_iter()
            .partition(|(k, y, _)| *k == control && (top..bottom).contains(y));
        self.table_notes = kept;
        let taken: Vec<(u32, i32, Frame)> = taken;
        let (x, w) = self.column();
        self.notes
            .add(x, w, taken.into_iter().map(|(_, _, f)| f).collect());
    }

    /// Places the page's waiting footnotes.
    fn flush_notes(&mut self) {
        let body = self.body;
        if let Some(page) = self.pages.last_mut() {
            self.notes.flush(page, body);
        }
    }

    /// Moves to the next column, or the next page after the last one.
    pub(crate) fn next_column(&mut self) {
        if self.col + 1 < self.cols.len() {
            self.col += 1;
            self.has_content = false;
        } else {
            self.new_page();
        }
    }

    /// Starts a new column region below everything placed in the current one.
    pub(crate) fn column_count(&self) -> usize {
        self.cols.len()
    }

    /// Page x of column `c` of the region.
    pub(crate) fn column_x(&self, c: usize) -> i32 {
        self.body.x + self.cols.get(c).map_or(0, |c| c.0)
    }

    pub(crate) fn region_top(&self) -> i32 {
        self.region_top
    }

    /// Replaces the column fills and the ink bottom after the region's
    /// lines moved; `col` is the column the flow continues in.
    pub(crate) fn set_fill(&mut self, fill: Vec<i32>, col: usize, ink: i32) {
        if fill.len() == self.fill.len() {
            self.fill = fill;
            self.col = col;
            self.ink = ink;
        }
    }

    /// Ends the current region on this page: a balanced region evens its
    /// columns.
    fn end_region(&mut self, layouts: &[ParaLayout]) {
        if self.balanced {
            crate::columns::balance(self, self.region_first_line, layouts);
        }
    }

    fn new_region(&mut self, cols: Vec<(i32, i32)>, separator: Option<BorderLine>) {
        self.region_first_line = self.pages.last().map_or(0, |p| p.lines.len());
        let bottom = self.fill.iter().copied().max().unwrap_or(0);
        self.close_region(self.region_top + bottom);
        self.separator = separator;
        self.region_top += bottom;
        self.region_closed = false;
        self.ink = 0;
        self.cols = cols;
        self.col = 0;
        self.fill = vec![0; self.cols.len()];
        self.has_content = false;
        if self.region_top >= self.body.h {
            self.new_page();
        } else {
            self.push_columns();
        }
    }
}

/// The body of a page definition. Top binding (attribute bits 1-2 = 2)
/// takes the gutter from the top instead of the left.
fn body_of(pd: &hm_model::PageDef) -> (i32, i32, i32, i32) {
    let (x, y, w, h) = pd.body();
    let gutter = i32::try_from(pd.gutter).unwrap_or(0);
    if (pd.attr >> 1) & 3 == 2 && gutter > 0 {
        (x - gutter, y + gutter, w + gutter, (h - gutter).max(0))
    } else {
        (x, y, w, h)
    }
}

/// Body x on odd and even pages for facing pages (attribute bits 1-2 = 1):
/// even pages put the gutter and the left margin on the right.
fn mirror_of(pd: &hm_model::PageDef) -> Option<(i32, i32)> {
    if (pd.attr >> 1) & 3 != 1 {
        return None;
    }
    let (x, _, w, _) = pd.body();
    let paper = pd.paper().0;
    Some((x, paper - x - w))
}

/// The separator a column definition draws between its columns.
fn separator(def: Option<&ColumnDef>) -> Option<BorderLine> {
    let d = def.filter(|d| d.count() > 1 && d.line_kind != 0)?;
    Some(BorderLine {
        kind: d.line_kind,
        width: d.line_width,
        color: d.line_color,
    })
}

/// Column `(x, width)` pairs. Hancom's formatter works in 4-HU units, so a
/// column's width drops to a multiple of 4.
fn column_layout(def: Option<&ColumnDef>, width: i32) -> Vec<(i32, i32)> {
    let cols = def.map_or_else(|| vec![(0, width)], |c| c.layout(width));
    let cols: Vec<(i32, i32)> = cols.into_iter().map(|(x, w)| (x, w & !3)).collect();
    // Direction (attribute bits 10-11): 1 fills the columns from the right.
    if def.is_some_and(|d| (d.attr >> 10) & 3 == 1) {
        mirror_columns(&cols, width)
    } else {
        cols
    }
}

/// The same columns ordered from the right edge of a body `width` wide.
fn mirror_columns(cols: &[(i32, i32)], width: i32) -> Vec<(i32, i32)> {
    cols.iter().map(|&(x, w)| (width - x - w, w)).collect()
}

/// Paragraph protection before placing a paragraph: keep its lines
/// together, keep it with the next paragraph's first line, and avoid a lone
/// first or last line across a column end. Moves to the next column when the
/// whole paragraph should go there, or returns the line to break before.
///
/// Hancom does not document the line counts; one line is treated as a
/// widow or orphan, and a block that would not fit an empty column is left
/// to split normally.
fn protect(
    flow: &mut Flow,
    layout: &ParaLayout,
    shape: Option<&hm_model::ParaShape>,
    next: Option<&Paragraph>,
    next_shape: Option<&hm_model::ParaShape>,
    mut format_next: impl FnMut(&Paragraph, i32) -> ParaLayout,
) -> Option<usize> {
    let shape = shape?;
    if !flow.has_content || layout.lines.is_empty() {
        return None;
    }
    let avail = flow.region_height() - flow.fill[flow.col];
    let full = flow.body.h;
    let mut y = layout.before;
    let mut fit = 0;
    for l in &layout.lines {
        if y + l.height >= avail {
            break;
        }
        y += l.advance();
        fit += 1;
    }
    let n = layout.lines.len();
    let total = layout.before + layout.lines_height();
    if fit < n {
        if shape.keep_lines() && total < full {
            flow.next_column();
            return None;
        }
        if shape.widow_orphan() && n >= 2 {
            if fit == 1 {
                flow.next_column();
                return None;
            }
            if n - fit == 1 && fit >= 2 {
                return Some(fit - 1);
            }
        }
        return None;
    }
    if shape.keep_with_next()
        && let Some(np) =
            next.filter(|p| p.header.break_kind & (break_kind::PAGE | break_kind::COLUMN) == 0)
    {
        let nl = format_next(np, layout.column_width);
        // The next paragraph's first line, or all of it when it keeps its
        // lines together.
        let keeps = next_shape.is_some_and(|s| s.keep_lines());
        let span = match (keeps, nl.lines.split_last()) {
            (true, Some((last, rest))) => {
                rest.iter().map(|l| l.advance()).sum::<i32>() + last.height
            }
            (_, _) => nl.lines.first().map_or(0, |l| l.height),
        };
        let need = y + layout.after + nl.before + span;
        if need >= avail && total + layout.after + nl.before + span < full {
            flow.next_column();
        }
    }
    None
}

/// Formats and paginates one section, appending its pages.
pub(crate) fn section(
    heads: &mut Heads,
    doc: &Document,
    index: usize,
    sec: &Section,
    styles: &mut Styles<'_>,
    pages: &mut Vec<Page>,
    previous_number: u32,
) -> SectionLayout {
    let def = sec.definition();
    // Section option: at most two empty lines at a page top take no height.
    let hide_empty = def.is_some_and(|d| d.hide_empty_lines());
    let page_def = def.and_then(|d| d.page_def()).cloned().unwrap_or_default();
    let outline = def.map_or(0, |d| d.outline_numbering);
    let paper = page_def.paper();
    let (bx, by, bw, bh) = body_of(&page_def);
    let default_tab = default_tab(sec);
    let first_cols = sec
        .paras
        .first()
        .and_then(|p| p.controls.iter().find_map(|c| c.as_columns()));
    let mut flow = Flow {
        pages: std::mem::take(pages),
        section: index as u32,
        paper,
        body: Rect {
            x: bx,
            y: by,
            w: bw,
            h: bh,
        },
        cols: column_layout(first_cols, bw),
        col: 0,
        region_top: 0,
        fill: Vec::new(),
        has_content: false,
        bands: Vec::new(),
        separator: separator(first_cols),
        notes: crate::notes::NoteArea::new(def.and_then(|d| d.note_shapes().next())),
        balanced: first_cols.is_some_and(|c| c.kind() == 1),
        region_first_line: 0,
        hidden_top: 0,
        mirror: None,
        number: def.map_or(previous_number + 1, |d| {
            crate::furniture::first_number(d, &doc.info, previous_number)
        }) - 1,
        table_notes: Vec::new(),
        ink: 0,
        region_closed: false,
        deferred: Default::default(),
    };
    flow.mirror = mirror_of(&page_def);
    flow.new_page();
    let hwp5 = doc.meta.format == hm_model::Format::Hwp5;
    let geometry = (paper.0, paper.1, flow.body);
    let mut out = SectionLayout {
        paras: Vec::with_capacity(sec.paras.len()),
    };
    for (pi, para) in sec.paras.iter().enumerate() {
        let shape = doc.info.para_shape(para.header.para_shape as u32);
        let new_section = para.controls.iter().find_map(|c| c.as_section());
        if pi > 0
            && let Some(sd) = new_section
        {
            // A section definition later in the stream starts a new section
            // on a new page, with its own paper and columns.
            let pd = sd.page_def().cloned().unwrap_or_default();
            let (bx, by, bw, bh) = body_of(&pd);
            let cd = para.controls.iter().find_map(|c| c.as_columns());
            flow.mirror = mirror_of(&pd);
            flow.end_region(&out.paras);
            flow.close_page_region();
            flow.paper = pd.paper();
            flow.body = Rect {
                x: bx,
                y: by,
                w: bw,
                h: bh,
            };
            flow.cols = column_layout(cd, bw);
            flow.separator = separator(cd);
            flow.balanced = cd.is_some_and(|c| c.kind() == 1);
            flow.new_page();
        } else if pi > 0 {
            // A header with both the page and the column bit breaks the
            // column only, as Hancom does.
            let bits = para.header.break_kind;
            let page_break = (bits & break_kind::PAGE != 0 && bits & break_kind::COLUMN == 0)
                || shape.is_some_and(|s| s.page_break_before());
            let bw = flow.body.w;
            if let Some(cd) = para.controls.iter().find_map(|c| c.as_columns()) {
                flow.end_region(&out.paras);
                if page_break {
                    flow.close_page_region();
                    flow.cols = column_layout(Some(cd), bw);
                    flow.separator = separator(Some(cd));
                    flow.balanced = cd.kind() == 1;
                    flow.new_page();
                } else {
                    flow.new_region(column_layout(Some(cd), bw), separator(Some(cd)));
                    flow.balanced = cd.kind() == 1;
                }
            } else if page_break && (flow.has_content || flow.col > 0 || flow.region_top > 0) {
                flow.end_region(&out.paras);
                flow.new_page();
            } else if para.header.break_kind & break_kind::COLUMN != 0 && flow.has_content {
                flow.next_column();
            }
        }
        let env = |w| ParaEnv {
            column_width: w,
            default_tab,
        };
        let head = shape.and_then(|s| heads.next(s, &doc.info, outline));
        // Automatic page numbers in the paragraph, its tables or its
        // objects show the page it lands on; a new page number in it
        // numbers that page.
        let restart = para
            .controls
            .iter()
            .find_map(|c| crate::furniture::new_page_number(c, para, hwp5));
        let shows_number = !para.controls.is_empty()
            && crate::furniture::shows_page_number(std::slice::from_ref(para));
        let number_here = |flow: &Flow| restart.unwrap_or(flow.number);
        if shows_number {
            styles.page_number = number_here(&flow);
            // Hancom breaks the paragraph's own lines with the number it
            // stored and fits the page's number into that room.
            if para
                .controls
                .iter()
                .any(|c| crate::numbers::stored_page_number(c).is_some())
            {
                styles.stored_number_room = para as *const Paragraph as usize;
            }
        }
        let mut layout = format(
            para,
            &doc.info,
            styles,
            env(flow.cols[flow.col].1),
            head.as_ref(),
        );
        let left = doc
            .info
            .para_shape(para.header.para_shape as u32)
            .map_or(0, |s| s.left(1000));
        let anchor = |flow: &Flow| Anchor {
            paper: flow.paper,
            paper_x: None,
            body: flow.body,
            column: flow.column(),
            para_top: flow.page_y(flow.fill[flow.col]),
            para_left: flow.column().0 + left,
        };
        let next = sec.paras.get(pi + 1);
        let next_shape = next.and_then(|p| doc.info.para_shape(p.header.para_shape as u32));
        let break_before = protect(
            &mut flow,
            &layout,
            shape,
            next,
            next_shape,
            |next_para: &Paragraph, width| format(next_para, &doc.info, styles, env(width), None),
        );
        flow.table_notes = table_footnotes(
            doc,
            index as u32,
            pi as u32,
            para,
            &layout,
            flow.column(),
            geometry,
            styles,
            default_tab,
        );
        // Footnotes referred to in the paragraph's lines: its own and those
        // in the cells of its tables set as characters.
        let has_notes = para.controls.iter().any(|c| {
            c.as_note().is_some()
                || c.as_table().is_some_and(|t| {
                    t.header.common.treat_as_char() && crate::notes::table_has_notes(t)
                })
        });
        let host_top = flow.fill[flow.col] + layout.before;
        crate::table::before_host(&mut flow, para, &layout, host_top);
        // Paragraph-relative objects push the content after their paragraph,
        // and some push the paragraph's own lines too.
        let mut anchor_at_start = anchor(&flow);
        let objects::Bands {
            now,
            mut own,
            after: mut pending,
        } = objects::bands(para, &layout, &anchor_at_start);
        flow.bands.extend(now);
        // The paragraph top its own objects hang from, once placed.
        let mut own_top = None;
        let mut anchor_page = flow.pages.len();
        // A paragraph of nothing but its end.
        let empty = para.controls.is_empty()
            && layout.lines.len() == 1
            && para
                .units()
                .all(|(_, u)| matches!(u, hm_model::Unit::CharControl(13)));
        // The paragraph after this one starts a new page or column anyway,
        // or there is none.
        let next_breaks = next.is_none_or(|n| {
            n.header.break_kind & (break_kind::PAGE | break_kind::COLUMN) != 0
                || next_shape.is_some_and(|s| s.page_break_before())
                || n.controls.iter().any(|c| c.as_section().is_some())
        });
        let breaks_here = para.header.break_kind & (break_kind::PAGE | break_kind::COLUMN) != 0
            || shape.is_some_and(|s| s.page_break_before());
        let mut collapsed = false;
        let mut hidden = false;
        let mut li = 0;
        while li < layout.lines.len() {
            if break_before == Some(li) && flow.has_content {
                flow.next_column();
            }
            // The paragraph moved to a page of another number before its
            // first line: its page numbers change.
            if li == 0 && shows_number && styles.page_number != number_here(&flow) {
                styles.page_number = number_here(&flow);
                layout = format(
                    para,
                    &doc.info,
                    styles,
                    env(flow.cols[flow.col].1),
                    head.as_ref(),
                );
            }
            let before = if li == 0 { layout.before } else { 0 };
            let natural = flow.fill[flow.col] + before;
            let para_left = flow.column().0 + left;
            crate::table::before_line(&mut flow, pi as u32, para, &layout, li, natural, para_left);
            // Beside a square-wrapped object a fresh line breaks again for
            // the gap the object leaves, the rest of the paragraph at full
            // width until another line meets the object.
            if !layout.lines[li].stored {
                let line = layout.lines[li];
                let (cx, cw) = flow.column();
                let (_, gap) = objects::square_gap(
                    &flow.bands,
                    flow.page_y(natural),
                    line.height,
                    cx,
                    cx + cw,
                );
                if let Some((gl, gr)) = gap {
                    let seg_left = cx + line.seg_x;
                    let (gl, gr) = (gl.max(seg_left), gr.min(seg_left + line.seg_width));
                    let content = (cx + line.x, cx + line.x + line.used);
                    let inside = content.0 >= gl && content.1 <= gr;
                    if !inside && gr - gl >= (2 * line.height).max(2000) {
                        let full = line.seg_width;
                        crate::para::reflow(&mut layout, para, &doc.info, styles, li, &mut |k| {
                            if k == li {
                                (gl - seg_left, gr - gl)
                            } else {
                                (0, full)
                            }
                        });
                    }
                }
            }
            let mut moved = false;
            let mut notes;
            let mut shift;
            let vpos = loop {
                // With the section's option on, an empty line that reaches
                // the top of a page (at most two) takes no height; one that
                // starts the page by its own break keeps its line.
                if empty && hide_empty && !breaks_here && !flow.has_content && flow.hidden_top < 2 {
                    collapsed = true;
                    hidden = true;
                    notes = Vec::new();
                    shift = 0;
                    break flow.fill[flow.col];
                }
                let line = &layout.lines[li];
                let want = flow.fill[flow.col] + before;
                let (cx, cw) = flow.column();
                // Footnotes referred to in the line go to this column too.
                notes = if !has_notes {
                    Vec::new()
                } else {
                    footnotes(
                        doc,
                        index as u32,
                        pi as u32,
                        para,
                        &layout,
                        line,
                        (cx, cw),
                        geometry,
                        styles,
                        default_tab,
                    )
                };
                let limit = flow.region_height() - flow.notes.extra(cx, &notes);
                // A stored line already sits beside the objects it wraps.
                let span = if line.stored {
                    (0, 0)
                } else {
                    (line.x, line.x + line.used)
                };
                let clear = |bands: &[objects::Band]| {
                    objects::clear_line(
                        bands,
                        flow.page_y(want),
                        (line.height, line.advance()),
                        span,
                        cx,
                        cx + cw,
                    )
                };
                let (top, dx) = if own.is_empty() || flow.pages.len() != anchor_page {
                    clear(&flow.bands)
                } else {
                    // The objects hang from where the paragraph starts
                    // before they push its first line.
                    if li == 0 {
                        let start = clear(&flow.bands).0 - flow.page_y(0);
                        let natural = flow.fill[flow.col];
                        let at = if start > natural + before {
                            start
                        } else {
                            natural
                        };
                        own_top = Some(at);
                        anchor_at_start.para_top = flow.page_y(at);
                        own = objects::bands(para, &layout, &anchor_at_start).own;
                    }
                    let all: Vec<objects::Band> = flow.bands.iter().chain(&own).copied().collect();
                    clear(&all)
                };
                shift = dx;
                let y = top - flow.page_y(0);
                // An empty column takes the line even when it does not fit,
                // so a line taller than the page cannot loop forever.
                let too_tall = want + line.height > limit;
                if y + line.height < limit || (!flow.has_content && (too_tall || moved)) {
                    break y;
                }
                // An empty paragraph that does not fit before a paragraph that
                // starts a new page anyway, or at the end, is tried again with
                // no extent and no gaps: it stays at the foot of the column,
                // taking no room, when the column's fill is still inside it.
                if empty && next_breaks && flow.has_content && flow.fill[flow.col] < limit {
                    collapsed = true;
                    break y;
                }
                moved = true;
                let page = flow.pages.len();
                flow.next_column();
                // A column of another width, or a page of another number,
                // reflows the paragraph from scratch.
                let renumber = shows_number && styles.page_number != number_here(&flow);
                if li == 0 && (flow.cols[flow.col].1 != layout.column_width || renumber) {
                    styles.page_number = if shows_number { number_here(&flow) } else { 0 };
                    layout = format(
                        para,
                        &doc.info,
                        styles,
                        env(flow.cols[flow.col].1),
                        head.as_ref(),
                    );
                }
                if li == 0 && flow.pages.len() != page {
                    anchor_at_start = anchor(&flow);
                    let b = objects::bands(para, &layout, &anchor_at_start);
                    flow.bands.extend(b.now);
                    (own, pending) = (b.own, b.after);
                    anchor_page = flow.pages.len();
                }
            };
            if li == 0 {
                // Paragraph-relative objects hang from where the paragraph
                // lands: below an object that pushed its first line down.
                let natural = flow.fill[flow.col];
                let top = own_top.unwrap_or(if vpos > natural + before {
                    vpos
                } else {
                    natural
                });
                anchor_at_start.para_top = flow.page_y(top);
                let b = objects::bands(para, &layout, &anchor_at_start);
                (own, pending) = (b.own, b.after);
            }
            let line = layout.lines[li];
            let x = flow.column().0 + shift;
            let y = flow.page_y(vpos);
            let line_top = vpos;
            let vpos = vpos + flow.region_top;
            flow.pages.last_mut().unwrap().lines.push(PlacedLine {
                para: pi as u32,
                line: li as u32,
                x,
                y,
                vpos,
            });
            flow.fill[flow.col] =
                vpos - flow.region_top + if collapsed { 0 } else { line.advance() };
            if !collapsed {
                flow.ink = flow.ink.max(line_top + line.height);
            }
            if hidden {
                flow.hidden_top += 1;
            } else {
                flow.has_content = true;
            }
            let (cx, cw) = flow.column();
            flow.notes.add(cx, cw, notes);
            // Tables anchored in this line: the rest of the paragraph
            // continues below the flowing ones.
            let para_left = flow.column().0 + left;
            crate::table::after_line(&mut flow, pi as u32, para, &layout, li, line_top, para_left);
            if let Some(v) = restart
                && para.units().any(|(pos, u)| {
                    pos >= line.start
                        && pos < line.end
                        && matches!(u, hm_model::Unit::Extended { index, .. }
                        if para.controls.get(index as usize).is_some_and(|c| {
                            crate::furniture::new_page_number(c, para, hwp5).is_some()
                        }))
                })
            {
                flow.number = v;
            }
            li += 1;
        }
        styles.page_number = 0;
        styles.stored_number_room = 0;
        flow.table_notes.clear();
        if !collapsed {
            flow.fill[flow.col] += layout.after;
        }
        if flow.pages.len() == anchor_page {
            flow.bands.extend(own);
            flow.bands.extend(pending);
        }
        // Floating objects sit on the page where their paragraph starts.
        if let Some(page) = flow.pages.get_mut(anchor_page - 1) {
            page.objects
                .extend(objects::place(para, pi as u32, &anchor_at_start));
        }
        out.paras.push(layout);
    }
    // Endnotes follow the body at the end of their section or of the
    // document, as the section's endnote shape (bits 8-9) asks.
    fn endnote_shape(s: &Section) -> Option<&hm_model::NoteShape> {
        s.definition().and_then(|d| d.note_shapes().nth(1))
    }
    let at_section_end = |s: &Section| endnote_shape(s).is_some_and(|n| (n.attr >> 8) & 3 == 1);
    let scope: Vec<usize> = if at_section_end(sec) {
        vec![index]
    } else if index + 1 == doc.sections.len() {
        (0..doc.sections.len())
            .filter(|&k| !at_section_end(&doc.sections[k]))
            .collect()
    } else {
        Vec::new()
    };
    let mut endnotes = Vec::new();
    for k in scope {
        for (pi, p) in doc.sections[k].paras.iter().enumerate() {
            for (ci, c) in p.controls.iter().enumerate() {
                if let Some(note) = c.as_note().filter(|n| n.endnote) {
                    endnotes.push(crate::notes::Endnote {
                        section: k as u32,
                        para: pi as u32,
                        control: ci as u32,
                        note,
                    });
                }
            }
        }
    }
    flow.end_region(&out.paras);
    crate::notes::place_endnotes(
        &mut flow,
        &endnotes,
        endnote_shape(sec),
        &doc.info,
        styles,
        default_tab,
    );
    flow.close_page_region();
    flow.flush_notes();
    *pages = flow.pages;
    out
}

/// A footnote's frame laid out for the column `(x, width)`, its top at 0.
#[allow(clippy::too_many_arguments)]
fn note_frame(
    doc: &Document,
    list: ListRef,
    note: &hm_model::Note,
    (x, w): (i32, i32),
    geometry: (i32, i32, Rect),
    styles: &mut Styles<'_>,
    default_tab: i32,
) -> Frame {
    crate::furniture::frame(
        crate::furniture::FrameKind::Footnotes,
        list,
        &note.list,
        Rect { x, y: 0, w, h: 0 },
        geometry,
        &doc.info,
        styles,
        ParaEnv {
            column_width: w,
            default_tab,
        },
    )
}

/// Frames of the footnotes referred to in `line`, laid out for the column
/// `(x, width)`, their tops at 0: the line's own notes and those in the
/// cells of its tables set as characters, in reading order.
#[allow(clippy::too_many_arguments)]
fn footnotes(
    doc: &Document,
    section: u32,
    pi: u32,
    para: &Paragraph,
    layout: &ParaLayout,
    line: &crate::para::Line,
    column: (i32, i32),
    geometry: (i32, i32, Rect),
    styles: &mut Styles<'_>,
    default_tab: i32,
) -> Vec<Frame> {
    let mut out = Vec::new();
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
        if let Some(note) = c.as_note().filter(|n| !n.endnote) {
            let list = ListRef::Control {
                section,
                para: pi,
                control: index,
            };
            out.push(note_frame(
                doc,
                list,
                note,
                column,
                geometry,
                styles,
                default_tab,
            ));
        } else if let Some(t) = c.as_table().filter(|t| t.header.common.treat_as_char())
            && let Some(tl) = layout.table(index)
        {
            for n in crate::notes::in_table(t, tl) {
                let list = ListRef::Cell {
                    section,
                    para: pi,
                    control: index,
                    steps: n.steps.into(),
                };
                out.push(note_frame(
                    doc,
                    list,
                    n.note,
                    column,
                    geometry,
                    styles,
                    default_tab,
                ));
            }
        }
    }
    out
}

/// The footnotes in the cells of a paragraph's tables that are not set as
/// characters, laid out for the column `(x, width)`: control index, table y
/// of the referring line and frame. Table placement takes them as it places
/// each fragment.
#[allow(clippy::too_many_arguments)]
fn table_footnotes(
    doc: &Document,
    section: u32,
    pi: u32,
    para: &Paragraph,
    layout: &ParaLayout,
    column: (i32, i32),
    geometry: (i32, i32, Rect),
    styles: &mut Styles<'_>,
    default_tab: i32,
) -> Vec<(u32, i32, Frame)> {
    let mut out = Vec::new();
    for (k, c) in para.controls.iter().enumerate() {
        let Some(t) = c.as_table() else { continue };
        if t.header.common.treat_as_char() || !crate::notes::table_has_notes(t) {
            continue;
        }
        let Some(tl) = layout.table(k as u32) else {
            continue;
        };
        for n in crate::notes::in_table(t, tl) {
            let list = ListRef::Cell {
                section,
                para: pi,
                control: k as u32,
                steps: n.steps.into(),
            };
            let f = note_frame(doc, list, n.note, column, geometry, styles, default_tab);
            out.push((k as u32, n.y, f));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn right_to_left_columns_fill_from_the_right() {
        let def = ColumnDef {
            attr: (2 << 2) | (1 << 10) | (1 << 12),
            gap: 1000,
            ..Default::default()
        };
        // Two 4000-wide columns in 9000: the first sits at the right.
        assert_eq!(
            column_layout(Some(&def), 9000),
            vec![(5000, 4000), (0, 4000)]
        );
    }
}
