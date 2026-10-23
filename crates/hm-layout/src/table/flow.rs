//! Placing tables that are not set as characters.
//!
//! A table anchored to its paragraph with top-and-bottom wrapping flows
//! with the text: it starts at the paragraph's first line plus its
//! vertical offset and outer top margin, splits across columns and pages as
//! its split mode allows, and the content after its paragraph continues
//! below its last fragment and outer bottom margin. A table that does not
//! fit where it starts moves to the next column with its paragraph unless
//! the column is empty. A table that may not split but is taller than a
//! whole column splits between rows anyway.
//!
//! Other tables sit at their anchored frame on the page of their paragraph;
//! the object bands keep text off the top-and-bottom ones.

use hm_model::{Control, ObjectCommon, Paragraph};

use std::sync::Arc;

use super::split::{
    CUT_ROOM, Carried, SplitMode, carried_break, carried_height, cut, cut_cells, first_cut,
    header_height, resume_after,
};
use crate::objects::{Anchor, Band, frame};
use crate::paginate::Flow;
use crate::para::ParaLayout;

/// A table fragment on a page.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlacedTable {
    /// Paragraph index in the section and control index in the paragraph.
    pub para: u32,
    pub control: u32,
    /// Top left of the table box on the page (outer margins excluded).
    pub x: i32,
    pub y: i32,
    /// The band of the table shown, `[top, bottom)` in table coordinates.
    pub top: i32,
    pub bottom: i32,
    /// Height of the repeated header band drawn above a continuation.
    pub header: i32,
    /// The fragment draws the table's caption: the first one for a caption
    /// above or beside the table, the last one for a caption below.
    pub caption: bool,
    /// Cells cut at the previous page end, carried to this fragment's top
    /// above the band.
    pub carried: Option<Arc<Carried>>,
    /// The page end (table height, bottom margin taken off) of a cut through
    /// the cells at the band's bottom: their lines show above it only when
    /// they stay by the cut rule.
    pub cut: Option<i32>,
}

impl PlacedTable {
    /// Height the fragment takes on the page.
    pub fn height(&self) -> i32 {
        self.header + self.carried.as_ref().map_or(0, |c| c.height) + self.bottom - self.top
    }
}

/// True for a table that flows with the text in a column `column` wide:
/// not set as a character, anchored to its paragraph, wrapped top and
/// bottom, or square when it leaves less than half the column beside it
/// (text does not wrap beside tables yet, so a wide square table blocks
/// its band and a narrow one stays at its frame).
pub fn flows_in(c: &Control, column: i32) -> bool {
    c.as_table().is_some_and(|t| {
        let o = &t.header.common;
        let outer = o.width + o.margins[0] as i32 + o.margins[1] as i32;
        !o.treat_as_char()
            && o.vert_rel_to() == 2
            && (o.text_wrap() == 1 || (o.text_wrap() == 0 && outer * 2 > column))
    })
}

/// [`flows_in`] for a top-and-bottom table, whatever the column.
pub fn flows(c: &Control) -> bool {
    c.as_table().is_some_and(|t| {
        let o = &t.header.common;
        !o.treat_as_char() && o.vert_rel_to() == 2 && o.text_wrap() == 1
    })
}

/// True for an object kept inside the page that may not overlap others:
/// Hancom moves it clear of the objects before it. Objects free to leave
/// the page may overlap whatever their flag says.
pub(crate) fn avoids(o: &ObjectCommon) -> bool {
    const FLOW_WITH_TEXT: u32 = 1 << 13;
    const ALLOW_OVERLAP: u32 = 1 << 14;
    o.attr & (FLOW_WITH_TEXT | ALLOW_OVERLAP) == FLOW_WITH_TEXT
}

fn mode_for(t: &hm_model::Table, height: i32, column: i32) -> SplitMode {
    match SplitMode::of(t) {
        SplitMode::Never if height > column => SplitMode::Rows,
        m => m,
    }
}

/// The page x of a table's box from its anchor.
fn box_x(o: &ObjectCommon, a: &Anchor) -> i32 {
    let [ml, mr, mt, mb] = o.margins.map(i32::from);
    let outer = ObjectCommon {
        width: ml + o.width + mr,
        height: mt + o.height + mb,
        ..*o
    };
    frame(&outer, a).0 + ml
}

