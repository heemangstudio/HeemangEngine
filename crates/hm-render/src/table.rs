//! Drawing tables: cell fills, cell text, then cell borders and diagonals.
//!
//! Cell sides are centered on their edges; identical shared sides are
//! painted once. Fills go first so no cell covers a neighbor's border.
//! A fragment of a split table shows a
//! band of the table: cells crossing the band's edges are closed there and
//! only the lines inside the band are drawn.

use hm_layout::objects::{Anchor, place};
use hm_layout::{CellLayout, Placed, PlacedTable, TableLayout};
use hm_model::{BorderFill, Cell, CellZone, Table};

use crate::objects::paint_placement;
use crate::paint::{self, Sides};
use crate::text::{Ctx, draw_line};
use crate::{DisplayList, clipped};

/// A band `[top, bottom)` of a table, in table coordinates.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Window<'a> {
    pub top: i32,
    pub bottom: i32,
    /// Physical room below the logical band, owned by its closing boxes.
    pub tail: i32,
    pub tail_lines: bool,
    /// Height of the carried part drawn right above the band, which the
    /// cells running through it into the band cover.
    pub lead: i32,
    /// The page end of a cut through the cells at the band's bottom: their
    /// lines show only when they stay by the cut rule.
    pub cut: Option<i32>,
    /// Lines cut there go on in the next fragment, also of cells whose box
    /// ends at the band's bottom.
    pub carries: bool,
    /// Source ownership at this row edge or in the carried part above it.
    pub spanning: Option<&'a hm_layout::Carried>,
    /// The band resumes a table nested in a cell at one of its cuts, the
    /// room that cut keeps above the rest reserved above the band.
    pub resumed: bool,
    /// The band is a part of a table nested in a cell that ends at a page
    /// end, its frame running `tail` below the band.
    pub part_end: bool,
}

impl Window<'_> {
    fn box_height(&self, top: i32, bottom: i32) -> i32 {
        bottom - top + if bottom == self.bottom { self.tail } else { 0 }
    }

    fn ownership(&self, cell: usize) -> Option<Option<i32>> {
        let carried = self.spanning?;
        let first = carried.cells.iter().find(|c| c.0 as usize == cell)?.1;
        (carried.spanning || first.is_some()).then_some(first)
    }

    fn first(&self, cell: usize) -> Option<i32> {
        self.ownership(cell).flatten()
    }

    /// A merged cell can own source lines in the carried part above the band.
    /// Its box/fill belongs to the band; those lines paint afterward in that box.
    fn prefix_height(&self, cell: usize) -> i32 {
        if self.spanning.is_some_and(|c| !c.spanning) && self.first(cell).is_some() {
            self.lead
        } else {
            0
        }
    }

    fn shows(&self, tl: &TableLayout, cell: usize, cl: &CellLayout, top: i32, bottom: i32) -> bool {
        if self.ownership(cell) == Some(None) {
            return false;
        }
        let (top, bottom) = if let Some(first) = self.first(cell) {
            if top < first {
                return false;
            }
            let base = self.top - self.prefix_height(cell) + cl.padding[2] - first;
            (base + top, base + bottom)
        } else {
            (top, bottom)
        };
        top >= self.top - self.prefix_height(cell)
            && (bottom <= self.lines_bottom(cl)
                || (self.tail_lines
                    && self.tail > 0
                    && cl.y < self.bottom
                    && self.bottom < cl.y + cl.h
                    && self
                        .cut
                        .is_some_and(|cut| bottom + cl.padding[3] + cl.cut_room() < cut))
                || (bottom <= self.bottom
                    && self.first(cell).is_none()
                    && self
                        .cut
                        .is_some_and(|cut| tl.first_line_stays(cell, self.top, top, bottom, cut))))
    }

    /// The lowest line bottom cell `cl` shows in the band.
    fn lines_bottom(&self, cl: &CellLayout) -> i32 {
        match self.cut {
            Some(cut)
                if cl.y + cl.h > self.bottom || (self.carries && cl.y + cl.h == self.bottom) =>
            {
                self.bottom.min(cut - cl.cut_room() - cl.padding[3] - 1)
            }
            _ => self.bottom,
        }
    }
}

/// Clip geometry with the same enclosing rectangles as the drawn table.
fn record_cell(dl: &DisplayList, ctx: &Ctx<'_>, cell: &Cell, (x, y, w, h): (i32, i32, i32, i32)) {
    let Some(sink) = ctx.cells else { return };
    let mut rect = (x, y, x + w, y + h);
    let mut closed = 0;
    for item in dl.items.iter().rev() {
        match item {
            crate::Item::Unclip => closed += 1,
            crate::Item::Clip { x, y, w, h } => {
                if closed > 0 {
                    closed -= 1;
                } else {
                    rect = (
                        rect.0.max(*x),
                        rect.1.max(*y),
                        rect.2.min(x + w),
                        rect.3.min(y + h),
                    );
                }
            }
            _ => {}
        }
    }
    if rect.2 > rect.0 && rect.3 > rect.1 {
        sink.borrow_mut().push(crate::CellBox {
            cell: cell as *const Cell as usize,
            x: rect.0,
            y: rect.1,
            width: rect.2 - rect.0,
            height: rect.3 - rect.1,
        });
    }
}

fn border_fill<'a>(ctx: &Ctx<'a>, id: u16) -> Option<&'a BorderFill> {
    ctx.doc.info.border_fills.get((id as usize).checked_sub(1)?)
}

