//! Cutting a table across pages and columns.
//!
//! A fragment shows a horizontal band `[top, bottom)` of the table. Cuts
//! fall on row edges, or, when the table splits anywhere, also inside rows
//! at a height no cell line straddles (a line's bottom, its spacing left
//! out). Splitting between rows does not cut cells that span them.
//! Header rows (rows of header cells at the top) repeat above each
//! continuation when the table asks for it.
//!
//! The legal cuts of a table are found once, in one pass over its cells
//! and lines, so placing a long table costs a search per fragment.

use hm_model::Table;

use super::TableLayout;

/// How a table may break across pages: never, between rows only (split
/// mode 1, HWPX `TABLE`), or inside cells too (mode 2, `CELL`, which
/// Hancom's stored layouts show cutting cells' lines across pages).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitMode {
    Never,
    Rows,
    Anywhere,
}

impl SplitMode {
    pub fn of(t: &Table) -> Self {
        match t.split() {
            1 => SplitMode::Rows,
            2 => SplitMode::Anywhere,
            _ => SplitMode::Never,
        }
    }
}

/// The heights a table may be cut at, ascending, table end excluded.
#[derive(Clone, Debug, Default)]
pub struct Cuts {
    /// Row edges no cell spans.
    rows: Vec<i32>,
    /// Those, row edges no line straddles, and line bottoms no line
    /// straddles.
    anywhere: Vec<i32>,
}

impl Cuts {
    pub fn new(t: &Table, tl: &TableLayout) -> Self {
        let rows = tl.row_y.len().saturating_sub(1);
        // Cells spanning edge `r` (the edge above row `r`), by a running sum.
        let mut crossing = vec![0i32; rows + 1];
        for c in &t.cells {
            let (top, end) = (c.row as usize + 1, (c.end_row() as usize).min(rows));
            if top < end {
                crossing[top] += 1;
                crossing[end] -= 1;
            }
        }
        let lines = Straddled::new(t, tl, false);
        let row_lines = std::sync::OnceLock::new();
        let within = |y: i32| 0 < y && y < tl.height;
        let mut out = Cuts::default();
        let mut open = crossing[0];
        for (r, &y) in tl.row_y.iter().enumerate().take(rows).skip(1) {
            open += crossing[r];
            let clean = open == 0;
            if !within(y) {
                continue;
            }
            if clean {
                out.rows.push(y);
            }
            if clean || !lines.at(y) || !row_lines.get_or_init(|| Straddled::new(t, tl, true)).at(y)
            {
                out.anywhere.push(y);
            }
        }
        for cl in &tl.cells {
            for (_, y) in spans(cl, false) {
                if y <= cl.y + cl.h && within(y) && !lines.at(y) {
                    out.anywhere.push(y);
                }
            }
        }
        out.rows.dedup();
        out.anywhere.sort_unstable();
        out.anywhere.dedup();
        out
    }

    pub(crate) fn of(&self, mode: SplitMode) -> &[i32] {
        match mode {
            SplitMode::Never => &[],
            SplitMode::Rows => &self.rows,
            SplitMode::Anywhere => &self.anywhere,
        }
    }
}

fn blank_para(p: &crate::para::ParaLayout) -> bool {
    p.head.is_none()
        && !p.measured.items.is_empty()
        && p.measured.items.iter().all(|&(_, kind)| {
            matches!(
                kind,
                crate::Kind::ParaEnd | crate::Kind::LineBreak | crate::Kind::Zero
            )
        })
}

fn empty_cell(cl: &super::CellLayout) -> bool {
    cl.paras.iter().all(blank_para)
}

/// The spans of a cell's lines above its trailing blank paragraphs, when it
/// ends with any: like the extra paragraphs of an empty cell, blank ones
/// after the last text of a merged cell may cross a cut through its other
/// rows.
fn spans_above_trailing_blanks(cl: &super::CellLayout) -> Option<Vec<(i32, i32)>> {
    let text = cl.paras.iter().rposition(|p| !blank_para(p))?;
    if text + 1 == cl.paras.len() || !cl.objects.is_empty() || !cl.tables.is_empty() {
        return None;
    }
    let top = cl.split_y();
    Some(
        cl.lines
            .iter()
            .filter(|l| l.para as usize <= text)
            .map(|l| (top + l.y, top + l.y + line_height(cl, l)))
            .collect(),
    )
}

