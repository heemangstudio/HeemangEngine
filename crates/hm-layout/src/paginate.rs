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
    /// When the section's footnotes restart on each page: every footnote
    /// with the number it shows, counted from 1 on the page holding its
    /// reference mark, in body order.
    pub note_numbers: Vec<(ListRef, u32)>,
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

/// The paginator's state before a body paragraph, kept so that a later
/// layout of the same section can go on from there instead of from the
/// section's start.
#[derive(Clone)]
pub(crate) struct Snap {
    /// The paragraph about to be placed.
    pub(crate) para: usize,
    /// The last paragraph the layout of the paragraphs before `para` looked
    /// at: an edit after it leaves this state as it is.
    pub(crate) reach: usize,
    /// Length of the page list, the page being filled included.
    pub(crate) pages: usize,
    /// The page being filled, as far as it is.
    page: Page,
    /// The rest of the flow, without its pages.
    flow: Box<Flow>,
    heads: Heads,
    line_grid: i32,
    char_grid: crate::style::CharGrid,
    /// Paper and body of the section's first page.
    geometry: (i32, i32, Rect),
}

impl Snap {
    #[inline(never)]
    fn take(
        flow: &mut Flow,
        para: usize,
        reach: usize,
        heads: &Heads,
        grids: (i32, crate::style::CharGrid),
        geometry: (i32, i32, Rect),
    ) -> Self {
        let pages = std::mem::take(&mut flow.pages);
        // Only sections numbering notes anew on each page read the marks,
        // and those are not resumed.
        let marks = std::mem::take(&mut flow.note_marks);
        let copy = flow.clone();
        flow.pages = pages;
        flow.note_marks = marks;
        Snap {
            para,
            reach,
            pages: flow.pages.len(),
            page: flow.pages.last().cloned().unwrap_or_default(),
            flow: Box::new(copy),
            heads: heads.clone(),
            line_grid: grids.0,
            char_grid: grids.1,
            geometry,
        }
    }
}

/// A page count to stop at: pagination stops before the first paragraph
/// that starts after the page list holds more pages than `pages`, once it
/// kept the state there ([`Snap`]); `hit` says it stopped.
pub(crate) struct Stop {
    pub(crate) pages: usize,
    pub(crate) hit: bool,
}

/// Where a section's pagination goes on: a snapshot of an earlier layout of
/// the same section and the paragraph layouts placed before it. The page
/// list handed over holds the pages before the snapshot's page.
pub(crate) struct Resume<'s> {
    /// The earlier snapshot pagination goes on from; `None` starts the
    /// section afresh.
    pub(crate) snap: Option<&'s Snap>,
    pub(crate) paras: Vec<ParaLayout>,
    /// Snapshots of the earlier layout after `snap`, and the paragraph from
    /// which every paragraph is as it was then: once the state before such
    /// a paragraph equals the earlier one, the rest of the section is as it
    /// was, and pagination stops there ([`Resume::met`]).
    pub(crate) later: &'s [Snap],
    pub(crate) same_from: usize,
    /// The paragraph pagination stopped at and the index of the earlier
    /// snapshot it met there.
    pub(crate) met: Option<(usize, usize)>,
}

impl Snap {
    /// Whether the paginator now stands where it stood at this snapshot,
    /// with the page being filled holding only paragraphs from `same_from`
    /// on, which are as they were.
    #[inline(never)]
    fn met(
        &self,
        flow: &Flow,
        heads: &Heads,
        grids: (i32, crate::style::CharGrid),
        same_from: usize,
    ) -> bool {
        let Some(page) = flow.pages.last() else {
            return false;
        };
        let own = |p: u32| p as usize >= same_from;
        self.pages == flow.pages.len()
            && flow.idle()
            && self.flow.idle()
            && page.lines.iter().all(|l| own(l.para))
            && page.tables.iter().all(|t| own(t.para))
            && page.objects.iter().all(|o| own(o.para))
            && page.frames.is_empty()
            && self.page.frames.is_empty()
            && page.lines == self.page.lines
            && page.tables == self.page.tables
            && page.objects == self.page.objects
            && page.columns == self.page.columns
            && page.rules == self.page.rules
            && page.body == self.page.body
            && (page.width, page.height) == (self.page.width, self.page.height)
            && (self.line_grid, self.char_grid) == grids
            && self.heads == *heads
            && self.flow.same_idle(flow)
    }
}

#[derive(Clone)]
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
    /// not set as characters and not placed yet.
    table_notes: Vec<TableNote>,
    /// Bottom of the lowest line set in the current region on this page,
    /// from the region top, without the spacing below it.
    pub(crate) ink: i32,
    /// The column of the last line or table placed in the current region on
    /// this page, and its bottom from the region top without the spacing
    /// below it.
    pub(crate) line_end: Option<(usize, i32)>,
    /// Per column of the current page, as `(page x, page y)`: the bottom of
    /// the lowest line or table set in it, without the spacing below.
    col_bottoms: Vec<(i32, i32)>,
    /// The footnotes whose reference marks were set, with the index of the
    /// page holding the mark.
    note_marks: Vec<(usize, ListRef)>,
    /// A table's last continuation fragment that advanced the fill of a
    /// column: page count and column then, its host paragraph, the fill
    /// before it and the fill it left. While the fill stands there, the
    /// next paragraph's spacing before counts from the fill before the
    /// fragment.
    pub(crate) absorbed: Option<(usize, usize, u32, i32, i32)>,
    /// The fill of the column the current paragraph's first line went to,
    /// with its page count and column: its tables do not rise above it.
    pub(crate) para_floor: (usize, usize, i32),
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
    /// Tables that wait for the next column behind the pending one, in
    /// order: they open it below it.
    pub(crate) queued_tables: Vec<Box<crate::table::Pending>>,
    /// Empty table anchors may remain on the page their tables left.
    pub(crate) hold_table_anchors: bool,
    pub(crate) unheld_table_page: Option<usize>,
    /// Top-and-bottom objects moved past the foot of the page their
    /// paragraph stays on: they open the next page, which their bands
    /// keep the lines off.
    carried_objects: Vec<crate::objects::Placement>,
    carried_bands: Vec<Band>,
}

impl Flow {
    /// True when nothing an earlier paragraph laid out waits to be placed:
    /// no notes, tables or carried objects.
    fn idle(&self) -> bool {
        self.notes.idle()
            && self.deferred.idle()
            && self.pending_table.is_none()
            && self.queued_tables.is_empty()
            && self.table_notes.is_empty()
            && self.carried_objects.is_empty()
            && self.carried_bands.is_empty()
    }

    /// Whether two idle flows, pages aside, place what follows alike.
    #[inline(never)]
    fn same_idle(&self, o: &Flow) -> bool {
        self.section == o.section
            && self.paper == o.paper
            && self.body == o.body
            && self.cols == o.cols
            && self.col == o.col
            && self.region_top == o.region_top
            && self.fill == o.fill
            && self.has_content == o.has_content
            && self.bands == o.bands
            && self.separator == o.separator
            && self.notes.same_idle(&o.notes)
            && self.balanced == o.balanced
            && self.parallel == o.parallel
            && self.region_first_line == o.region_first_line
            && self.hidden_top == o.hidden_top
            && self.mirror == o.mirror
            && self.number == o.number
            && self.ink == o.ink
            && self.line_end == o.line_end
            && self.col_bottoms == o.col_bottoms
            && self.absorbed == o.absorbed
            && self.region_closed == o.region_closed
            && self.region_gap == o.region_gap
            && self.first_page == o.first_page
            && self.merged_reserve == o.merged_reserve
            && self.merged_seen == o.merged_seen
            && self.deferred == o.deferred
            && self.hold_table_anchors == o.hold_table_anchors
            && self.unheld_table_page == o.unheld_table_page
    }

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

    pub(crate) fn new_page(&mut self) {
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
        self.col_bottoms.clear();
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
        self.adopt_notes(true);
    }

    /// Takes the notes going on from the last column or page into the
    /// current column's note area, as many of their lines as its room
    /// holds; the rest go on further. `fresh` marks a new page, which takes
    /// them whole when not even one line fits.
    fn adopt_notes(&mut self, fresh: bool) {
        let (x, w) = self.note_column();
        // The text already set in the columns over the note area.
        let text = self
            .cols
            .iter()
            .zip(&self.fill)
            .filter(|((cx, _), _)| (x..x + w).contains(&(self.body.x + cx)))
            .map(|(_, f)| *f)
            .max()
            .unwrap_or(0);
        let room = self.body.h - self.region_top - text - self.notes.placed(x);
        self.notes.adopt_carried(x, w, room, fresh);
    }

