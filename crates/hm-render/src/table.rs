//! Drawing tables: cell fills, cell text, then cell borders and diagonals.
//!
//! Every cell draws its own four sides centered on its edges, so shared
//! edges are drawn by both neighbors, as Hancom does; fills go first so no
//! cell covers a neighbor's border.

use hm_layout::{Placed, TableLayout};
use hm_model::{BorderFill, Table};

use crate::DisplayList;
use crate::paint::{self, Sides};
use crate::text::{Ctx, draw_line};

/// Rows of a table fragment to draw, with the fragment's vertical shift.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Window {
    /// Table-relative band `[top, bottom)` shown in this fragment.
    pub top: i32,
    pub bottom: i32,
}

fn border_fill<'a>(ctx: &Ctx<'a>, id: u16) -> Option<&'a BorderFill> {
    ctx.doc.info.border_fills.get((id as usize).checked_sub(1)?)
}

/// Draws table `t` laid out as `tl` with its top left at `(x, y)`.
pub(crate) fn draw(
    dl: &mut DisplayList,
    ctx: &Ctx<'_>,
    t: &Table,
    tl: &TableLayout,
    x: i32,
    y: i32,
    window: Option<Window>,
) {
    let visible = |top: i32, bottom: i32| window.is_none_or(|w| bottom > w.top && top < w.bottom);
    for (c, cl) in t.cells.iter().zip(&tl.cells) {
        if !visible(cl.y, cl.y + cl.h) {
            continue;
        }
        if let Some(f) = border_fill(ctx, c.border_fill).and_then(|b| b.fill.as_ref()) {
            paint::fill_rect(dl, x + cl.x, y + cl.y, cl.w, cl.h, f);
        }
    }
    let mut placed: Vec<Placed> = Vec::new();
    for (c, cl) in t.cells.iter().zip(&tl.cells) {
        if !visible(cl.y, cl.y + cl.h) {
            continue;
        }
        let (cx, cy) = (x + cl.content_x(), y + cl.content_y());
        for l in &cl.lines {
            let (Some(para), Some(pal)) =
                (c.paras.get(l.para as usize), cl.paras.get(l.para as usize))
            else {
                continue;
            };
            let top = cl.content_y() + l.y;
            let h = pal.lines.get(l.line as usize).map_or(0, |ln| ln.height);
            if !visible(top, top + h) {
                continue;
            }
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
    for (c, cl) in t.cells.iter().zip(&tl.cells) {
        if !visible(cl.y, cl.y + cl.h) {
            continue;
        }
        if let Some(bf) = border_fill(ctx, c.border_fill) {
            paint::diagonals(dl, x + cl.x, y + cl.y, cl.w, cl.h, bf);
            paint::border_sides(dl, x + cl.x, y + cl.y, cl.w, cl.h, bf, Sides::ALL);
        }
    }
}