/// What a cell's content occupies in table heights, `(top, bottom)`.
/// Extra empty paragraphs may cross a cut through other rows of the cell.
fn spans(cl: &super::CellLayout, skip_blanks: bool) -> Vec<(i32, i32)> {
    let top = cl.split_y();
    // Vertical text runs the cell's whole height, which moves as one.
    if cl.vertical {
        return vec![(top, cl.y + cl.h - cl.padding[3])];
    }
    let mut out: Vec<(i32, i32)> = cl
        .lines
        .iter()
        .filter(|l| !skip_blanks || l.para == 0)
        .map(|l| (top + l.y, top + l.y + line_height(cl, l)))
        .collect();
    // Square-wrapped held objects move whole. Hancom moves a top-and-bottom
    // one that does not fit to the next page while the text after it fills
    // this one; the cell's lines stand where they are, so a cut may pass
    // through such an object and the text keeps its pages.
    out.extend(
        cl.objects
            .iter()
            .filter(|o| o.square)
            .map(|o| (top + o.y, top + o.y + o.h)),
    );
    // A table splits into the pieces between its legal cuts.
    for t in &cl.tables {
        let mut from = 0;
        for &c in t.cuts.iter().chain([t.height].iter()) {
            if c > from {
                out.push((top + t.y + from, top + t.y + c));
                from = c;
            }
        }
    }
    out
}

/// The spans of carried cell `k` that size its carried part: a merged cell
/// going on past the carried row's end `until` leaves its trailing blank
/// paragraphs beside the rows below.
pub(crate) fn carried_spans(
    tl: &TableLayout,
    k: usize,
    until: i32,
) -> std::borrow::Cow<'_, [(i32, i32)]> {
    let all = tl.cell_spans(k);
    match tl.cells.get(k) {
        Some(cl) if !cl.vertical && cl.y + cl.h > until => match spans_above_trailing_blanks(cl) {
            Some(mut v) => {
                v.sort_unstable();
                std::borrow::Cow::Owned(v)
            }
            None => std::borrow::Cow::Borrowed(all),
        },
        _ => std::borrow::Cow::Borrowed(all),
    }
}

/// [`spans`] sorted by top.
pub(crate) fn sorted_spans(cl: &super::CellLayout) -> Vec<(i32, i32)> {
    let mut v = spans(cl, false);
    v.sort_unstable();
    v
}

fn line_height(cl: &super::CellLayout, l: &super::CellLine) -> i32 {
    cl.paras
        .get(l.para as usize)
        .and_then(|p| p.lines.get(l.line as usize))
        .map_or(0, |ln| ln.height)
}

/// The open spans `(top, bottom)` of the cell lines, each clipped to its
/// cell, merged where they overlap.
struct Straddled(Vec<(i32, i32)>);

impl Straddled {
    fn new(t: &Table, tl: &TableLayout, row_edges: bool) -> Self {
        let mut v: Vec<(i32, i32)> = Vec::new();
        for (k, cl) in tl.cells.iter().enumerate() {
            // Extra blank paragraphs in a merged side cell may cross a
            // grid-row edge. Its full spans still size carried fragments.
            let skip_blanks =
                row_edges && t.cells.get(k).is_some_and(|c| c.row_span > 1) && empty_cell(cl);
            for (top, bottom) in spans(cl, skip_blanks) {
                let (a, b) = (top.max(cl.y), bottom.min(cl.y + cl.h));
                if a < b {
                    v.push((a, b));
                }
            }
        }
        v.sort_unstable();
        let mut merged: Vec<(i32, i32)> = Vec::with_capacity(v.len());
        for (a, b) in v {
            match merged.last_mut() {
                // Open spans that only touch leave the shared end free.
                Some(last) if a < last.1 => last.1 = last.1.max(b),
                _ => merged.push((a, b)),
            }
        }
        Straddled(merged)
    }

    /// True when a cut at `y` would split a line.
    fn at(&self, y: i32) -> bool {
        let i = self.0.partition_point(|&(a, _)| a < y);
        i > 0 && y < self.0[i - 1].1
    }
}