/// Before a paragraph's lines are placed: moves to the next column when the
/// paragraph's first flowing table cannot start in this one. `top` is the
/// region y of the paragraph's first line.
pub(crate) fn before_host(flow: &mut Flow, para: &Paragraph, pl: &ParaLayout, top: i32) {
    if !flow.has_content {
        return;
    }
    let column = flow.column().1;
    let Some((k, c)) = para
        .controls
        .iter()
        .enumerate()
        .find(|(_, c)| flows_in(c, column))
    else {
        return;
    };
    let (Some(t), Some(tl)) = (c.as_table(), pl.table(k as u32)) else {
        return;
    };
    let o = &t.header.common;
    let [_, _, mt, mb] = o.margins.map(i32::from);
    let start = (top - pl.before).max(0) + o.y_offset.max(0) + mt;
    let avail = flow.region_height() - start;
    let mode = mode_for(t, tl.height, flow.body.h);
    // A table that never splits waits for the next column on its own.
    if mode == SplitMode::Never {
        return;
    }
    let fits = tl.height + mb <= avail
        || fragment_end(t, tl, mode, 0, avail, flow.body.h - mt).is_some_and(|c| c < tl.height);
    if !fits {
        flow.next_column();
    }
}

/// True when a line holds anything besides controls and the paragraph end.
pub(crate) fn has_content(para: &Paragraph, line: &crate::para::Line) -> bool {
    para.units().any(|(pos, u)| {
        pos >= line.start
            && pos < line.end
            && match u {
                hm_model::Unit::Extended { index, .. } => para
                    .controls
                    .get(index as usize)
                    .and_then(|c| c.object_common())
                    .is_some_and(|o| o.treat_as_char()),
                hm_model::Unit::CharControl(13) => false,
                _ => true,
            }
    })
}

/// Flowing tables a paragraph's lines so far end above: they wait for the
/// first line that reaches them, or for the paragraph's end.
#[derive(Clone, Debug, Default)]
pub(crate) struct Deferred {
    /// The paragraph's top (region y) and the page and column it is on.
    top: i32,
    at: (usize, usize),
    controls: Vec<u32>,
}

/// Before line `li` of a paragraph is placed, its natural top at region y
/// `top`: a line with content makes way for the flowing tables anchored in
/// it or waiting, so those it reaches are placed first and the line goes
/// below them; the others wait below it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn before_line(
    flow: &mut Flow,
    pi: u32,
    para: &Paragraph,
    pl: &ParaLayout,
    li: usize,
    top: i32,
    para_left: i32,
) {
    if li == 0 {
        let (cx, cw) = flow.column();
        flow.deferred = Deferred {
            top: clear_bands(flow, None, (top - pl.before).max(0), (cx, cx + cw)),
            at: (flow.pages.len(), flow.col),
            controls: Vec::new(),
        };
    }
    let Some(line) = pl.lines.get(li) else {
        return;
    };
    if !has_content(para, line) {
        return;
    }
    let column = flow.column().1;
    let mut waiting: Vec<u32> = std::mem::take(&mut flow.deferred.controls);
    waiting.extend(line_controls(para, line).into_iter().filter(|&k| {
        para.controls
            .get(k as usize)
            .is_some_and(|c| flows_in(c, column))
    }));
    // The line goes below the tables it reaches, and from there may reach
    // more: each round places those and moves the line below them.
    let (cx, cw) = flow.column();
    let mut at = clear_bands(flow, None, top, (cx, cx + cw));
    let anchor_at = (flow.pages.len(), flow.col);
    loop {
        // Tables hang from the paragraph's top while it is on the same
        // column; a line that ends above one leaves it waiting.
        let same = flow.deferred.at == (flow.pages.len(), flow.col);
        let (wait, ready): (Vec<u32>, Vec<u32>) = waiting.iter().partition(|&&k| {
            let o = para.controls[k as usize]
                .object_common()
                .unwrap_or_default();
            same && at + line.height <= flow.deferred.top + o.y_offset.max(0)
        });
        waiting = wait;
        if ready.is_empty() {
            break;
        }
        let anchor_top = if same { flow.deferred.top } else { top };
        let bands = flow.bands.len();
        place_tables(flow, pi, para, pl, &ready, anchor_top, para_left);
        if (flow.pages.len(), flow.col) != anchor_at {
            break;
        }
        let page_top = flow.page_y(0);
        match flow
            .bands
            .get(bands..)
            .and_then(|b| b.iter().map(|b| b.bottom).max())
        {
            Some(bottom) if bottom - page_top > at => at = bottom - page_top,
            _ => break,
        }
    }
    flow.deferred.controls = waiting;
}