/// Draws the band `window` (the whole table when `None`) of table `t`, laid
/// out as `tl`, with the band's top left at page `(x, y)`.
pub(crate) fn draw(
    dl: &mut DisplayList,
    ctx: &Ctx<'_>,
    t: &Table,
    tl: &TableLayout,
    x: i32,
    y: i32,
    window: Option<Window<'_>>,
) {
    let w = window.unwrap_or(Window {
        top: 0,
        bottom: tl.height,
        tail: 0,
        tail_lines: false,
        lead: 0,
        cut: None,
        carries: false,
        spanning: None,
        resumed: false,
        part_end: false,
    });
    // Page y of a table y.
    let py = |ty: i32| y + ty - w.top;
    // A cell's box clipped to the band, when any of it shows.
    let clip = |top: i32, bottom: i32| {
        let (a, b) = (top.max(w.top), bottom.min(w.bottom));
        (b > a).then_some((a, b))
    };
    // A new row keeps its spacing above every cell, including a spanning
    // cell carried across that row edge without an earlier carried part.
    let row_gap = if w.lead == 0 && tl.row_y.binary_search(&w.top).is_ok() {
        tl.spacing
    } else {
        0
    };
    // A cell's box on the page from its band part `(a, b)`, reaching up
    // over the carried part above the band when it runs through it.
    let boxed = |cl: &CellLayout, (a, b): (i32, i32)| {
        let lead = if cl.y < w.top { w.lead } else { 0 };
        let gap = match w.spanning {
            _ if cl.y >= w.top => 0,
            Some(c) if w.lead > 0 => c.gap,
            _ => row_gap,
        };
        (py(a) - lead + gap, w.box_height(a, b) + lead - gap)
    };
    // The table's own fill lies under its cells, and so do the fills of
    // cell zones: a cell's own fill covers the zone's, as in Hancom.
    if let Some(f) = border_fill(ctx, t.border_fill).and_then(|b| b.fill.as_ref()) {
        paint::fill_rect(dl, x, py(w.top), tl.width, w.box_height(w.top, w.bottom), f);
    }
    zones(dl, ctx, t, tl, x, w, &py, true);
    for (c, cl) in t.cells.iter().zip(&tl.cells) {
        let Some((a, b)) = clip(cl.y, cl.y + cl.h) else {
            continue;
        };
        let (top, h) = boxed(cl, (a, b));
        record_cell(dl, ctx, c, (x + cl.x, top, cl.w, h));
        if let Some(f) = border_fill(ctx, c.border_fill).and_then(|b| b.fill.as_ref()) {
            paint::fill_rect(dl, x + cl.x, top, cl.w, h, f);
        }
    }
    let free = t
        .header
        .common
        .treat_as_char()
        .then_some(ctx.body_anchor)
        .flatten();
    if let Some(anchor) = free {
        for (c, cl) in t.cells.iter().zip(&tl.cells) {
            for (pi, (p, pal)) in c.paras.iter().zip(&cl.paras).enumerate() {
                for placement in hm_layout::objects::place_of(p, pi as u32, &anchor, &|k| {
                    p.controls[k]
                        .object_common()
                        .is_some_and(|o| hm_layout::table::escapes_cell(&o))
                }) {
                    paint_placement(dl, ctx, p, pal, &placement);
                }
            }
        }
    }
    let cell_ctx = Ctx {
        body_anchor: free,
        ..*ctx
    };
    // Objects behind the text go under every cell's lines (passport p1: a
    // white ellipse in the photo cell, its text drawn over it).
    for (k, (c, cl)) in t.cells.iter().zip(&tl.cells).enumerate() {
        let Some((a, b)) = clip(cl.y, cl.y + cl.h) else {
            continue;
        };
        let base = shown_top(tl, k, c, cl, w);
        let top = boxed(cl, (a, b)).0.max(py(a));
        let h = w.box_height(a, b) - (top - py(a));
        clipped(dl, (x + cl.x, top, cl.w, h), |dl| {
            cell_objects(dl, &cell_ctx, c, cl, x, py(base), w.top..w.bottom, true);
        });
    }
    // Hancom clips each cell's content to the cell's box.
    let mut placed: Vec<Placed> = Vec::new();
    for (k, (c, cl)) in t.cells.iter().zip(&tl.cells).enumerate() {
        let Some((a, b)) = clip(cl.y, cl.y + cl.h) else {
            continue;
        };
        let cx = x + cl.content_x();
        let base = shown_top(tl, k, c, cl, w);
        let top = boxed(cl, (a, b)).0.max(py(a) - w.prefix_height(k));
        let h = w.box_height(a, b) - (top - py(a));
        clipped(dl, (x + cl.x, top, cl.w, h), |dl| {
            if cl.vertical {
                vertical_cell(dl, ctx, c, cl, (x + cl.x, py(cl.y)));
                return;
            }
            for l in &cl.lines {
                let (Some(para), Some(pal)) =
                    (c.paras.get(l.para as usize), cl.paras.get(l.para as usize))
                else {
                    continue;
                };
                let flow = cl.split_y() + l.y;
                let h = pal.lines.get(l.line as usize).map_or(0, |ln| ln.height);
                if !w.shows(tl, k, cl, flow, flow + h) {
                    continue;
                }
                draw_line(
                    dl,
                    ctx,
                    para,
                    pal,
                    l.line as usize,
                    cx,
                    py(base + l.y),
                    &mut placed,
                );
            }
        });
    }
    for (k, (c, cl)) in t.cells.iter().zip(&tl.cells).enumerate() {
        let Some((a, b)) = clip(cl.y, cl.y + cl.h) else {
            continue;
        };
        let base = shown_top(tl, k, c, cl, w);
        let top = boxed(cl, (a, b)).0.max(py(a));
        let h = w.box_height(a, b) - (top - py(a));
        // A piece resumed at the band's top under the room its cut keeps
        // repeats the table's header rows in that room, above the band.
        let mut heads = Vec::new();
        clipped(dl, (x + cl.x, top, cl.w, h), |dl| {
            cell_objects(dl, &cell_ctx, c, cl, x, py(base), w.top..w.bottom, false);
            // Tables flowing in the cell's text, the pieces in the band.
            for ct in &cl.tables {
                let flow = cl.split_y() + ct.y;
                let Some(band) = ct.band(w.top - flow, w.lines_bottom(cl) - flow) else {
                    continue;
                };
                let at = py(base + ct.y + band.0);
                let resumed = w.resumed && band.0 > 0 && flow + band.0 == w.top;
                if resumed {
                    heads.push((ct, at, band.0));
                }
                draw_nested(
                    dl,
                    ctx,
                    c,
                    cl,
                    ct,
                    (x + cl.content_x() + ct.x, at),
                    band,
                    resumed,
                );
            }
        });
        for (ct, at, cut) in heads {
            nested_header(dl, ctx, c, cl, ct, (x + cl.content_x() + ct.x, at), cut);
        }
    }
    // Cell diagonals lie under the grid, including the shared column
    // lines a thick center line crosses.
    for (c, cl) in t.cells.iter().zip(&tl.cells) {
        if cl.y >= w.top
            && cl.y + cl.h <= w.bottom
            && cl.h > 0
            && let Some(bf) = border_fill(ctx, c.border_fill)
        {
            paint::diagonals(
                dl,
                x + cl.x,
                py(cl.y),
                cl.w,
                w.box_height(cl.y, cl.y + cl.h),
                bf,
            );
        }
    }
    let meets = column_lines(ctx, t);
    let sides_start = dl.items.len();
    // Every vertical side first, then the horizontal ones over them, as
    // Hancom draws a table's grid.
    for sides in [
        Sides {
            left: true,
            right: true,
            ..Sides::NONE
        },
        Sides {
            top: true,
            bottom: true,
            ..Sides::NONE
        },
    ] {
        for (c, cl) in t.cells.iter().zip(&tl.cells) {
            let Some((a, b)) = clip(cl.y, cl.y + cl.h) else {
                continue;
            };
            if let Some(bf) = border_fill(ctx, c.border_fill) {
                // Row lines reach over the widest column line at each end.
                let at = |col: u32, row: u32| meets.get(&(col, row)).copied().unwrap_or(0);
                let (c0, c1) = (c.col as u32, c.end_col());
                let (r0, r1) = (c.row as u32, c.end_row());
                let meet = [at(c0, r0), at(c1, r0), at(c0, r1), at(c1, r1)];
                let (top, h) = boxed(cl, (a, b));
                paint::border_sides_meeting(dl, x + cl.x, top, cl.w, h, bf, sides, meet);
            }
        }
    }
    // A side two neighbors share in the same style is drawn once, as
    // Hancom draws it: a second stroke over it would darken its edges.
    dedup_lines(dl, sides_start);
    // Hancom then draws the table's top edge once more, from the last cell
    // back to the first, so the left one wins where two meet.
    for (c, cl) in t.cells.iter().zip(&tl.cells).rev() {
        if cl.y != w.top || cl.h <= 0 {
            continue;
        }
        let Some((a, b)) = clip(cl.y, cl.y + cl.h) else {
            continue;
        };
        if let Some(bf) = border_fill(ctx, c.border_fill) {
            let at = |col: u32, row: u32| meets.get(&(col, row)).copied().unwrap_or(0);
            let (c0, c1, r0) = (c.col as u32, c.end_col(), c.row as u32);
            let meet = [at(c0, r0), at(c1, r0), 0, 0];
            let top = Sides {
                top: true,
                ..Sides::NONE
            };
            paint::border_sides_meeting(
                dl,
                x + cl.x,
                py(a),
                cl.w,
                w.box_height(a, b),
                bf,
                top,
                meet,
            );
        }
    }
    zones(dl, ctx, t, tl, x, w, &py, false);
    // A table whose cells stand apart draws its own border around them,
    // closed where the band ends the table.
    if t.spacing > 0
        && let Some(bf) = border_fill(ctx, t.border_fill)
    {
        let sides = Sides {
            left: true,
            right: true,
            top: w.top == 0,
            bottom: w.bottom >= tl.height,
        };
        paint::border_sides(
            dl,
            x,
            py(w.top),
            tl.width,
            w.box_height(w.top, w.bottom),
            bf,
            sides,
        );
    }
}