/// Height of the header band repeated on continuation fragments, when
/// repeating is on: the rows from the top holding header cells, merged
/// cells among them taken whole.
pub fn header_height(t: &Table, tl: &TableLayout) -> i32 {
    if !t.repeat_header() {
        return 0;
    }
    let rows = (t.rows as usize).min(tl.row_y.len().saturating_sub(1));
    // The block runs down the rows that start a header cell (Hancom takes
    // a row as a title row when any of its cells is one) and ends at an
    // edge no cell crosses.
    let mut titled = vec![None::<bool>; rows];
    for c in &t.cells {
        if let Some(r) = titled.get_mut(c.row as usize) {
            *r = Some(r.unwrap_or(false) || c.header());
        }
    }
    let plain = titled
        .iter()
        .position(|r| *r == Some(false))
        .unwrap_or(rows);
    let mut ends = vec![0usize; plain];
    for c in t.cells.iter().filter(|c| (c.row as usize) < plain) {
        let e = &mut ends[c.row as usize];
        *e = (*e).max(c.end_row() as usize);
    }
    let mut reach = 0;
    let mut block = 0;
    for (k, &e) in ends.iter().enumerate() {
        reach = reach.max(e);
        if reach <= k + 1 {
            block = k + 1;
        }
    }
    if block == 0 || block >= rows {
        return 0;
    }
    tl.row_y[block]
}

/// The lowest cut (the largest height) in `(from, limit]`, or `None` when
/// none fits.
pub fn cut(t: &Table, tl: &TableLayout, mode: SplitMode, from: i32, limit: i32) -> Option<i32> {
    if limit >= tl.height {
        return Some(tl.height);
    }
    let list = tl.cuts(t).of(mode);
    let i = list.partition_point(|&y| y <= limit);
    (i > 0 && list[i - 1] > from).then(|| list[i - 1])
}

/// Cells cut at a page end whose lines left over go on at the top of the
/// next fragment, each below its own top padding, as Hancom splits the
/// cells of a row across pages.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Carried {
    /// Source ownership in merged cells resumed at a grid edge, without extra height.
    pub spanning: bool,
    /// For each cell cut: its index and the table height of the top of its
    /// first line shown here (`None` when it has nothing left).
    pub cells: Vec<(u32, Option<i32>)>,
    /// The row edge where the cut cells end.
    pub until: i32,
    /// Height the carried part takes in this fragment.
    pub height: i32,
    /// Where the fragment before the cut ends: its lines and bottom
    /// padding, or the row's stored height up to the cut.
    pub end: i32,
    /// Authored row-height minimum still unoccupied by earlier fragments.
    pub minimum: i32,
    /// First excluded content top per cell when this fragment splits again.
    /// Empty for a completed carried fragment.
    pub next: Vec<(u32, Option<i32>)>,
}

/// Source ownership when merged lines fit beyond a logical row edge.
pub(crate) fn row_tail_break(
    t: &Table,
    tl: &TableLayout,
    from: i32,
    edge: i32,
    limit: i32,
) -> Option<Carried> {
    if edge <= from.max(header_height(t, tl))
        || edge >= limit - tl.bottom_room
        || !tl.cells.iter().enumerate().any(|(k, cl)| {
            cl.y == edge + tl.spacing
                && tl
                    .cell_spans(k)
                    .first()
                    .is_some_and(|&(_, b)| !stays(cl, b, limit))
        })
        || tl
            .cells
            .iter()
            .any(|cl| cl.vertical && cl.y < edge && edge <= cl.y + cl.h)
    {
        return None;
    }
    let mut cells = Vec::new();
    let mut extra = false;
    for (k, (c, cl)) in t.cells.iter().zip(&tl.cells).enumerate() {
        if !(cl.y < edge && edge < cl.y + cl.h) {
            continue;
        }
        if c.row_span <= 1 || cl.vertical || !cl.objects.is_empty() || !cl.tables.is_empty() {
            return None;
        }
        let items = tl.cell_spans(k);
        if items
            .iter()
            .any(|&(a, b)| from <= a && a < edge && !stays(cl, b, limit))
        {
            return None;
        }
        let first = items
            .iter()
            .find(|&&(a, b)| a >= from && !stays(cl, b, limit))
            .map(|&(a, _)| a);
        if first.is_some_and(|first| {
            items
                .iter()
                .any(|&(a, b)| a >= first && stays(cl, b, limit))
        }) {
            return None;
        }
        extra |= items
            .iter()
            .any(|&(a, b)| a >= from && b > edge && stays(cl, b, limit));
        cells.push((k as u32, first));
    }
    extra.then_some(Carried {
        spanning: true,
        cells,
        until: edge,
        end: edge,
        ..Carried::default()
    })
}