/// After line `li` of a paragraph is placed at region y `top`: places the
/// other tables anchored in it, and after the last line the flowing tables
/// still waiting below the paragraph. A line of nothing but controls stays
/// where it is, under its flowing tables.
#[allow(clippy::too_many_arguments)]
pub(crate) fn after_line(
    flow: &mut Flow,
    pi: u32,
    para: &Paragraph,
    pl: &ParaLayout,
    li: usize,
    top: i32,
    para_left: i32,
) {
    let bands = flow.bands.len();
    place_line_tables(flow, pi, para, pl, li, top, para_left, false);
    // A content-less host line whose own table hangs below its top but
    // within its height goes below the table when Hancom stored it at a
    // width: one it kept beside the table it stored at none.
    if let Some(line) = pl.lines.get(li)
        && !has_content(para, line)
        && para.line_segs.get(li).is_some_and(|s| s.width > 0)
        && let Some(bottom) = flow
            .bands
            .get(bands..)
            .unwrap_or(&[])
            .iter()
            .filter(|b| b.top > flow.page_y(top) && b.top < flow.page_y(top + line.height))
            .map(|b| b.bottom)
            .max()
    {
        let below = bottom - flow.page_y(0) + line.advance();
        let col = flow.col;
        flow.fill[col] = flow.fill[col].max(below);
    }
    if li + 1 == pl.lines.len() && !flow.deferred.controls.is_empty() {
        let waiting = std::mem::take(&mut flow.deferred.controls);
        let anchor_top = if flow.deferred.at == (flow.pages.len(), flow.col) {
            flow.deferred.top
        } else {
            top
        };
        place_tables(flow, pi, para, pl, &waiting, anchor_top, para_left);
    }
}

/// The controls anchored in a line.
fn line_controls(para: &Paragraph, line: &crate::para::Line) -> Vec<u32> {
    para.units()
        .filter_map(|(pos, u)| match u {
            hm_model::Unit::Extended { index, .. } if pos >= line.start && pos < line.end => {
                Some(index)
            }
            _ => None,
        })
        .collect()
}

/// Places the tables anchored in line `li`: with `flowing_only`, the
/// flowing ones; otherwise the anchored ones, and the flowing ones when the
/// line has no content. Flowing tables start at region y `top` and block
/// their band, or push the flow below their last fragment when they split.
#[allow(clippy::too_many_arguments)]
fn place_line_tables(
    flow: &mut Flow,
    pi: u32,
    para: &Paragraph,
    pl: &ParaLayout,
    li: usize,
    top: i32,
    para_left: i32,
    flowing_only: bool,
) {
    let Some(line) = pl.lines.get(li) else {
        return;
    };
    let content = has_content(para, line);
    // Tables hang from the paragraph's top, above its space before.
    let top = if li == 0 {
        (top - pl.before).max(0)
    } else {
        top
    };
    let column = flow.column().1;
    let chosen: Vec<u32> = line_controls(para, line)
        .into_iter()
        .filter(|&k| {
            let Some(c) = para.controls.get(k as usize) else {
                return false;
            };
            let flowing = flows_in(c, column);
            !((flowing && content != flowing_only) || (flowing_only && !flowing))
        })
        .collect();
    place_tables(flow, pi, para, pl, &chosen, top, para_left);
}