/// Drops each line among the display list's items from `start` on that
/// repeats an earlier one there exactly.
fn dedup_lines(dl: &mut DisplayList, start: usize) {
    let mut seen = std::collections::HashSet::new();
    let tail = dl.items.split_off(start);
    for item in tail {
        if let crate::Item::Line {
            x1,
            y1,
            x2,
            y2,
            stroke,
        } = item
            && !seen.insert((x1, y1, x2, y2, stroke))
        {
            continue;
        }
        dl.items.push(item);
    }
}

/// Half the width of the widest column line through or ending at each
/// grid point `(column edge, row edge)`.
fn column_lines(ctx: &Ctx<'_>, t: &Table) -> std::collections::HashMap<(u32, u32), i32> {
    let mut out = std::collections::HashMap::new();
    for c in &t.cells {
        let Some(bf) = border_fill(ctx, c.border_fill) else {
            continue;
        };
        for (side, col) in [(&bf.sides[0], c.col as u32), (&bf.sides[1], c.end_col())] {
            if !side.visible() {
                continue;
            }
            let half = side.width_hu() / 2;
            for row in c.row as u32..=c.end_row() {
                let e = out.entry((col, row)).or_insert(0);
                *e = (*e).max(half);
            }
        }
    }
    out
}

/// A cell zone's block as table-relative `(x, y, w, h)`. The block grows
/// to take in whole every merged cell it touches.
fn zone_box(t: &Table, tl: &TableLayout, z: &CellZone) -> Option<(i32, i32, i32, i32)> {
    let (mut c0, mut c1) = (
        z.start_col.min(z.end_col) as usize,
        z.start_col.max(z.end_col) as usize + 1,
    );
    let (mut r0, mut r1) = (
        z.start_row.min(z.end_row) as usize,
        z.start_row.max(z.end_row) as usize + 1,
    );
    if c1 > t.cols as usize || r1 > t.rows as usize {
        return None;
    }
    loop {
        let before = (c0, c1, r0, r1);
        for c in &t.cells {
            let (a, b) = (c.col as usize, c.end_col() as usize);
            let (p, q) = (c.row as usize, c.end_row() as usize);
            if a < c1 && b > c0 && p < r1 && q > r0 {
                (c0, c1, r0, r1) = (c0.min(a), c1.max(b), r0.min(p), r1.max(q));
            }
        }
        if (c0, c1, r0, r1) == before {
            break;
        }
    }
    let (c1, r1) = (c1.min(t.cols as usize), r1.min(t.rows as usize));
    let sp = tl.spacing;
    let (x0, y0) = (tl.col_x[c0] + sp, tl.row_y[r0] + sp);
    Some((x0, y0, tl.col_x[c1] - x0, tl.row_y[r1] - y0))
}

/// Border fills applied to blocks of cells as one cell: the fill spans the
/// block (`fills`), the diagonals and visible sides run along it.
#[allow(clippy::too_many_arguments)]
fn zones(
    dl: &mut DisplayList,
    ctx: &Ctx<'_>,
    t: &Table,
    tl: &TableLayout,
    x: i32,
    w: Window<'_>,
    py: &dyn Fn(i32) -> i32,
    fills: bool,
) {
    for z in t.zones.iter().flatten() {
        let (Some(bf), Some((zx, zy, zw, zh))) =
            (border_fill(ctx, z.border_fill), zone_box(t, tl, z))
        else {
            continue;
        };
        let (a, b) = (zy.max(w.top), (zy + zh).min(w.bottom));
        if b <= a {
            continue;
        }
        if fills {
            if let Some(f) = &bf.fill {
                paint::fill_rect(dl, x + zx, py(a), zw, w.box_height(a, b), f);
            }
        } else {
            if a == zy && b == zy + zh {
                paint::diagonals(dl, x + zx, py(a), zw, w.box_height(a, b), bf);
            }
            paint::border_sides(dl, x + zx, py(a), zw, w.box_height(a, b), bf, Sides::ALL);
        }
    }
}