/// Whether the first carried line restarts an invalid imported vertical
/// cache. Its wire position survives recomposition and identifies the
/// cell fragment boundary whose pagination must be refreshed.
pub(crate) fn reflowed_break(t: &Table, tl: &TableLayout, carry: &Carried) -> bool {
    // A spanning cell's coordinates start in an earlier row. Its cache
    // restart cannot identify the fragment boundary of this simple row.
    if carry
        .cells
        .iter()
        .any(|&(k, _)| t.cells.get(k as usize).is_none_or(|c| c.row_span != 1))
    {
        return false;
    }
    carry.cells.iter().any(|&(k, first)| {
        let Some(first) = first else { return false };
        let (Some(c), Some(cl)) = (t.cells.get(k as usize), tl.cells.get(k as usize)) else {
            return false;
        };
        cl.lines.iter().any(|line| {
            if cl.split_y() + line.y != first {
                return false;
            }
            let (Some(p), Some(pl)) = (
                c.paras.get(line.para as usize),
                cl.paras.get(line.para as usize),
            ) else {
                return false;
            };
            let Some(laid) = pl.lines.get(line.line as usize) else {
                return false;
            };
            !laid.stored
                && p.src.is_some()
                && p.line_segs
                    .windows(2)
                    .any(|w| w[1].text_start == laid.start && w[1].vpos == w[0].vpos)
        })
    })
}

/// Whether a carried cell resumes at a paragraph whose stored lines restart
/// the cell's vertical cache: an unedited paragraph stored above the end of
/// the unedited one before it was set at the top of a new page by Hancom,
/// so the fragment ends before it even when the line would run into the
/// table's bottom margin. Only the line that reaches the page end `limit`
/// here counts: elsewhere Hancom's page break fell on other content.
pub(crate) fn stored_page_restart(
    t: &Table,
    tl: &TableLayout,
    carry: &Carried,
    limit: i32,
) -> bool {
    carry.cells.iter().any(|&(k, first)| {
        let Some(first) = first else { return false };
        let (Some(c), Some(cl)) = (t.cells.get(k as usize), tl.cells.get(k as usize)) else {
            return false;
        };
        cl.lines.iter().any(|line| {
            let pi = line.para as usize;
            let top = cl.split_y() + line.y;
            if line.line != 0
                || pi == 0
                || top - lead(cl, first) != first
                || top >= limit
                || stays(cl, top + line_height(cl, line), limit)
            {
                return false;
            }
            let (Some(p), Some(prev)) = (c.paras.get(pi), c.paras.get(pi - 1)) else {
                return false;
            };
            let (Some(a), Some(b)) = (p.line_segs.first(), prev.line_segs.last()) else {
                return false;
            };
            // The lines after it go on from the new page top, not from
            // below the paragraph before.
            let after = p
                .line_segs
                .get(1)
                .or_else(|| c.paras.get(pi + 1).and_then(|q| q.line_segs.first()));
            p.src.is_some()
                && prev.src.is_some()
                && restarts(b, a)
                && after.is_none_or(|n| n.vpos <= b.vpos + b.height)
        })
    })
}

/// Room Hancom keeps between a cut cell's last line and the page end: a
/// line stays above the cut only when its bottom, the cell's bottom padding
/// included, ends more than this above the page end less the table's
/// bottom margin.
pub const CUT_ROOM: i32 = 100;

/// Whether a line of cell `cl` ending at table height `bottom` stays above a
/// cut at page end `limit` (the table's bottom margin taken off).
pub fn stays(cl: &super::CellLayout, bottom: i32, limit: i32) -> bool {
    bottom + cl.padding[3] + CUT_ROOM < limit
}

