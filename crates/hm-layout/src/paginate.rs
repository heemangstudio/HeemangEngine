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
    /// Per page of the section, the height its notes under all columns
    /// took (for a second pass that reserves it from the page's start).
    pub(crate) merged_notes: Vec<i32>,
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
    /// The current region's columns are parallel: each runs on in itself.
    parallel: bool,
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
    /// The column of the last line or table placed in the current region on
    /// this page, and its bottom from the region top without the spacing
    /// below it.
    pub(crate) line_end: Option<(usize, i32)>,
    /// The current region's separators are drawn.
    region_closed: bool,
    /// The section's gap between column regions on one page.
    region_gap: i32,
    /// Index of the section's first page in `pages`.
    first_page: usize,
    /// Per page of the section, the height to keep clear for notes under
    /// all columns from the page's start, known from a first pass.
    merged_reserve: Vec<i32>,
    /// Per page of the section, the height those notes took.
    merged_seen: Vec<i32>,
    /// Flowing tables of the paragraph being placed that sit below its
    /// lines so far.
    pub(crate) deferred: crate::table::Deferred,
    /// A split square-wrapped table whose next fragment waits for the next
    /// column.
    pub(crate) pending_table: Option<Box<crate::table::Pending>>,
    /// Top-and-bottom objects moved past the foot of the page their
    /// paragraph stays on: they open the next page, which their bands
    /// keep the lines off.
    carried_objects: Vec<crate::objects::Placement>,
    carried_bands: Vec<Band>,
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
        self.flush_tables();
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
        self.line_end = None;
        self.col = 0;
        self.fill = vec![0; self.cols.len()];
        self.has_content = false;
        self.bands.clear();
        self.bands.append(&mut self.carried_bands);
        let carried = std::mem::take(&mut self.carried_objects);
        if let Some(page) = self.pages.last_mut() {
            page.objects.extend(carried);
        }
        self.push_columns();
        let (x, w) = self.note_column();
        self.notes.adopt_carried(x, w);
    }

    /// Page y of a region-relative y.
    pub(crate) fn page_y(&self, y: i32) -> i32 {
        self.body.y + self.region_top + y
    }

    /// Where the current column's footnotes go, as page `(x, width)`: its
    /// own foot, the full body width, or the rightmost column, as the note
    /// shape asks.
    pub(crate) fn note_column(&self) -> (i32, i32) {
        match self.notes.placement() {
            1 => (self.body.x, self.body.w),
            2 => {
                let (x, w) = self
                    .cols
                    .iter()
                    .copied()
                    .max_by_key(|c| c.0)
                    .unwrap_or((0, 0));
                (self.body.x + x, w)
            }
            _ => self.column(),
        }
    }

    /// The room left in the current column with `extra` more note height:
    /// notes under all columns keep at least the room a first pass found
    /// they take on this page.
    pub(crate) fn notes_limit(&self, extra: i32) -> i32 {
        if self.notes.placement() == 1 {
            let k = self.pages.len().saturating_sub(1 + self.first_page);
            let known = self.merged_reserve.get(k).copied().unwrap_or(0);
            let (nx, _) = self.note_column();
            return self.body.h - self.region_top - (self.notes.reserve(nx) + extra).max(known);
        }
        self.region_height() - extra
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
        let (nx, _) = self.note_column();
        // Notes kept in the rightmost column shorten only that column.
        let reserve = match self.notes.placement() {
            2 if self.column().0 != nx => 0,
            1 => {
                let k = self.pages.len().saturating_sub(1 + self.first_page);
                let known = self.merged_reserve.get(k).copied().unwrap_or(0);
                self.notes.reserve(nx).max(known)
            }
            _ => self.notes.reserve(nx),
        };
        self.body.h - self.region_top - reserve
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
        let (x, w) = self.note_column();
        self.notes
            .add(x, w, taken.into_iter().map(|(_, _, f)| f).collect());
    }

    /// Places the page's waiting footnotes.
    fn flush_notes(&mut self) {
        let body = self.body;
        if self.notes.placement() == 1 {
            let k = self.pages.len().saturating_sub(1 + self.first_page);
            if self.merged_seen.len() <= k {
                self.merged_seen.resize(k + 1, 0);
            }
            self.merged_seen[k] = self.notes.reserve(body.x);
        }
        if let Some(page) = self.pages.last_mut() {
            self.notes.flush(page, body);
        }
    }

    /// Moves to the next column, or the next page after the last one.
    /// Moves to the next column for a column break, which parallel columns
    /// take like any other.
    fn break_column(&mut self) {
        let parallel = std::mem::replace(&mut self.parallel, false);
        self.next_column();
        self.parallel = parallel;
    }

    pub(crate) fn next_column(&mut self) {
        let waiting = self.pending_table.take();
        self.move_column();
        if let Some(p) = waiting {
            crate::table::resume(self, *p);
        }
    }

    /// Places the rest of a waiting table, column by column.
    pub(crate) fn flush_tables(&mut self) {
        let mut guard = 0;
        while self.pending_table.is_some() && guard < 100_000 {
            guard += 1;
            self.next_column();
        }
        self.pending_table = None;
    }

    pub(crate) fn table_notes_mut(&mut self) -> &mut Vec<(u32, i32, Frame)> {
        &mut self.table_notes
    }

    /// Takes the waiting footnotes of the cells of table `control`.
    pub(crate) fn take_notes_of(&mut self, control: u32) -> Vec<(u32, i32, Frame)> {
        let (taken, kept) = std::mem::take(&mut self.table_notes)
            .into_iter()
            .partition(|(k, _, _)| *k == control);
        self.table_notes = kept;
        taken
    }

    fn move_column(&mut self) {
        // A parallel column that is full runs on in the same column of the
        // next page.
        if self.parallel && self.has_content {
            let col = self.col;
            self.new_page();
            if col < self.cols.len() {
                self.col = col;
                let (x, w) = self.note_column();
                self.notes.adopt_carried(x, w);
            }
            return;
        }
        if self.col + 1 < self.cols.len() {
            self.col += 1;
            self.has_content = false;
            let (x, w) = self.note_column();
            self.notes.adopt_carried(x, w);
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
        // Hancom starts the new region the section's region gap below the
        // lowest line of the last one, without that line's spacing.
        self.region_top += if self.ink > 0 {
            self.ink + self.region_gap
        } else {
            bottom
        };
        self.region_closed = false;
        self.ink = 0;
        self.line_end = None;
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

/// A section's character grid; manuscript paper is attribute bit 22.
fn char_grid_of(d: &hm_model::SectionDef) -> crate::style::CharGrid {
    crate::style::CharGrid {
        pitch: i32::from(d.horizontal_grid).max(0),
        manuscript: d.attr & (1 << 22) != 0,
    }
}

/// `v` moved down to the next line of a grid of pitch `grid` whose lines
/// start `origin` above the zero of `v`; any `v` when there is no grid.
fn on_grid(v: i32, origin: i32, grid: i32) -> i32 {
    if grid <= 0 {
        return v;
    }
    (v + origin + grid - 1).div_euclid(grid) * grid - origin
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
#[allow(clippy::too_many_arguments)]
pub(crate) fn section(
    heads: &mut Heads,
    doc: &Document,
    index: usize,
    sec: &Section,
    styles: &mut Styles<'_>,
    pages: &mut Vec<Page>,
    previous_number: u32,
    merged_reserve: &[i32],
) -> SectionLayout {
    let def = sec.definition();
    // Section option: at most two empty lines at a page top take no height.
    let hide_empty = def.is_some_and(|d| d.hide_empty_lines());
    // Line grid pitch: paragraphs that snap to it start on its lines and
    // take whole grid lines.
    let mut line_grid = def.map_or(0, |d| i32::from(d.vertical_grid));
    let mut char_grid = def.map(char_grid_of).unwrap_or_default();
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
        first_page: pages.len(),
        merged_reserve: merged_reserve.to_vec(),
        merged_seen: Vec::new(),
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
        parallel: first_cols.is_some_and(|c| c.kind() == 2),
        region_first_line: 0,
        hidden_top: 0,
        mirror: None,
        number: def.map_or(previous_number + 1, |d| {
            crate::furniture::first_number(d, &doc.info, previous_number)
        }) - 1,
        table_notes: Vec::new(),
        ink: 0,
        line_end: None,
        region_closed: false,
        region_gap: def.map_or(0, |d| i32::from(d.column_gap)),
        deferred: Default::default(),
        pending_table: None,
        carried_objects: Vec::new(),
        carried_bands: Vec::new(),
    };
    flow.mirror = mirror_of(&page_def);
    flow.new_page();
    let hwp5 = doc.meta.format != hm_model::Format::Hwpx;
    let geometry = (paper.0, paper.1, flow.body);
    let mut out = SectionLayout {
        paras: Vec::with_capacity(sec.paras.len()),
        merged_notes: Vec::new(),
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
            flow.parallel = cd.is_some_and(|c| c.kind() == 2);
            flow.region_gap = i32::from(sd.column_gap);
            line_grid = i32::from(sd.vertical_grid);
            char_grid = char_grid_of(sd);
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
                    flow.parallel = cd.kind() == 2;
                    flow.new_page();
                } else {
                    flow.new_region(column_layout(Some(cd), bw), separator(Some(cd)));
                    flow.balanced = cd.kind() == 1;
                    flow.parallel = cd.kind() == 2;
                }
            } else if page_break && (flow.has_content || flow.col > 0 || flow.region_top > 0) {
                flow.end_region(&out.paras);
                flow.new_page();
            } else if para.header.break_kind & break_kind::COLUMN != 0
                && flow.has_content
                && !(is_empty(para) && page_follows(sec, pi))
            {
                // An empty paragraph does not break its column when a new
                // page follows it anyway. In balanced columns a column break
                // ends the balanced block: the text goes on in a new block
                // of the same columns below it.
                // Parallel columns go back to the first column below the
                // block when the last one breaks.
                if flow.balanced || (flow.parallel && flow.col + 1 == flow.column_count()) {
                    flow.end_region(&out.paras);
                    let (cols, sep) = (flow.cols.clone(), flow.separator);
                    flow.new_region(cols, sep);
                } else {
                    flow.break_column();
                }
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
        let snaps = shape.is_some_and(|s| s.snap_to_grid());
        styles.char_grid = if snaps && char_grid.pitch > 0 {
            (para as *const Paragraph as usize, char_grid)
        } else {
            Default::default()
        };
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
            flow.note_column(),
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
                || c.as_object()
                    .is_some_and(|o| !crate::notes::in_object(o).is_empty())
        });
        let host_top = flow.fill[flow.col] + layout.before;
        crate::table::before_host(&mut flow, para, &layout, host_top);
        // Paragraph-relative objects hang from the line holding them: those
        // of the first line from the paragraph's top, the others as their
        // line comes (`hung` lists those placed).
        let mut hung: Vec<usize> = Vec::new();
        let first_line =
            |pl: &crate::para::ParaLayout, k: usize| objects::anchor_line(para, pl, k) == 0;
        // Paragraph-relative objects push the content after their paragraph,
        // and some push the paragraph's own lines too.
        let mut anchor_at_start = anchor(&flow);
        let objects::Bands {
            now,
            mut own,
            after: mut pending,
        } = objects::bands_of(para, &layout, &anchor_at_start, &|k| first_line(&layout, k));
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
        // Lines sharing a row beside square-wrapped objects: first and last
        // index, and the row's top once its first line is placed.
        let mut row: Option<(usize, usize, Option<i32>)> = None;
        let grid = if snaps { line_grid.max(0) } else { 0 };
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
            // Objects hanging from this line block their bands from its top
            // on and sit on this page.
            if li > 0 {
                let late =
                    |k: usize| !hung.contains(&k) && objects::anchor_line(para, &layout, k) == li;
                let at = Anchor {
                    para_top: flow.page_y(natural),
                    ..anchor(&flow)
                };
                let b = objects::bands_of(para, &layout, &at, &late);
                let placed = objects::place_of(para, pi as u32, &at, &late);
                hung.extend(placed.iter().map(|p| p.control as usize));
                flow.bands
                    .extend(b.now.into_iter().chain(b.own).chain(b.after));
                if let Some(page) = flow.pages.last_mut() {
                    page.objects.extend(placed);
                }
            }
            // Beside square-wrapped objects a fresh line breaks again for
            // the gaps they leave, left to right on one row; the rest of the
            // paragraph goes on at full width until another line meets one.
            let in_row = row.is_some_and(|(start, end, _)| li > start && li <= end);
            if !in_row {
                row = None;
            }
            if !in_row && !layout.lines[li].stored {
                let line = layout.lines[li];
                let (cx, cw) = flow.column();
                let near: Vec<objects::Band> = if flow.pages.len() == anchor_page {
                    flow.bands.iter().chain(&own).copied().collect()
                } else {
                    flow.bands.clone()
                };
                let (_, gaps) =
                    objects::square_gaps(&near, flow.page_y(natural), line.height, cx, cx + cw);
                let seg_left = cx + line.seg_x;
                let seg_right = seg_left + line.seg_width;
                let mut gaps: Vec<(i32, i32)> = gaps
                    .into_iter()
                    .map(|(a, z)| (a.max(seg_left), z.min(seg_right)))
                    .filter(|g| g.1 - g.0 >= objects::MIN_GAP)
                    .collect();
                // A gap too narrow for the next character stays empty.
                let first = layout
                    .measured
                    .items
                    .get(line.first_item as usize)
                    .and_then(|&(pos, _)| layout.measured.advances.get(pos as usize))
                    .copied()
                    .unwrap_or(0);
                while gaps.len() > 1 && gaps[0].1 - gaps[0].0 < first {
                    gaps.remove(0);
                }
                let content = (cx + line.x, cx + line.x + line.used);
                let inside = gaps.len() == 1 && content.0 >= gaps[0].0 && content.1 <= gaps[0].1;
                if !gaps.is_empty() && !inside {
                    let full = line.seg_width;
                    crate::para::reflow(&mut layout, para, &doc.info, styles, li, &mut |k| match k
                        .checked_sub(li)
                        .and_then(|i| gaps.get(i))
                    {
                        Some(&(a, z)) => (a - seg_left, z - a),
                        None => (0, full),
                    });
                    if gaps.len() > 1 {
                        row = Some((li, li + gaps.len() - 1, None));
                    }
                }
            }
            let mut moved = false;
            let mut carry = false;
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
                // A later fragment of a row sits beside the first.
                if let Some((_, _, Some(top))) = row.filter(|_| in_row) {
                    notes = Vec::new();
                    shift = 0;
                    break top;
                }
                let want = on_grid(flow.fill[flow.col] + before, flow.region_top, grid);
                // A grid line fits by the cells its text occupies, even
                // when the glyphs alone would fit at the foot of the page.
                let extent = if grid > 0 {
                    on_grid(line.height, 0, grid).max(grid)
                } else {
                    line.height
                };
                let (cx, cw) = flow.column();
                let (nx, nw) = flow.note_column();
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
                        (nx, nw),
                        geometry,
                        styles,
                        default_tab,
                    )
                };
                let extra = if flow.notes.placement() == 2 && cx != nx {
                    0
                } else {
                    flow.notes.extra(nx, &notes)
                };
                let limit = flow.notes_limit(extra);
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
                        line.stored
                            .then_some((line.seg_x, line.seg_x + line.seg_width)),
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
                        own = objects::bands_of(para, &layout, &anchor_at_start, &|k| {
                            first_line(&layout, k)
                        })
                        .own;
                    }
                    let all: Vec<objects::Band> = flow.bands.iter().chain(&own).copied().collect();
                    clear(&all)
                };
                shift = dx;
                let y = on_grid(top - flow.page_y(0), flow.region_top, grid);
                // An empty column takes the line even when it does not fit,
                // so a line taller than the page cannot loop forever.
                let too_tall = want + extent > limit;
                if y + extent < limit || (!flow.has_content && (too_tall || moved)) {
                    break y;
                }
                // A line that fits while its footnotes do not stays; the
                // notes go on in the next column.
                if !notes.is_empty() && y + extent < flow.region_height() {
                    carry = true;
                    break y;
                }
                // An empty paragraph that does not fit before a paragraph that
                // starts a new page anyway, or at the end, is tried again with
                // no extent and no gaps: it stays at the foot of the column,
                // taking no room, when the column's fill is still inside it.
                // A replayed page start still moves on when another break
                // follows it. Rejected placeholder segments do not mark one.
                let next_empty = next.is_some_and(|n| {
                    n.controls.is_empty()
                        && n.units()
                            .all(|(_, u)| matches!(u, hm_model::Unit::CharControl(13)))
                }) && para.line_segs.first().is_none_or(|s| s.vpos != 0);
                // Paragraphs typed since reading (no source) move on as
                // Hancom moves them when it is editing.
                if empty
                    && para.src.is_some()
                    && (!line.stored || para.line_segs.first().is_none_or(|s| s.vpos != 0))
                    && (next_breaks || next_empty)
                    && flow.has_content
                    && flow.fill[flow.col] <= limit
                {
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
                    let b = objects::bands_of(para, &layout, &anchor_at_start, &|k| {
                        first_line(&layout, k)
                    });
                    flow.bands.extend(b.now);
                    (own, pending) = (b.own, b.after);
                    anchor_page = flow.pages.len();
                }
            };
            if let Some((start, end, None)) = row
                && start == li
            {
                row = Some((start, end, Some(vpos)));
            }
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
                let b =
                    objects::bands_of(para, &layout, &anchor_at_start, &|k| first_line(&layout, k));
                (own, pending) = (b.own, b.after);
            }
            let line = layout.lines[li];
            let x = flow.column().0 + shift;
            // A line on the grid takes the grid lines its height and spacing
            // need, centred in them.
            let span = if grid > 0 {
                on_grid(line.advance(), 0, grid).max(grid)
            } else {
                line.advance()
            };
            let y = flow.page_y(vpos) + (span - line.advance()) / 2;
            let line_top = vpos;
            let vpos = vpos + flow.region_top;
            flow.pages.last_mut().unwrap().lines.push(PlacedLine {
                para: pi as u32,
                line: li as u32,
                x,
                y,
                vpos,
            });
            let bottom = vpos - flow.region_top + if collapsed { 0 } else { span };
            flow.fill[flow.col] = if in_row {
                flow.fill[flow.col].max(bottom)
            } else {
                bottom
            };
            if !collapsed {
                flow.ink = flow.ink.max(line_top + line.height);
                flow.line_end = Some((flow.col, line_top + line.height));
            }
            if hidden {
                flow.hidden_top += 1;
            } else {
                flow.has_content = true;
            }
            let (nx, nw) = flow.note_column();
            if carry {
                // The column's foot takes what fits of the notes below the
                // line; they go on in the next column.
                let room = flow.region_height() - (line_top + line.height) - 1;
                let (fit, rest) = flow.notes.split_to_fit(nx, notes, room);
                flow.notes.add(nx, nw, fit);
                flow.notes.carry(nx, nw, rest);
            } else {
                flow.notes.add(nx, nw, notes);
            }
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
            // An odd/even page adjustment renumbers the page that holds it.
            for (pos, u) in para.units() {
                if let hm_model::Unit::Extended { index, .. } = u
                    && pos >= line.start
                    && pos < line.end
                    && let Some(hm_model::ControlKind::PageParity(p)) =
                        para.controls.get(index as usize).map(|c| &c.kind)
                {
                    flow.number = crate::furniture::with_parity(flow.number, p.attr & 3);
                }
            }
            li += 1;
        }
        styles.page_number = 0;
        styles.stored_number_room = 0;
        styles.char_grid = Default::default();
        flow.table_notes.clear();
        if !collapsed {
            flow.fill[flow.col] =
                on_grid(flow.fill[flow.col] + layout.after, flow.region_top, grid);
        }
        // Floating objects sit on the page where their paragraph starts;
        // one kept inside the body that would cross its foot goes on with
        // the paragraph to the next page, hanging from the body top there.
        let spill_top = Anchor {
            para_top: flow.body.y,
            ..anchor(&flow)
        };
        let placed = objects::place_of(para, pi as u32, &anchor_at_start, &|k| {
            first_line(&layout, k)
        });
        // A top-and-bottom one crossing the foot of the page its paragraph
        // stays on opens the next page instead, and the text after it goes
        // on filling this one.
        let moved: Vec<usize> = if flow.pages.len() == anchor_page && flow.cols.len() == 1 {
            placed
                .iter()
                .filter_map(|p| objects::spills(para, p, flow.body))
                .map(|k| k as usize)
                .filter(|&k| {
                    para.controls[k].object_common().is_some_and(|o| {
                        o.text_wrap() == 1
                            && o.height + i32::from(o.margins[2]) + i32::from(o.margins[3])
                                <= flow.body.h
                    })
                })
                .collect()
        } else {
            Vec::new()
        };
        let moved_anchor = |k: usize| Anchor {
            para_top: spill_top.para_top
                - para.controls[k]
                    .object_common()
                    .map_or(0, |o| o.y_offset.max(0)),
            ..spill_top
        };
        if !moved.is_empty() {
            let b = objects::bands_of(para, &layout, &anchor_at_start, &|k| {
                first_line(&layout, k) && !moved.contains(&k)
            });
            (own, pending) = (b.own, b.after);
            // It opens the next page at the body top, its vertical offset
            // dropped, and the text after the paragraph goes below it.
            let top = flow.body.y;
            for &k in &moved {
                let at = moved_anchor(k);
                let next = objects::bands_of(para, &layout, &at, &|j| j == k);
                flow.carried_bands.extend(
                    next.now
                        .into_iter()
                        .chain(next.own)
                        .chain(next.after)
                        .map(|b| Band { top, ..b }),
                );
            }
        }
        if flow.pages.len() == anchor_page {
            flow.bands.extend(own);
            flow.bands.extend(pending);
        }
        for p in placed {
            if moved.contains(&(p.control as usize)) {
                let at = moved_anchor(p.control as usize);
                let at = objects::place_one(para, pi as u32, p.control, &at);
                flow.carried_objects.push(at.unwrap_or(p));
                continue;
            }
            let spill = objects::spills(para, &p, flow.body);
            let (page, p) = match spill {
                Some(k) if flow.pages.len() > anchor_page => {
                    let moved = objects::place_one(para, pi as u32, k, &spill_top);
                    (anchor_page, moved.unwrap_or(p))
                }
                _ => (anchor_page - 1, p),
            };
            if let Some(page) = flow.pages.get_mut(page) {
                page.objects.push(p);
            }
        }
        out.paras.push(layout);
    }
    // An object moved past the last page opens a page of its own.
    if !flow.carried_objects.is_empty() {
        flow.new_page();
    }
    flow.flush_tables();
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
    out.merged_notes = flow.merged_seen;
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
        } else if let Some(o) = c.as_object() {
            // Notes in a text box belong to the line holding the box.
            for (step, note) in crate::notes::in_object(o) {
                let list = ListRef::Cell {
                    section,
                    para: pi,
                    control: index,
                    steps: Box::new([step]),
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

/// True when the paragraph after `pi` starts a new page: the section ends
/// there, or the next paragraph breaks the page.
fn page_follows(sec: &Section, pi: usize) -> bool {
    sec.paras.get(pi + 1).is_none_or(|n| {
        n.header.break_kind & break_kind::PAGE != 0 && n.header.break_kind & break_kind::COLUMN == 0
    })
}

/// True for a paragraph holding nothing but its end.
fn is_empty(para: &Paragraph) -> bool {
    para.controls.is_empty()
        && para
            .units()
            .all(|(_, u)| matches!(u, hm_model::Unit::CharControl(13)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_paragraphs_stored_before_explicit_breaks_keep_their_column() {
        use hm_model::{
            Control, ControlKind, LineSeg, PageDef, SectionChild, SectionDef, SharedBytes,
        };

        for (kind, columns) in [(break_kind::PAGE, 1), (break_kind::COLUMN, 2)] {
            for stored_vpos in [Some(0), Some(9000), None] {
                let paragraph = |text: Vec<u16>, vpos, height| Paragraph {
                    text,
                    line_segs: vec![LineSeg {
                        vpos,
                        height,
                        text_height: height,
                        baseline: height * 85 / 100,
                        width: 20000 / columns,
                        flags: LineSeg::FIRST_SEGMENT | LineSeg::LAST_SEGMENT,
                        ..Default::default()
                    }],
                    src: Some(SharedBytes::from(vec![1])),
                    ..Default::default()
                };
                let mut filler = paragraph(vec![b'A' as u16, 13], 0, 9000);
                filler.controls = vec![
                    Control {
                        kind: ControlKind::Section(Box::new(SectionDef {
                            children: vec![SectionChild::Page(PageDef {
                                width: 20000,
                                height: 10000,
                                ..Default::default()
                            })],
                            ..Default::default()
                        })),
                        src: None,
                    },
                    Control {
                        kind: ControlKind::Columns(ColumnDef {
                            attr: ((columns as u16) << 2) | (1 << 12),
                            ..Default::default()
                        }),
                        src: None,
                    },
                ];
                let mut empty = paragraph(vec![13], stored_vpos.unwrap_or(0), 1500);
                if stored_vpos.is_none() {
                    empty.line_segs[0] = LineSeg::default();
                }
                let mut heading = paragraph(vec![b'B' as u16, 13], 0, 1500);
                heading.header.break_kind = kind;
                let doc = Document {
                    meta: hm_model::DocMeta {
                        format: hm_model::Format::Hwpx,
                        version: hm_model::Version(0),
                        flags: 0,
                    },
                    info: hm_model::DocInfo {
                        char_shapes: vec![hm_model::CharShape {
                            base_size: 1500,
                            ratios: [100; 7],
                            rel_sizes: [100; 7],
                            ..Default::default()
                        }],
                        ..Default::default()
                    },
                    sections: vec![Section {
                        paras: vec![filler, empty, heading],
                        ..Default::default()
                    }],
                    source: None,
                    bins: None,
                    diagnostics: Vec::new(),
                };
                let layout = crate::layout(&doc, &hm_fonts::FontSystem::new());
                let position = |para| {
                    layout
                        .pages
                        .iter()
                        .enumerate()
                        .find_map(|(page, p)| {
                            p.lines
                                .iter()
                                .find(|l| l.para == para)
                                .map(|l| (page, l.x, l.vpos))
                        })
                        .expect("placed paragraph")
                };
                let (empty_at, heading_at) = match (kind, stored_vpos) {
                    (break_kind::PAGE, Some(0)) => ((1, 0, 0), (2, 0, 0)),
                    (break_kind::COLUMN, Some(0)) => ((0, 10000, 0), (1, 0, 0)),
                    (break_kind::PAGE, _) => ((0, 0, 9000), (1, 0, 0)),
                    _ => ((0, 0, 9000), (0, 10000, 0)),
                };
                assert_eq!(position(1), empty_at, "break {kind}, vpos {stored_vpos:?}");
                assert_eq!(
                    position(2),
                    heading_at,
                    "break {kind}, vpos {stored_vpos:?}"
                );
                assert_eq!(layout.para(0, 1).lines[0].height, 1500);
                assert_eq!(layout.para(0, 1).lines[0].stored, stored_vpos.is_some());
            }
        }
    }

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