/// Table height of the top of a cell's content as drawn in band `w`. A
/// cell the band holds whole aligns its content in its box; a cell the band
/// cuts aligns the lines it shows (those whose unaligned place lies in the
/// band) in the part of its box the band holds, as Hancom aligns each part
/// of a cell split across pages.
fn shown_top(tl: &TableLayout, cell: usize, c: &Cell, cl: &CellLayout, w: Window<'_>) -> i32 {
    let restarted = w.first(cell).is_some();
    let started = !restarted
        && w.cut.is_some_and(|cut| {
            tl.cell_spans(cell).first().is_some_and(|&(top, bottom)| {
                bottom <= w.bottom
                    && bottom + cl.padding[3] + hm_layout::CUT_ROOM >= cut
                    && tl.first_line_stays(cell, w.top, top, bottom, cut)
            })
        });
    if !restarted && cl.y >= w.top && cl.y + cl.h == w.bottom {
        // A nested row's rest dropped at its part's end leaves the cell less
        // high than its content: it aligns in the frame down to the part's
        // end (kps-ai p65).
        let short = (cl.padding[2] + cl.content + cl.padding[3] - cl.h).max(0);
        let tail = (w.tail - if w.part_end { short } else { 0 }).max(0);
        let shift = match c.vertical_align() {
            1 => tail / 2,
            2 => tail,
            _ => 0,
        };
        return cl.content_y() + shift;
    }
    // A centered or bottom-aligned cell of lines begun in the band and cut
    // at its bottom aligns what it shows from its content's top in the part
    // of its box on this page, between its top padding and the band's
    // bottom less its bottom padding (Hancom probes g-valign,
    // s-center-minh), also one its content fills (1342000 p68) or runs
    // past (pr-1674 p4, 36388711 p1: the lines that fit centered between
    // them), tables nested further down included (pr-1674 p5).
    // The band ends at the page end's cut room, the cell running on below.
    let at_page_end = cl.y + cl.h > w.bottom
        && (w.part_end
            || w.cut
                .is_some_and(|cut| w.bottom + hm_layout::CUT_ROOM >= cut));
    let cut_aligned = c.vertical_align() != 0
        && at_page_end
        && cl.y >= w.top
        && cl.y + cl.h > w.bottom
        && cl.objects.is_empty();
    // The rest of a nested cell cut above aligns its lines in its own part
    // (probes ncc3000, ncb3000, nc2p).
    let resumed_part = w.resumed
        && c.vertical_align() != 0
        && cl.y < w.top
        && cl.tables.is_empty()
        && cl.objects.is_empty();
    if !restarted
        && !resumed_part
        && ((!started && !cut_aligned && cl.offset == 0)
            || (cl.y >= w.top && cl.y + cl.h <= w.bottom))
    {
        return cl.content_y();
    }
    let flow = cl.split_y();
    let lines = cl.lines.iter().map(|l| {
        let h = cl
            .paras
            .get(l.para as usize)
            .and_then(|p| p.lines.get(l.line as usize))
            .map_or(0, |ln| ln.height);
        (l.y, l.y + h)
    });
    let tables = cl.tables.iter().map(|t| (t.y, t.y + t.height));
    let Some((mut first, mut last)) = lines
        .chain(tables)
        .map(|(a, b)| (flow + a, flow + b))
        .filter(|&(a, b)| w.shows(tl, cell, cl, a, b))
        .reduce(|p, q| (p.0.min(q.0), p.1.max(q.1)))
    else {
        return flow;
    };
    if restarted && cl.keep_end_spacing {
        last = last.max(flow + cl.content);
    }
    // The first part of a cut cell measures from the content's top, the
    // first paragraph's spacing before included.
    if started || (cut_aligned && !restarted) {
        first = flow;
    }
    if resumed_part {
        first -= hm_layout::table::lead(cl, first);
    }
    // A table nested in the cell whose part ends in the band takes room down
    // to that part's end.
    for ct in &cl.tables {
        if let Some(end) = ct.ended.filter(|e| flow + ct.y + e.cut <= w.bottom) {
            last = last.max(flow + ct.y + end.cut + end.reach);
        }
    }
    let prefix = w.prefix_height(cell);
    let top = if prefix > 0 {
        w.top - prefix
    } else if w.resumed {
        cl.y.max(w.top - w.lead)
    } else {
        cl.y.max(w.top)
    } + cl.padding[2];
    let bottom = if prefix > 0 && c.vertical_align() == 0 {
        w.top - cl.padding[3]
    } else if started {
        (cl.y + cl.h).min(w.bottom) - cl.padding[3]
    } else if w.tail > 0 && cl.y + cl.h > w.bottom {
        w.bottom + w.tail - cl.padding[3]
    } else if cl.y + cl.h > w.bottom && w.cut.is_some() {
        // A part cut at the page end keeps the cell's bottom padding above
        // the band's bottom, as Hancom centers it, also where the band ends
        // at its kept lines above the cut room (probe mx-center p2).
        w.bottom - cl.padding[3]
    } else if cl.y + cl.h > w.bottom {
        w.bottom
    } else {
        cl.y + cl.h - cl.padding[3]
    };
    let slack = (bottom - top - (last - first)).max(0);
    let at = match c.vertical_align() {
        0 => top,
        2 => top + slack,
        _ => top + slack / 2,
    };
    flow + at - first
}

/// Draws the band `(top, bottom)` of a table flowing in a cell's text,
/// the band's top left at page `(x, y)`; a whole table keeps its caption.
fn draw_nested(
    dl: &mut DisplayList,
    ctx: &Ctx<'_>,
    c: &Cell,
    cl: &CellLayout,
    ct: &hm_layout::CellTable,
    (x, y): (i32, i32),
    (top, bottom): (i32, i32),
    resumed: bool,
) {
    let nested = c
        .paras
        .get(ct.para as usize)
        .and_then(|p| p.controls.get(ct.control as usize))
        .and_then(|k| k.as_table());
    let layout = cl
        .paras
        .get(ct.para as usize)
        .and_then(|p| p.table(ct.control));
    let (Some(nt), Some(ntl)) = (nested, layout) else {
        return;
    };
    if top == 0 && bottom >= ntl.height {
        draw_with_caption(dl, ctx, nt, ntl, x, y);
    } else {
        // Resumed inside a row, the cut cells' frame starts at the table's
        // top margin, under its repeated header rows, whatever the room kept
        // for their lines' top padding (table-giant-cell-overfill p44; probe
        // ncc0).
        let header = hm_layout::table::header_height(nt, ntl);
        let header = if header > 0 && top >= header {
            header
        } else {
            0
        };
        let lead = if resumed {
            ct.cuts
                .iter()
                .position(|&k| k == top)
                .and_then(|i| ct.above.get(i))
                .map_or(0, |&above| {
                    above - i32::from(nt.header.common.margins[2]).max(0) - header
                })
        } else {
            0
        };
        // Cut at a page end, its frame runs on to the part's end.
        let tail = ct.ended.filter(|e| e.cut == bottom).map_or(0, |e| e.tail);
        let band = Window {
            top,
            bottom,
            tail,
            tail_lines: false,
            lead,
            cut: None,
            carries: false,
            spanning: None,
            resumed,
            part_end: tail > 0,
        };
        draw(dl, ctx, nt, ntl, x, y, Some(band));
    }
}

/// Draws the header rows a table nested in a cell repeats above the piece
/// of it resumed at grid height `cut` at the top of a fragment, the piece's
/// top at page `(x, y)`: they sit below the table's top margin, in the room
/// the cut keeps above the piece.
fn nested_header(
    dl: &mut DisplayList,
    ctx: &Ctx<'_>,
    c: &Cell,
    cl: &CellLayout,
    ct: &hm_layout::CellTable,
    (x, y): (i32, i32),
    cut: i32,
) {
    let nested = c
        .paras
        .get(ct.para as usize)
        .and_then(|p| p.controls.get(ct.control as usize))
        .and_then(|k| k.as_table());
    let layout = cl
        .paras
        .get(ct.para as usize)
        .and_then(|p| p.table(ct.control));
    let (Some(nt), Some(ntl)) = (nested, layout) else {
        return;
    };
    let header = hm_layout::table::header_height(nt, ntl);
    let Some(i) = ct.cuts.iter().position(|&k| k == cut) else {
        return;
    };
    let (Some(&above), true) = (ct.above.get(i), header > 0 && cut >= header) else {
        return;
    };
    let top = y - above + i32::from(nt.header.common.margins[2]).max(0);
    let head = Window {
        top: 0,
        bottom: header,
        tail: 0,
        tail_lines: false,
        lead: 0,
        cut: None,
        carries: false,
        spanning: None,
        resumed: false,
        part_end: false,
    };
    draw(dl, ctx, nt, ntl, x, top, Some(head));
}

