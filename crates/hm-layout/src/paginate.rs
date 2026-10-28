//! Pages and columns.

use hm_model::units::unpack_distance;
use hm_model::{BorderLine, ColumnDef, Document, Paragraph, Section, break_kind};

use crate::furniture::{Frame, ListRef, Mark, PageBorder, Rule};
use crate::heads::Heads;
use crate::objects::{self, Anchor, Band};
use crate::para::{ParaEnv, ParaLayout, format_body as format};
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
    /// Paragraph anchor before its own floating objects displace the line.
    pub anchor_y: i32,
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
    /// Empty table anchors may remain on the page their tables left.
    pub(crate) hold_table_anchors: bool,
    pub(crate) unheld_table_page: Option<usize>,
    /// Top-and-bottom objects moved past the foot of the page their
    /// paragraph stays on: they open the next page, which their bands
    /// keep the lines off.
    carried_objects: Vec<crate::objects::Placement>,
    carried_bands: Vec<Band>,
    /// A continued flowing table of the current paragraph ended here:
    /// page, column, the fill before it and its bottom with margin.
    pub(crate) continued_floor: Option<(usize, usize, i32, i32)>,
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

/// Lookahead belongs to this immutable section and DocInfo, for this one
/// section pass's font/style environment. Only width, page number and stored
/// line policy vary. The current body's number-room/grid address never names
/// a following paragraph, so following paragraphs use neither override.
struct KeepChains<'a> {
    paras: &'a [Paragraph],
    info: &'a hm_model::DocInfo,
    default_tab: i32,
    preview: Heads,
    outline: u16,
    resolved_heads: Vec<Option<crate::heads::Head>>,
    budgets: std::collections::HashMap<(i32, u32, bool), std::collections::HashMap<usize, i32>>,
}

impl<'a> KeepChains<'a> {
    fn new(
        sec: &'a Section,
        info: &'a hm_model::DocInfo,
        default_tab: i32,
        heads: &Heads,
        outline: u16,
    ) -> Self {
        Self {
            paras: &sec.paras,
            info,
            default_tab,
            preview: heads.clone(),
            outline,
            resolved_heads: Vec::new(),
            budgets: Default::default(),
        }
    }

    /// Body consumes a head for every source paragraph, including empty or
    /// hidden ones, before placing any lines. Replay that same order once
    /// from the section pass's initial counters, without advancing the live
    /// counters. Heads do not depend on width or page regions; their resolved
    /// values belong only to this immutable source and outline environment.
    fn preview_through(&mut self, at: usize) {
        while self.resolved_heads.len() <= at {
            let p = &self.paras[self.resolved_heads.len()];
            let head = self
                .info
                .para_shape(p.header.para_shape as u32)
                .and_then(|s| {
                    self.preview
                        .next_with_restart(s, self.info, self.outline, p.list_restart)
                });
            self.resolved_heads.push(head);
            #[cfg(test)]
            KEEP_HEAD_WORK.with(|w| w.set(w.get() + 1));
        }
    }

    fn boundary(&self, p: &Paragraph) -> bool {
        p.header.break_kind
            & (break_kind::PAGE
                | break_kind::COLUMN
                | break_kind::SECTION
                | break_kind::MULTI_COLUMN)
            != 0
            || self
                .info
                .para_shape(p.header.para_shape as u32)
                .is_some_and(|s| s.page_break_before())
            || p.controls
                .iter()
                .any(|c| c.as_section().is_some() || c.as_columns().is_some())
    }

    fn budget(&mut self, start: usize, width: i32, styles: &mut Styles<'_>) -> Option<i32> {
        let context = (width, styles.page_number, styles.stored_lines);
        let mut pending = Vec::new();
        let mut at = start;
        let mut need = loop {
            #[cfg(test)]
            KEEP_WORK.with(|w| {
                let (formats, visits) = w.get();
                w.set((formats, visits + 1));
            });
            if let Some(&need) = self.budgets.get(&context).and_then(|b| b.get(&at)) {
                break need;
            }
            let p = self.paras.get(at)?;
            if self.boundary(p) {
                return None;
            }
            let shape = self.info.para_shape(p.header.para_shape as u32);
            self.preview_through(at);
            #[cfg(test)]
            KEEP_WORK.with(|w| {
                let (formats, visits) = w.get();
                w.set((formats + 1, visits));
            });
            let nl = format(
                p,
                self.info,
                styles,
                ParaEnv {
                    column_width: width,
                    default_tab: self.default_tab,
                },
                self.resolved_heads[at].as_ref(),
            );
            let linked = shape.is_some_and(|s| s.keep_with_next())
                && self.paras.get(at + 1).is_some_and(|p| !self.boundary(p));
            if linked {
                // Signed spacing can shrink a later suffix, even after an
                // earlier prefix exceeds a page. Always reach the terminal.
                pending.push((at, nl.before + nl.lines_height() + nl.after));
                at += 1;
                continue;
            }
            let span = match (shape.is_some_and(|s| s.keep_lines()), nl.lines.split_last()) {
                (true, Some((last, rest))) => {
                    rest.iter().map(|l| l.advance()).sum::<i32>() + last.height
                }
                _ => nl.lines.first().map_or(0, |l| l.height),
            };
            let need = nl.before + span;
            self.budgets.entry(context).or_default().insert(at, need);
            break need;
        };
        let budgets = self.budgets.entry(context).or_default();
        for (at, height) in pending.into_iter().rev() {
            need += height;
            budgets.insert(at, need);
        }
        Some(need)
    }
}