/// The cells a cut at page end `limit` (table height, bottom margin taken
/// off) passes through, each with the top of its first line that does not
/// stay. A cell running on past the row edge after the cut goes through the
/// carried part empty (`None`) when nothing of it lies between the cut and
/// that edge. `None` when the cut does not fall inside a row, such a cell
/// holds content there, or a cut cell cannot keep its first remaining line
/// (the row then moves on whole).
pub fn cut_cells(t: &Table, tl: &TableLayout, from: i32, limit: i32) -> Option<Carried> {
    let cap = limit - CUT_ROOM;
    let r = tl.row_y.iter().rposition(|&y| y <= cap)?;
    let (row_top, next) = (tl.row_y[r], *tl.row_y.get(r + 1)?);
    if cap <= row_top {
        return None;
    }
    let mut cells = Vec::new();
    let mut kept = row_top;
    let mut stored = row_top;
    for (k, cl) in tl.cells.iter().enumerate() {
        if !(cl.y < cap && cap < cl.y + cl.h) {
            continue;
        }
        let items = tl.cell_spans(k);
        let keeps = |top: i32, bottom: i32| {
            stays(cl, bottom, limit) || tl.first_line_stays(k, from, top, bottom, limit)
        };
        if cl.y + cl.h > next {
            let relaxed;
            let blockers: &[(i32, i32)] = if empty_cell(cl) {
                relaxed = spans(cl, true);
                &relaxed
            } else if !cl.vertical
                && let Some(text) = spans_above_trailing_blanks(cl)
            {
                relaxed = text;
                &relaxed
            } else {
                items
            };
            if let Some(&(first, _)) = blockers
                .iter()
                .find(|&&(top, b)| !keeps(top, b) && top < next)
            {
                let source = &t.cells[k];
                let prefix = t.split() == 2
                    && source.row as usize == r
                    && source.row_span > 1
                    && cl.y >= from
                    && !cl.vertical
                    && cl.tables.is_empty()
                    && cl.objects.is_empty()
                    && items.first().is_some_and(|&(top, b)| keeps(top, b))
                    && blockers.iter().all(|&(_, b)| b + cl.padding[3] <= next)
                    && source_restart(source, cl, first);
                if !prefix {
                    return None;
                }
                for &(top, bottom) in items.iter().take_while(|&&(top, _)| top < first) {
                    if !keeps(top, bottom) {
                        return None;
                    }
                    kept = kept.max(bottom + cl.padding[3]);
                }
                cells.push((k as u32, Some(first - lead(cl, first))));
                continue;
            }
            for &(top, bottom) in items {
                if !stays(cl, bottom, limit) && tl.first_line_stays(k, from, top, bottom, limit) {
                    kept = kept.max(bottom + cl.padding[3]);
                }
            }
            cells.push((k as u32, None));
            continue;
        }
        if let Some(c) = t.cells.get(k).filter(|c| c.row as usize == r) {
            stored = stored.max(row_top + c.height);
        }
        let mut first = None::<i32>;
        let mut any = false;
        for &(t, b) in items.iter().filter(|&&(t, _)| t >= cl.y.max(from)) {
            if keeps(t, b) {
                any = true;
                kept = kept.max(b + cl.padding[3]);
            } else if first.is_none() {
                // A cell that cannot start here moves its row on whole.
                if !any {
                    return None;
                }
                first = Some(t - lead(cl, t));
            }
        }
        cells.push((k as u32, first));
    }
    let end = kept.max(stored.min(cap));
    Some(Carried {
        spanning: false,
        cells,
        until: next,
        height: 0,
        end,
        minimum: (stored - end).max(0),
        next: Vec::new(),
    })
}

/// Whether stored line `next` restarts the vertical cache after `prev`:
/// it stands above it, or at the same height in the same segment (the top
/// of the next page), where segments beside one another differ.
fn restarts(prev: &hm_model::LineSeg, next: &hm_model::LineSeg) -> bool {
    next.vpos < prev.vpos
        || next.vpos == prev.vpos && next.col_start == prev.col_start && next.width == prev.width
}

/// An unchanged paragraph's wire boundary restarts on the next physical page.
/// Horizontal line membership may still be replayed; its vertical cache is local
/// to that page, so the split must retain explicit ownership of the rest.
fn source_restart(c: &hm_model::Cell, cl: &super::CellLayout, top: i32) -> bool {
    cl.lines.iter().any(|line| {
        if line.line == 0 || cl.split_y() + line.y != top {
            return false;
        }
        let (Some(p), Some(pl)) = (
            c.paras.get(line.para as usize),
            cl.paras.get(line.para as usize),
        ) else {
            return false;
        };
        let Some(laid) = pl.lines.get(line.line as usize) else {
            return false;
        };
        let Some(prev) = pl.lines.get(line.line as usize - 1) else {
            return false;
        };
        p.src.is_some()
            && p.line_segs.windows(2).any(|w| {
                w[0].text_start == prev.start
                    && w[1].text_start == laid.start
                    && prev.end == laid.start
                    && restarts(&w[0], &w[1])
            })
    })
}