    /// Records the bottom of a line or table just set in the current
    /// column, from the region top and without the spacing below it.
    pub(crate) fn set_line_end(&mut self, bottom: i32) {
        self.line_end = Some((self.col, bottom));
        let (x, y) = (self.column().0, self.page_y(bottom));
        match self.col_bottoms.iter_mut().find(|c| c.0 == x) {
            Some(c) => c.1 = c.1.max(y),
            None => self.col_bottoms.push((x, y)),
        }
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

    /// The spacing before a paragraph starting at the column's fill. Below
    /// a table's last continuation fragment whose host line stayed on an
    /// earlier page, Hancom places the paragraph's spacing before from
    /// where the text stood above the fragment: it overlaps the fragment
    /// and takes no room below it.
    pub(crate) fn before_here(&self, before: i32) -> i32 {
        match self.absorbed {
            Some((page, col, _, above, fill))
                if page == self.pages.len() && col == self.col && self.fill[col] == fill =>
            {
                (above + before - fill).max(0)
            }
            _ => before,
        }
    }

    /// The fill the current paragraph's first line went to when it stands
    /// in this column, else 0.
    pub(crate) fn para_floor(&self) -> i32 {
        match self.para_floor {
            (page, col, fill) if page == self.pages.len() && col == self.col => fill,
            _ => 0,
        }
    }

    /// Ends paragraph `pi` with its spacing `after` overlapping the last
    /// continuation fragment of its own table, as its spacing before does
    /// for the next paragraph; false when no such fragment holds the fill.
    pub(crate) fn after_here(&mut self, pi: u32, after: i32) -> bool {
        let Some((page, col, host, above, fill)) = self.absorbed else {
            return false;
        };
        if page != self.pages.len() || col != self.col || host != pi || self.fill[col] != fill {
            return false;
        }
        let fill = fill.max(above + after);
        self.fill[col] = fill;
        self.absorbed = Some((page, col, host, above + after, fill));
        true
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

    /// Moves every footnote of the current paragraph's table `control` to
    /// the foot of the current column, as for a table at its anchored frame.
    pub(crate) fn take_table_notes(&mut self, control: u32) {
        let taken = self.take_notes_of(control);
        if taken.is_empty() {
            return;
        }
        let (x, w) = self.note_column();
        self.notes
            .add(x, w, taken.into_iter().map(|n| n.frame).collect());
    }

    /// Marks the footnotes of the flowing table `control` referred to above
    /// table y `upto` (all of them without it) as referred to on this page.
    pub(crate) fn mark_table_notes(&mut self, control: u32, upto: Option<i32>) {
        let mut marks = Vec::new();
        for n in &mut self.table_notes {
            if n.control == control && !n.marked && upto.is_none_or(|u| n.y < u) {
                n.marked = true;
                marks.push(n.frame.list.clone());
            }
        }
        self.notes.mark_refs(marks);
    }

    /// Sets the waiting footnotes of the flowing table `control`, in order,
    /// in the room left below region y `bottom` (a fragment's end with its
    /// bottom margin), splitting one between its lines. Hancom never moves
    /// a table's cut for the notes of its cells: they fill the room under
    /// each fragment from the table's first page on, and those left after
    /// its last fragment go on at the foot of the next column or page
    /// without pushing the text after the table (probes i-fn-long,
    /// i-fn-full, i-fn-table-mode, i-fn-after-cut).
    pub(crate) fn place_table_notes(&mut self, control: u32, bottom: i32, last: bool) {
        let mine = self.take_notes_of(control);
        if mine.is_empty() {
            return;
        }
        let (x, w) = self.note_column();
        let room = self.region_height() - bottom - 1;
        let n = mine.len();
        let frames = mine.iter().map(|t| t.frame.clone()).collect();
        let (fit, rest) = self.notes.split_to_fit(x, frames, room);
        self.notes.push(x, w, fit);
        if last {
            self.notes.defer(rest);
            return;
        }
        // A note split between its lines keeps its entry for the rest.
        let kept = mine
            .into_iter()
            .skip(n - rest.len())
            .zip(rest)
            .map(|(t, frame)| TableNote { frame, ..t });
        let mut left: Vec<TableNote> = kept.collect();
        left.append(&mut self.table_notes);
        self.table_notes = left;
    }

    /// Places the page's waiting footnotes.
    fn flush_notes(&mut self) {
        let page = self.pages.len().saturating_sub(1);
        let marked = self.notes.take_marked();
        self.note_marks
            .extend(marked.into_iter().map(|l| (page, l)));
        let body = self.body;
        if self.notes.placement() == 1 {
            let k = self.pages.len().saturating_sub(1 + self.first_page);
            if self.merged_seen.len() <= k {
                self.merged_seen.resize(k + 1, 0);
            }
            self.merged_seen[k] = self.notes.placed_here(body.x);
        }
        if let Some(page) = self.pages.last_mut() {
            self.notes.flush(page, body, &self.col_bottoms);
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
        let queued = std::mem::take(&mut self.queued_tables);
        self.move_column();
        if let Some(p) = waiting {
            crate::table::resume(self, *p);
        }
        for q in queued {
            if self.pending_table.is_some() {
                self.queued_tables.push(q);
            } else {
                crate::table::resume_queued(self, *q);
            }
        }
    }

    /// Places the rest of the waiting tables, column by column.
    pub(crate) fn flush_tables(&mut self) {
        let mut guard = 0;
        while (self.pending_table.is_some() || !self.queued_tables.is_empty()) && guard < 100_000 {
            guard += 1;
            self.next_column();
        }
        self.pending_table = None;
        self.queued_tables.clear();
    }

    pub(crate) fn table_notes_mut(&mut self) -> &mut Vec<TableNote> {
        &mut self.table_notes
    }

    /// Takes the waiting footnotes of the cells of table `control`.
    pub(crate) fn take_notes_of(&mut self, control: u32) -> Vec<TableNote> {
        let (taken, kept) = std::mem::take(&mut self.table_notes)
            .into_iter()
            .partition(|n| n.control == control);
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
                self.adopt_notes(false);
            }
            return;
        }
        if self.col + 1 < self.cols.len() {
            self.col += 1;
            self.has_content = false;
            self.adopt_notes(false);
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

    /// Replaces the column fills, the ink bottom and each column's last
    /// line bottom (`None` for a column left empty) after the region's
    /// lines moved; `col` is the column the flow continues in.
    pub(crate) fn set_fill(
        &mut self,
        fill: Vec<i32>,
        col: usize,
        ink: i32,
        bottoms: &[Option<i32>],
    ) {
        if fill.len() != self.fill.len() {
            return;
        }
        self.fill = fill;
        self.col = col;
        self.ink = ink;
        for (c, bottom) in bottoms.iter().enumerate() {
            let x = self.column_x(c);
            // The region's lines set the column's bottom anew; an emptied
            // column ends where the region starts.
            let y = self.page_y(bottom.unwrap_or(0));
            match self.col_bottoms.iter_mut().find(|e| e.0 == x) {
                Some(e) => e.1 = y,
                None if bottom.is_some() => self.col_bottoms.push((x, y)),
                None => {}
            }
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
    // Columns of differing widths stay as Hancom scales them (ktx p1 stores
    // lines of 33355 and 44863 HU); the others round down to 4 HU.
    let unequal = cols.windows(2).any(|w| w[0].1 != w[1].1);
    let cols: Vec<(i32, i32)> = cols
        .into_iter()
        .map(|(x, w)| (x, if unequal { w } else { w & !3 }))
        .collect();
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
    /// The paragraph `preview` stands before: heads resolve from there on.
    base: usize,
    resolved_heads: Vec<Option<crate::heads::Head>>,
    budgets: std::collections::HashMap<(i32, u32, bool), std::collections::HashMap<usize, i32>>,
    /// The last paragraph a budget looked at.
    reach: usize,
}

impl<'a> KeepChains<'a> {
    /// Chains looked up from paragraph `base` on, `heads` being the
    /// counters before it.
    fn new(
        sec: &'a Section,
        info: &'a hm_model::DocInfo,
        default_tab: i32,
        heads: &Heads,
        outline: u16,
        base: usize,
    ) -> Self {
        Self {
            paras: &sec.paras,
            info,
            default_tab,
            preview: heads.clone(),
            outline,
            base,
            resolved_heads: Vec::new(),
            budgets: Default::default(),
            reach: 0,
        }
    }

    /// Body consumes a head for every source paragraph, including empty or
    /// hidden ones, before placing any lines. Replay that same order once
    /// from the section pass's initial counters, without advancing the live
    /// counters. Heads do not depend on width or page regions; their resolved
    /// values belong only to this immutable source and outline environment.
    fn preview_through(&mut self, at: usize) {
        while self.base + self.resolved_heads.len() <= at {
            let p = &self.paras[self.base + self.resolved_heads.len()];
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
            self.reach = self.reach.max(at);
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
                self.resolved_heads[at - self.base].as_ref(),
            );
            if shape.is_some_and(|s| s.keep_with_next()) {
                self.reach = self.reach.max(at + 1);
            }
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
    mut resume: Option<&mut Resume<'_>>,
    mut snaps: Option<&mut Vec<Snap>>,
    mut stop: Option<&mut Stop>,
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
    let mut shapes = def.into_iter().flat_map(|d| d.note_shapes());
    let footnote = shapes.next().map(crate::numbers::MarkFormat::of);
    let endnote = shapes.next().map(crate::numbers::MarkFormat::of);
    styles.note_marks = [footnote.unwrap_or_default(), endnote.unwrap_or_default()];
    let paper = page_def.paper();
    styles.paper = paper;
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
        col_bottoms: Vec::new(),
        note_marks: Vec::new(),
        absorbed: None,
        para_floor: (0, 0, 0),
        region_closed: false,
        region_gap: def.map_or(0, |d| i32::from(d.column_gap)),
        deferred: Default::default(),
        pending_table: None,
        queued_tables: Vec::new(),
        hold_table_anchors: crate::compat::effective(&doc.info)[3] & (1 << 6) == 0,
        unheld_table_page: None,
        carried_objects: Vec::new(),
        carried_bands: Vec::new(),
    };
    flow.mirror = mirror_of(&page_def);
    let mut out = SectionLayout {
        paras: Vec::with_capacity(sec.paras.len()),
        merged_notes: Vec::new(),
        note_numbers: Vec::new(),
    };
    let mut start = 0;
    let mut reach = 0;
    let resume_geometry = resume.as_ref().and_then(|r| r.snap).map(|s| s.geometry);
    match resume.as_deref_mut().and_then(|r| r.snap.map(|s| (s, r))) {
        Some((snap, r)) => {
            let mut restored = (*snap.flow).clone();
            restored.pages = std::mem::take(&mut flow.pages);
            restored.pages.push(snap.page.clone());
            flow = restored;
            *heads = snap.heads.clone();
            (line_grid, char_grid) = (snap.line_grid, snap.char_grid);
            out.paras = std::mem::take(&mut r.paras);
            out.paras.reserve(sec.paras.len().saturating_sub(snap.para));
            (start, reach) = (snap.para, snap.reach);
        }
        None => flow.new_page(),
    }
    // Paper and body of the section's first page.
    let geometry = match &resume_geometry {
        Some(g) => *g,
        None => (paper.0, paper.1, flow.body),
    };
    let mut snapped = flow.pages.len();
    let hwp5 = doc.meta.format != hm_model::Format::Hwpx;
    let mut keep_chains = KeepChains::new(sec, &doc.info, default_tab, heads, outline, start);
    for (pi, para) in sec.paras.iter().enumerate().skip(start) {
        // A paragraph starting after a new page keeps the state it starts
        // from, for later layouts to go on from.
        if pi > 0 && flow.pages.len() != snapped {
            // Standing where an earlier layout stood before an unchanged
            // paragraph, the rest of the section is as it was.
            if let Some(r) = resume.as_deref_mut()
                && pi >= r.same_from
                && let Ok(k) = r.later.binary_search_by_key(&pi, |s| s.para)
                && r.later[k].met(&flow, heads, (line_grid, char_grid), r.same_from)
            {
                r.met = Some((pi, k));
                *pages = flow.pages;
                return out;
            }
        }
        if let Some(snaps) = snaps.as_deref_mut()
            && pi > 0
            && flow.pages.len() != snapped
        {
            snapped = flow.pages.len();
            snaps.push(Snap::take(
                &mut flow,
                pi,
                reach,
                heads,
                (line_grid, char_grid),
                geometry,
            ));
            if let Some(stop) = stop.as_deref_mut()
                && flow.pages.len() > stop.pages
            {
                stop.hit = true;
                *pages = flow.pages;
                return out;
            }
        }
        // The paragraph after this one is looked at below.
        reach = reach.max(pi + 1);
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
        reach = reach.max(keep_chains.reach);
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
        let host_top = flow.fill[flow.col] + flow.before_here(layout.before);
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
            let mut before = if li == 0 {
                flow.before_here(layout.before)
            } else {
                0
            };
            if li == 0 {
                flow.para_floor = (flow.pages.len(), flow.col, flow.fill[flow.col]);
            }
            let natural = flow.fill[flow.col] + before;
            let para_left = flow.column().0 + left;
            crate::table::before_line(&mut flow, pi as u32, para, &layout, li, natural, para_left);
            // Objects hanging from this line block their bands from its top
            // on and sit on this page.
            let mut late_hung: Vec<usize> = Vec::new();
            let mut late_page = 0;
            if li > 0 {
                let late =
                    |k: usize| !hung.contains(&k) && objects::anchor_line(para, &layout, k) == li;
                let at = Anchor {
                    para_top: flow.page_y(natural),
                    ..anchor(&flow)
                };
                let b = objects::bands_of(para, &layout, &at, &late);
                let placed = objects::place_of(para, pi as u32, &at, &late);
                late_hung = placed.iter().map(|p| p.control as usize).collect();
                late_page = flow.pages.len();
                hung.extend(late_hung.iter().copied());
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
                if y + extent < limit {
                    break y;
                }
                // A line that fits while its footnotes do not stays; the
                // notes go on in the next column, also from an empty one.
                if !notes.is_empty() && y + extent < flow.region_height() {
                    carry = true;
                    break y;
                }
                // A column whose note area runs over with notes going on
                // holds no more text: the line moves on with them.
                if !flow.has_content && (too_tall || moved) && !flow.notes.has_carried() {
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
                // follows it. A stored first line starts a page when it sits
                // at the paragraph's spacing before (Hancom stores the top
                // line of a page there); a document composing its lines
                // afresh stores none that count.
                let stored = styles
                    .replay_imported_lines()
                    .then(|| para.line_segs.first())
                    .flatten();
                let off_top = |s: &hm_model::LineSeg| s.vpos != layout.before;
                let next_empty = next.is_some_and(|n| {
                    n.controls.is_empty()
                        && n.units()
                            .all(|(_, u)| matches!(u, hm_model::Unit::CharControl(13)))
                }) && stored.map_or(hide_empty, off_top);
                // Paragraphs typed since reading (no source) move on as
                // Hancom moves them when it is editing.
                if empty
                    && para.src.is_some()
                    && (!line.stored || stored.is_none_or(off_top))
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
                // A table waiting for this column went ahead of the line:
                // the paragraph's spacing before may overlap its foot.
                if li == 0 {
                    before = flow.before_here(layout.before);
                    flow.para_floor = (flow.pages.len(), flow.col, flow.fill[flow.col]);
                }
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
            // The line went on to another column: the objects hanging from it
            // go with it (hwp3-sample p7, Hancom PDF: a picture 8400 HU below
            // the line holding it opens the next page below that line).
            if moved && !late_hung.is_empty() {
                if let Some(page) = flow.pages.get_mut(late_page.wrapping_sub(1)) {
                    page.objects.retain(|o| {
                        o.para != pi as u32 || !late_hung.contains(&(o.control as usize))
                    });
                }
                let again = |k: usize| late_hung.contains(&k);
                let at = Anchor {
                    para_top: flow.page_y(line_top),
                    ..anchor(&flow)
                };
                let b = objects::bands_of(para, &layout, &at, &again);
                let placed = objects::place_of(para, pi as u32, &at, &again);
                flow.bands
                    .extend(b.now.into_iter().chain(b.own).chain(b.after));
                if let Some(page) = flow.pages.last_mut() {
                    page.objects.extend(placed);
                }
            }
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
            if flow.absorbed.is_some_and(|a| a.2 != pi as u32) {
                flow.absorbed = None;
            }
            if !collapsed {
                flow.ink = flow.ink.max(line_top + line.height);
                flow.set_line_end(line_top + line.height);
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
        if !collapsed && !flow.after_here(pi as u32, layout.after) {
            // Spacing after a paragraph that ends inside the column does not
            // carry the fill past the column's end: an empty paragraph after
            // it may still stay at the foot.
            let fill = flow.fill[flow.col];
            let room = flow.region_height().max(fill);
            flow.fill[flow.col] = on_grid(fill + layout.after, flow.region_top, grid).min(room);
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
        // One crossing the foot of the page its paragraph stays on opens the
        // next column, or the next page from the last one, instead, and the
        // text after it goes on filling this column, whatever its wrapping
        // (hwp3-sample4 p25/p26: square; text-footnote-tail p210/p211: behind).
        let next_col = flow.col + 1 < flow.cols.len();
        let moved: Vec<usize> = if flow.pages.len() == anchor_page
            && (flow.cols.len() == 1 || next_col)
        {
            placed
                .iter()
                .filter_map(|p| objects::spills(para, p, flow.body))
                .map(|k| k as usize)
                .filter(|&k| {
                    para.controls[k].object_common().is_some_and(|o| {
                        o.height + i32::from(o.margins[2]) + i32::from(o.margins[3]) <= flow.body.h
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
            // A content-less host line that holds more than a lone
            // top-and-bottom object goes below it, as below a table
            // (synth-cell-enter p1: Hancom's next line a line under the box).
            if layout.lines.len() == 1
                && !layout.line_facts(para).content(0)
                && crate::table::host_takes_room(para, layout.line_facts(para).controls(0))
                && para.line_segs.first().is_none_or(|s| s.width > 0)
            {
                let para_top = anchor_at_start.para_top;
                let line_end = para_top + layout.before + layout.lines[0].height;
                let bottom = pending
                    .iter()
                    .filter(|b| b.top >= para_top && b.top < line_end)
                    .map(|b| b.bottom)
                    .max();
                if let Some(bottom) = bottom {
                    let below = bottom - flow.page_y(0) + layout.lines[0].advance() + layout.after;
                    let col = flow.col;
                    flow.fill[col] = flow.fill[col].max(below);
                }
            }
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
    // Footnotes carried past the body's last page go on in the note areas
    // of pages of their own.
    let mut guard = 0;
    while flow.notes.has_carried() && guard < 100_000 {
        flow.new_page();
        guard += 1;
    }
    flow.close_page_region();
    flow.flush_notes();
    // Footnotes numbered anew on each page (shape bits 10-11 = 2) count
    // from 1 on the page of their mark, whatever number they start at.
    let restart = def
        .and_then(|d| d.note_shapes().next())
        .is_some_and(|s| (s.attr >> 10) & 3 == 2);
    if restart {
        let mut on = None;
        let mut n = 0;
        for (page, list) in std::mem::take(&mut flow.note_marks) {
            n = if on == Some(page) { n + 1 } else { 1 };
            on = Some(page);
            out.note_numbers.push((list, n));
        }
    }
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
    // The note's own number shows what its mark shows.
    styles.note_number = styles.note_numbers.get(&crate::numbers::key(note)).copied();
    let frame = crate::furniture::frame(
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
    );
    styles.note_number = None;
    frame
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

/// A footnote referred to in a cell of a table that is not set as a
/// character, waiting for its table's fragments.
#[derive(Clone, Debug)]
pub(crate) struct TableNote {
    /// The table's control index in its paragraph.
    pub control: u32,
    /// Table y of the referring line.
    pub y: i32,
    /// Its mark was counted on a page.
    pub marked: bool,
    pub frame: Frame,
}

/// The footnotes in the cells of a paragraph's tables that are not set as
/// characters, laid out for the column `(x, width)`, in cell order. Table
/// placement takes them as it places each fragment.
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
) -> Vec<TableNote> {
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
            let frame = note_frame(doc, list, n.note, column, geometry, styles, default_tab);
            out.push(TableNote {
                control: k as u32,
                y: n.y,
                marked: false,
                frame,
            });
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
        // Each numbering counts on its own: the deeper paragraph starts
        // numbering 2 (its unused level 1 counted as started), the outline
        // paragraph continues it, numbering 1 continues past both.
        for (para, text) in [(1, "9"), (2, "10"), (3, "1.1"), (4, "11"), (5, "2")] {
            assert_eq!(head_text(&layout, 0, para), text);
        }
        assert_eq!(head_text(&layout, 1, 0), "12");
        assert_eq!(head_text(&layout, 1, 1), "13");
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

    // text-footnote-tail p210/p211, hwp3-sample4 p25/p26 (Hancom PDFs): a
    // picture kept in the body, behind the text or square, that would cross
    // the foot opens the next page at its top, its offset dropped, while the
    // text after its paragraph stays.
    #[test]
    fn objects_of_any_wrapping_crossing_the_foot_open_the_next_page() {
        use hm_model::{Control, ControlKind, CtrlId, ObjectCommon, ObjectControl, ObjectHeader};

        for wrap in [0u32, 2, 3] {
            let filler = vec!["a"; 7].join("\n");
            let mut doc = keep_document(&[(&filler, 0), ("", 0), ("after", 0)]);
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
                                | (wrap << 21),
                            y_offset: 1300,
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
            assert!(layout.pages[0].objects.is_empty(), "wrap {wrap}");
            assert_eq!(layout.pages[1].objects[0].rect.y, layout.pages[1].body.y);
            assert_eq!(placements(&layout, 1)[0].0, 0);
            assert_eq!(placements(&layout, 2)[0].0, 0);
        }
    }

    // synth-cell-enter p1 (Hancom's stored lines and PDF): a content-less host
    // line holding a top-and-bottom object goes below it when another object
    // shares the line; a lone one takes no room, as a lone table.
    #[test]
    fn a_host_line_goes_below_a_top_and_bottom_object_it_shares_with_another() {
        use hm_model::{Control, ControlKind, CtrlId, ObjectCommon, ObjectControl, ObjectHeader};

        let object = |wrap: u32| Control {
            kind: ControlKind::Object(Box::new(ObjectControl {
                id: CtrlId::SHAPE,
                header: ObjectHeader {
                    common: ObjectCommon {
                        attr: (2 << 3)
                            | (2 << 8)
                            | (1 << 13)
                            | (4 << 15)
                            | (2 << 18)
                            | (wrap << 21),
                        width: 5000,
                        height: 3000,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                parts: Vec::new(),
            })),
            src: None,
        };
        for (others, room) in [(0, 0), (1, 1000)] {
            let mut doc = keep_document(&[("intro", 0), ("", 0), ("after", 0)]);
            let host = &mut doc.sections[0].paras[1];
            host.text = vec![11, 0, 0, 0, 0, 0, 0, 11];
            host.controls.push(object(1));
            for _ in 0..others {
                host.text.extend([11, 0, 0, 0, 0, 0, 0, 11]);
                host.controls.push(object(3));
            }
            host.text.push(13);
            let layout = crate::layout(&doc, &hm_fonts::FontSystem::new());
            assert_eq!(placements(&layout, 2)[0].2, 1000 + 3000 + room);
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
            // The filler's spacing after, past the page end, takes no room
            // there (curriculum map p64: Hancom keeps the empty paragraph
            // at the foot instead of opening a page for it).
            for (stored_vpos, after) in [Some(0), Some(9000), None]
                .into_iter()
                .flat_map(|v| [(v, 0), (v, 2000)])
            {
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
                        para_shapes: vec![hm_model::ParaShape {
                            space_after: hm_model::units::pack_distance(after),
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
                let foot = if after > 0 { 10000 } else { 9000 };
                let (empty_at, heading_at) = match (kind, stored_vpos) {
                    (break_kind::PAGE, Some(0) | None) => ((1, 0, 0), (2, 0, 0)),
                    (break_kind::COLUMN, Some(0) | None) => ((0, 10000, 0), (1, 0, 0)),
                    (break_kind::PAGE, _) => ((0, 0, foot), (1, 0, 0)),
                    _ => ((0, 0, foot), (0, 10000, 0)),
                };
                assert_eq!(
                    position(1),
                    empty_at,
                    "break {kind}, vpos {stored_vpos:?}, after {after}"
                );
                assert_eq!(
                    position(2),
                    heading_at,
                    "break {kind}, vpos {stored_vpos:?}, after {after}"
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

    // ktx p1 stores lines of 33355 and 44863 HU: only equal columns round
    // down to 4 HU.
    #[test]
    fn columns_of_differing_widths_keep_their_scaled_widths() {
        let def = ColumnDef {
            attr: 2 << 2,
            columns: vec![(10000, 1000), (21768, 0)],
            ..Default::default()
        };
        let cols = column_layout(Some(&def), 10001);
        assert_eq!(cols, vec![(0, 3052), (3357, 6643)]);
        let equal = ColumnDef {
            attr: (2 << 2) | (1 << 12),
            gap: 1000,
            ..Default::default()
        };
        let cols = column_layout(Some(&equal), 10003);
        assert_eq!(cols, vec![(0, 4500), (5501, 4500)]);
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

    // 2025 행정업무운영 편람 p152-p155 (Hancom PDF): in a table with cell
    // spacing 255 split inside rows, each cut row ends 255 + 100 above the
    // page end less the bottom margin, and its rest starts 255 below the
    // repeated title row, so the row's parts add up to its stored height.
    #[test]
    fn cut_rows_keep_the_cell_spacing_on_both_sides_of_the_page_break() {
        let fonts = hm_fonts::FontSystem::new();
        let lines = std::iter::repeat_n("line", 14)
            .collect::<Vec<_>>()
            .join("\n");
        let run = |spacing: i16| {
            let mut doc = keep_document(&[("intro", 0), ("", 0)]);
            let host = &mut doc.sections[0].paras[1];
            host.text = vec![11, 0, 0, 0, 0, 0, 0, 11, 13];
            let mut control = repeated_title_table(0, 0);
            let hm_model::ControlKind::Table(t) = &mut control.kind else {
                unreachable!()
            };
            t.rows = 2;
            t.cells.truncate(2);
            t.cells[1].height = 1000;
            t.cells[1].paras[0].text = lines.encode_utf16().chain([13]).collect();
            t.spacing = spacing;
            host.controls.push(control);
            crate::layout(&doc, &fonts)
        };
        for spacing in [0, 255] {
            let layout = run(spacing);
            let first = &layout.pages[0];
            let ft = first.tables.iter().find(|t| t.para == 1).unwrap();
            let cut = ft.cut.expect("the row is cut inside");
            let body_end = first.body.y + first.body.h;
            // The cut sits the spacing and the bottom margin above the page end.
            assert_eq!(
                ft.y + cut - ft.top,
                body_end - 200 - spacing as i32,
                "{spacing}"
            );
            assert!(ft.y + ft.height() <= body_end - 200 - spacing as i32 - crate::table::CUT_ROOM);
            let next = &layout.pages[1];
            let nt = next.tables.iter().find(|t| t.para == 1).unwrap();
            let carried = nt.carried.as_deref().expect("the row's rest is carried");
            assert_eq!(carried.gap, spacing as i32);
            // The rest: the spacing, the paddings and whole lines.
            assert_eq!((carried.height - carried.gap - 200) % 1000, 0, "{spacing}");
        }
    }

    // 2025 행정업무운영 편람 HWPX p289-p290 (Hancom PDF) and probes mz-1000,
    // mz-700 (Hancom 2022 on windows1): a cell line ending inside the
    // table's outer bottom margin goes to the next page, even when the
    // cell's bottom padding below it would still end on the page.
    #[test]
    fn a_cell_line_ending_in_the_bottom_margin_goes_to_the_next_page() {
        let fonts = hm_fonts::FontSystem::new();
        let lines = std::iter::repeat_n("line", 10)
            .collect::<Vec<_>>()
            .join("\n");
        let run = |bottom: i16| {
            let mut doc = keep_document(&[("intro", 0), ("", 0)]);
            let host = &mut doc.sections[0].paras[1];
            host.text = vec![11, 0, 0, 0, 0, 0, 0, 11, 13];
            let mut control = repeated_title_table(0, 0);
            let hm_model::ControlKind::Table(t) = &mut control.kind else {
                unreachable!()
            };
            t.rows = 2;
            t.cells.truncate(2);
            let cell = &mut t.cells[1];
            cell.height = 1000;
            cell.list_attr |= 1 << 16;
            cell.padding = [0, 0, 850, bottom];
            cell.paras[0].text = lines.encode_utf16().chain([13]).collect();
            host.controls.push(control);
            crate::layout(&doc, &fonts)
        };
        let ends = |bottom: i16| {
            let layout = run(bottom);
            let first = &layout.pages[0];
            let ft = first.tables.iter().find(|t| t.para == 1).unwrap();
            (
                first.body.y + first.body.h,
                ft.y + ft.bottom - ft.top,
                ft.cut,
            )
        };
        // The sixth line ends at 9850, inside the 200 margin: the cell is
        // cut above it, with padding 100 as with padding 300, and the line
        // goes on the next page.
        for padding in [100, 300] {
            let (_, bottom, cut) = ends(padding);
            assert_eq!((bottom, cut.is_some()), (8850 + i32::from(padding), true));
        }
    }

    // issue1949 p14/15 (Hancom 2022 PDF and its stored lines): in a cell cut
    // across pages, two top-and-bottom pictures whose paragraph stays on p14
    // but which would cross its end open the cell's part on p15, their
    // offsets dropped; the text after their paragraph fills p14 right under
    // it and goes on below the pictures on p15.
    #[test]
    fn top_and_bottom_cell_objects_crossing_the_page_end_open_the_next_part() {
        use hm_model::{Control, ControlKind, CtrlId, ObjectCommon, ObjectControl, ObjectHeader};
        let fonts = hm_fonts::FontSystem::new();
        let text = |n: usize| {
            std::iter::repeat_n("line", n)
                .collect::<Vec<_>>()
                .join("\n")
                .encode_utf16()
                .chain([13])
                .collect::<Vec<u16>>()
        };
        let mut doc = keep_document(&[("intro", 0), ("", 0)]);
        let host = &mut doc.sections[0].paras[1];
        host.text = vec![11, 0, 0, 0, 0, 0, 0, 11, 13];
        let mut control = repeated_title_table(0, 0);
        let ControlKind::Table(t) = &mut control.kind else {
            unreachable!()
        };
        t.rows = 1;
        t.cells.truncate(1);
        let cell = &mut t.cells[0];
        cell.list_attr = 0;
        cell.height = 1000;
        let picture = |x_offset: i32, y_offset: i32| Control {
            kind: ControlKind::Object(Box::new(ObjectControl {
                id: CtrlId::SHAPE,
                header: ObjectHeader {
                    common: ObjectCommon {
                        // Paragraph-relative, flowing with the text, top and bottom.
                        attr: (2 << 3) | (1 << 13) | (4 << 15) | (2 << 18) | (1 << 21),
                        x_offset,
                        y_offset,
                        width: 5000,
                        height: 6000,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                parts: Vec::new(),
            })),
            src: None,
        };
        let mut anchor: Vec<u16> = [11, 0, 0, 0, 0, 0, 0, 11, 11, 0, 0, 0, 0, 0, 0, 11].to_vec();
        anchor.extend("anchor".encode_utf16().chain([13]));
        cell.paras = vec![
            Paragraph {
                text: text(12),
                ..Default::default()
            },
            Paragraph {
                text: anchor,
                controls: vec![picture(0, 300), picture(6000, 0)],
                ..Default::default()
            },
            Paragraph {
                text: text(6),
                ..Default::default()
            },
        ];
        host.controls.push(control);
        let layout = crate::layout(&doc, &fonts);
        let part = |page: usize| {
            let pt = layout.pages[page]
                .tables
                .iter()
                .find(|t| t.para == 1)
                .unwrap();
            let tl = pt.projected.clone().unwrap_or_else(|| {
                std::sync::Arc::new(layout.para(0, 1).table(0).unwrap().clone())
            });
            let first = pt.carried.as_deref().and_then(|c| c.cells[0].1);
            (tl, first)
        };
        let (tl, first) = part(2);
        let cl = &tl.cells[0];
        let top = cl.split_y();
        let line = |para: u32, line: u32| {
            top + cl
                .lines
                .iter()
                .find(|l| l.para == para && l.line == line)
                .unwrap()
                .y
        };
        // Both pictures open p3's part at its top, side by side.
        assert!(
            cl.objects
                .iter()
                .all(|o| o.moved && top + o.y == first.unwrap())
        );
        // p2 shows the anchor's line and the text after it up to the page end.
        let (_, second) = part(1);
        assert!(second.unwrap() < line(1, 0));
        let anchor_end = line(1, 0) + 1000;
        assert_eq!(line(2, 0), anchor_end);
        // p2's part holds 9,400 HU: four lines of the cell's first
        // paragraph, the anchor's line and four lines after it.
        let kept = (0..6).filter(|&k| line(2, k) < first.unwrap()).count();
        assert_eq!(kept, 4);
        // The rest goes on below them on p3.
        assert_eq!(line(2, kept as u32), first.unwrap() + 6000);
        assert_eq!(layout.pages.len(), 3);
    }

    // aift p45, hwpctl-api p27 and issue1853 p25 (Hancom PDFs): the
    // paragraph after a table's last continuation, its host line left on
    // an earlier page, starts at the fragment's foot or where its spacing
    // before reaches, not below both.
    #[test]
    fn continued_tables_own_following_paragraph_clearance() {
        for (captioned, blank, before, after) in (0..16).map(|k| {
            (
                k & 1 != 0,
                k & 2 != 0,
                if k & 4 != 0 { 2000 } else { 500 },
                if k & 8 != 0 { 300 } else { 0 },
            )
        }) {
            let mut doc = keep_document(&[
                ("intro\nintro\nintro", 0),
                ("", 0),
                (if blank { "" } else { "next" }, 0),
                ("", 0),
            ]);
            doc.info.para_shapes[2].space_before = hm_model::units::pack_distance(before);
            doc.info.para_shapes[3].space_before = hm_model::units::pack_distance(500);
            doc.info.para_shapes[1].space_after = hm_model::units::pack_distance(after);
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
                blocked.max(after + before),
                "captioned={captioned}, blank={blank}, before={before}, after={after}"
            );
            let ordinary = page.lines.iter().find(|l| l.para == 3).unwrap();
            assert_eq!(ordinary.vpos, next.vpos + 1500);
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

    /// windows1 batch22 (a table split inside cells that waits for the next
    /// page, 30 paragraphs after it with 1000 HU before) and hwpspec p31,
    /// p35, p40 (stored line segments): the paragraph that follows a table
    /// deferred whole to the next page starts at the table's bottom margin;
    /// its spacing before overlaps the table.
    #[test]
    fn a_paragraph_after_a_deferred_table_starts_at_its_bottom_margin() {
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
        assert_eq!(next.vpos, table.y - page.body.y + table.height() + 200);
    }

    /// Probe j-cell-firstrow-tall (Hancom 2022 on windows1) and issue1891
    /// p50/51: a table split inside cells none of whose rows can start below
    /// its paragraph waits for the next page, as one split between rows
    /// does; its paragraph and the text after it stay on this page.
    #[test]
    fn a_cell_split_table_that_cannot_start_leaves_its_paragraph_and_the_text_after() {
        let mut doc = keep_document(&[
            ("a", 0),
            ("b", 0),
            ("c", 0),
            ("d", 0),
            ("e", 0),
            ("f", 0),
            ("", 0),
            ("after", 0),
        ]);
        let host = &mut doc.sections[0].paras[6];
        host.text = vec![11, 0, 0, 0, 0, 0, 0, 11, 13];
        // Hanging 3000 below its paragraph's top: 1000 HU left on the page.
        host.controls.push(repeated_title_table(3000, 0));
        let layout = crate::layout(&doc, &hm_fonts::FontSystem::new());
        let page_of =
            |para| (layout.pages.iter()).position(|p| p.lines.iter().any(|l| l.para == para));
        assert_eq!((page_of(6), page_of(7)), (Some(0), Some(0)));
        assert!(!layout.pages[0].tables.iter().any(|t| t.para == 6));
        assert!(
            layout.pages[1]
                .tables
                .iter()
                .any(|t| t.para == 6 && t.top == 0)
        );
    }

    /// pr-1674 p3/p4 (Hancom PDF) and windows1 batch21: a table that starts
    /// a page after its anchor's, here the second table of a paragraph whose
    /// first one ran on to the next page, opens its page at the top; its
    /// offset from the paragraph does not carry over.
    #[test]
    fn a_table_starting_a_page_after_its_anchor_drops_its_offset() {
        use hm_model::ControlKind;
        let mut doc = keep_document(&[("a", 0), ("", 0), ("after", 0)]);
        let host = &mut doc.sections[0].paras[1];
        host.text = [11, 0, 0, 0, 0, 0, 0, 11]
            .repeat(2)
            .into_iter()
            .chain([13])
            .collect();
        let mut long = repeated_title_table(0, 0);
        let ControlKind::Table(t) = &mut long.kind else {
            unreachable!()
        };
        let row = t.cells[1].clone();
        t.cells = (0..40)
            .map(|r| hm_model::Cell {
                row: r,
                ..row.clone()
            })
            .collect();
        t.rows = 40;
        host.controls.push(long);
        host.controls.push(repeated_title_table(3000, 0));
        let layout = crate::layout(&doc, &hm_fonts::FontSystem::new());
        let (page, second) = (layout.pages.iter())
            .find_map(|p| {
                p.tables
                    .iter()
                    .find(|t| t.para == 1 && t.control == 1 && t.top == 0)
                    .map(|t| (p, t))
            })
            .unwrap();
        assert_eq!(second.y, page.body.y);
    }

    /// 2025 행정업무운영 편람 p375 (Hancom PDF): a table in front of the text,
    /// kept inside the body, hanging 2040 HU above the page's first paragraph
    /// opens at the body top instead of above it.
    #[test]
    fn a_table_kept_in_the_body_stays_below_its_top() {
        use hm_model::ControlKind;
        // The host is the second paragraph, 1000 HU down the page.
        for (keep, top) in [(true, 0), (false, -1040)] {
            let mut doc = keep_document(&[("a", 0), ("", 0), ("after", 0)]);
            let host = &mut doc.sections[0].paras[1];
            host.text = vec![11, 0, 0, 0, 0, 0, 0, 11, 13];
            let mut c = repeated_title_table(-2040, 0);
            let ControlKind::Table(t) = &mut c.kind else {
                unreachable!()
            };
            let o = &mut t.header.common;
            o.attr = (o.attr & !(7 << 21)) | (3 << 21) | (u32::from(keep) << 13);
            host.controls.push(c);
            let layout = crate::layout(&doc, &hm_fonts::FontSystem::new());
            let page = &layout.pages[0];
            let table = page.tables.iter().find(|t| t.para == 1).unwrap();
            assert_eq!(table.y - page.body.y, top, "kept {keep}");
        }
    }

    /// hwpspec p42/43 and p43/44 (stored line segments) and probes hold0-two,
    /// hold1-two (Hancom 2022 on windows1): two paragraphs in a row whose
    /// tables, split inside cells, cannot start below them both stay, kept
    /// with their anchors (holdAnchorAndSO) or not, and so does the text
    /// after them; the tables open the next page one below the other.
    #[test]
    fn waiting_tables_of_successive_paragraphs_stack_on_the_next_page() {
        for kept in [None, Some(1)] {
            let texts = ["a", "b", "c", "d", "e", "f", "", "", "after"];
            let mut doc = keep_document(&texts.map(|t| (t, 0)));
            for k in [6, 7] {
                let host = &mut doc.sections[0].paras[k];
                host.text = vec![11, 0, 0, 0, 0, 0, 0, 11, 13];
                let mut table = repeated_title_table(3000, 0);
                if let hm_model::ControlKind::Table(t) = &mut table.kind {
                    t.header.prevent_split = kept;
                }
                host.controls.push(table);
            }
            let layout = crate::layout(&doc, &hm_fonts::FontSystem::new());
            let page_of =
                |para| (layout.pages.iter()).position(|p| p.lines.iter().any(|l| l.para == para));
            assert_eq!(
                (page_of(6), page_of(7), page_of(8)),
                (Some(0), Some(0), Some(0)),
                "{kept:?}"
            );
            // The first opens the next page; the second does not start on
            // top of it but below it, here on the page after, as no row of
            // it fits in the 1800 HU left.
            let at = |para| {
                (layout.pages.iter().enumerate())
                    .find_map(|(i, p)| p.tables.iter().find(|t| t.para == para).map(|t| (i, t.y)))
            };
            let (first, second) = (at(6).unwrap(), at(7).unwrap());
            assert_eq!(first.0, 1);
            assert!(second.0 > first.0);
        }
    }

    /// hwpspec p31, p35, p40 (Hancom PDF rules and stored line segments): a
    /// paragraph after a table deferred whole to the next page starts at its
    /// bottom margin, its spacing before overlapping it, and its own table
    /// does not rise above that margin; one hanging further down keeps its
    /// offset from the paragraph's top above the spacing (rowbreak-problem-
    /// pages p12, after a continuation).
    #[test]
    fn tables_of_a_paragraph_overlapping_a_deferred_table_stay_below_it() {
        for offset in [0, 800] {
            let prefix = std::iter::repeat_n("intro", 7)
                .collect::<Vec<_>>()
                .join("\n");
            let mut doc = keep_document(&[(&prefix, 0), ("", 0), ("heading", 0), ("", 0)]);
            doc.info.para_shapes[3].space_before = hm_model::units::pack_distance(500);
            let host = &mut doc.sections[0].paras[1];
            host.text = vec![11, 0, 0, 0, 0, 0, 0, 11, 13];
            let mut control = repeated_title_table(0, 0);
            let hm_model::ControlKind::Table(t) = &mut control.kind else {
                unreachable!()
            };
            t.attr = 0;
            t.rows = 2;
            t.cells.truncate(2);
            host.controls.push(control);
            let next = &mut doc.sections[0].paras[3];
            next.text = vec![11, 0, 0, 0, 0, 0, 0, 11, 13];
            let mut small = repeated_title_table(offset, 0);
            let hm_model::ControlKind::Table(t) = &mut small.kind else {
                unreachable!()
            };
            t.rows = 1;
            t.cells.truncate(1);
            next.controls.push(small);
            let layout = crate::layout(&doc, &hm_fonts::FontSystem::new());
            let page = &layout.pages[1];
            let first = page.tables.iter().find(|t| t.para == 1).unwrap();
            let line = page.lines.iter().find(|l| l.para == 3).unwrap();
            assert_eq!(line.vpos, first.y - page.body.y + first.height() + 200);
            let own = page.tables.iter().find(|t| t.para == 3).unwrap();
            assert_eq!(
                own.y - page.body.y,
                line.vpos.max(line.vpos - 500 + offset),
                "{offset}"
            );
        }
    }

    /// hwpspec p17/18 (stored line segments, Hancom PDF): a table kept on its
    /// anchor's page (holdAnchorAndSO) that cannot start below its paragraph
    /// takes the paragraph with it; one not kept leaves it (probe
    /// j-table-firstrow).
    #[test]
    fn a_table_kept_with_its_anchor_that_cannot_start_takes_its_paragraph() {
        let layout = |kept: i32| {
            let texts = ["a", "b", "c", "d", "e", "f", "", "after"];
            let mut doc = keep_document(&texts.map(|t| (t, 0)));
            let host = &mut doc.sections[0].paras[6];
            host.text = vec![11, 0, 0, 0, 0, 0, 0, 11, 13];
            let mut table = repeated_title_table(0, 0);
            if let hm_model::ControlKind::Table(t) = &mut table.kind {
                // Never split: 8000 HU tall below a paragraph 6000 down.
                t.attr = 0;
                t.header.prevent_split = Some(kept);
            }
            host.controls.push(table);
            crate::layout(&doc, &hm_fonts::FontSystem::new())
        };
        let page_of = |layout: &crate::DocLayout, para| {
            (layout.pages.iter()).position(|p| p.lines.iter().any(|l| l.para == para))
        };
        let (kept, free) = (layout(1), layout(0));
        assert_eq!((page_of(&kept, 6), page_of(&kept, 7)), (Some(1), Some(1)));
        assert_eq!((page_of(&free, 6), page_of(&free, 7)), (Some(0), Some(0)));
    }

    /// 59043 p10/11 (Hancom PDF): a table split inside cells that hangs
    /// above its paragraph's top and cannot start below it takes the
    /// paragraph with it to the next page.
    #[test]
    fn a_hanging_cell_split_table_that_cannot_start_takes_its_paragraph() {
        let texts = ["a", "b", "c", "d", "e", "f", "g", "h", "", "after"];
        let mut doc = keep_document(&texts.map(|t| (t, 0)));
        let host = &mut doc.sections[0].paras[8];
        host.text = vec![11, 0, 0, 0, 0, 0, 0, 11, 13];
        // Its top 500 above the paragraph's, 2500 HU above the page end;
        // the title row is 4000 tall.
        let mut table = repeated_title_table(-500, 0);
        if let hm_model::ControlKind::Table(t) = &mut table.kind {
            t.cells[0].height = 4000;
        }
        host.controls.push(table);
        let layout = crate::layout(&doc, &hm_fonts::FontSystem::new());
        let page_of =
            |para| (layout.pages.iter()).position(|p| p.lines.iter().any(|l| l.para == para));
        assert_eq!(page_of(8), Some(1));
        assert!(
            layout.pages[1]
                .tables
                .iter()
                .any(|t| t.para == 8 && t.top == 0)
        );
    }

    /// issue2019 p1 (Hancom PDF): a stored line narrower than its margins
    /// and set in from them, in a document whose text wraps beside no
    /// object, is a stale cache: Hancom sets the line at its margin. Beside
    /// a square-wrapped object such a line keeps its stored place.
    #[test]
    fn stored_lines_set_in_without_objects_beside_them_take_their_margins() {
        let layout = |square: bool| {
            let mut doc = keep_document(&[("intro", 0), ("short", 0)]);
            let p = &mut doc.sections[0].paras[1];
            p.src = Some(hm_model::SharedBytes::from(vec![1]));
            p.line_segs = vec![hm_model::LineSeg {
                text_start: 0,
                vpos: 1000,
                height: 1000,
                text_height: 1000,
                baseline: 850,
                spacing: 0,
                col_start: 7090,
                width: 8540,
                flags: (1 << 17) | (1 << 18),
            }];
            if square {
                let host = &mut doc.sections[0].paras[0];
                let mut table = repeated_title_table(0, 0);
                if let hm_model::ControlKind::Table(t) = &mut table.kind {
                    // Square wrapping, far below the text.
                    t.header.common.attr &= !(7 << 21);
                    t.header.common.y_offset = 6000;
                    t.header.common.width = 4000;
                }
                host.text.splice(0..0, [11, 0, 0, 0, 0, 0, 0, 11]);
                host.controls.push(table);
            }
            crate::layout(&doc, &hm_fonts::FontSystem::new())
        };
        let seg_x = |layout: &crate::DocLayout| {
            let pl = layout.para(0, 1);
            pl.lines[0].seg_x
        };
        assert_eq!(seg_x(&layout(false)), 0);
        assert_eq!(seg_x(&layout(true)), 7090);
    }

    /// hwpx-02 p1 (Hancom PDF): a first line holding only the section and
    /// column definitions, in a 14 pt run, before the paragraph's end in a
    /// 1 pt run, is 14 pt high; the next paragraph starts below it.
    #[test]
    fn a_line_of_controls_alone_takes_the_size_of_their_run() {
        let mut doc = keep_document(&[("", 0), ("after", 0)]);
        doc.info.char_shapes[0].base_size = 1400;
        doc.info.char_shapes.push(hm_model::CharShape {
            base_size: 100,
            ratios: [100; 7],
            rel_sizes: [100; 7],
            ..Default::default()
        });
        let p = &mut doc.sections[0].paras[0];
        p.text = vec![2, 0, 0, 0, 0, 0, 0, 2, 13];
        p.runs = vec![
            hm_model::CharRun { pos: 0, shape: 0 },
            hm_model::CharRun { pos: 8, shape: 1 },
        ];
        let layout = crate::layout(&doc, &hm_fonts::FontSystem::new());
        let next = layout.pages[0].lines.iter().find(|l| l.para == 1).unwrap();
        assert_eq!(next.vpos, 1400);
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

    /// A one-line intro, then a table split anywhere (`attr` adds to mode 2)
    /// of one-line rows `heights` high in a column beside, when `merged` is
    /// given, a cell that tall spanning them all.
    fn split_rows_document(heights: &[i32], merged: Option<i32>, attr: u32) -> Document {
        split_rows_lines_document(heights, merged, attr, 1)
    }

    /// [`split_rows_document`] with `lines` lines in the merged cell.
    fn split_rows_lines_document(
        heights: &[i32],
        merged: Option<i32>,
        attr: u32,
        lines: usize,
    ) -> Document {
        use hm_model::{Cell, Control, ControlKind, ObjectCommon, ObjectHeader, Table};
        let mut doc = keep_document(&[("intro", 0), ("", 0)]);
        let line = |text: &str| Paragraph {
            text: text.encode_utf16().chain([13]).collect(),
            ..Default::default()
        };
        let rows = heights.len() as u16;
        let col = u16::from(merged.is_some());
        let mut cells: Vec<Cell> = merged
            .map(|height| Cell {
                row_span: rows,
                col_span: 1,
                width: 9000,
                height,
                paras: vec![line(&vec!["m"; lines].join("\n"))],
                ..Default::default()
            })
            .into_iter()
            .collect();
        cells.extend(heights.iter().enumerate().map(|(row, &height)| Cell {
            row: row as u16,
            col,
            row_span: 1,
            col_span: 1,
            width: 9000,
            height,
            paras: vec![line("x")],
            ..Default::default()
        }));
        let host = &mut doc.sections[0].paras[1];
        host.text = vec![11, 0, 0, 0, 0, 0, 0, 11, 13];
        host.controls.push(Control {
            kind: ControlKind::Table(Box::new(Table {
                header: ObjectHeader {
                    common: ObjectCommon {
                        attr: (2 << 3) | (4 << 15) | (2 << 18) | (1 << 21),
                        width: 9000 * (1 + i32::from(col)),
                        height: heights.iter().sum(),
                        ..Default::default()
                    },
                    ..Default::default()
                },
                attr: 2 | attr,
                rows,
                cols: 1 + col,
                padding: [100; 4],
                cells,
                ..Default::default()
            })),
            src: None,
        });
        doc
    }

    // Probes c-table-giant-flow, c-table-giant-mid and c-none-taller
    // (Hancom 2022 on windows1): a table split only between rows never cuts
    // inside a row taller than the page. One starting the table takes it
    // whole to the next page, running past the page end, while the text
    // after the table stays on the anchor page; a later one starts the next
    // page and the rows below it go on the page after. A table that never
    // splits stays whole even when taller than the page.
    #[test]
    fn rows_taller_than_a_page_are_never_cut_between_rows_only() {
        let fonts = hm_fonts::FontSystem::new();
        let layout = |heights: &[i32], mode: u32| {
            let mut doc = split_rows_document(heights, None, 0);
            let host = &mut doc.sections[0].paras[1];
            if let hm_model::ControlKind::Table(t) = &mut host.controls[0].kind {
                t.attr = mode;
            }
            doc.sections[0].paras.push(Paragraph {
                text: "after\r".encode_utf16().collect(),
                ..Default::default()
            });
            crate::layout(&doc, &fonts)
        };
        let bands = |layout: &crate::DocLayout| -> Vec<(usize, i32, i32)> {
            (layout.pages.iter().enumerate())
                .flat_map(|(i, p)| p.tables.iter().map(move |t| (i, t.top, t.bottom)))
                .collect()
        };
        let giant = layout(&[30000, 2000], 1);
        assert_eq!(bands(&giant), [(1, 0, 30000), (2, 30000, 32000)]);
        assert_eq!(placements(&giant, 2)[0].0, 0);

        let mid = layout(&[2000, 30000, 2000], 1);
        assert_eq!(
            bands(&mid),
            [(0, 0, 2000), (1, 2000, 32000), (2, 32000, 34000)]
        );
        assert_eq!(placements(&mid, 2)[0].0, 2);

        let taller = layout(&[2000; 8], 0);
        assert_eq!(bands(&taller), [(1, 0, 16000)]);
        assert_eq!(placements(&taller, 2)[0].0, 0);
    }

    // Probes j-table-firstrow, j-table-firstrow-long and d-nested-table-cell
    // (Hancom 2022 on windows1): a table split only between rows whose first
    // row does not fit the rest of the anchor page goes whole to the next
    // page, its caption above it, while the text after it fills the anchor
    // page. Split inside cells, the first row is cut on the anchor page.
    #[test]
    fn a_table_whose_first_row_misses_the_anchor_page_leaves_the_text_there() {
        let fonts = hm_fonts::FontSystem::new();
        let layout = |mode: u32, caption: bool| {
            let mut doc = split_rows_document(&[8000, 1200], None, 0);
            doc.sections[0].paras[0].text = "intro\nintro\nintro\r".encode_utf16().collect();
            let host = &mut doc.sections[0].paras[1];
            if let hm_model::ControlKind::Table(t) = &mut host.controls[0].kind {
                t.attr = mode;
                if caption {
                    t.caption = Some(hm_model::Caption {
                        count: 1,
                        attr: Some(2),
                        gap: Some(500),
                        paras: vec![Paragraph {
                            text: "caption\r".encode_utf16().collect(),
                            ..Default::default()
                        }],
                        ..Default::default()
                    });
                }
            }
            doc.sections[0].paras.push(Paragraph {
                text: "after\r".encode_utf16().collect(),
                ..Default::default()
            });
            crate::layout(&doc, &fonts)
        };
        let bands = |layout: &crate::DocLayout| -> Vec<(usize, i32, i32)> {
            (layout.pages.iter().enumerate())
                .flat_map(|(i, p)| p.tables.iter().map(move |t| (i, t.top, t.bottom)))
                .collect()
        };
        let held = layout(1, false);
        assert_eq!(bands(&held), [(1, 0, 9200)]);
        assert_eq!(placements(&held, 2)[0].0, 0);

        let captioned = layout(1, true);
        let page = &captioned.pages[1];
        let cap = captioned
            .para(0, 1)
            .table(0)
            .unwrap()
            .caption
            .clone()
            .unwrap();
        assert_eq!(page.tables[0].y - page.body.y, cap.height + cap.gap);
        assert_eq!(placements(&captioned, 2)[0].0, 0);

        let cut = layout(2, false);
        assert_eq!(bands(&cut)[0].0, 0);
        assert_eq!(placements(&cut, 2)[0].0, 1);
    }

    // Probes f-caption-bottom, f-caption-bottom-3p and cb-m* (Hancom 2022 on
    // windows1): every fragment of a split table but the last keeps room for
    // the bottom caption's gap and height and the outer bottom margin below
    // it, though only the last draws the caption. A first cut Hancom saved
    // as the table's height stands when it leaves the gap but not the
    // caption's height (hwpspec p28).
    #[test]
    fn fragments_before_the_last_keep_room_for_a_bottom_caption() {
        let fonts = hm_fonts::FontSystem::new();
        let layout = |stored: Option<i32>| {
            let mut doc = split_rows_document(&[1500; 14], None, 0);
            let host = &mut doc.sections[0].paras[1];
            if let hm_model::ControlKind::Table(t) = &mut host.controls[0].kind {
                t.header.common.margins = [0, 0, 0, 300];
                if let Some(h) = stored {
                    t.header.common.height = h;
                }
                t.caption = Some(hm_model::Caption {
                    count: 1,
                    attr: Some(3),
                    gap: Some(500),
                    paras: vec![Paragraph {
                        text: "caption\ncaption\r".encode_utf16().collect(),
                        ..Default::default()
                    }],
                    ..Default::default()
                });
            }
            crate::layout(&doc, &fonts)
        };
        let reserved = layout(None);
        let cap = reserved
            .para(0, 1)
            .table(0)
            .unwrap()
            .caption
            .clone()
            .unwrap();
        let frame = 300 + cap.gap + cap.height;
        let fragments: Vec<(usize, &crate::table::PlacedTable)> = (reserved.pages.iter())
            .enumerate()
            .flat_map(|(i, p)| p.tables.iter().map(move |t| (i, t)))
            .collect();
        assert!(fragments.len() >= 3);
        for &(i, t) in &fragments[..fragments.len() - 1] {
            let page = &reserved.pages[i];
            let room = page.body.y + page.body.h - (t.y + t.height());
            assert!(room >= frame && room < frame + 1500, "page {i}: {room}");
            assert!(!t.caption);
        }
        assert!(fragments.last().unwrap().1.caption);

        let first = fragments[0].1.bottom;
        let saved = layout(Some(first + 1500));
        assert_eq!(saved.pages[0].tables[0].bottom, first + 1500);
    }

    // Probes i-fn-long, i-fn-after-cut and i-fn-before-cut (Hancom 2022 on
    // windows1): footnotes referred to in the cells of a split table never
    // move its cuts. They wait for room under its fragments and are set
    // under the last one when the others leave none, also a note referred
    // to in a row cut at a page end and continued on the next page.
    #[test]
    fn footnotes_in_split_table_cells_fill_the_room_under_its_fragments() {
        use hm_model::{Control, ControlKind};
        let fonts = hm_fonts::FontSystem::new();
        let note = || Control {
            kind: ControlKind::Note(Box::new(hm_model::Note {
                number: 1,
                suffix: ')' as u16,
                list: hm_model::ParaList {
                    paras: vec![Paragraph {
                        text: "note\r".encode_utf16().collect(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                ..Default::default()
            })),
            src: None,
        };
        let with_note = |text: &str| -> Vec<u16> {
            (text.encode_utf16())
                .chain([17, 0, 0, 0, 0, 0, 0, 17, 13])
                .collect()
        };
        let layout = |notes: bool| {
            let mut doc = split_rows_document(&[1500; 10], None, 0);
            if let ControlKind::Table(t) = &mut doc.sections[0].paras[1].controls[0].kind {
                let tall = "a\nb\nc\nd\ne\nf\ng\nh";
                t.cells[4].paras[0].text = format!("{tall}\r").encode_utf16().collect();
                if notes {
                    t.cells[0].paras[0].text = with_note("x");
                    t.cells[0].paras[0].controls.push(note());
                    t.cells[4].paras[0].text = with_note(tall);
                    t.cells[4].paras[0].controls.push(note());
                }
            }
            doc.sections[0].paras.push(Paragraph {
                text: "after\r".encode_utf16().collect(),
                ..Default::default()
            });
            crate::layout(&doc, &fonts)
        };
        let bands = |layout: &crate::DocLayout| -> Vec<(usize, i32, i32)> {
            (layout.pages.iter().enumerate())
                .flat_map(|(i, p)| p.tables.iter().map(move |t| (i, t.top, t.bottom)))
                .collect()
        };
        let plain = layout(false);
        let noted = layout(true);
        assert_eq!(bands(&noted), bands(&plain));
        let last = bands(&noted).last().unwrap().0;
        assert!(last > 0);
        let notes: Vec<(usize, i32)> = (noted.pages.iter().enumerate())
            .flat_map(|(i, p)| {
                (p.frames.iter())
                    .filter(|f| f.kind == crate::furniture::FrameKind::Footnotes)
                    .map(move |f| (i, f.rect.y))
            })
            .collect();
        assert_eq!(notes.len(), 2);
        let table = &noted.pages[last].tables[0];
        for (page, y) in notes {
            assert_eq!(page, last);
            assert!(y >= table.y + table.height());
        }
    }

    /// hwp3-sample p7 (Hancom PDF): a top-and-bottom picture anchored in a
    /// paragraph's second line, hung 8400 HU below it, goes with that line
    /// when the line moves on to the next page.
    #[test]
    fn an_object_hung_from_a_line_moves_with_it_to_the_next_page() {
        use hm_model::{
            Control, ControlKind, CtrlId, Matrix, ObjectCommon, ObjectControl, ObjectHeader,
            ObjectPart, Shape, ShapeComponent,
        };
        let texts = ["a", "b", "c", "d", "e", "f", "g", "h", "", "after"];
        let mut doc = keep_document(&texts.map(|t| (t, 0)));
        let host = &mut doc.sections[0].paras[8];
        host.text = ("first\nsecond".encode_utf16())
            .chain([11, 0, 0, 0, 0, 0, 0, 11, 13])
            .collect();
        host.controls.push(Control {
            kind: ControlKind::Object(Box::new(ObjectControl {
                id: CtrlId::SHAPE,
                header: ObjectHeader {
                    common: ObjectCommon {
                        attr: (2 << 3) | (2 << 8) | (1 << 13) | (1 << 21),
                        width: 10000,
                        height: 2000,
                        y_offset: 2000,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                parts: vec![ObjectPart::Shape(Box::new(Shape {
                    component: ShapeComponent {
                        kind: CtrlId::from_chars(b"$rec"),
                        doubled: true,
                        offset: (0, 0),
                        group_level: 0,
                        version: 0,
                        initial_size: (10000, 2000),
                        size: (10000, 2000),
                        flip: 0,
                        rotation: 0,
                        rotation_center: (0, 0),
                        translation: Matrix::IDENTITY,
                        transforms: Vec::new(),
                        style: None,
                        members: None,
                        group_instance: None,
                        tail: Default::default(),
                    },
                    parts: Vec::new(),
                }))],
            })),
            src: None,
        });
        let layout = crate::layout(&doc, &hm_fonts::FontSystem::new());
        let lines = placements(&layout, 8);
        assert_eq!((lines[0].0, lines[1].0), (0, 1));
        let (page, object) = (layout.pages.iter().enumerate())
            .find_map(|(i, p)| p.objects.iter().find(|o| o.para == 8).map(|o| (i, o)))
            .unwrap();
        let body = layout.pages[page].body;
        assert_eq!((page, object.rect.y - body.y), (1, lines[1].2 + 2000));
    }

    // 3067979 p1 (Hancom PDF): a flowing table anchored at the end of a
    // paragraph's second line, its vertical offset below that line, hangs
    // the offset from the line's top, not from the paragraph's.
    #[test]
    fn a_table_anchored_in_a_later_line_hangs_from_that_line() {
        let mut doc = split_rows_document(&[1500; 2], None, 0);
        let host = &mut doc.sections[0].paras[1];
        host.text = ("first\nsecond".encode_utf16())
            .chain([11, 0, 0, 0, 0, 0, 0, 11, 13])
            .collect();
        if let hm_model::ControlKind::Table(t) = &mut host.controls[0].kind {
            t.header.common.y_offset = 1500;
        }
        let layout = crate::layout(&doc, &hm_fonts::FontSystem::new());
        let lines = placements(&layout, 1);
        assert_eq!(lines.len(), 2);
        let page = &layout.pages[0];
        assert_eq!(page.tables[0].y - page.body.y, lines[1].2 + 1500);
    }

    // hwpspec p22/23 (Hancom PDF): a paragraph holding two flowing tables,
    // the second hung so far below the paragraph's top that it would start
    // below the page end, goes to the next page with both tables.
    #[test]
    fn a_later_table_starting_below_the_page_end_moves_its_paragraph() {
        let mut doc = split_rows_document(&[1500; 2], None, 0);
        doc.sections[0].paras[0].text = "intro\nintro\nintro\nintro\r".encode_utf16().collect();
        let host = &mut doc.sections[0].paras[1];
        let mut second = host.controls[0].clone();
        if let hm_model::ControlKind::Table(t) = &mut second.kind {
            t.header.common.y_offset = 6500;
        }
        host.controls.push(second);
        host.text = [11, 0, 0, 0, 0, 0, 0, 11, 11, 0, 0, 0, 0, 0, 0, 11, 13].to_vec();
        let layout = crate::layout(&doc, &hm_fonts::FontSystem::new());
        let tables: Vec<(usize, u32)> = (layout.pages.iter().enumerate())
            .flat_map(|(i, p)| p.tables.iter().map(move |t| (i, t.control)))
            .collect();
        assert_eq!(tables[..2], [(1, 0), (1, 1)]);
        assert_eq!(placements(&layout, 1)[0].0, 1);
    }

    // issue1891 p28-30 and 1790387 p53 (Hancom PDF): a square-wrapped table
    // that leaves no room for text beside it flows like a top-and-bottom
    // one. The text after it in its own paragraph and the next paragraph go
    // below its last fragment, the paragraph right at its bottom.
    #[test]
    fn a_square_table_without_room_beside_it_flows_like_top_and_bottom() {
        let mut doc = split_rows_document(&[1300; 10], None, 0);
        let host = &mut doc.sections[0].paras[1];
        host.text = [11, 0, 0, 0, 0, 0, 0, 11]
            .into_iter()
            .chain("tail\r".encode_utf16())
            .collect();
        if let hm_model::ControlKind::Table(t) = &mut host.controls[0].kind {
            t.header.common.attr &= !(7 << 21);
            t.header.common.width = 19000;
            t.cells.iter_mut().for_each(|c| c.width = 19000);
        }
        doc.sections[0].paras.push(Paragraph {
            text: "after\r".encode_utf16().collect(),
            ..Default::default()
        });
        let layout = crate::layout(&doc, &hm_fonts::FontSystem::new());
        let (page, last) = (layout.pages.iter().enumerate())
            .flat_map(|(i, p)| p.tables.iter().map(move |t| (i, t)))
            .last()
            .unwrap();
        assert!(page > 0 && last.top > 0);
        let end = last.y - layout.pages[page].body.y + last.height();
        let tail = placements(&layout, 1)[0];
        assert_eq!((tail.0, tail.2), (page, end));
        let after = placements(&layout, 2)[0];
        assert_eq!(after.0, page);
        assert!(after.2 > end);
    }

    // Lane L probes batch24-35 (Hancom 2022 on windows1): what follows a
    // content-less host line starts at its lone table's outer bottom, also
    // beside a bookmark. A second object, a field or a table that may overlap
    // others leave the line below the table, except a line Hancom stored at
    // no width.
    #[test]
    fn a_content_less_host_line_takes_room_only_beside_more_than_its_table() {
        use hm_model::{Bookmark, Control, ControlKind, Field, LineSeg, SharedBytes};
        const ALLOW_OVERLAP: u32 = 1 << 14;
        let bookmark = ControlKind::Bookmark(Bookmark::default());
        let field = ControlKind::Field(Box::default());
        let lone = split_rows_document(&[1300], None, 0).sections[0].paras[1].controls[0].clone();
        for (extra, overlap, stored_empty, room) in [
            (None, false, false, false),
            (Some(bookmark), false, false, false),
            (None, true, false, true),
            (Some(field), false, false, true),
            (Some(lone.kind.clone()), false, false, true),
            (Some(lone.kind.clone()), false, true, false),
        ] {
            let mut doc = split_rows_document(&[1300], None, 0);
            let host = &mut doc.sections[0].paras[1];
            if overlap {
                let ControlKind::Table(t) = &mut host.controls[0].kind else {
                    unreachable!()
                };
                t.header.common.attr |= ALLOW_OVERLAP;
            }
            if let Some(kind) = extra.clone() {
                host.text = [11, 0, 0, 0, 0, 0, 0, 11, 11, 0, 0, 0, 0, 0, 0, 11, 13].into();
                host.controls.push(Control { kind, src: None });
            }
            if stored_empty {
                host.line_segs = vec![LineSeg {
                    height: 1000,
                    text_height: 1000,
                    baseline: 850,
                    flags: LineSeg::FIRST_SEGMENT | LineSeg::LAST_SEGMENT,
                    ..Default::default()
                }];
                host.src = Some(SharedBytes::from(vec![1]));
            }
            doc.sections[0].paras.push(Paragraph {
                text: "after\r".encode_utf16().collect(),
                ..Default::default()
            });
            let layout = crate::layout(&doc, &hm_fonts::FontSystem::new());
            let page = &layout.pages[0];
            let bottom = page.tables.iter().map(|t| t.y + t.height()).max().unwrap() - page.body.y;
            let advance = layout.para(0, 1).lines[0].advance();
            let expected = bottom + if room { advance } else { 0 };
            assert_eq!(
                placements(&layout, 2)[0],
                (0, 0, expected),
                "{extra:?} overlap {overlap} stored {stored_empty}"
            );
        }
        // hwpspec p23: its tables hang from the paragraph's top, above the
        // space before that sets the line lower.
        let mut doc = split_rows_document(&[1300], None, 0);
        doc.info.para_shapes[1].space_before = hm_model::units::pack_distance(500);
        let host = &mut doc.sections[0].paras[1];
        host.text = [11, 0, 0, 0, 0, 0, 0, 11, 11, 0, 0, 0, 0, 0, 0, 11, 13].into();
        host.controls.push(lone);
        doc.sections[0].paras.push(Paragraph {
            text: "after\r".encode_utf16().collect(),
            ..Default::default()
        });
        let layout = crate::layout(&doc, &hm_fonts::FontSystem::new());
        let page = &layout.pages[0];
        let bottom = page.tables.iter().map(|t| t.y + t.height()).max().unwrap() - page.body.y;
        let advance = layout.para(0, 1).lines[0].advance();
        assert_eq!(placements(&layout, 2)[0].2, bottom + advance);
    }

    // issue1853 p14 (Hancom PDF): the rest of a cut row whose cell stands
    // shorter than its stored height goes on as what the cut left of the
    // row, however little of it stands empty below the lines.
    #[test]
    fn a_cut_rows_rest_keeps_the_rows_height_below_its_lines() {
        // Six lines of 1000 HU and the paddings are left of the row: 6200 HU,
        // and 800 HU more stand empty in its stored height.
        let mut doc = split_rows_document(&[15800], None, 0);
        let hm_model::ControlKind::Table(t) = &mut doc.sections[0].paras[1].controls[0].kind else {
            unreachable!()
        };
        t.cells[0].paras[0].text = vec!["m"; 14]
            .join("\n")
            .encode_utf16()
            .chain([13])
            .collect();
        let layout = crate::layout(&doc, &hm_fonts::FontSystem::new());
        let (first, rest) = (&layout.pages[0].tables[0], &layout.pages[1].tables[0]);
        assert_eq!(rest.carried.as_ref().unwrap().cells, [(0, Some(8100))]);
        assert_eq!(rest.height(), 15800 - first.bottom);
    }

    // 1342000 p196 (Hancom PDF): a row of a table that keeps no room above
    // the page end, its lines all fitting and its edge 40 HU above the page
    // end, closes at that edge (frame at 53712) instead of the cut room
    // above the page end (53619), and the next page goes on with the next
    // row.
    #[test]
    fn rows_ending_within_the_cut_room_of_a_table_without_bottom_room_go_whole() {
        let fonts = hm_fonts::FontSystem::new();
        let probe = crate::layout(&split_rows_document(&[2000; 6], None, 8), &fonts);
        let end = 10000 - probe.pages[0].tables[0].y;
        let rows = [2000, 2000, 2000, end - 6040, 2000];
        let layout = crate::layout(&split_rows_document(&rows, None, 8), &fonts);
        assert_eq!(layout.pages[0].tables[0].bottom, end - 40);
        assert_eq!(layout.pages[1].tables[0].top, end - 40);
        // A table keeping room above the page end cuts the row there.
        let layout = crate::layout(&split_rows_document(&rows, None, 0), &fonts);
        assert_eq!(layout.pages[0].tables[0].bottom, end - 100);
    }

    // 1342000 p223/p224 (Hancom PDF): a row's rest dropped within the cut
    // room, the next page going on with the next row, is room a merged cell
    // running past it has not drawn: its last row grows by that rest too.
    #[test]
    fn merged_cells_ask_again_for_a_row_rest_dropped_within_the_cut_room() {
        let fonts = hm_fonts::FontSystem::new();
        let probe = crate::layout(&split_rows_document(&[2000; 6], None, 0), &fonts);
        let end = 10000 - probe.pages[0].tables[0].y;
        let rows = [2000, 2000, 2000, end - 6040, 2000];
        let total: i32 = rows.iter().sum();
        let doc = split_rows_document(&rows, Some(total + 1000), 0);
        let layout = crate::layout(&doc, &fonts);
        let (first, next) = (&layout.pages[0].tables[0], &layout.pages[1].tables[0]);
        assert_eq!((first.bottom, next.top), (end - 100, end - 40));
        assert_eq!(next.bottom, total + 1060);
    }

    // 1342000 p48 (Hancom PDF): a top-aligned merged cell's lines straddle
    // the last row edge above the page end and the row below cannot start
    // there. The fragment ends at that edge, the merged lines filling down to
    // the page end (frame at 53580 as Hancom's), instead of at the lowest cut
    // no line straddles, rows above the edge.
    #[test]
    fn a_row_edge_straddled_by_merged_lines_ends_the_fragment() {
        let fonts = hm_fonts::FontSystem::new();
        let probe = crate::layout(&split_rows_document(&[2000; 6], None, 8), &fonts);
        let end = 10000 - probe.pages[0].tables[0].y;
        // Merged lines are 1000 high from 100 down: one straddles the edge
        // 1150 above the page end, another the next row's line, which ends
        // past the page end with its padding, so no cut lies between that
        // edge and the page end.
        assert!(
            (151..1000).contains(&((end - 100) % 1000)),
            "page end {end}"
        );
        let edge = end - 1150;
        let mut rows = vec![edge / 5; 5];
        rows[4] += edge % 5;
        rows.extend([2000, 2000]);
        let doc = split_rows_lines_document(&rows, Some(rows.iter().sum()), 8, 10);
        let layout = crate::layout(&doc, &fonts);
        let (first, next) = (&layout.pages[0].tables[0], &layout.pages[1].tables[0]);
        // The merged cell keeps the line that fits with its padding inside
        // the next row: the fragment ends at the edge, that line in the room
        // below it, and the cut row goes on whole at the next page's top
        // (probes b-cell-rowspan, st-*).
        assert_eq!(first.bottom, edge);
        assert!(first.tail > 0 && edge + first.tail <= end, "{}", first.tail);
        assert_eq!(next.top, edge);
    }

    // 1342000 p83 (Hancom PDF): a merged cell running on past a cut row whose
    // own cells have no lines left: its part ends at the edge of its last
    // row, and it needs 8,066 HU, so the carried part is what the rows below
    // the cut row do not hold of that (Hancom's part: 8,064), not its lines
    // down to the edge plus those rows.
    #[test]
    fn a_merged_cells_rest_takes_what_its_rows_below_the_cut_do_not_hold() {
        let fonts = hm_fonts::FontSystem::new();
        let doc = split_rows_lines_document(&[1000, 9000, 3000], Some(13000), 8, 15);
        let layout = crate::layout(&doc, &fonts);
        let next = &layout.pages[1].tables[0];
        let carried = next.carried.as_deref().expect("the row's rest is carried");
        // Lines of 1000: eight stay above the page end 9000 down the table.
        // The other seven with padding take 7,200; the last row, grown to
        // fit the merged cell, holds 5,000 of that.
        assert_eq!(carried.cells[0].1, Some(8100));
        assert_eq!(carried.height, 2200);
    }

    // 1342000 p202 (Hancom PDF): the next row cannot start and a merged
    // cell's first line going on straddles that row's top edge: the fragment
    // ends at the edge, its frame running on to the margin line, and the
    // line opens the next page beside that row.
    #[test]
    fn a_merged_line_straddling_the_edge_of_a_row_that_cannot_start_opens_the_next_page() {
        let fonts = hm_fonts::FontSystem::new();
        let doc = split_rows_lines_document(&[4000, 4500, 2000], Some(10500), 8, 10);
        let layout = crate::layout(&doc, &fonts);
        let (first, next) = (&layout.pages[0].tables[0], &layout.pages[1].tables[0]);
        assert_eq!((first.bottom, first.tail), (8500, 500));
        let carried = next.carried.as_deref().expect("the merged cell goes on");
        assert!(carried.spanning);
        assert_eq!(carried.cells, [(0, Some(8100))]);
    }

    // 1342000 p129 and p304 (Hancom PDF): in a table whose size is not adjusted,
    // a row edge inside the bottom margin holds only while the row's lines
    // end above the margin line with their padding: its last line goes on
    // otherwise.
    #[test]
    fn a_row_edge_in_the_margin_holds_while_its_lines_end_above_the_margin_line() {
        let fonts = hm_fonts::FontSystem::new();
        let run = |lines: usize| {
            let mut doc = keep_document(&[("intro", 0), ("", 0)]);
            let host = &mut doc.sections[0].paras[1];
            host.text = vec![11, 0, 0, 0, 0, 0, 0, 11, 13];
            let mut control = repeated_title_table(0, 0);
            let hm_model::ControlKind::Table(t) = &mut control.kind else {
                unreachable!()
            };
            t.attr = 2 | 8;
            t.cells[0].list_attr = 0;
            t.cells[0].height = 3700;
            t.cells[1].height = 5200;
            t.cells[1].paras[0].text = vec!["line"; lines]
                .join("\n")
                .encode_utf16()
                .chain([13])
                .collect();
            host.controls.push(control);
            crate::layout(&doc, &fonts)
        };
        // Row 1 ends at 8900, inside the margin from 8800. Five lines end
        // there with their padding, four 1000 HU above it.
        let five = run(5);
        assert_eq!(
            five.pages[1].tables[0]
                .carried
                .as_deref()
                .map(|c| c.cells.clone()),
            Some(vec![(1, Some(7800))])
        );
        let four = run(4);
        assert_eq!(four.pages[0].tables[0].bottom, 8900);
        assert!(four.pages[1].tables[0].carried.is_none());
    }

    // Probes cnm40-na, cnm80-na, cnm120-na and their -adj twins (Hancom 2022
    // on windows1), 1342000 p137 and p361: in a table whose size is not adjusted
    // a cut line stays when it ends above the bottom margin with its padding;
    // in an adjusted one it needs 100 HU more.
    #[test]
    fn lines_of_a_table_whose_size_is_not_adjusted_need_no_cut_room() {
        let fonts = hm_fonts::FontSystem::new();
        let run = |attr: u32| {
            let mut doc = keep_document(&[("intro", 0), ("", 0)]);
            let host = &mut doc.sections[0].paras[1];
            host.text = vec![11, 0, 0, 0, 0, 0, 0, 11, 13];
            let mut control = repeated_title_table(0, 0);
            let hm_model::ControlKind::Table(t) = &mut control.kind else {
                unreachable!()
            };
            t.attr = 2 | attr;
            t.rows = 2;
            t.cells.truncate(2);
            t.cells[0].list_attr = 0;
            t.cells[0].height = 2550;
            t.cells[1].height = 1000;
            t.cells[1].paras[0].text = vec!["line"; 8]
                .join("\n")
                .encode_utf16()
                .chain([13])
                .collect();
            host.controls.push(control);
            crate::layout(&doc, &fonts)
        };
        // Page limit 8800 down the table. Row 1's lines of 1000 start at 2650:
        // the sixth, from 7650, ends at 8650 and with its padding 50 above
        // the limit, less than the cut room.
        let first = |attr: u32| {
            run(attr).pages[1].tables[0]
                .carried
                .as_deref()
                .and_then(|c| c.cells.iter().find(|c| c.0 == 1).and_then(|c| c.1))
        };
        assert_eq!(first(0), Some(7650));
        assert_eq!(first(8), Some(8650));
    }

    // table-scattered-header p14/15 (Hancom PDF): a continued table whose last
    // row ends above the page end, its bottom margin past it, moves that
    // row on: a fragment ends short of the table's end unless the margin fits.
    #[test]
    fn a_table_ends_on_a_page_only_when_its_bottom_margin_fits() {
        let fonts = hm_fonts::FontSystem::new();
        let mut doc = split_rows_lines_document(&[3000, 3000, 2000, 4000, 4750, 1200], None, 0, 1);
        let host = &mut doc.sections[0].paras[1];
        let hm_model::ControlKind::Table(t) = &mut host.controls[0].kind else {
            unreachable!()
        };
        t.header.common.margins = [0, 0, 0, 200];
        let layout = crate::layout(&doc, &fonts);
        // The first fragment ends at row edge 8000; the rest, 9,950 HU, ends
        // 50 above the end of the next page, its margin of 200 past it.
        let fragments: Vec<(i32, i32)> = layout
            .pages
            .iter()
            .flat_map(|p| p.tables.iter().map(|t| (t.top, t.bottom)))
            .collect();
        assert_eq!(fragments, [(0, 8000), (8000, 16750), (16750, 17950)]);
    }

    // 1342000 p137 (Hancom PDF) and probes mb-*, mp-*: a merged cell begun in
    // the cut row, with no stored restart, splits at the page end as the row's
    // own cells do: its lines above the limit stay, instead of the row moving.
    #[test]
    fn a_merged_cell_begun_in_the_cut_row_splits_without_a_stored_restart() {
        let fonts = hm_fonts::FontSystem::new();
        let doc = split_rows_lines_document(&[10000, 3000], Some(13000), 8, 12);
        let layout = crate::layout(&doc, &fonts);
        let next = &layout.pages[1].tables[0];
        let carried = next.carried.as_deref().expect("the merged cell goes on");
        // Limit 9000 down the table: of the merged cell's lines of 1000 from
        // 100, eight stay with their padding.
        assert_eq!(carried.cells[0], (0, Some(8100)));
    }

    /// A one-cell table on the second paragraph, 1000 down the page, whose
    /// cell (top padding `pad`) holds `lines` lines and then a table of two
    /// rows, the first `row` tall with one line, `top` its top margin.
    fn nested_row_document(pad: i16, lines: usize, row: i32, top: i16) -> Document {
        use hm_model::ControlKind;
        let mut doc = keep_document(&[("intro", 0), ("", 0)]);
        let host = &mut doc.sections[0].paras[1];
        host.text = vec![11, 0, 0, 0, 0, 0, 0, 11, 13];
        let mut inner = repeated_title_table(0, 0);
        let ControlKind::Table(t) = &mut inner.kind else {
            unreachable!()
        };
        t.attr = 2;
        t.rows = 2;
        t.cells.truncate(2);
        t.cells[0].list_attr = 0;
        t.cells[0].height = row;
        t.cells[1].height = 1200;
        t.header.common.margins = [0, 0, top, 0];
        t.header.common.height = row + 1200;
        let mut outer = repeated_title_table(0, 0);
        let ControlKind::Table(t) = &mut outer.kind else {
            unreachable!()
        };
        t.attr = 2;
        t.rows = 1;
        t.cells.truncate(1);
        t.header.common.margins = [0, 0, 0, 0];
        let cell = &mut t.cells[0];
        cell.list_attr = 1 << 16;
        cell.padding = [0, 0, pad, 100];
        cell.height = 1000;
        cell.paras = vec![
            Paragraph {
                text: vec!["line"; lines]
                    .join("\n")
                    .encode_utf16()
                    .chain([13])
                    .collect(),
                ..Default::default()
            },
            Paragraph {
                text: vec![11, 0, 0, 0, 0, 0, 0, 11, 13],
                controls: vec![inner],
                ..Default::default()
            },
            Paragraph {
                text: "after".encode_utf16().chain([13]).collect(),
                ..Default::default()
            },
        ];
        host.controls.push(outer);
        doc
    }

    fn carried_first(doc: &Document) -> Option<i32> {
        let layout = crate::layout(doc, &hm_fonts::FontSystem::new());
        layout.pages[1].tables[0]
            .carried
            .as_deref()
            .and_then(|c| c.cells[0].1)
    }

    // Probes nf1565p100, nf1565p200 (Hancom 2022 on windows1): a nested row's
    // line stays only with the cut room of each nesting level: its own and
    // the outer cell's. Page limit 9000 down the outer table: the line ends
    // at 8600, with 100 of padding and 200 of cut room 100 short of it.
    #[test]
    fn a_nested_line_keeps_the_cut_room_of_each_nesting_level() {
        assert_eq!(
            carried_first(&nested_row_document(500, 7, 3000, 0)),
            Some(7500)
        );
    }

    // Probes nf1565p200-p600: the nested table that moves on goes on below its
    // top margin.
    #[test]
    fn a_nested_table_moved_on_keeps_its_top_margin() {
        assert_eq!(
            carried_first(&nested_row_document(100, 7, 3000, 400)),
            Some(7100)
        );
    }

    // Probes nf2797p1000, nf2797p1400: the rest of a nested row holding nothing
    // below the page end is dropped up to the rest Hancom carries on empty, and
    // the next part opens with the next row. Page limit 9000: the cut part of
    // the row from 7100 ends at 8700.
    #[test]
    fn the_short_empty_rest_of_a_nested_row_is_dropped() {
        // Row edge 9600, a rest of 900 HU.
        assert_eq!(
            carried_first(&nested_row_document(100, 7, 2500, 0)),
            Some(9600)
        );
        // Row edge 10100, a rest of 1,400 HU: it goes on from the cut.
        assert_eq!(
            carried_first(&nested_row_document(100, 7, 3000, 0)),
            Some(8200)
        );
    }

    // 1342000 p49/p50 (Hancom PDF): every cell begun in the cut row spans
    // the row below too, so the two rows split as one: the merged cell's
    // first line stays above the page end with the row's other cells, the
    // others go on at the next page's top, instead of the row moving on
    // whole.
    #[test]
    fn rows_whose_cells_all_span_the_next_row_split_as_one_row() {
        use hm_model::{Cell, ControlKind};
        let fonts = hm_fonts::FontSystem::new();
        let probe = crate::layout(&split_rows_document(&[2000; 6], None, 8), &fonts);
        let end = 10000 - probe.pages[0].tables[0].y;
        // The cut row starts 2000 above the page end: the merged cell's
        // first line stays, its second does not.
        let top = end - 2000;
        let mut doc = split_rows_lines_document(&[top / 2, 2000, 2000], None, 8, 1);
        let ControlKind::Table(table) = &mut doc.sections[0].paras[1].controls[0].kind else {
            unreachable!()
        };
        table.rows = 4;
        table.cols = 2;
        table.header.common.width = 18000;
        let cell = |row, col, row_span, height, text: &str| Cell {
            row,
            col,
            row_span,
            col_span: 1,
            width: 9000,
            height,
            paras: vec![Paragraph {
                text: text.encode_utf16().chain([13]).collect(),
                ..Default::default()
            }],
            ..Default::default()
        };
        // A cell begun above ends at the cut row's foot; only the merged
        // cell begins in the cut row.
        table.cells = vec![
            cell(0, 0, 1, top / 2, "y"),
            cell(1, 0, 1, top - top / 2, "y"),
            cell(2, 0, 2, 4500, "m\nm\nm"),
            cell(0, 1, 1, top / 2, "x"),
            cell(1, 1, 2, top - top / 2 + 2500, "x"),
            cell(3, 1, 1, 2000, "x"),
        ];
        let layout = crate::layout(&doc, &fonts);
        let (first, next) = (&layout.pages[0].tables[0], &layout.pages[1].tables[0]);
        assert!(first.bottom > top, "the cut row starts above the page end");
        let carried = next
            .carried
            .as_ref()
            .expect("the merged cell's lines go on");
        assert!(carried.cells.iter().any(|&(_, line)| line.is_some()));
    }
}