/// Draws a cell whose text runs down it, the cell's top left at page
/// `(x, y)`: each line laid out across is turned to run down a column, the
/// columns stacking from the right edge, glyphs upright, and the cell's
/// alignment placing the columns across it.
fn vertical_cell(
    dl: &mut DisplayList,
    ctx: &Ctx<'_>,
    c: &Cell,
    cl: &CellLayout,
    (x, y): (i32, i32),
) {
    let [pl, pr, pt, _] = cl.padding;
    let room = cl.w - pl - pr;
    let offset = match c.vertical_align() {
        1 => (room - cl.content) / 2,
        2 => room - cl.content,
        _ => 0,
    }
    .max(0);
    let (right, top) = (x + cl.w - pr - offset, y + pt);
    let mut flat = DisplayList::default();
    let mut placed: Vec<Placed> = Vec::new();
    for l in &cl.lines {
        let (Some(para), Some(pal)) = (c.paras.get(l.para as usize), cl.paras.get(l.para as usize))
        else {
            continue;
        };
        draw_line(
            &mut flat,
            ctx,
            para,
            pal,
            l.line as usize,
            0,
            l.y,
            &mut placed,
        );
        let Some(line) = pal.lines.get(l.line as usize) else {
            continue;
        };
        let mid = l.y + line.height / 2;
        for item in flat.items.drain(..) {
            match item {
                crate::Item::Glyphs(run) => {
                    // Each glyph stands upright on the column's middle, its em
                    // box starting where it stood along the line.
                    let n = run.glyphs.len();
                    for (i, &(g, u)) in run.glyphs.iter().enumerate() {
                        let advance = match run.glyphs.get(i + 1) {
                            Some(&(_, next)) if i + 1 < n => next - u,
                            _ => run.size * run.ratio / 100,
                        };
                        dl.items.push(crate::Item::Glyphs(crate::GlyphRun {
                            baseline: top + u + run.size * 85 / 100,
                            glyphs: vec![(g, right - mid - advance / 2)],
                            ..run.clone()
                        }));
                    }
                }
                crate::Item::Rect {
                    x: rx,
                    y: ry,
                    w,
                    h,
                    color,
                } => dl.items.push(crate::Item::Rect {
                    x: right - ry - h,
                    y: top + rx,
                    w: h,
                    h: w,
                    color,
                }),
                _ => {}
            }
        }
    }
}

/// Objects anchored in a cell's paragraphs, placed with the cell content
/// as their column. `top` is the page y of the content top; `visible` is
/// the table-height interval drawn. An object that takes room in the cell
/// draws wherever its box intersects that interval; the others draw with
/// their paragraph's first line.
#[allow(clippy::too_many_arguments)]
fn cell_objects(
    dl: &mut DisplayList,
    ctx: &Ctx<'_>,
    c: &Cell,
    cl: &CellLayout,
    x: i32,
    top: i32,
    visible: std::ops::Range<i32>,
    behind: bool,
) {
    let Some(page) = ctx.page else { return };
    for (pi, (para, pal)) in c.paras.iter().zip(&cl.paras).enumerate() {
        if !para.controls.iter().any(|k| k.as_object().is_some()) {
            continue;
        }
        let Some(first) = cl.lines.iter().find(|l| l.para == pi as u32) else {
            continue;
        };
        let line_shown = visible.contains(&(cl.split_y() + first.y));
        let held = |k: u32| {
            cl.objects
                .iter()
                .find(|o| o.para == pi as u32 && o.control == k)
        };
        let left = hm_layout::table::para_left(&ctx.doc.info, para);
        let anchor = Anchor {
            paper: (page.width, page.height),
            paper_x: None,
            paper_y: None,
            flow_top: None,
            body: page.body,
            column: (x + cl.content_x(), pal.column_width),
            // Objects hang from the paragraph's top above its space before.
            para_top: top + first.y - pal.before,
            para_left: x + cl.content_x() + left,
            para_right: pal.right,
        };
        // Objects that take room in the cell are placed as the cell's
        // layout placed them: those kept inside their area stay below the
        // cell's content top, not the page body's.
        let in_cell = Anchor {
            body: hm_layout::Rect {
                x: x + cl.content_x(),
                y: top,
                w: pal.column_width,
                h: 1 << 28,
            },
            flow_top: Some(top),
            ..anchor
        };
        let in_cell = place(para, pi as u32, &in_cell);
        let mut list: Vec<_> = place(para, pi as u32, &anchor)
            .into_iter()
            .filter(|p| {
                ctx.body_anchor.is_none()
                    || !para.controls[p.control as usize]
                        .object_common()
                        .is_some_and(|o| hm_layout::table::escapes_cell(&o))
            })
            .filter_map(|mut p| match held(p.control) {
                Some(o) => {
                    if let Some(q) = in_cell.iter().find(|q| q.control == p.control) {
                        p.rect = q.rect;
                    }
                    p.rect.y += o.shift;
                    let y = cl.split_y() + o.y;
                    (y < visible.end && visible.start < y + o.h).then_some(p)
                }
                // Hancom hangs objects that take no room from the
                // paragraph's place before the cell's vertical alignment.
                None => {
                    let room = cl.h - cl.padding[2] - cl.padding[3];
                    let unaligned = top - cl.offset;
                    let reach = cl.content.max(p.rect.y - cl.offset + p.rect.h - unaligned);
                    let slack = room - reach;
                    let offset = match c.vertical_align() {
                        1 if slack > 0 => slack / 2,
                        2 if slack > 0 => slack,
                        _ => 0,
                    };
                    p.rect.y += offset - cl.offset;
                    // One kept inside its area that its offset lifts does
                    // not rise above the cell's box.
                    let kept = para
                        .controls
                        .get(p.control as usize)
                        .and_then(|c| c.object_common())
                        .is_some_and(|o| o.attr & (1 << 13) != 0 && o.y_offset < 0);
                    if kept {
                        p.rect.y = p.rect.y.max(unaligned - cl.padding[2]);
                    }
                    line_shown.then_some(p)
                }
            })
            .collect();
        list.retain(|p| (p.layer == hm_layout::objects::Layer::Behind) == behind);
        list.sort_by_key(|p| (p.layer, p.z));
        for p in &list {
            paint_placement(dl, ctx, para, pal, p);
        }
    }
}

/// Draws a whole table with its caption, the grid's top left at `(x, y)`.
pub(crate) fn draw_with_caption(
    dl: &mut DisplayList,
    ctx: &Ctx<'_>,
    t: &Table,
    tl: &TableLayout,
    x: i32,
    y: i32,
) {
    crate::record_object(ctx, t as *const Table as usize, (x, y, tl.width, tl.height));
    draw(dl, ctx, t, tl, x, y, None);
    caption(dl, ctx, t, tl, x, y, y + tl.height);
}

/// Draws a table's caption beside its grid (top left at `(x, y)`); a
/// caption below goes under `bottom`.
#[allow(clippy::too_many_arguments)]
fn caption(
    dl: &mut DisplayList,
    ctx: &Ctx<'_>,
    t: &Table,
    tl: &TableLayout,
    x: i32,
    y: i32,
    bottom: i32,
) {
    let (Some(c), Some(cl)) = (t.caption.as_ref(), tl.caption.as_ref()) else {
        return;
    };
    let (cx, cy) = match cl.side {
        0 => (x - cl.gap - cl.width, y),
        1 => (x + tl.width + cl.gap, y),
        2 => (x, y - cl.gap - cl.height),
        _ => (x, bottom + cl.gap),
    };
    let mut placed: Vec<Placed> = Vec::new();
    for l in &cl.lines {
        let (Some(para), Some(pal)) = (c.paras.get(l.para as usize), cl.paras.get(l.para as usize))
        else {
            continue;
        };
        draw_line(
            dl,
            ctx,
            para,
            pal,
            l.line as usize,
            cx,
            cy + l.y,
            &mut placed,
        );
    }
}