/// Places tables `controls` of a paragraph whose top is at region y `top`.
#[allow(clippy::too_many_arguments)]
fn place_tables(
    flow: &mut Flow,
    pi: u32,
    para: &Paragraph,
    pl: &ParaLayout,
    controls: &[u32],
    top: i32,
    para_left: i32,
) {
    // Earlier flowing tables of the line ending on this page: their span
    // and the region y below them. A table stacks below those it meets.
    let mut stacked: Vec<(i32, i32, i32)> = Vec::new();
    let mut anchor_at = (flow.pages.len(), flow.col);
    let mut top = top;
    for &k in controls {
        let Some(c) = para.controls.get(k as usize) else {
            continue;
        };
        let (Some(t), Some(tl)) = (c.as_table(), pl.table(k)) else {
            continue;
        };
        let o = t.header.common;
        let flowing = flows_in(c, flow.column().1);
        if o.treat_as_char() {
            continue;
        }
        let anchor = Anchor {
            paper: flow.paper,
            paper_x: None,
            body: flow.body,
            column: flow.column(),
            para_top: flow.page_y(top),
            para_left,
        };
        let x = box_x(&o, &anchor);
        let [_, _, mt, mb] = o.margins.map(i32::from);
        if !flowing {
            let [ml, mr, _, _] = o.margins.map(i32::from);
            let outer = ObjectCommon {
                width: ml + o.width + mr,
                height: mt + o.height + mb,
                ..o
            };
            let (_, y, _, _) = frame(&outer, &anchor);
            let (gx, gy) = tl.grid_offset();
            push(
                flow,
                PlacedTable {
                    para: pi,
                    control: k,
                    x: x + gx,
                    y: y + mt + gy,
                    top: 0,
                    bottom: tl.height,
                    header: 0,
                    caption: tl.caption.is_some(),
                    carried: None,
                    cut: None,
                },
            );
            continue;
        }
        // A table still waiting to go on is placed first, and this one
        // starts below its last fragment. One waiting for the next page
        // keeps waiting unless this one would wait too.
        if flow
            .pending_table
            .as_ref()
            .is_some_and(|p| !p.fresh || o.text_wrap() == 0)
        {
            flow.flush_tables();
            let page_top = flow.page_y(0);
            top = flow
                .bands
                .iter()
                .filter(|b| b.table)
                .map(|b| b.bottom - page_top)
                .max()
                .unwrap_or(0);
            anchor_at = (flow.pages.len(), flow.col);
            stacked.clear();
        }
        let mode = mode_for(t, tl.height, flow.body.h);
        let header = header_height(t, tl);
        // A caption above the table comes first; one below follows the
        // last fragment; one beside it moves the grid over.
        let (gx, gy) = tl.grid_offset();
        let x = x + gx;
        let cap_below = tl
            .caption
            .as_ref()
            .filter(|c| c.side == 3)
            .map_or(0, |c| c.gap + c.height);
        // A table whose anchor an earlier one of the same line has carried
        // past starts a page of its own.
        let [ml, mr, _, _] = o.margins.map(i32::from);
        let span = (x - ml, x + tl.width + mr);
        // Only a table kept on the page that may not overlap others moves
        // below them, and stays below its paragraph's top; a free one
        // hangs where its offset puts it.
        let below = if avoids(&o) {
            stacked
                .iter()
                .filter(|s| s.0 < span.1 && span.0 < s.1)
                .map(|s| s.2)
                .fold(top, i32::max)
        } else {
            i32::MIN
        };
        let fresh = (flow.pages.len(), flow.col) != anchor_at;
        // Text goes on filling this page below the host's line, which
        // comes down below the table that went ahead.
        let defer = fresh && flow.column_count() == 1 && flow.pending_table.is_none();
        if defer
            && let [line] = pl.lines.as_slice()
            && !has_content(para, line)
        {
            let col = flow.col;
            flow.fill[col] += line.advance();
        }
        let y = if fresh {
            if !defer {
                flow.next_column();
                stacked.clear();
            }
            o.y_offset.max(0) + mt + gy
        } else {
            clear_bands(flow, Some(top), (top + o.y_offset).max(below), span) + mt + gy
        };
        let mut st = Run {
            o,
            anchor,
            gx,
            x,
            mode,
            header,
            cap_below,
            span,
            mt,
            mb,
            y,
            from: 0,
            repeat: 0,
            carry: None,
            start: (flow.pages.len(), flow.col),
            passes: 0,
            bound: 4 * (tl.cells.iter().map(|c| c.lines.len() + 1).sum::<usize>() + tl.row_y.len()),
        };
        if defer {
            // A table a page ahead of its anchor starts the next page.
            flow.pending_table = Some(Box::new(Pending {
                para: pi,
                control: k,
                table: t.clone(),
                layout: tl.clone(),
                run: st,
                notes: flow.take_notes_of(k),
                fresh: true,
            }));
            continue;
        }
        if run(flow, pi, k, t, tl, &mut st, &mut stacked) {
            let notes = flow.take_notes_of(k);
            flow.pending_table = Some(Box::new(Pending {
                para: pi,
                control: k,
                table: t.clone(),
                layout: tl.clone(),
                run: st,
                notes,
                fresh: false,
            }));
        }
    }
}