/// Height the rest of carried cell `cl` (content `spans`, sorted) takes
/// from line top `first`: its paddings and the lines from there on.
pub fn carried_height(cl: &super::CellLayout, spans: &[(i32, i32)], first: Option<i32>) -> i32 {
    let pads = cl.padding[2] + cl.padding[3];
    let Some(first) = first else {
        return pads;
    };
    let from = spans.partition_point(|&(t, _)| t < first);
    let last = spans[from..].iter().map(|&(_, b)| b).max().unwrap_or(first);
    let last = if cl.keep_end_spacing {
        last.max(cl.split_y() + cl.content)
    } else {
        last
    };
    pads + last - first
}

/// The top of the first line of carried cell `cl` (shown from line top
/// `first`) that does not stay in `room` by the cut rule, or `None` when
/// the rest does.
/// With `force` at least one line goes in, so a line taller than any page
/// still moves on.
pub fn carried_break(
    cl: &super::CellLayout,
    spans: &[(i32, i32)],
    first: Option<i32>,
    room: i32,
    force: bool,
) -> Option<i32> {
    let first = first?;
    let from = spans.partition_point(|&(t, _)| t < first);
    for (k, &(t, b)) in spans[from..].iter().enumerate() {
        // What starts with the first item goes with it.
        let may_stop = !force || k > 0;
        if may_stop && t > first && cl.padding[2] + b - first + cl.padding[3] + CUT_ROOM >= room {
            return Some(t - lead(cl, t));
        }
    }
    None
}

/// The space above a cell line at table height `top` that goes with it to
/// the top of the next fragment: its paragraph's space before, when it is
/// the paragraph's first line. Hancom's stored lines show a continued cell
/// starting that far down.
fn lead(cl: &super::CellLayout, top: i32) -> i32 {
    let base = cl.split_y();
    cl.lines
        .iter()
        .find(|l| l.line == 0 && base + l.y == top)
        .and_then(|l| cl.paras.get(l.para as usize))
        .map_or(0, |p| p.before.max(0))
}

/// Where the fragment after a cut at table height `cut` begins: at the
/// first line below the cut in the cells crossing it, less that cell's top
/// padding, so the line keeps its padding at the top of the next page;
/// never above the cut.
pub fn resume_after(tl: &TableLayout, cut: i32) -> i32 {
    // A cut on a row edge goes on with the next row as it stands.
    let Some(&next) = tl.row_y.iter().find(|&&y| y > cut) else {
        return cut;
    };
    if tl.row_y.contains(&cut) {
        return cut;
    }
    let mut resume = None::<i32>;
    for (k, cl) in tl
        .cells
        .iter()
        .enumerate()
        .filter(|(_, cl)| cl.y < cut && cut < cl.y + cl.h)
    {
        let first = tl
            .cell_spans(k)
            .iter()
            .map(|&(t, _)| t)
            .filter(|&y| y >= cut)
            .min();
        if let Some(y) = first {
            let at = (y - cl.padding[2]).max(cut);
            resume = Some(resume.map_or(at, |r: i32| r.min(at)));
        }
    }
    resume.unwrap_or(cut).min(next)
}

