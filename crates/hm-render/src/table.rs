//! Drawing tables: cell fills, cell text, then cell borders and diagonals.
//!
//! Every cell draws its own four sides centered on its edges, so shared
//! edges are drawn by both neighbors, as Hancom does; fills go first so no
//! cell covers a neighbor's border. A fragment of a split table shows a
//! band of the table: cells crossing the band's edges are closed there and
//! only the lines inside the band are drawn.

use hm_layout::{Page, Placed, PlacedTable, TableLayout};
use hm_model::{BorderFill, Table};

use crate::DisplayList;
use crate::paint::{self, Sides};
use crate::text::{Ctx, draw_line};

/// A band `[top, bottom)` of a table, in table coordinates.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Window {
    pub top: i32,
    pub bottom: i32,
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
    });
    // Page y of a table y.
    let py = |ty: i32| y + ty - w.top;
    // A cell's box clipped to the band, when any of it shows.
    let clip = |top: i32, bottom: i32| {
        let (a, b) = (top.max(w.top), bottom.min(w.bottom));
        (b > a).then_some((a, b))
    };
    for (c, cl) in t.cells.iter().zip(&tl.cells) {
        let Some((a, b)) = clip(cl.y, cl.y + cl.h) else {
            continue;
        };
        if let Some(f) = border_fill(ctx, c.border_fill).and_then(|b| b.fill.as_ref()) {
            paint::fill_rect(dl, x + cl.x, py(a), cl.w, b - a, f);
        }
    }
    let mut placed: Vec<Placed> = Vec::new();
    for (c, cl) in t.cells.iter().zip(&tl.cells) {
        if clip(cl.y, cl.y + cl.h).is_none() {
            continue;
        }
        let cx = x + cl.content_x();
        for l in &cl.lines {
            let (Some(para), Some(pal)) =
                (c.paras.get(l.para as usize), cl.paras.get(l.para as usize))
            else {
                continue;
            };
            let top = cl.content_y() + l.y;
            let h = pal.lines.get(l.line as usize).map_or(0, |ln| ln.height);
            if top < w.top || top + h > w.bottom {
                continue;
            }
            draw_line(
                dl,
                ctx,
                para,
                pal,
                l.line as usize,
                cx,
                py(top),
                &mut placed,
            );
        }
    }
    for (c, cl) in t.cells.iter().zip(&tl.cells) {
        let Some((a, b)) = clip(cl.y, cl.y + cl.h) else {
            continue;
        };
        if let Some(bf) = border_fill(ctx, c.border_fill) {
            if a == cl.y && b == cl.y + cl.h {
                paint::diagonals(dl, x + cl.x, py(a), cl.w, b - a, bf);
            }
            paint::border_sides(dl, x + cl.x, py(a), cl.w, b - a, bf, Sides::ALL);
        }
    }
}

/// Draws the table fragments placed on a page.
pub(crate) fn draw_placed(
    dl: &mut DisplayList,
    ctx: &Ctx<'_>,
    layout: &hm_layout::DocLayout,
    page: &Page,
) {
    for pt in &page.tables {
        draw_fragment(dl, ctx, layout, page.section, pt);
    }
}

fn draw_fragment(
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
        };
        draw(dl, ctx, t, tl, pt.x, pt.y, Some(head));
    }
    let band = Window {
        top: pt.top,
        bottom: pt.bottom,
    };
    draw(dl, ctx, t, tl, pt.x, pt.y + pt.header, Some(band));
}