/// Where a table placed fragment by fragment stands between fragments.
#[derive(Clone, Debug)]
pub(crate) struct Run {
    o: ObjectCommon,
    anchor: Anchor,
    gx: i32,
    x: i32,
    mode: SplitMode,
    header: i32,
    cap_below: i32,
    span: (i32, i32),
    mt: i32,
    mb: i32,
    /// Region y where the next fragment starts.
    y: i32,
    from: i32,
    repeat: i32,
    carry: Option<Carried>,
    start: (usize, usize),
    passes: usize,
    bound: usize,
}

/// A square-wrapped table whose fragments after the first wait for the
/// text beside it to reach the next column: Hancom lets the paragraphs after
/// it wrap beside every fragment in turn.
#[derive(Clone, Debug)]
pub(crate) struct Pending {
    para: u32,
    control: u32,
    table: hm_model::Table,
    layout: super::TableLayout,
    run: Run,
    /// Footnotes in its cells not placed yet.
    notes: Vec<(u32, i32, crate::furniture::Frame)>,
    /// It starts on the next page, while the text after its paragraph
    /// goes on filling this one.
    fresh: bool,
}

/// Places the next fragments of a waiting table at the top of the column
/// the flow just moved to.
pub(crate) fn resume(flow: &mut Flow, mut p: Pending) {
    let saved = std::mem::replace(flow.table_notes_mut(), std::mem::take(&mut p.notes));
    let mut stacked = Vec::new();
    let waits = run(
        flow,
        p.para,
        p.control,
        &p.table,
        &p.layout,
        &mut p.run,
        &mut stacked,
    );
    p.notes = std::mem::replace(flow.table_notes_mut(), saved);
    if waits {
        flow.pending_table = Some(Box::new(p));
    }
}

/// The page x of a fragment in the current column: where the table would
/// sit anchored in that column.
fn x_in(flow: &Flow, st: &Run) -> i32 {
    let dx = flow.column().0 - st.anchor.column.0;
    let moved = Anchor {
        column: flow.column(),
        para_left: st.anchor.para_left + dx,
        ..st.anchor
    };
    box_x(&st.o, &moved) + st.gx
}