/// Draws the cells carried over from a cut through their row, the top of
/// the grid at page `(x, y)`: each cell's box takes the carried height and
/// its lines left over start below its top padding, as many as fit.
fn draw_carried(
    dl: &mut DisplayList,
    ctx: &Ctx<'_>,
    t: &Table,
    tl: &TableLayout,
    (x, y): (i32, i32),
    carried: &hm_layout::Carried,
    banded: bool,
) {
    if carried.spanning {
        return;
    }
    // The cell spacing above the carried cells stays open.
    let (y, h) = (y + carried.gap, carried.height - carried.gap);
    // Cells running on past the carried row are drawn with the band below
    // it, when there is one.
    let cells = || {
        carried.cells.iter().filter_map(|&(k, first)| {
            let cl = tl.cells.get(k as usize)?;
            (!banded || cl.y + cl.h <= carried.until).then_some((
                k,
                t.cells.get(k as usize)?,
                cl,
                first,
            ))
        })
    };
    for (_, c, cl, _) in cells() {
        record_cell(dl, ctx, c, (x + cl.x, y, cl.w, h));
        if let Some(f) = border_fill(ctx, c.border_fill).and_then(|b| b.fill.as_ref()) {
            paint::fill_rect(dl, x + cl.x, y, cl.w, h, f);
        }
    }
    let mut placed: Vec<Placed> = Vec::new();
    for (k, c, cl, first) in cells() {
        let Some(first) = first else { continue };
        let end = carried
            .next
            .iter()
            .find(|&&(cell, _)| cell == k)
            .and_then(|&(_, next)| next)
            .unwrap_or(i32::MAX);
        let cx = x + cl.content_x();
        // A table cut at the top starts again below its top margin: what
        // the part shows is drawn from `start`.
        let start = first - cl.resume_offset(first);
        // The lines shown, as table heights from their top.
        let lines: Vec<(&hm_layout::CellLine, i32, i32)> = {
            let mut shown = 0;
            cl.lines
                .iter()
                .filter_map(|l| {
                    let pal = cl.paras.get(l.para as usize)?;
                    let top = cl.split_y() + l.y;
                    let lh = pal.lines.get(l.line as usize).map_or(0, |ln| ln.height);
                    if top < first
                        || top >= end
                        || (shown > 0 && cl.padding[2] + top + lh - start > h)
                    {
                        return None;
                    }
                    shown += 1;
                    Some((l, top, top + lh))
                })
                .collect()
        };
        // A centered or bottom-aligned cell aligns what it shows in the
        // carried part, as Hancom aligns each part of a cut cell.
        // What the part shows: its lines and the tables and objects among
        // them that fit in it.
        let room = h - cl.padding[2] - cl.padding[3];
        let extent = tl
            .cell_spans(k as usize)
            .iter()
            .filter(|&&(a, b)| a >= first && a < end && b - start <= room)
            .map(|&(_, b)| b - start)
            .chain(lines.iter().map(|&(_, _, b)| b - start))
            .max()
            .unwrap_or(0);
        let slack = (h - cl.padding[2] - cl.padding[3] - extent).max(0);
        let shift = match c.vertical_align() {
            1 => slack / 2,
            2 => slack,
            _ => 0,
        };
        clipped(dl, (x + cl.x, y, cl.w, h), |dl| {
            let room = (h - cl.padding[2]).min(end.saturating_sub(start));
            let top = y + cl.padding[2] + shift + cl.split_y() - start;
            cell_objects(dl, ctx, c, cl, x, top, first..start + room, true);
            for &(l, top, _) in &lines {
                let (Some(para), Some(pal)) =
                    (c.paras.get(l.para as usize), cl.paras.get(l.para as usize))
                else {
                    continue;
                };
                let at = y + cl.padding[2] + shift + top - start;
                draw_line(dl, ctx, para, pal, l.line as usize, cx, at, &mut placed);
            }
            cell_objects(dl, ctx, c, cl, x, top, first..start + room, false);
            for ct in &cl.tables {
                let top = cl.split_y() + ct.y;
                let Some(band) = ct.band(first - top, start + room - top) else {
                    continue;
                };
                let at = y + cl.padding[2] + shift + top + band.0 - start;
                let resumed = band.0 > 0 && top + band.0 == first;
                if resumed {
                    nested_header(dl, ctx, c, cl, ct, (x + cl.content_x() + ct.x, at), band.0);
                }
                draw_nested(
                    dl,
                    ctx,
                    c,
                    cl,
                    ct,
                    (x + cl.content_x() + ct.x, at),
                    band,
                    resumed,
                );
            }
        });
    }
    for (_, c, cl, _) in cells() {
        if let Some(bf) = border_fill(ctx, c.border_fill) {
            paint::border_sides(dl, x + cl.x, y, cl.w, h, bf, Sides::ALL);
        }
    }
}