/// The first cut below `from`, or the table end.
pub fn first_cut(t: &Table, tl: &TableLayout, mode: SplitMode, from: i32) -> i32 {
    let list = tl.cuts(t).of(mode);
    let i = list.partition_point(|&y| y <= from);
    list.get(i).copied().unwrap_or(tl.height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::para::{Line, ParaLayout};
    use crate::table::{CellLayout, CellLine};
    use hm_model::Cell;

    /// A table of `rows` rows of 1000, one cell per row with lines of 300.
    fn table(rows: u16, mode: u32, span_first: bool) -> (Table, TableLayout) {
        let mut t = Table {
            rows,
            cols: 1,
            attr: mode,
            ..Table::default()
        };
        let mut tl = TableLayout {
            row_y: (0..=rows as i32).map(|r| r * 1000).collect(),
            height: rows as i32 * 1000,
            ..TableLayout::default()
        };
        for r in 0..rows {
            let span = if span_first && r == 0 { 2 } else { 1 };
            if span_first && r == 1 {
                continue;
            }
            t.cells.push(Cell {
                row: r,
                row_span: span,
                col_span: 1,
                ..Cell::default()
            });
            let lines = 3 * span as u32;
            tl.cells.push(CellLayout {
                y: r as i32 * 1000,
                h: span as i32 * 1000,
                paras: vec![ParaLayout {
                    lines: vec![
                        Line {
                            height: 300,
                            ..Line::default()
                        };
                        lines as usize
                    ],
                    ..ParaLayout::default()
                }],
                lines: (0..lines)
                    .map(|k| CellLine {
                        para: 0,
                        line: k,
                        y: k as i32 * 300,
                    })
                    .collect(),
                ..CellLayout::default()
            });
        }
        (t, tl)
    }

    #[test]
    fn row_splits_stop_at_row_edges() {
        let (t, tl) = table(4, 1, false);
        assert_eq!(cut(&t, &tl, SplitMode::of(&t), 0, 2500), Some(2000));
        assert_eq!(cut(&t, &tl, SplitMode::of(&t), 0, 999), None);
        // A spanning cell keeps its rows together.
        let (t, tl) = table(4, 1, true);
        assert_eq!(cut(&t, &tl, SplitMode::of(&t), 0, 1500), None);
    }

    #[test]
    fn surplus_empty_paragraphs_do_not_block_rows_beside_a_spanning_cell() {
        use crate::measure::{Kind, Measured};
        use std::sync::Arc;

        let (mut t, mut tl) = table(4, 2, false);
        t.cols = 2;
        t.cells.push(Cell {
            col: 1,
            row_span: 4,
            col_span: 1,
            ..Cell::default()
        });
        let empty = ParaLayout {
            lines: vec![Line {
                height: 1500,
                end: 1,
                ..Line::default()
            }],
            measured: Arc::new(Measured {
                items: vec![(0, Kind::ParaEnd)],
                ..Measured::default()
            }),
            ..ParaLayout::default()
        };
        let mut first = empty.clone();
        first.lines[0].height = 100;
        tl.cells.push(CellLayout {
            h: 4000,
            paras: vec![first, empty.clone(), empty],
            lines: [0, 600, 2200]
                .into_iter()
                .enumerate()
                .map(|(para, y)| CellLine {
                    para: para as u32,
                    line: 0,
                    y,
                })
                .collect(),
            ..CellLayout::default()
        });
        assert_eq!(cut(&t, &tl, SplitMode::Anywhere, 0, 2050), Some(2000));
        assert!(cut_cells(&t, &tl, 0, 2700).is_some());
        for kind in [Kind::Char('X'), Kind::Object(0)] {
            let mut visible = tl.clone();
            visible.cuts = Default::default();
            Arc::make_mut(&mut visible.cells[4].paras[1].measured).items[0].1 = kind;
            assert_eq!(cut(&t, &visible, SplitMode::Anywhere, 0, 2050), Some(600));
        }
        let mut held = tl.clone();
        held.cuts = Default::default();
        held.cells[4].objects.push(crate::table::CellObject {
            y: 600,
            h: 1500,
            square: true,
            ..Default::default()
        });
        assert_eq!(cut(&t, &held, SplitMode::Anywhere, 0, 2050), Some(600));
        let (mut single, mut sl) = table(1, 2, false);
        single.cols = 2;
        single.cells.push(Cell {
            col: 1,
            row_span: 1,
            col_span: 1,
            ..Cell::default()
        });
        sl.height = 4000;
        sl.row_y = vec![0, 4000];
        sl.cells[0].h = 4000;
        sl.cells.push(tl.cells[4].clone());
        assert_eq!(
            (
                carried_height(&tl.cells[4], tl.cell_spans(4), Some(600)),
                cut(&single, &sl, SplitMode::Anywhere, 0, 2050),
            ),
            (3100, Some(600))
        );
    }

    #[test]
    fn recomposed_cell_fragments_refresh_only_unmerged_row_boundaries() {
        use hm_model::{LineSeg, Paragraph, SharedBytes};
        let (mut t, mut tl) = table(2, 2, false);
        t.cells[0].paras.push(Paragraph {
            src: Some(SharedBytes::from(vec![1])),
            line_segs: vec![
                LineSeg::default(),
                LineSeg {
                    text_start: 1,
                    ..Default::default()
                },
            ],
            ..Default::default()
        });
        tl.cells[0].paras[0].lines[1].start = 1;
        let carry = cut_cells(&t, &tl, 0, 650).unwrap();
        assert_eq!(carry.cells, [(0, Some(300))]);
        assert!(reflowed_break(&t, &tl, &carry));
        tl.cells[0].paras[0].lines[1].stored = true;
        assert!(!reflowed_break(&t, &tl, &carry));
        tl.cells[0].paras[0].lines[1].stored = false;
        t.cells[0].row_span = 2;
        assert!(!reflowed_break(&t, &tl, &carry));
    }

    #[test]
    fn splitting_anywhere_cuts_between_lines() {
        let (t, tl) = table(4, 2, false);
        // Inside the third row the last whole line ends at 2600.
        assert_eq!(cut(&t, &tl, SplitMode::of(&t), 0, 2750), Some(2600));
        // Never splitting only takes the whole table.
        let (t, tl) = table(4, 0, false);
        assert_eq!(cut(&t, &tl, SplitMode::of(&t), 0, 3999), None);
        assert_eq!(cut(&t, &tl, SplitMode::of(&t), 0, 4000), Some(4000));
    }

    #[test]
    fn cut_cells_keep_lines_clear_of_the_page_end() {
        // Column 0: a centered one-line cell over three rows. Column 1: one
        // line in row 0, three lines in row 1, one line in row 2.
        let cell = |y, h, lines: i32, offset| CellLayout {
            y,
            h,
            offset,
            content: lines * 300,
            paras: vec![ParaLayout {
                lines: vec![
                    Line {
                        height: 300,
                        ..Line::default()
                    };
                    lines as usize
                ],
                ..ParaLayout::default()
            }],
            lines: (0..lines)
                .map(|k| CellLine {
                    para: 0,
                    line: k as u32,
                    y: k * 300,
                })
                .collect(),
            ..CellLayout::default()
        };
        let model = |row, row_span, col| Cell {
            row,
            row_span,
            col,
            col_span: 1,
            height: 1000 * row_span as i32,
            ..Cell::default()
        };
        let t = Table {
            rows: 3,
            cols: 2,
            attr: 2,
            cells: vec![
                model(0, 3, 0),
                model(0, 1, 1),
                model(1, 1, 1),
                model(2, 1, 1),
            ],
            ..Table::default()
        };
        let tl = TableLayout {
            row_y: vec![0, 1000, 2000, 3000],
            height: 3000,
            bottom_room: CUT_ROOM + 1,
            cells: vec![
                cell(0, 3000, 1, 1400),
                cell(0, 1000, 1, 0),
                cell(1000, 1000, 3, 0),
                cell(2000, 1000, 1, 0),
            ],
            ..TableLayout::default()
        };
        // A line stays when it ends more than CUT_ROOM above the page end;
        // the spanning cell's line counts before its alignment and it runs
        // through the carried part empty.
        let carried = cut_cells(&t, &tl, 0, 1750).unwrap();
        assert_eq!(carried.cells, [(0, None), (2, Some(1600))]);
        assert_eq!(carried.until, 2000);
        // The fragment ends at the cut where the row's stored height runs on.
        assert_eq!(carried.end, 1650);
        // A cut cell that cannot keep its first line moves its row whole.
        assert!(cut_cells(&t, &tl, 0, 1350).is_none());
    }

    #[test]
    fn tables_in_cells_split_between_their_own_cuts() {
        // One row holding only a table of three 300-high pieces.
        let (t, mut tl) = table(1, 2, false);
        let nested = crate::table::CellTable {
            height: 900,
            cuts: vec![300, 600].into(),
            ..Default::default()
        };
        tl.cells[0].lines.clear();
        tl.cells[0].tables = vec![nested.clone()];
        assert_eq!(cut(&t, &tl, SplitMode::of(&t), 0, 700), Some(600));
        // The part drawn in a band is whole pieces only.
        assert_eq!(nested.band(250, 700), Some((300, 600)));
        assert_eq!(nested.band(0, 299), None);
    }
}