/// Places a flowing table's fragments from where `st` stands, moving on
/// column by column. A square-wrapped table stops after each fragment that
/// is not its last and returns true: the rest waits for the next column.
#[allow(clippy::too_many_arguments)]
fn run(
    flow: &mut Flow,
    pi: u32,
    k: u32,
    t: &hm_model::Table,
    tl: &super::TableLayout,
    st: &mut Run,
    stacked: &mut Vec<(i32, i32, i32)>,
) -> bool {
    let Run {
        o,
        gx: _,
        x,
        mode,
        header,
        cap_below,
        span,
        mt,
        mb,
        start,
        bound,
        ..
    } = st.clone();
    let mut y = st.y;
    let mut from = st.from;
    let mut repeat = st.repeat;
    let mut carry = st.carry.take();
    let square = o.text_wrap() == 0;
    // The rest waits for the text to move on to the next column.
    macro_rules! wait {
        () => {{
            st.y = mt;
            st.from = from;
            st.repeat = if from > 0 { header } else { 0 };
            st.carry = carry;
            return true;
        }};
    }
    loop {
        st.passes += 1;
        if st.passes > bound {
            return false;
        }
        let avail = flow.region_height() - y - repeat;
        // Cells carried over from a cut through their row come first,
        // split again when even they do not fit.
        let mut carried = None;
        // Carried cells with no lines left just close their row.
        if let Some(cs) = carry.take_if(|cs| cs.cells.iter().all(|c| c.1.is_none())) {
            from = cs.until;
        }
        if let Some(cs) = carry.take() {
            let cell = |k: u32| &tl.cells[k as usize];
            let full = cs
                .cells
                .iter()
                .filter(|c| c.1.is_some())
                .map(|&(k, f)| carried_height(cell(k), tl.cell_spans(k as usize), f))
                .max()
                .unwrap_or(0);
            let room = avail - mb;
            if full <= room {
                from = cs.until;
                carried = Some(Carried { height: full, ..cs });
            } else {
                let force = !flow.has_content;
                let next: Vec<(u32, Option<i32>)> = cs
                    .cells
                    .iter()
                    .map(|&(k, f)| {
                        (
                            k,
                            carried_break(cell(k), tl.cell_spans(k as usize), f, room, force),
                        )
                    })
                    .collect();
                if next == cs.cells {
                    // Nothing fits here: go on in the next column.
                    carry = Some(cs);
                    flow.next_column();
                    y = mt;
                    repeat = header;
                    continue;
                }
                let until = cs.until;
                // The part ends below the lines it keeps and their padding.
                let kept = cs
                    .cells
                    .iter()
                    .zip(&next)
                    .filter_map(|(&(k, f), &(_, nb))| {
                        let (cl, f) = (cell(k), f?);
                        let bottom = tl
                            .cell_spans(k as usize)
                            .iter()
                            .filter(|&&(t, _)| t >= f && nb.is_none_or(|nb| t < nb))
                            .map(|&(_, b)| b)
                            .max()?;
                        Some(cl.padding[2] + bottom - f + cl.padding[3])
                    })
                    .max()
                    .unwrap_or(room)
                    .min(room.max(0));
                push(
                    flow,
                    PlacedTable {
                        para: pi,
                        control: k,
                        x: x_in(flow, st),
                        y: flow.page_y(y),
                        top: until,
                        bottom: until,
                        header: repeat,
                        caption: false,
                        carried: Some(Arc::new(Carried { height: kept, ..cs })),
                        cut: None,
                    },
                );
                flow.has_content = true;
                if next.iter().any(|c| c.1.is_some()) {
                    carry = Some(Carried {
                        cells: next,
                        until,
                        ..Carried::default()
                    });
                } else {
                    from = until;
                }
                if square {
                    wait!();
                }
                flow.next_column();
                y = mt;
                repeat = header;
                continue;
            }
        }
        let lead = carried.as_ref().map_or(0, |c| c.height);
        let avail = avail - lead;
        // The table's bottom margin counts where it ends; a fragment cut
        // at a row edge may reach the page end. The cell spacing below the
        // last row may run past it.
        let rows_end = tl.row_y.last().copied().unwrap_or(tl.height).max(from);
        let c = if rows_end - from + mb <= avail {
            Some(tl.height)
        } else {
            // A cut between rows keeps the last row only when it ends,
            // with the bottom margin, more than CUT_ROOM above the page end.
            let room = if mode == SplitMode::Anywhere {
                avail
            } else {
                avail - mb - CUT_ROOM - 1
            };
            fragment_end(t, tl, mode, from, room, flow.body.h - mt)
        };
        let c = match c {
            Some(c) => c,
            // After carried cells the fragment may hold nothing more.
            None if carried.is_some() => from,
            // Nothing fits here: an empty column takes the smallest
            // piece it can, anything else hands over to the next one.
            None if !flow.has_content => first_cut(t, tl, mode, from),
            // A table that never splits leaves the text here and goes on
            // at the top of the next column.
            None if mode == SplitMode::Never && from == 0 => wait!(),
            None => {
                flow.next_column();
                y = mt;
                repeat = if from > 0 { header } else { 0 };
                continue;
            }
        };
        let last = c >= tl.height;
        // Splitting inside cells, a row whose cells hold no more lines
        // below the page end is drawn down to it and the table goes on
        // with the next row; otherwise the cells crossing the page end
        // carry their lines left over to the top of the next page.
        let limit = from + avail - mb;
        let mut cut_at = None;
        let (c, resume, next_carry) = match (last, mode) {
            (false, SplitMode::Anywhere) if limit > c => {
                match cut_cells(t, tl, from, limit) {
                    // Nothing left in the cut row: go on with the next.
                    Some(cut) if cut.cells.iter().all(|c| c.1.is_none()) => {
                        cut_at = Some(limit);
                        (cut.end, cut.until, None)
                    }
                    Some(cut) => {
                        cut_at = Some(limit);
                        (cut.end, cut.end, Some(cut))
                    }
                    // The row moves on whole when it can: the fragment
                    // ends at the last cut above it.
                    None => {
                        let cap = limit - CUT_ROOM;
                        let row_top = tl.row_y.iter().rev().find(|&&y| y <= cap).copied();
                        let c = match row_top {
                            Some(top) if top > from && c > top => {
                                cut(t, tl, mode, from, top).unwrap_or(c)
                            }
                            _ => c,
                        };
                        (c, resume_after(tl, c), None)
                    }
                }
            }
            (false, SplitMode::Anywhere) => (c, resume_after(tl, c), None),
            _ => (c, c, None),
        };
        // A cut through the last row that leaves nothing of it ends the
        // table here.
        let last = last || (next_carry.is_none() && resume >= tl.height);
        let caption = tl
            .caption
            .as_ref()
            .is_some_and(|cap| if cap.side == 3 { last } else { from == 0 });
        push(
            flow,
            PlacedTable {
                para: pi,
                control: k,
                x: x_in(flow, st),
                y: flow.page_y(y),
                top: from,
                bottom: c,
                header: repeat,
                caption,
                carried: carried.map(Arc::new),
                cut: cut_at,
            },
        );
        flow.has_content = true;
        let end = y + repeat + lead + (c - from) + if last { cap_below } else { 0 };
        flow.ink = flow.ink.max(end);
        // A square-wrapped table lets text use the room beside each of
        // its fragments.
        if square {
            let dx = x_in(flow, st) - x;
            flow.bands.push(Band {
                top: flow.page_y(y - mt),
                bottom: flow.page_y(end + mb),
                left: span.0 + dx,
                right: span.1 + dx,
                square: true,
                side: ((o.attr >> 24) & 3) as u8,
                table: true,
                ..Band::default()
            });
        }
        flow.line_end = Some((flow.col, end));
        if last {
            let below = end + mb;
            stacked.push((span.0, span.1, below));
            if square {
                // Its bands already hold its room.
            } else if (flow.pages.len(), flow.col) == start {
                // On the anchor's page the table blocks its band: later
                // lines that overlap it, the host's own included, move
                // below it.
                flow.bands.push(Band {
                    top: flow.page_y(y - mt),
                    bottom: flow.page_y(below),
                    left: span.0,
                    right: span.1,
                    ..Band::default()
                });
            } else {
                let col = flow.col;
                flow.fill[col] = flow.fill[col].max(below);
            }
            return false;
        }
        from = resume;
        carry = next_carry;
        if square {
            wait!();
        }
        flow.next_column();
        y = mt;
        repeat = header;
    }
}