/// Draws one table fragment placed on a page.
pub(crate) fn draw_fragment(
    dl: &mut DisplayList,
    ctx: &Ctx<'_>,
    layout: &hm_layout::DocLayout,
    section: u32,
    pt: &PlacedTable,
) {
    let Some(control) = ctx
        .doc
        .sections
        .get(section as usize)
        .and_then(|s| s.paras.get(pt.para as usize))
        .and_then(|p| p.controls.get(pt.control as usize))
    else {
        return;
    };
    let Some(t) = control.as_table() else { return };
    let Some(tl) = pt
        .projected
        .as_deref()
        .or_else(|| layout.para(section, pt.para).table(pt.control))
    else {
        return;
    };
    crate::record_object(
        ctx,
        t as *const Table as usize,
        (pt.x, pt.y, tl.width, pt.height()),
    );
    if pt.header > 0 {
        let head = Window {
            top: 0,
            bottom: pt.header,
            tail: 0,
            tail_lines: false,
            lead: 0,
            cut: None,
            carries: false,
            spanning: None,
            resumed: false,
            part_end: false,
        };
        draw(dl, ctx, t, tl, pt.x, pt.y, Some(head));
    }
    let mut y = pt.y + pt.header;
    let mut lead = 0;
    if let Some(carried) = &pt.carried {
        draw_carried(dl, ctx, t, tl, (pt.x, y), carried, pt.bottom > pt.top);
        y += carried.height;
        lead = carried.height;
    }
    if pt.bottom > pt.top {
        let band = Window {
            top: pt.top,
            bottom: pt.bottom,
            tail: pt.tail,
            tail_lines: pt.tail_lines,
            lead,
            cut: pt.cut,
            carries: pt.carries,
            spanning: pt.carried.as_deref(),
            resumed: false,
            part_end: false,
        };
        draw(dl, ctx, t, tl, pt.x, y, Some(band));
    }
    if pt.caption {
        let bottom = pt.y + pt.height();
        caption(dl, ctx, t, tl, pt.x, pt.y, bottom);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hm_fonts::FontSystem;
    use hm_model::{BorderLine, DocMeta, Document, Format};

    fn document() -> Document {
        Document {
            meta: DocMeta {
                format: Format::Hwp5,
                version: Default::default(),
                flags: 0,
            },
            info: Default::default(),
            sections: Vec::new(),
            source: None,
            bins: None,
            diagnostics: Vec::new(),
        }
    }

    // Probes ncc3000, ncb3000, nc2p (Hancom 2022 on windows1): each part of a
    // nested cell cut across pages aligns what it shows in its own frame.
    #[test]
    fn nested_parts_align_what_they_show_in_their_frames() {
        let cl = CellLayout {
            h: 10000,
            padding: [0, 0, 100, 100],
            paras: vec![hm_layout::ParaLayout {
                lines: vec![
                    hm_layout::Line {
                        height: 1000,
                        ..Default::default()
                    };
                    4
                ],
                ..Default::default()
            }],
            lines: (0..4)
                .map(|i| hm_layout::CellLine {
                    para: 0,
                    line: i as u32,
                    y: i * 1600,
                })
                .collect(),
            content: 5800,
            offset: 2000,
            ..Default::default()
        };
        let mut tl = TableLayout::default();
        tl.row_y = vec![0, 10000];
        tl.height = 10000;
        tl.cells = vec![cl.clone()];
        let c = Cell {
            list_attr: 1 << 5,
            ..Default::default()
        };
        let band = Window {
            top: 0,
            bottom: 2700,
            tail: 300,
            tail_lines: false,
            lead: 0,
            cut: None,
            carries: false,
            spanning: None,
            resumed: false,
            part_end: true,
        };
        // The first part's frame ends at 3000: two lines, 2,600 high, centered
        // between the paddings.
        assert_eq!(shown_top(&tl, 0, &c, &cl, band), 200);
        // The rest's frame starts at 3200, its first line's top less the
        // padding, and ends at 10000.
        let rest = Window {
            top: 2700,
            bottom: 10000,
            tail: 0,
            lead: -500,
            resumed: true,
            part_end: false,
            ..band
        };
        assert_eq!(shown_top(&tl, 0, &c, &cl, rest) + 3200, 3300 + 2000);
    }

    #[test]
    fn continued_spanning_cells_keep_the_new_rows_spacing() {
        let mut doc = document();
        doc.info.border_fills.push(BorderFill {
            fill: Some(hm_model::Fill {
                kind: 1,
                solid: Some(hm_model::SolidFill {
                    background: 0,
                    pattern: -1,
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        });
        let table = Table {
            rows: 2,
            cols: 2,
            cells: vec![
                Cell {
                    row_span: 2,
                    col_span: 1,
                    border_fill: 1,
                    ..Default::default()
                },
                Cell {
                    row_span: 1,
                    col_span: 1,
                    col: 1,
                    border_fill: 1,
                    ..Default::default()
                },
                Cell {
                    row_span: 1,
                    col_span: 1,
                    row: 1,
                    col: 1,
                    border_fill: 1,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let fonts = FontSystem::new();
        for spacing in [0, 100] {
            let mut layout = TableLayout::default();
            layout.width = 2000;
            layout.height = 4000;
            layout.spacing = spacing;
            layout.row_y = vec![0, 2000, 4000];
            layout.cells = vec![CellLayout {
                y: spacing,
                w: 1000,
                h: 4000 - spacing,
                ..Default::default()
            }];
            layout.cells.extend((0..2).map(|row| CellLayout {
                x: 1000,
                y: row * 2000 + spacing,
                w: 1000,
                h: 2000 - spacing,
                ..Default::default()
            }));
            let boxes = std::cell::RefCell::new(Vec::new());
            let ctx = Ctx {
                doc: &doc,
                styles: &[],
                fonts: &fonts,
                page: None,
                body_anchor: None,
                lines: None,
                cells: Some(&boxes),
                objects: None,
                index_only: false,
            };
            let mut dl = DisplayList {
                width: 2000,
                height: 2000,
                items: Vec::new(),
            };
            draw(
                &mut dl,
                &ctx,
                &table,
                &layout,
                0,
                0,
                Some(Window {
                    top: 2000,
                    bottom: 4000,
                    tail: 0,
                    tail_lines: false,
                    lead: 0,
                    cut: None,
                    carries: false,
                    spanning: None,
                    resumed: false,
                    part_end: false,
                }),
            );
            let bounds: Vec<_> = boxes
                .borrow()
                .iter()
                .map(|b| (b.x, b.y, b.width, b.height))
                .collect();
            assert_eq!(
                bounds,
                vec![
                    (0, spacing, 1000, 2000 - spacing),
                    (1000, spacing, 1000, 2000 - spacing),
                ]
            );
            let pixmap = crate::raster::render(&dl, &fonts, 72.0).unwrap();
            for x in [5, 15] {
                assert_eq!(
                    pixmap.pixel(x, 0).unwrap().red(),
                    if spacing == 0 { 0 } else { 255 }
                );
                assert_eq!(pixmap.pixel(x, 1).unwrap().red(), 0);
            }
        }
    }

    #[test]
    fn held_cell_objects_paint_every_intersecting_fragment() {
        use hm_layout::table::CellObject;
        use hm_layout::{CellLine, Page, ParaLayout};
        use hm_model::{
            Control, ControlKind, CtrlId, Matrix, ObjectCommon, ObjectControl, ObjectHeader,
            ObjectPart, Paragraph, RectGeom, Shape, ShapeComponent, ShapePart, ShapeStyle,
        };

        let doc = document();
        let fonts = FontSystem::new();
        let page = Page {
            width: 1000,
            height: 2000,
            ..Default::default()
        };
        let ctx = Ctx {
            doc: &doc,
            styles: &[],
            fonts: &fonts,
            page: Some(&page),
            body_anchor: None,
            lines: None,
            cells: None,
            objects: None,
            index_only: false,
        };
        let shape = Shape {
            component: ShapeComponent {
                kind: CtrlId::from_chars(b"$rec"),
                doubled: true,
                offset: (0, 0),
                group_level: 0,
                version: 1,
                initial_size: (1000, 1000),
                size: (1000, 1000),
                flip: 0,
                rotation: 0,
                rotation_center: (500, 500),
                translation: Matrix::IDENTITY,
                transforms: Vec::new(),
                style: Some(ShapeStyle {
                    fill: hm_model::Fill {
                        kind: 1,
                        solid: Some(hm_model::SolidFill {
                            background: 0,
                            pattern: -1,
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
                members: None,
                group_instance: None,
                tail: Default::default(),
            },
            parts: vec![ShapePart::Rect(RectGeom {
                corners: [(0, 0), (1000, 0), (1000, 1000), (0, 1000)],
                ..Default::default()
            })],
        };
        // A held drawing can start above its cell or cross a page cut.
        // Both fragments retain the drawing and clip it to their own box.
        for (object_top, first, rows) in [
            (-200, 0, 0..6),
            (-200, 600, 0..2),
            (500, 0, 5..6),
            (500, 600, 0..6),
            (1200, 0, 0..0),
            (-1000, 0, 0..0),
        ] {
            let table = Table {
                rows: 1,
                cols: 1,
                cells: vec![Cell {
                    paras: vec![Paragraph {
                        controls: vec![Control {
                            kind: ControlKind::Object(Box::new(ObjectControl {
                                id: CtrlId::from_chars(b"gso "),
                                header: ObjectHeader {
                                    common: ObjectCommon {
                                        attr: (2 << 3) | (2 << 8),
                                        y_offset: object_top,
                                        width: 1000,
                                        height: 1000,
                                        ..Default::default()
                                    },
                                    ..Default::default()
                                },
                                parts: vec![ObjectPart::Shape(Box::new(shape.clone()))],
                            })),
                            src: None,
                        }],
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            };
            let mut layout = TableLayout::default();
            layout.width = 1000;
            layout.height = 2000;
            layout.cells = vec![CellLayout {
                w: 1000,
                h: 2000,
                paras: vec![ParaLayout::default()],
                lines: vec![CellLine::default()],
                objects: vec![CellObject {
                    y: object_top,
                    w: 1000,
                    h: 1000,
                    ..Default::default()
                }],
                ..Default::default()
            }];
            let mut dl = DisplayList {
                width: 1000,
                height: 600,
                items: Vec::new(),
            };
            draw(
                &mut dl,
                &ctx,
                &table,
                &layout,
                0,
                0,
                Some(Window {
                    top: first,
                    bottom: first + 600,
                    tail: 0,
                    tail_lines: false,
                    lead: 0,
                    cut: None,
                    carries: false,
                    spanning: None,
                    resumed: false,
                    part_end: false,
                }),
            );
            let pixmap = crate::raster::render(&dl, &fonts, 72.0).unwrap();
            let painted: Vec<_> = (0..6)
                .filter(|&y| pixmap.pixel(5, y).unwrap().red() < 128)
                .collect();
            assert_eq!(
                painted,
                rows.collect::<Vec<_>>(),
                "top {object_top}, band {first}"
            );
        }
    }

    #[test]
    fn shared_grid_remains_visible_where_cell_center_lines_cross_it() {
        let mut doc = document();
        doc.info.border_fills.push(BorderFill {
            attr: (1 << 13) | (1 << 8),
            sides: [BorderLine {
                kind: 1,
                width: 7,
                color: 0,
            }; 4],
            diagonal: BorderLine {
                kind: 1,
                width: 15,
                color: 0xFF0000,
            },
            ..Default::default()
        });
        let table = Table {
            rows: 1,
            cols: 2,
            cells: (0..2)
                .map(|col| Cell {
                    col,
                    border_fill: 1,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        let mut layout = TableLayout::default();
        layout.width = 6000;
        layout.height = 2000;
        layout.cells = (0..2)
            .map(|col| CellLayout {
                x: col * 3000,
                w: 3000,
                h: 2000,
                ..Default::default()
            })
            .collect();
        let fonts = FontSystem::new();
        let ctx = Ctx {
            doc: &doc,
            styles: &[],
            fonts: &fonts,
            page: None,
            body_anchor: None,
            lines: None,
            cells: None,
            objects: None,
            index_only: false,
        };
        let mut dl = DisplayList {
            width: 8000,
            height: 4000,
            items: Vec::new(),
        };
        draw(&mut dl, &ctx, &table, &layout, 1000, 1000, None);
        let pixmap = crate::raster::render(&dl, &fonts, 144.0).unwrap();
        let rgb = |x, y| {
            let p = pixmap.pixel(x, y).unwrap();
            (p.red(), p.green(), p.blue())
        };
        assert_eq!(rgb(80, 40), (0, 0, 0));
        assert_eq!(rgb(70, 40), (0, 0, 255));
    }

    #[test]
    fn cell_fill_covers_the_fill_of_its_zone() {
        let solid = |color| BorderFill {
            fill: Some(hm_model::Fill {
                kind: 1,
                solid: Some(hm_model::SolidFill {
                    background: color,
                    pattern: -1,
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut doc = document();
        // 1 no fill, 2 red, 3 blue (COLORREF is 0x00BBGGRR).
        doc.info.border_fills = vec![BorderFill::default(), solid(0x0000FF), solid(0xFF0000)];
        let table = Table {
            rows: 1,
            cols: 2,
            cells: vec![
                Cell {
                    border_fill: 2,
                    ..Default::default()
                },
                Cell {
                    col: 1,
                    border_fill: 1,
                    ..Default::default()
                },
            ],
            zones: Some(vec![hm_model::CellZone {
                start_col: 0,
                start_row: 0,
                end_col: 1,
                end_row: 0,
                border_fill: 3,
            }]),
            ..Default::default()
        };
        let mut layout = TableLayout::default();
        layout.width = 6000;
        layout.height = 2000;
        layout.col_x = vec![0, 3000, 6000];
        layout.row_y = vec![0, 2000];
        layout.cells = (0..2)
            .map(|col| CellLayout {
                x: col * 3000,
                w: 3000,
                h: 2000,
                ..Default::default()
            })
            .collect();
        let fonts = FontSystem::new();
        let ctx = Ctx {
            doc: &doc,
            styles: &[],
            fonts: &fonts,
            page: None,
            body_anchor: None,
            lines: None,
            cells: None,
            objects: None,
            index_only: false,
        };
        let mut dl = DisplayList {
            width: 6000,
            height: 2000,
            items: Vec::new(),
        };
        draw(&mut dl, &ctx, &table, &layout, 0, 0, None);
        let pixmap = crate::raster::render(&dl, &fonts, 72.0).unwrap();
        let rgb = |x, y| {
            let p = pixmap.pixel(x, y).unwrap();
            (p.red(), p.green(), p.blue())
        };
        // The red cell keeps its own fill; the unfilled cell shows the zone.
        assert_eq!(rgb(15, 10), (255, 0, 0));
        assert_eq!(rgb(45, 10), (0, 0, 255));
    }

    #[test]
    fn shared_cell_border_is_painted_once_unless_the_styles_differ() {
        let line = BorderLine {
            kind: 1,
            width: 0,
            color: 0,
        };
        let mut doc = document();
        doc.info.border_fills = vec![
            BorderFill {
                sides: [line; 4],
                ..Default::default()
            },
            BorderFill {
                sides: [BorderLine {
                    color: 0xFF,
                    ..line
                }; 4],
                ..Default::default()
            },
        ];
        let mut table = Table {
            rows: 2,
            cols: 1,
            cells: (0..2)
                .map(|row| Cell {
                    row,
                    border_fill: 1,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        let mut layout = TableLayout::default();
        layout.width = 3000;
        layout.height = 2000;
        layout.cells = (0..2)
            .map(|row| CellLayout {
                y: row * 1000,
                w: 3000,
                h: 1000,
                ..Default::default()
            })
            .collect();
        let fonts = FontSystem::new();
        let ctx = Ctx {
            doc: &doc,
            styles: &[],
            fonts: &fonts,
            page: None,
            body_anchor: None,
            lines: None,
            cells: None,
            objects: None,
            index_only: false,
        };
        for (border, expected) in [(1, 1), (2, 2)] {
            table.cells[1].border_fill = border;
            let mut dl = DisplayList::default();
            draw(&mut dl, &ctx, &table, &layout, 0, 0, None);
            let shared = dl.items.iter().filter(|item| {
                matches!(
                    item,
                    crate::Item::Line {
                        y1: 1000,
                        y2: 1000,
                        ..
                    }
                )
            });
            assert_eq!(shared.count(), expected);
        }
    }
}