#[cfg(test)]
thread_local! {
    static KEEP_WORK: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
    static KEEP_HEAD_WORK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Paragraph protection before placing a paragraph: keep its lines
/// together, keep a linked run with its terminal paragraph's first line, and avoid a lone
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
    mut next_budget: impl FnMut() -> Option<i32>,
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
        && let Some(next) = next_budget()
    {
        let need = total + layout.after + next;
        if need >= avail && need < full {
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
    styles.outline_numbering = outline;
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
        hold_table_anchors: crate::compat::effective(&doc.info)[3] & (1 << 6) == 0,
        unheld_table_page: None,
        carried_objects: Vec::new(),
        carried_bands: Vec::new(),
        continued_floor: None,
    };
    flow.mirror = mirror_of(&page_def);
    flow.new_page();
    let hwp5 = doc.meta.format != hm_model::Format::Hwpx;
    let geometry = (paper.0, paper.1, flow.body);
    let mut out = SectionLayout {
        paras: Vec::with_capacity(sec.paras.len()),
        merged_notes: Vec::new(),
    };
    let mut keep_chains = KeepChains::new(sec, &doc.info, default_tab, heads, outline);
    for (pi, para) in sec.paras.iter().enumerate() {
        let shape = doc.info.para_shape(para.header.para_shape as u32);
        if para.header.break_kind & (break_kind::PAGE | break_kind::COLUMN | break_kind::SECTION)
            != 0
            || shape.is_some_and(|s| s.page_break_before())
        {
            flow.unheld_table_page = None;
        }
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
        let head =
            shape.and_then(|s| heads.next_with_restart(s, &doc.info, outline, para.list_restart));
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
        let right = layout.right;
        let anchor = |flow: &Flow| Anchor {
            paper: flow.paper,
            paper_x: None,
            paper_y: None,
            flow_top: Some(flow.body.y),
            body: flow.body,
            column: flow.column(),
            para_top: flow.page_y(flow.fill[flow.col]),
            para_left: flow.column().0 + left,
            para_right: right,
        };
        let next = sec.paras.get(pi + 1);
        let next_shape = next.and_then(|p| doc.info.para_shape(p.header.para_shape as u32));
        let break_before = protect(&mut flow, &layout, shape, || {
            keep_chains.budget(pi + 1, layout.column_width, styles)
        });
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
            // A preceding paragraph's square band can invalidate a stored
            // span too. Keep replay authority when that span still fits;
            // otherwise break again in the gaps left on this row.
            let in_row = row.is_some_and(|(start, end, _)| li > start && li <= end);
            if !in_row {
                row = None;
            }
            if !in_row {
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
                let inside = gaps.len() == 1
                    && seg_left >= gaps[0].0
                    && seg_right <= gaps[0].1
                    && (line.stored || content.0 >= gaps[0].0 && content.1 <= gaps[0].1);
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
                    crate::para::content_span(&layout, line)
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
                // Before a break, only a replayed line stays there: Hancom's
                // own composition moves that paragraph on like any other
                // line, while it keeps a run of empty paragraphs it stored at
                // the foot. A run with no stored lines moves on too, unless
                // the section hides empty lines at a page top.
                // A replayed page start still moves on when another break
                // follows it.
                let next_empty = next.is_some_and(|n| {
                    n.controls.is_empty()
                        && n.units()
                            .all(|(_, u)| matches!(u, hm_model::Unit::CharControl(13)))
                }) && para.line_segs.first().map_or(hide_empty, |s| s.vpos != 0);
                // Paragraphs typed since reading (no source) move on as
                // Hancom moves them when it is editing.
                if empty
                    && para.src.is_some()
                    && (!line.stored || para.line_segs.first().is_none_or(|s| s.vpos != 0))
                    && (next_empty || (next_breaks && line.stored))
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
                anchor_y: anchor_at_start.para_top,
                vpos,
            });
            // A line kept at the column's foot takes no room there: its
            // spacing before does not carry the fill past the column, so a
            // following empty paragraph stays at the foot as well.
            let bottom = if collapsed {
                (vpos - flow.region_top).min(flow.region_height())
            } else {
                vpos - flow.region_top + span
            };
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
            // The controls in this line, in text order (none without controls).
            let in_line = if para.controls.is_empty() {
                &[][..]
            } else {
                layout.line_facts(para).controls(li)
            };
            if let Some(v) = restart
                && in_line.iter().any(|&index| {
                    para.controls
                        .get(index as usize)
                        .is_some_and(|c| crate::furniture::new_page_number(c, para, hwp5).is_some())
                })
            {
                flow.number = v;
            }
            // An odd/even page adjustment renumbers the page that holds it.
            for &index in in_line {
                if let Some(hm_model::ControlKind::PageParity(p)) =
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
        // Below a continued table's last fragment the paragraph's spacing
        // after counts from its text, as on the table's first page, where
        // the table only blocks its band: Hancom starts the next paragraph
        // at the table's bottom margin.
        let floor = flow
            .continued_floor
            .take()
            .filter(|&(page, col, _, floor)| {
                page == flow.pages.len() && col == flow.col && flow.fill[col] == floor
            });
        if let Some((_, col, text, floor)) = floor {
            flow.fill[col] = floor.max(on_grid(text + layout.after, flow.region_top, grid));
        } else if !collapsed {
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
        // stays on opens the next column, or the next page from the last
        // one, instead, and the text after it goes on filling this column.
        let next_col = flow.col + 1 < flow.cols.len();
        let moved: Vec<usize> =
            if flow.pages.len() == anchor_page && (flow.cols.len() == 1 || next_col) {
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
        // A spill starts below earlier top-and-bottom objects at the next
        // column's top, just as the column's first line does.
        let opened = if next_col {
            let (x, w) = flow.cols[flow.col + 1];
            let top = objects::clear_line(
                &flow.bands,
                flow.page_y(0),
                (1, 1),
                (0, 0),
                None,
                flow.body.x + x,
                flow.body.x + x + w,
            )
            .0;
            Anchor {
                para_top: top,
                column: (flow.body.x + x, w),
                para_left: flow.body.x + x + left,
                ..spill_top
            }
        } else {
            spill_top
        };
        let moved_anchor = |k: usize| Anchor {
            para_top: opened.para_top
                - para.controls[k]
                    .object_common()
                    .map_or(0, |o| o.y_offset.max(0)),
            ..opened
        };
        if !moved.is_empty() {
            let b = objects::bands_of(para, &layout, &anchor_at_start, &|k| {
                first_line(&layout, k) && !moved.contains(&k)
            });
            (own, pending) = (b.own, b.after);
            // It opens the next column or page at its top, its vertical
            // offset dropped, and the text after the paragraph goes below it.
            let top = opened.para_top;
            for &k in &moved {
                let at = moved_anchor(k);
                let next = objects::bands_of(para, &layout, &at, &|j| j == k);
                let bands = next
                    .now
                    .into_iter()
                    .chain(next.own)
                    .chain(next.after)
                    .map(|b| Band { top, ..b });
                if next_col {
                    flow.bands.extend(bands);
                } else {
                    flow.carried_bands.extend(bands);
                }
            }
        }
        if flow.pages.len() == anchor_page {
            flow.bands.extend(own);
            flow.bands.extend(pending);
        }
        for p in placed {
            if moved.contains(&(p.control as usize)) {
                let at = moved_anchor(p.control as usize);
                let at = objects::place_one(para, pi as u32, p.control, &at).unwrap_or(p);
                match flow.pages.last_mut() {
                    Some(page) if next_col => page.objects.push(at),
                    _ => flow.carried_objects.push(at),
                }
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
        endnotes.extend(crate::notes::endnotes(k as u32, &doc.sections[k].paras));
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
    if para.controls.is_empty() {
        return out;
    }
    for (pos, unit) in para.units().take_while(|&(pos, _)| pos < line.end) {
        let hm_model::Unit::Extended { index, .. } = unit else {
            continue;
        };
        if pos < line.start {
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

    fn keep_document(paras: &[(&str, u32)]) -> Document {
        use hm_model::{Control, ControlKind, PageDef, SectionChild, SectionDef};

        let mut doc = Document {
            meta: hm_model::DocMeta {
                format: hm_model::Format::Hwpx,
                version: hm_model::Version(0),
                flags: 0,
            },
            info: hm_model::DocInfo {
                char_shapes: vec![hm_model::CharShape {
                    base_size: 1000,
                    ratios: [100; 7],
                    rel_sizes: [100; 7],
                    ..Default::default()
                }],
                para_shapes: paras
                    .iter()
                    .map(|(_, attr1)| hm_model::ParaShape {
                        attr1: *attr1,
                        line_spacing_legacy: 100,
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            },
            sections: vec![Section {
                paras: paras
                    .iter()
                    .enumerate()
                    .map(|(i, (text, _))| Paragraph {
                        header: hm_model::ParaHeader {
                            para_shape: i as u16,
                            ..Default::default()
                        },
                        text: text.encode_utf16().chain([13]).collect(),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            }],
            source: None,
            bins: None,
            diagnostics: Vec::new(),
        };
        doc.sections[0].paras[0].controls.push(Control {
            kind: ControlKind::Section(Box::new(SectionDef {
                children: vec![SectionChild::Page(PageDef {
                    width: 20000,
                    height: 10000,
                    ..Default::default()
                })],
                ..Default::default()
            })),
            src: None,
        });
        doc
    }

    fn placements(layout: &crate::DocLayout, para: u32) -> Vec<(usize, u32, i32)> {
        layout
            .pages
            .iter()
            .enumerate()
            .flat_map(|(page, p)| {
                p.lines
                    .iter()
                    .filter(move |l| l.para == para)
                    .map(move |l| (page, l.line, l.vpos))
            })
            .collect()
    }

    fn decimal_numbering(start: u16, distance: i16) -> hm_model::Numbering {
        hm_model::Numbering {
            start: Some(start),
            levels: vec![hm_model::NumberingLevel {
                head: hm_model::ParaHead {
                    attr: 1 << 4,
                    text_distance: distance,
                    ..Default::default()
                },
                format: hm_model::HwpString("^1".encode_utf16().collect()),
            }],
            ..Default::default()
        }
    }

    fn head_text(layout: &crate::DocLayout, section: u32, para: u32) -> String {
        layout
            .para(section, para)
            .head
            .as_ref()
            .unwrap()
            .glyphs
            .iter()
            .map(|g| g.0)
            .collect()
    }

    #[test]
    fn keep_chain_includes_bullet_wrap_and_head_height() {
        for (text, size, lines) in [
            ("aaaa aaaa aaaa aaaa aaaa aaaa aaaa aaaa", 1000, 2),
            ("middle", 3000, 1),
        ] {
            let mut doc = keep_document(&[
                ("f\nf\nf\nf\nf\nf", 0),
                ("start", 1 << 17),
                (text, (1 << 17) | (3 << 23)),
                ("terminal", 0),
            ]);
            let mut head_shape = doc.info.char_shapes[0].clone();
            head_shape.base_size = size;
            doc.info.char_shapes.push(head_shape);
            doc.info.para_shapes[2].numbering = 1;
            doc.info.bullets.push(hm_model::Bullet {
                head: hm_model::ParaHead {
                    attr: 1 << 4,
                    char_shape: 1,
                    text_distance: 2000,
                    ..Default::default()
                },
                bullet_char: b'*' as u16,
                ..Default::default()
            });
            let fonts = hm_fonts::FontSystem::new();
            let layout = crate::layout(&doc, &fonts);
            assert_eq!(placements(&layout, 1), [(1, 0, 0)]);
            assert_eq!(placements(&layout, 2).len(), lines);
            assert_eq!(placements(&layout, 2)[0], (1, 0, 1000));
            assert_eq!(layout.para(0, 2).lines[0].height, size);
            assert_eq!(
                placements(&layout, 3),
                [(1, 0, 1000 + size + if lines == 2 { 1000 } else { 0 })]
            );
            assert_eq!(head_text(&layout, 0, 2), "*");
            // An explicit fresh-page start must produce the same chain.
            doc.info.para_shapes[1].attr1 |= 1 << 19;
            let fresh = crate::layout(&doc, &fonts);
            for para in 1..=3 {
                assert_eq!(placements(&layout, para), placements(&fresh, para));
            }
        }
    }

    #[test]
    fn keep_chain_uses_number_width_without_advancing_live_counters() {
        let mut doc = keep_document(&[
            ("f\nf\nf\nf\nf\nf", 2 << 23),
            ("start", (1 << 17) | (2 << 23)),
            (
                "aaaa aaaa aaaa aaaa aaaa aaaa aaaa aaaa",
                (1 << 17) | (2 << 23),
            ),
            ("terminal", 2 << 23),
        ]);
        doc.info.numberings.push(decimal_numbering(8, 1500));
        for shape in &mut doc.info.para_shapes {
            shape.numbering = 1;
        }
        let mut next = doc.sections[0].clone();
        next.paras.truncate(2);
        doc.sections.push(next);
        let fonts = hm_fonts::FontSystem::new();
        let layout = crate::layout(&doc, &fonts);
        assert_eq!(placements(&layout, 1)[0], (1, 0, 0));
        assert_eq!(layout.para(0, 2).lines.len(), 2);
        for (section, expected) in [(0, vec!["8", "9", "10", "11"]), (1, vec!["12", "13"])] {
            for (para, text) in expected.into_iter().enumerate() {
                assert_eq!(head_text(&layout, section, para as u32), text);
            }
        }
        for shape in &mut doc.info.para_shapes {
            shape.attr1 &= !(1 << 17);
        }
        let unlinked = crate::layout(&doc, &fonts);
        let repeated = crate::layout(&doc, &fonts);
        for (section, sec) in doc.sections.iter().enumerate() {
            for para in 0..sec.paras.len() {
                let text = head_text(&layout, section as u32, para as u32);
                assert_eq!(head_text(&unlinked, section as u32, para as u32), text);
                assert_eq!(head_text(&repeated, section as u32, para as u32), text);
            }
        }
    }

    #[test]
    fn keep_chain_head_preview_consumes_hidden_paragraphs_and_section_outlines() {
        let keep = 1 << 17;
        let mut doc = keep_document(&[
            ("f\nf\nf\nf\nf\nf\nf\nf\nf", 0),
            ("", 2 << 23),
            ("start", keep | (2 << 23)),
            ("deeper", keep | (2 << 23) | (1 << 25)),
            ("restart", keep | (2 << 23)),
            ("outline", 1 << 23),
        ]);
        doc.info.numberings.push(decimal_numbering(9, 0));
        let mut other = decimal_numbering(1, 0);
        let mut deeper = other.levels[0].clone();
        deeper.format = hm_model::HwpString("^1.^2".encode_utf16().collect());
        other.levels.push(deeper);
        doc.info.numberings.push(other);
        for shape in &mut doc.info.para_shapes {
            shape.numbering = 1;
        }
        doc.info.para_shapes[3].numbering = 2;
        let hm_model::ControlKind::Section(def) = &mut doc.sections[0].paras[0].controls[0].kind
        else {
            unreachable!()
        };
        def.attr |= 1 << 19;
        def.outline_numbering = 2;
        let mut next = doc.sections[0].clone();
        next.paras.truncate(2);
        next.paras[0].header.para_shape = 5;
        let hm_model::ControlKind::Section(def) = &mut next.paras[0].controls[0].kind else {
            unreachable!()
        };
        def.outline_numbering = 1;
        doc.sections.push(next);
        let fonts = hm_fonts::FontSystem::new();
        let layout = crate::layout(&doc, &fonts);
        assert_eq!(placements(&layout, 1)[0], (1, 0, 0));
        assert_eq!(placements(&layout, 2)[0], (1, 0, 0));
        for (para, text) in [(1, "9"), (2, "10"), (3, "10.1"), (4, "9"), (5, "11")] {
            assert_eq!(head_text(&layout, 0, para), text);
        }
        assert_eq!(head_text(&layout, 1, 0), "10");
        assert_eq!(head_text(&layout, 1, 1), "11");
        let again = crate::layout(&doc, &fonts);
        for (section, sec) in doc.sections.iter().enumerate() {
            for para in 1..sec.paras.len() {
                assert_eq!(
                    head_text(&layout, section as u32, para as u32),
                    head_text(&again, section as u32, para as u32)
                );
            }
        }
    }

    #[test]
    fn keep_chain_carries_linked_empty_paragraphs() {
        let keep = 1 << 17;
        let doc = keep_document(&[
            ("f\nf\nf\nf\nf\nf", 0),
            ("start", keep),
            ("", keep),
            ("middle", keep),
            ("", keep),
            ("last link", keep),
            ("terminal", 0),
        ]);
        let layout = crate::layout(&doc, &hm_fonts::FontSystem::new());
        assert_eq!(layout.pages.len(), 2);
        assert_eq!(layout.pages[0].lines.len(), 6);
        for para in 1..=6 {
            assert_eq!(
                placements(&layout, para),
                [(1, 0, (para as i32 - 1) * 1000)]
            );
        }
    }

    #[test]
    fn keep_chain_uses_spacing_and_the_terminals_line_protection() {
        use hm_model::units::pack_distance;

        for keep_lines in [false, true] {
            let mut doc = keep_document(&[
                ("f\nf", 0),
                ("start", 1 << 17),
                ("middle\nmiddle", 1 << 17),
                ("terminal\nterminal\nterminal", u32::from(keep_lines) << 18),
            ]);
            for (shape, before, after, percent) in
                [(1, 200, 300, 150), (2, 400, 500, 120), (3, 250, 700, 130)]
            {
                let s = &mut doc.info.para_shapes[shape];
                s.space_before = pack_distance(before);
                s.space_after = pack_distance(after);
                s.line_spacing_legacy = percent;
            }
            let layout = crate::layout(&doc, &hm_fonts::FontSystem::new());
            // Through the terminal's first line: 6550 HU. Through its
            // last line, without its trailing spacing: 9150 HU.
            let page = usize::from(keep_lines);
            let offset = if keep_lines { 0 } else { 2000 };
            assert_eq!(placements(&layout, 1), [(page, 0, offset + 200)]);
            assert_eq!(
                placements(&layout, 2),
                [(page, 0, offset + 2400), (page, 1, offset + 3600)]
            );
            let terminal = placements(&layout, 3);
            assert_eq!(terminal[0], (page, 0, offset + 5550));
            assert_eq!(terminal.len(), 3);
            if keep_lines {
                assert!(terminal.iter().all(|l| l.0 == page));
            } else {
                assert_eq!(terminal[2].0, 1);
            }
        }
    }

    #[test]
    fn keep_chains_stop_at_explicit_breaks_and_definitions() {
        use hm_model::{Control, ControlKind};

        for boundary in 0..7 {
            let mut doc = keep_document(&[
                ("f\nf\nf\nf\nf\nf\nf", 0),
                ("start", 1 << 17),
                ("last link", 1 << 17),
                ("terminal\nterminal\nterminal", 0),
            ]);
            let p = &mut doc.sections[0].paras[3];
            match boundary {
                0 => p.header.break_kind = break_kind::PAGE,
                1 => p.header.break_kind = break_kind::COLUMN,
                2 => p.header.break_kind = break_kind::SECTION,
                3 => p.header.break_kind = break_kind::MULTI_COLUMN,
                4 => doc.info.para_shapes[3].attr1 |= 1 << 19,
                5 => p.controls.push(Control {
                    kind: ControlKind::Section(Box::default()),
                    src: None,
                }),
                _ => p.controls.push(Control {
                    kind: ControlKind::Columns(ColumnDef {
                        attr: (2 << 2) | (1 << 12),
                        ..Default::default()
                    }),
                    src: None,
                }),
            }
            let layout = crate::layout(&doc, &hm_fonts::FontSystem::new());
            assert_eq!(placements(&layout, 1), [(0, 0, 7000)], "{boundary}");
            assert_eq!(placements(&layout, 2), [(0, 0, 8000)], "{boundary}");
        }
    }

    #[test]
    fn top_and_bottom_objects_crossing_the_foot_open_the_next_column() {
        use hm_model::{Control, ControlKind, CtrlId, ObjectCommon, ObjectControl, ObjectHeader};

        // `143e433f503322bd33`: a picture kept in the body that would cross
        // the left column's foot opens the right column, and the text after
        // it goes on filling the left one.
        for top in [0, 700] {
            let mut doc = keep_document(&[("a\na\na\na\na\na", 0), ("", 0), ("after", 0)]);
            doc.sections[0].paras[0].controls.push(Control {
                kind: ControlKind::Columns(ColumnDef {
                    attr: 2 << 2,
                    columns: vec![(9000, 0), (9000, 0)],
                    ..Default::default()
                }),
                src: None,
            });
            if top > 0 {
                doc.sections[0].paras[0].controls.push(Control {
                    kind: ControlKind::Object(Box::new(ObjectControl {
                        id: CtrlId::SHAPE,
                        header: ObjectHeader {
                            common: ObjectCommon {
                                attr: (4 << 15) | (2 << 18) | (1 << 21),
                                width: 20000,
                                height: top,
                                ..Default::default()
                            },
                            ..Default::default()
                        },
                        parts: Vec::new(),
                    })),
                    src: None,
                });
            }
            let host = &mut doc.sections[0].paras[1];
            host.text = vec![11, 0, 0, 0, 0, 0, 0, 11, 13];
            host.controls.push(Control {
                kind: ControlKind::Object(Box::new(ObjectControl {
                    id: CtrlId::SHAPE,
                    header: ObjectHeader {
                        common: ObjectCommon {
                            attr: (2 << 3)
                                | (2 << 8)
                                | (1 << 13)
                                | (4 << 15)
                                | (2 << 18)
                                | (1 << 21),
                            width: 5000,
                            height: 6000,
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                    parts: Vec::new(),
                })),
                src: None,
            });
            let layout = crate::layout(&doc, &hm_fonts::FontSystem::new());
            assert_eq!(layout.pages.len(), 1);
            let page = &layout.pages[0];
            let right = page.columns[1];
            let object = page.objects.iter().find(|o| o.para == 1).unwrap().rect;
            assert_eq!((object.x, object.y), (right.x, right.y + top));
            let host = page.lines.iter().find(|l| l.para == 1).unwrap();
            let after = page.lines.iter().find(|l| l.para == 2).unwrap();
            assert_eq!(after.x, host.x);
            assert!(after.y > host.y);
        }
    }

    #[test]
    fn a_flowing_table_after_an_inline_table_anchors_to_its_own_line() {
        use hm_model::{Cell, Control, ControlKind, ObjectCommon, ObjectHeader, Table};

        let table = |height, inline| Control {
            kind: ControlKind::Table(Box::new(Table {
                header: ObjectHeader {
                    common: ObjectCommon {
                        attr: u32::from(inline) | (2 << 3) | (4 << 15) | (2 << 18) | (1 << 21),
                        width: 18000,
                        height,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                rows: 1,
                cols: 1,
                cells: vec![Cell {
                    row_span: 1,
                    col_span: 1,
                    width: 18000,
                    height,
                    ..Default::default()
                }],
                ..Default::default()
            })),
            src: None,
        };
        let mut doc = keep_document(&[("intro", 0), ("", 0)]);
        let host = &mut doc.sections[0].paras[1];
        let marker = [11, 0, 0, 0, 0, 0, 0, 11];
        host.text = [marker.as_slice(), &[10], &marker, &marker, &[13]].concat();
        host.controls = vec![table(3000, true), table(4000, false), table(1000, true)];
        let layout = crate::layout(&doc, &hm_fonts::FontSystem::new());
        let page = &layout.pages[0];
        let first = page
            .lines
            .iter()
            .find(|l| l.para == 1 && l.line == 0)
            .unwrap();
        let floating = &page.tables[0];
        assert!(floating.y >= first.y + 3000);
        let title = page
            .lines
            .iter()
            .find(|l| l.para == 1 && l.line == 1)
            .unwrap();
        assert!(title.y >= floating.y + floating.height());
    }

    #[test]
    fn overlong_keep_chains_place_every_line_once() {
        for multiline in [false, true] {
            let mut spec = vec![("filler", 0)];
            if multiline {
                spec.push(("a\na\na\na\na\na\na\na\na\na\na", 1 << 17));
            } else {
                spec.extend(std::iter::repeat_n(("", 1 << 17), 11));
            }
            spec.extend([("last link", 1 << 17), ("terminal", 0)]);
            let doc = keep_document(&spec);
            let layout = crate::layout(&doc, &hm_fonts::FontSystem::new());
            assert_eq!(placements(&layout, 1)[0], (0, 0, 1000));
            assert!(layout.pages.len() <= 3);
            for (para, p) in doc.sections[0].paras.iter().enumerate() {
                let expected = p.text.iter().filter(|&&c| c == 10).count() + 1;
                let placed = placements(&layout, para as u32);
                assert_eq!(placed.len(), expected);
                assert_eq!(
                    placed.iter().map(|l| l.1).collect::<Vec<_>>(),
                    (0..expected as u32).collect::<Vec<_>>()
                );
            }
        }
    }

    #[test]
    fn long_keep_chains_do_linear_lookahead_through_public_layout() {
        for count in [400, 800, 1600] {
            for widths in [1, 2] {
                for numbered in [false, true] {
                    let mut spec = vec![("filler", 0)];
                    spec.extend(std::iter::repeat_n(
                        ("ordinary words for wrapping ", 1 << 17),
                        count,
                    ));
                    spec.push(("terminal", 0));
                    let mut doc = keep_document(&spec);
                    if numbered {
                        doc.info.numberings.push(decimal_numbering(1, 1500));
                        for shape in &mut doc.info.para_shapes {
                            shape.attr1 |= 2 << 23;
                            shape.numbering = 1;
                        }
                    }
                    if widths == 2 {
                        doc.sections[0].paras[0].controls.push(hm_model::Control {
                            kind: hm_model::ControlKind::Columns(ColumnDef {
                                attr: 2 << 2,
                                columns: vec![(10922, 0), (21846, 0)],
                                ..Default::default()
                            }),
                            src: None,
                        });
                    }
                    KEEP_WORK.with(|w| w.set((0, 0)));
                    KEEP_HEAD_WORK.with(|w| w.set(0));
                    let layout = crate::layout(&doc, &hm_fonts::FontSystem::new());
                    let (formats, visits) = KEEP_WORK.with(|w| w.get());
                    let head_steps = KEEP_HEAD_WORK.with(|w| w.get());
                    assert_eq!(head_steps, count + 2, "one source-order head preview");
                    assert!(
                        formats > 0 && formats <= widths * (count + 1),
                        "{count}: {formats} formats"
                    );
                    assert!(
                        visits <= (widths + 1) * (count + 1),
                        "{count}: {visits} suffix visits"
                    );
                    let columns: std::collections::HashSet<_> = layout
                        .pages
                        .iter()
                        .flat_map(|p| p.columns.iter().map(|c| c.w))
                        .collect();
                    assert_eq!(columns.len(), widths);
                    let mut seen = vec![Vec::new(); doc.sections[0].paras.len()];
                    for page in &layout.pages {
                        for line in &page.lines {
                            seen[line.para as usize].push(line.line);
                        }
                    }
                    for (para, lines) in seen.iter().enumerate() {
                        assert_eq!(
                            *lines,
                            (0..layout.para(0, para as u32).lines.len() as u32).collect::<Vec<_>>()
                        );
                        if numbered {
                            assert_eq!(head_text(&layout, 0, para as u32), (para + 1).to_string());
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn keep_chain_can_shrink_after_a_prefix_exceeds_a_page() {
        let mut doc = keep_document(&[
            ("f\nf\nf\nf\nf\nf", 0),
            ("start", 1 << 17),
            ("large before", 1 << 17),
            ("negative before", 1 << 17),
            ("terminal", 0),
        ]);
        doc.info.para_shapes[2].space_before = hm_model::units::pack_distance(12000);
        doc.info.para_shapes[3].space_before = hm_model::units::pack_distance(-10000);
        let layout = crate::layout(&doc, &hm_fonts::FontSystem::new());
        // The full chain needs 6000 HU. The 14000-HU prefix alone would
        // incorrectly disable protection if lookahead stopped there.
        assert_eq!(placements(&layout, 1), [(1, 0, 0)]);
        for para in 0..5 {
            assert_eq!(
                placements(&layout, para).len(),
                if para == 0 { 6 } else { 1 }
            );
        }
    }

    #[test]
    fn preceding_square_bands_reflow_only_the_stored_spans_they_block() {
        use hm_model::{
            Control, ControlKind, CtrlId, LineSeg, ObjectCommon, ObjectControl, ObjectHeader,
            PageDef, SectionChild, SectionDef, SharedBytes,
        };

        let mut doc = Document {
            meta: hm_model::DocMeta {
                format: hm_model::Format::Hwpx,
                version: hm_model::Version(0),
                flags: 0,
            },
            info: hm_model::DocInfo {
                para_shapes: vec![hm_model::ParaShape {
                    line_spacing_legacy: 100,
                    ..Default::default()
                }],
                ..Default::default()
            },
            sections: Vec::new(),
            source: None,
            bins: None,
            diagnostics: Vec::new(),
        };
        doc.sections.push(Section {
            paras: vec![
                Paragraph {
                    text: vec![11, 0, 0, 0, 0, 0, 0, 11, 13],
                    controls: vec![
                        Control {
                            kind: ControlKind::Object(Box::new(ObjectControl {
                                id: CtrlId::SHAPE,
                                header: ObjectHeader {
                                    common: ObjectCommon {
                                        attr: (2 << 3) | (3 << 8) | (4 << 15) | (2 << 18),
                                        x_offset: 6000,
                                        width: 4000,
                                        height: 1000,
                                        ..Default::default()
                                    },
                                    ..Default::default()
                                },
                                parts: Vec::new(),
                            })),
                            src: None,
                        },
                        Control {
                            kind: ControlKind::Section(Box::new(SectionDef {
                                children: vec![SectionChild::Page(PageDef {
                                    width: 10000,
                                    height: 40000,
                                    ..Default::default()
                                })],
                                ..Default::default()
                            })),
                            src: None,
                        },
                    ],
                    ..Default::default()
                },
                Paragraph {
                    text: "가나다라마바사아자차카타파하가나다라마바사아"
                        .encode_utf16()
                        .chain([13])
                        .collect(),
                    line_segs: [0, 10]
                        .into_iter()
                        .map(|text_start| LineSeg {
                            text_start,
                            vpos: text_start as i32 * 100,
                            text_height: 1000,
                            baseline: 850,
                            width: 10000,
                            flags: LineSeg::FIRST_SEGMENT | LineSeg::LAST_SEGMENT,
                            ..Default::default()
                        })
                        .collect(),
                    src: Some(SharedBytes::from(vec![2])),
                    ..Default::default()
                },
            ],
            ..Default::default()
        });
        let fonts = hm_fonts::FontSystem::new();
        let mut cache = crate::LayoutCache::new();
        let before = crate::layout_cached(&doc, &fonts, Default::default(), &mut cache);
        let kept = before.para(0, 1).lines.clone();
        assert!(kept.iter().all(|line| line.stored));
        assert_eq!(
            kept.iter().map(|line| line.start).collect::<Vec<_>>(),
            [0, 10]
        );

        let ControlKind::Object(object) = &mut doc.sections[0].paras[0].controls[0].kind else {
            unreachable!()
        };
        object.header.common.height = 3000;
        let changed = crate::layout_cached(&doc, &fonts, Default::default(), &mut cache);
        let fresh = crate::layout(&doc, &fonts);
        let lines = &changed.para(0, 1).lines;
        assert_eq!(lines, &fresh.para(0, 1).lines);
        assert_eq!(lines[0].seg_width, 6000);
        assert!(!lines[0].stored);
        assert!(lines[0].end < kept[0].end);

        let ControlKind::Object(object) = &mut doc.sections[0].paras[0].controls[0].kind else {
            unreachable!()
        };
        object.header.common.height = 1000;
        let restored = crate::layout_cached(&doc, &fonts, Default::default(), &mut cache);
        assert_eq!(restored.para(0, 1).lines, kept);
    }

    #[test]
    fn square_objects_leaving_no_gap_push_the_next_paragraph_below_them() {
        use hm_model::{
            Control, ControlKind, CtrlId, LineSeg, ObjectCommon, ObjectControl, ObjectHeader,
            PageDef, SectionChild, SectionDef, SharedBytes,
        };

        // `tac-img-02`: a square picture as wide as the column, 512 below
        // its paragraph's top, starts the next paragraph at its bottom; a
        // narrower one keeps that paragraph beside it.
        for (x_offset, width, pushed) in [(0, 10000, true), (300, 9500, true), (0, 8000, false)] {
            let stored = |vpos| LineSeg {
                vpos,
                height: 1000,
                text_height: 1000,
                baseline: 850,
                width: 10000,
                flags: LineSeg::FIRST_SEGMENT | LineSeg::LAST_SEGMENT,
                ..Default::default()
            };
            let doc = Document {
                meta: hm_model::DocMeta {
                    format: hm_model::Format::Hwpx,
                    version: hm_model::Version(0),
                    flags: 0,
                },
                info: hm_model::DocInfo {
                    para_shapes: vec![hm_model::ParaShape {
                        line_spacing_legacy: 100,
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                sections: vec![Section {
                    paras: vec![
                        Paragraph {
                            text: vec![11, 0, 0, 0, 0, 0, 0, 11, 13],
                            controls: vec![
                                Control {
                                    kind: ControlKind::Object(Box::new(ObjectControl {
                                        id: CtrlId::SHAPE,
                                        header: ObjectHeader {
                                            common: ObjectCommon {
                                                attr: (2 << 3) | (3 << 8) | (4 << 15) | (2 << 18),
                                                x_offset,
                                                y_offset: 512,
                                                width,
                                                height: 5000,
                                                ..Default::default()
                                            },
                                            ..Default::default()
                                        },
                                        parts: Vec::new(),
                                    })),
                                    src: None,
                                },
                                Control {
                                    kind: ControlKind::Section(Box::new(SectionDef {
                                        children: vec![SectionChild::Page(PageDef {
                                            width: 10000,
                                            height: 40000,
                                            ..Default::default()
                                        })],
                                        ..Default::default()
                                    })),
                                    src: None,
                                },
                            ],
                            line_segs: vec![LineSeg {
                                width: 0,
                                ..stored(0)
                            }],
                            src: Some(SharedBytes::from(vec![1])),
                            ..Default::default()
                        },
                        Paragraph {
                            text: vec![13],
                            line_segs: vec![stored(if pushed { 5512 } else { 1000 })],
                            src: Some(SharedBytes::from(vec![1])),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                }],
                source: None,
                bins: None,
                diagnostics: Vec::new(),
            };
            let layout = crate::layout(&doc, &hm_fonts::FontSystem::new());
            let page = &layout.pages[0];
            let object = page.objects[0].rect;
            let host = page.lines.iter().find(|l| l.para == 0).unwrap();
            let next = page.lines.iter().find(|l| l.para == 1).unwrap();
            assert_eq!(object.y, host.y + 512, "{width}");
            assert_eq!(host.vpos, 0, "{width}");
            if pushed {
                assert_eq!(next.y, object.y + object.h, "{width}");
            } else {
                assert!(next.y < object.y + object.h, "{width}");
            }
        }
    }

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
                // A stored page start, or a placeholder segment composed
                // fresh, moves the empty paragraph on; only a replayed foot
                // line stays.
                let (empty_at, heading_at) = match (kind, stored_vpos) {
                    (break_kind::PAGE, Some(0) | None) => ((1, 0, 0), (2, 0, 0)),
                    (break_kind::COLUMN, Some(0) | None) => ((0, 10000, 0), (1, 0, 0)),
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

    /// A run of empty paragraphs Hancom stored no lines for moves on like
    /// any other overflowing line: `76076-regulatory-analysis.hwp` (no line
    /// cache) opens Hancom's page 56 with paragraphs 548 and 549, 1200 HU
    /// above the next heading. A run Hancom stored at the foot stays, and so
    /// does a fresh run in a section hiding empty lines at a page top
    /// (`1130000-201900011.hwp` keeps paragraphs 742-743 at page 75's foot
    /// and opens page 76 with its text at the body top).
    #[test]
    fn fresh_empty_paragraph_runs_move_past_the_page_foot() {
        use hm_model::{
            Control, ControlKind, LineSeg, PageDef, SectionChild, SectionDef, SharedBytes,
        };

        for (stored, hide) in [(false, false), (true, false), (false, true)] {
            let paragraph = |text: Vec<u16>, vpos: i32, height: i32| Paragraph {
                text,
                line_segs: if stored || height == 9000 {
                    vec![LineSeg {
                        vpos,
                        height,
                        text_height: height,
                        baseline: height * 85 / 100,
                        width: 20000,
                        flags: LineSeg::FIRST_SEGMENT | LineSeg::LAST_SEGMENT,
                        ..Default::default()
                    }]
                } else {
                    Vec::new()
                },
                src: Some(SharedBytes::from(vec![1])),
                ..Default::default()
            };
            let mut filler = paragraph(vec![b'A' as u16, 13], 0, 9000);
            filler.controls = vec![Control {
                kind: ControlKind::Section(Box::new(SectionDef {
                    attr: if hide { 1 << 19 } else { 0 },
                    children: vec![SectionChild::Page(PageDef {
                        width: 20000,
                        height: 10000,
                        ..Default::default()
                    })],
                    ..Default::default()
                })),
                src: None,
            }];
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
                    paras: vec![
                        filler,
                        paragraph(vec![13], 9000, 1500),
                        paragraph(vec![13], 0, 1500),
                        paragraph(vec![b'B' as u16, 13], 1500, 1500),
                    ],
                    ..Default::default()
                }],
                source: None,
                bins: None,
                diagnostics: Vec::new(),
            };
            let layout = crate::layout(&doc, &hm_fonts::FontSystem::new());
            let page_of = |para| {
                layout
                    .pages
                    .iter()
                    .position(|p| p.lines.iter().any(|l| l.para == para))
                    .expect("placed paragraph")
            };
            let moves = !stored && !hide;
            assert_eq!(
                page_of(1),
                usize::from(moves),
                "stored {stored}, hide {hide}"
            );
            assert_eq!(page_of(2), 1, "stored {stored}, hide {hide}");
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
    fn repeated_title_table(offset: i32, no_adjust: u32) -> hm_model::Control {
        use hm_model::{Cell, Control, ControlKind, ObjectCommon, ObjectHeader, Table};
        Control {
            kind: ControlKind::Table(Box::new(Table {
                header: ObjectHeader {
                    common: ObjectCommon {
                        attr: (2 << 3) | (4 << 15) | (2 << 18) | (1 << 21),
                        width: 18000,
                        height: 8000,
                        y_offset: offset,
                        margins: [0, 0, 0, 200],
                        ..Default::default()
                    },
                    ..Default::default()
                },
                attr: 6 | no_adjust,
                rows: 3,
                cols: 1,
                padding: [0, 0, 100, 100],
                cells: (0..3)
                    .map(|row| Cell {
                        row,
                        row_span: 1,
                        col_span: 1,
                        width: 18000,
                        height: if row == 0 { 2000 } else { 3000 },
                        list_attr: if row == 0 { 1 << 18 } else { 0 },
                        paras: vec![Paragraph {
                            text: if row == 0 { "title" } else { "one\ntwo" }
                                .encode_utf16()
                                .chain([13])
                                .collect(),
                            ..Default::default()
                        }],
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            })),
            src: None,
        }
    }

    #[test]
    fn continued_bottom_captions_own_following_paragraph_clearance() {
        for captioned in [false, true] {
            for blank in [false, true] {
                for before in [500, 2000] {
                    let mut doc = keep_document(&[
                        ("intro\nintro\nintro", 0),
                        ("", 0),
                        (if blank { "" } else { "next" }, 0),
                        ("", 0),
                    ]);
                    doc.info.para_shapes[2].space_before = hm_model::units::pack_distance(before);
                    doc.info.para_shapes[3].space_before = hm_model::units::pack_distance(500);
                    let host = &mut doc.sections[0].paras[1];
                    host.text = vec![11, 0, 0, 0, 0, 0, 0, 11, 13];
                    let mut control = repeated_title_table(0, 0);
                    if captioned {
                        let hm_model::ControlKind::Table(t) = &mut control.kind else {
                            unreachable!()
                        };
                        t.caption = Some(hm_model::Caption {
                            attr: Some(3),
                            gap: Some(500),
                            paras: vec![Paragraph {
                                text: "caption\r".encode_utf16().collect(),
                                ..Default::default()
                            }],
                            ..Default::default()
                        });
                    }
                    host.controls.push(control);
                    let layout = crate::layout(&doc, &hm_fonts::FontSystem::new());
                    let page = &layout.pages[1];
                    let table = page.tables.last().unwrap();
                    assert!(table.top > 0);
                    let caption = layout
                        .para(0, 1)
                        .table(0)
                        .unwrap()
                        .caption
                        .as_ref()
                        .map_or(0, |c| c.gap + c.height);
                    let blocked = table.y - page.body.y + table.height() + caption + 200;
                    let next = page.lines.iter().find(|l| l.para == 2).unwrap();
                    assert_eq!(
                        next.vpos,
                        if captioned {
                            blocked.max(before)
                        } else {
                            blocked + before
                        },
                        "captioned={captioned}, blank={blank}, before={before}"
                    );
                    let ordinary = page.lines.iter().find(|l| l.para == 3).unwrap();
                    assert_eq!(ordinary.vpos, next.vpos + 1500);
                }
            }
        }
    }

    #[test]
    fn table_hosts_keep_the_origin_after_caption_clearance() {
        let mut doc = keep_document(&[("intro\nintro", 0), ("", 0), ("", 0)]);
        doc.info.para_shapes[2].space_before = hm_model::units::pack_distance(500);
        for pi in [1, 2] {
            doc.sections[0].paras[pi].text = vec![11, 0, 0, 0, 0, 0, 0, 11, 13];
            let mut c = repeated_title_table(0, 0);
            let hm_model::ControlKind::Table(t) = &mut c.kind else {
                unreachable!()
            };
            if pi == 1 {
                t.caption = Some(hm_model::Caption {
                    attr: Some(3),
                    gap: Some(500),
                    paras: vec![Paragraph {
                        text: "caption\r".encode_utf16().collect(),
                        ..Default::default()
                    }],
                    ..Default::default()
                });
            } else {
                t.rows = 1;
                t.cells.truncate(1);
                t.cells[0].height = 1000;
                t.cells[0].list_attr = 0;
                t.cells[0].paras = vec![Paragraph {
                    text: "next table\r".encode_utf16().collect(),
                    ..Default::default()
                }];
            }
            doc.sections[0].paras[pi].controls.push(c);
        }
        let layout = crate::layout(&doc, &hm_fonts::FontSystem::new());
        let page = &layout.pages[1];
        let prior = page.tables.iter().find(|t| t.para == 1).unwrap();
        let next = page.tables.iter().find(|t| t.para == 2).unwrap();
        let cap = layout
            .para(0, 1)
            .table(0)
            .unwrap()
            .caption
            .as_ref()
            .unwrap();
        assert_eq!(
            next.y,
            prior.y + prior.height() + cap.gap + cap.height + 200
        );
        assert_eq!(next.y, page.lines.iter().find(|l| l.para == 2).unwrap().y);
    }

    #[test]
    fn deferred_first_table_fragments_preserve_following_before_spacing() {
        let prefix = std::iter::repeat_n("intro", 7)
            .collect::<Vec<_>>()
            .join("\n");
        let mut doc = keep_document(&[(&prefix, 0), ("", 0), ("heading", 0), ("next", 0)]);
        doc.info.para_shapes[3].space_before = hm_model::units::pack_distance(500);
        let host = &mut doc.sections[0].paras[1];
        host.text = vec![11, 0, 0, 0, 0, 0, 0, 11, 13];
        let mut control = repeated_title_table(0, 0);
        let hm_model::ControlKind::Table(t) = &mut control.kind else {
            unreachable!()
        };
        t.attr = 0;
        host.controls.push(control);
        let layout = crate::layout(&doc, &hm_fonts::FontSystem::new());
        assert!(layout.pages[0].lines.iter().any(|l| l.para == 1));
        let page = &layout.pages[1];
        let table = &page.tables[0];
        assert_eq!(table.top, 0);
        let next = page.lines.iter().find(|l| l.para == 3).unwrap();
        assert_eq!(
            next.vpos,
            table.y - page.body.y + table.height() + 200 + 500
        );
    }

    /// A captioned table whose whole frame does not fit starts here with the
    /// rows that leave its margin and caption gap, as Hancom starts hwpspec
    /// table 328 on p50 with a 9739 HU first frame.
    #[test]
    fn a_table_host_stays_when_a_smaller_captioned_fragment_fits() {
        let mut doc = keep_document(&[("intro", 0), ("", 0), ("after", 0)]);
        doc.info.para_shapes[0].space_after = hm_model::units::pack_distance(900);
        let host = &mut doc.sections[0].paras[1];
        host.text = vec![11, 0, 0, 0, 0, 0, 0, 11, 13];
        let mut control = repeated_title_table(0, 0);
        let plain = {
            let mut plain = doc.clone();
            plain.sections[0].paras[1].controls.push(control.clone());
            crate::layout(&plain, &hm_fonts::FontSystem::new())
        };
        assert!(!plain.pages[0].tables.iter().any(|t| t.para == 1));
        let host = &mut doc.sections[0].paras[1];
        let hm_model::ControlKind::Table(table) = &mut control.kind else {
            unreachable!()
        };
        table.caption = Some(hm_model::Caption {
            count: 1,
            attr: Some(3),
            gap: Some(500),
            paras: vec![Paragraph {
                text: "caption\r".encode_utf16().collect(),
                ..Default::default()
            }],
            ..Default::default()
        });
        host.controls.push(control);
        let layout = crate::layout(&doc, &hm_fonts::FontSystem::new());
        assert_eq!(layout.pages.len(), 2);
        let first = layout.pages[0].tables.iter().find(|t| t.para == 1).unwrap();
        assert_eq!(first.top, 0);
        assert!(first.bottom > 0 && first.bottom < 8000);
        assert!(!first.caption);
        assert!(
            layout.pages[1]
                .tables
                .iter()
                .any(|t| t.para == 1 && t.caption)
        );
        assert_eq!(placements(&layout, 2).len(), 1);
    }

    #[test]
    fn imported_merged_restarts_follow_the_actual_row_cut_without_polluting_the_cache() {
        use hm_model::{
            Cell, Control, ControlKind, LineSeg, ObjectCommon, ObjectHeader, ParaHeader, ParaShape,
            SharedBytes, Table,
        };

        let fixture = |restart: bool, wrap: u32| {
            let mut doc = keep_document(&[
                ("intro\nintro\nintro\nintro\nintro\nintro\nintro", 0),
                ("", 0),
                ("after", 0),
            ]);
            doc.info.para_shapes.push(ParaShape {
                line_spacing_legacy: 130,
                ..Default::default()
            });
            let para = Paragraph {
                header: ParaHeader {
                    para_shape: 3,
                    ..Default::default()
                },
                text: "A\nB\r".encode_utf16().collect(),
                line_segs: vec![
                    LineSeg {
                        text_start: 0,
                        vpos: 0,
                        height: 1000,
                        text_height: 1000,
                        baseline: 850,
                        spacing: 300,
                        width: 8800,
                        flags: (1 << 17) | (1 << 18),
                        ..Default::default()
                    },
                    LineSeg {
                        text_start: 2,
                        vpos: if restart { 0 } else { 1300 },
                        height: 1000,
                        text_height: 1000,
                        baseline: 850,
                        spacing: 300,
                        width: 8800,
                        flags: (1 << 17) | (1 << 18),
                        ..Default::default()
                    },
                ],
                src: Some(SharedBytes::from(vec![1])),
                ..Default::default()
            };
            let mut cells = vec![Cell {
                row_span: 4,
                col_span: 1,
                width: 9000,
                height: 8000,
                list_attr: 1 << 5,
                paras: vec![para.clone()],
                ..Default::default()
            }];
            for row in 0..4 {
                cells.push(Cell {
                    row,
                    col: 1,
                    row_span: 1,
                    col_span: 1,
                    width: 9000,
                    height: 2000,
                    paras: vec![Paragraph {
                        text: "x\r".encode_utf16().collect(),
                        src: Some(SharedBytes::from(vec![2 + row as u8])),
                        ..Default::default()
                    }],
                    ..Default::default()
                });
            }
            let host = &mut doc.sections[0].paras[1];
            host.text = vec![11, 0, 0, 0, 0, 0, 0, 11, 13];
            host.src = Some(SharedBytes::from(vec![9]));
            host.controls.push(Control {
                kind: ControlKind::Table(Box::new(Table {
                    header: ObjectHeader {
                        common: ObjectCommon {
                            attr: (2 << 3) | (4 << 15) | (2 << 18) | (wrap << 21),
                            width: 18000,
                            height: 8000,
                            y_offset: 800,
                            margins: [0, 0, 0, 100],
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                    attr: 2,
                    rows: 4,
                    cols: 2,
                    padding: [100; 4],
                    cells,
                    ..Default::default()
                })),
                src: Some(SharedBytes::from(vec![10])),
            });
            doc
        };
        let fonts = hm_fonts::FontSystem::new();
        for wrap in [0, 1] {
            let mut doc = fixture(true, wrap);
            let mut cache = crate::LayoutCache::new();
            let layout = crate::layout_cached(&doc, &fonts, crate::Options::default(), &mut cache);
            assert_eq!(layout.pages[0].tables[0].bottom, 2000);
            assert_eq!(layout.pages[1].tables[0].top, 2000);
            let cl = &layout.para(0, 1).table(0).unwrap().cells[0];
            assert_eq!(cl.paras[0].lines[0].spacing, 300);
            assert_eq!(cl.paras[0].lines[1].height, 1000);
            // Move the table onto one page without editing the cached host.
            doc.sections[0].paras[0].text = "intro\r".encode_utf16().collect();
            let moved = crate::layout_cached(&doc, &fonts, crate::Options::default(), &mut cache);
            assert!(cache.stats().0 > 0);
            assert_eq!(moved.pages[0].tables[0].bottom, 8000);
            assert_eq!(moved.para(0, 1).table(0).unwrap().cells[0].lines[1].y, 1300);
            let valid = crate::layout(&fixture(false, wrap), &fonts);
            assert_eq!(valid.para(0, 1).table(0).unwrap().cells[0].lines[1].y, 1300);
        }
    }
}
