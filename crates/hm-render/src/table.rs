//! Drawing tables: cell fills, cell text, then cell borders and diagonals.
//!
//! Every cell draws its own four sides centered on its edges, so shared
//! edges are drawn by both neighbors, as Hancom does; fills go first so no
//! cell covers a neighbor's border. A fragment of a split table shows a
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
pub(crate) struct Window {
    pub top: i32,
    pub bottom: i32,
    /// Height of the carried part drawn right above the band, which the
    /// cells running through it into the band cover.
    pub lead: i32,
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
    window: Option<Window>,
) {
    let w = window.unwrap_or(Window {
        top: 0,
        bottom: tl.height,
        lead: 0,
    });
    // Page y of a table y.
    let py = |ty: i32| y + ty - w.top;
    // A cell's box clipped to the band, when any of it shows.
    let clip = |top: i32, bottom: i32| {
        let (a, b) = (top.max(w.top), bottom.min(w.bottom));
        (b > a).then_some((a, b))
    };
    // A cell's box on the page from its band part `(a, b)`, reaching up
    // over the carried part above the band when it runs through it.
    let boxed = |cl: &CellLayout, (a, b): (i32, i32)| {
        let lead = if cl.y < w.top { w.lead } else { 0 };
        (py(a) - lead, b - a + lead)
    };
    for (c, cl) in t.cells.iter().zip(&tl.cells) {
        let Some((a, b)) = clip(cl.y, cl.y + cl.h) else {
            continue;
        };
        if let Some(f) = border_fill(ctx, c.border_fill).and_then(|b| b.fill.as_ref()) {
            let (top, h) = boxed(cl, (a, b));
            paint::fill_rect(dl, x + cl.x, top, cl.w, h, f);
        }
    }
    zones(dl, ctx, t, tl, x, w, &py, true);
    // Hancom clips each cell's content to the cell's box.
    let mut placed: Vec<Placed> = Vec::new();
    for (c, cl) in t.cells.iter().zip(&tl.cells) {
        let Some((a, b)) = clip(cl.y, cl.y + cl.h) else {
            continue;
        };
        let cx = x + cl.content_x();
        let base = shown_top(c, cl, w);
        clipped(dl, (x + cl.x, py(a), cl.w, b - a), |dl| {
            for l in &cl.lines {
                let (Some(para), Some(pal)) =
                    (c.paras.get(l.para as usize), cl.paras.get(l.para as usize))
                else {
                    continue;
                };
                let flow = cl.split_y() + l.y;
                let h = pal.lines.get(l.line as usize).map_or(0, |ln| ln.height);
                if flow < w.top || flow + h > w.bottom {
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
    for (c, cl) in t.cells.iter().zip(&tl.cells) {
        let Some((a, b)) = clip(cl.y, cl.y + cl.h) else {
            continue;
        };
        let base = shown_top(c, cl, w);
        clipped(dl, (x + cl.x, py(a), cl.w, b - a), |dl| {
            cell_objects(dl, ctx, c, cl, x, py(base), w);
            // Tables flowing in the cell's text, the pieces in the band.
            for ct in &cl.tables {
                let flow = cl.split_y() + ct.y;
                let Some(band) = ct.band(w.top - flow, w.bottom - flow) else {
                    continue;
                };
                let at = py(base + ct.y + band.0);
                draw_nested(dl, ctx, c, cl, ct, (x + cl.content_x() + ct.x, at), band);
            }
        });
    }
    let meets = column_lines(ctx, t);
    // Every vertical side first, then the horizontal ones over them, as
    // Hancom draws a table's grid.
    for (across, sides) in [
        (
            false,
            Sides {
                left: true,
                right: true,
                ..Sides::NONE
            },
        ),
        (
            true,
            Sides {
                top: true,
                bottom: true,
                ..Sides::NONE
            },
        ),
    ] {
        for (c, cl) in t.cells.iter().zip(&tl.cells) {
            let Some((a, b)) = clip(cl.y, cl.y + cl.h) else {
                continue;
            };
            if let Some(bf) = border_fill(ctx, c.border_fill) {
                if across && a == cl.y && b == cl.y + cl.h {
                    paint::diagonals(dl, x + cl.x, py(a), cl.w, b - a, bf);
                }
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
            paint::border_sides_meeting(dl, x + cl.x, py(a), cl.w, b - a, bf, top, meet);
        }
    }
    zones(dl, ctx, t, tl, x, w, &py, false);
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
    w: Window,
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
                paint::fill_rect(dl, x + zx, py(a), zw, b - a, f);
            }
        } else {
            if a == zy && b == zy + zh {
                paint::diagonals(dl, x + zx, py(a), zw, b - a, bf);
            }
            paint::border_sides(dl, x + zx, py(a), zw, b - a, bf, Sides::ALL);
        }
    }
}

/// Table height of the top of a cell's content as drawn in band `w`. A
/// cell the band holds whole aligns its content in its box; a cell the band
/// cuts aligns the lines it shows (those whose unaligned place lies in the
/// band) in the part of its box the band holds, as Hancom aligns each part
/// of a cell split across pages.
fn shown_top(c: &Cell, cl: &CellLayout, w: Window) -> i32 {
    if cl.offset == 0 || (cl.y >= w.top && cl.y + cl.h <= w.bottom) {
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
    let Some((first, last)) = lines
        .chain(tables)
        .map(|(a, b)| (flow + a, flow + b))
        .filter(|&(a, b)| a >= w.top && b <= w.bottom)
        .reduce(|p, q| (p.0.min(q.0), p.1.max(q.1)))
    else {
        return flow;
    };
    let top = cl.y.max(w.top) + cl.padding[2];
    let bottom = if cl.y + cl.h > w.bottom {
        w.bottom
    } else {
        cl.y + cl.h - cl.padding[3]
    };
    let slack = (bottom - top - (last - first)).max(0);
    let at = if c.vertical_align() == 2 {
        top + slack
    } else {
        top + slack / 2
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
        let band = Window {
            top,
            bottom,
            lead: 0,
        };
        draw(dl, ctx, nt, ntl, x, y, Some(band));
    }
}

/// Objects anchored in a cell's paragraphs, placed with the cell content
/// as their column. `top` is the page y of the content top.
fn cell_objects(
    dl: &mut DisplayList,
    ctx: &Ctx<'_>,
    c: &Cell,
    cl: &CellLayout,
    x: i32,
    top: i32,
    w: Window,
) {
    let Some(page) = ctx.page else { return };
    for (pi, (para, pal)) in c.paras.iter().zip(&cl.paras).enumerate() {
        if !para.controls.iter().any(|k| k.as_object().is_some()) {
            continue;
        }
        let Some(first) = cl.lines.iter().find(|l| l.para == pi as u32) else {
            continue;
        };
        let ty = cl.split_y() + first.y;
        if ty < w.top || ty >= w.bottom {
            continue;
        }
        let left = pal.lines.first().map_or(0, |l| l.seg_x);
        let anchor = Anchor {
            paper: (page.width, page.height),
            paper_x: None,
            body: page.body,
            column: (x + cl.content_x(), pal.column_width),
            // Objects hang from the paragraph's top above its space before.
            para_top: top + first.y - pal.before,
            para_left: x + cl.content_x() + left,
        };
        let mut list = place(para, pi as u32, &anchor);
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
    let h = carried.height;
    // Cells running on past the carried row are drawn with the band below
    // it, when there is one.
    let cells = || {
        carried.cells.iter().filter_map(|&(k, first)| {
            let cl = tl.cells.get(k as usize)?;
            (!banded || cl.y + cl.h <= carried.until).then_some((
                t.cells.get(k as usize)?,
                cl,
                first,
            ))
        })
    };
    for (c, cl, _) in cells() {
        if let Some(f) = border_fill(ctx, c.border_fill).and_then(|b| b.fill.as_ref()) {
            paint::fill_rect(dl, x + cl.x, y, cl.w, h, f);
        }
    }
    let mut placed: Vec<Placed> = Vec::new();
    for (c, cl, first) in cells() {
        let Some(first) = first else { continue };
        let cx = x + cl.content_x();
        clipped(dl, (x + cl.x, y, cl.w, h), |dl| {
            let mut shown = 0;
            for l in &cl.lines {
                let (Some(para), Some(pal)) =
                    (c.paras.get(l.para as usize), cl.paras.get(l.para as usize))
                else {
                    continue;
                };
                let top = cl.split_y() + l.y;
                let lh = pal.lines.get(l.line as usize).map_or(0, |ln| ln.height);
                if top < first || (shown > 0 && cl.padding[2] + top + lh - first > h) {
                    continue;
                }
                shown += 1;
                let at = y + cl.padding[2] + top - first;
                draw_line(dl, ctx, para, pal, l.line as usize, cx, at, &mut placed);
            }
            for ct in &cl.tables {
                let top = cl.split_y() + ct.y;
                let room = h - cl.padding[2];
                let Some(band) = ct.band(first - top, first + room - top) else {
                    continue;
                };
                let at = y + cl.padding[2] + top + band.0 - first;
                draw_nested(dl, ctx, c, cl, ct, (x + cl.content_x() + ct.x, at), band);
            }
        });
    }
    for (c, cl, _) in cells() {
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
    let Some(tl) = layout.para(section, pt.para).table(pt.control) else {
        return;
    };
    if pt.header > 0 {
        let head = Window {
            top: 0,
            bottom: pt.header,
            lead: 0,
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
            lead,
        };
        draw(dl, ctx, t, tl, pt.x, y, Some(band));
    }
    if pt.caption {
        let bottom = pt.y + pt.height();
        caption(dl, ctx, t, tl, pt.x, pt.y, bottom);
    }
}