/// Where a fragment starting at table height `from` with `avail` below it
/// ends when the rest does not fit: the lowest legal cut, except that rows
/// that cannot fit even a whole column (`column` high) split inside where
/// they stand rather than starting the next column.
fn fragment_end(
    t: &hm_model::Table,
    tl: &super::TableLayout,
    mode: SplitMode,
    from: i32,
    avail: i32,
    column: i32,
) -> Option<i32> {
    let by_rows = cut(t, tl, mode, from, from + avail);
    let at = by_rows.unwrap_or(from);
    let oversized =
        mode != SplitMode::Anywhere && at < tl.height && first_cut(t, tl, mode, at) - at > column;
    if oversized {
        cut(t, tl, SplitMode::Anywhere, from, from + avail).or(by_rows)
    } else {
        by_rows
    }
}

/// The region y at or below `y` where something spanning `span` starts
/// clear of the top-and-bottom objects on the page (those that begin at or
/// below region y `from`, when given): a paragraph that begins beside an earlier table starts
/// below it, and a table hanging from a paragraph below its own earlier
/// tables.
fn clear_bands(flow: &Flow, from: Option<i32>, mut y: i32, span: (i32, i32)) -> i32 {
    let from = from.map_or(i32::MIN, |f| flow.page_y(f));
    while let Some(b) = flow.bands.iter().find(|b| {
        !b.square
            && b.top >= from
            && b.left < span.1
            && span.0 < b.right
            && b.top <= flow.page_y(y)
            && flow.page_y(y) < b.bottom
    }) {
        y = b.bottom - flow.page_y(0);
    }
    y
}

fn push(flow: &mut Flow, at: PlacedTable) {
    flow.take_table_notes(at.control, at.top, at.bottom);
    if let Some(page) = flow.pages.last_mut() {
        page.tables.push(at);
    }
}
