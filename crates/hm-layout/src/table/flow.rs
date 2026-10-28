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
    CUT_ROOM, Carried, SplitMode, carried_break, carried_height, carried_spans, cut, cut_cells,
    first_cut, header_height, reflowed_break, resume_after, row_tail_break, spanning_break, stays,
    stored_page_restart,
};
use crate::objects::{Anchor, Band, frame};
use crate::paginate::Flow;
use crate::para::ParaLayout;

/// A table fragment on a page.
#[derive(Clone, Default)]
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
    /// Physical room below the logical source band.
    pub tail: i32,
    /// Merged lines selected by the physical cut beyond the row edge.
    pub tail_lines: bool,
    /// This fragment's grid after earlier physical tails spent merged minima.
    pub projected: Option<Arc<super::TableLayout>>,
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

impl std::fmt::Debug for PlacedTable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlacedTable")
            .field("para", &self.para)
            .field("control", &self.control)
            .field("x", &self.x)
            .field("y", &self.y)
            .field("top", &self.top)
            .field("bottom", &self.bottom)
            .field("tail", &self.tail)
            .field("tail_lines", &self.tail_lines)
            .field("row_y", &self.projected.as_ref().map(|p| &p.row_y))
            .field("header", &self.header)
            .field("caption", &self.caption)
            .field("carried", &self.carried)
            .field("cut", &self.cut)
            .finish()
    }
}

impl PartialEq for PlacedTable {
    fn eq(&self, other: &Self) -> bool {
        self.para == other.para
            && self.control == other.control
            && self.x == other.x
            && self.y == other.y
            && self.top == other.top
            && self.bottom == other.bottom
            && self.tail == other.tail
            && self.tail_lines == other.tail_lines
            && self.header == other.header
            && self.caption == other.caption
            && self.carried == other.carried
            && self.cut == other.cut
            && self.projected.as_ref().map(|p| &p.row_y)
                == other.projected.as_ref().map(|p| &p.row_y)
    }
}

impl Eq for PlacedTable {}

impl PlacedTable {
    /// Height the fragment takes on the page.
    pub fn height(&self) -> i32 {
        self.header + self.carried.as_ref().map_or(0, |c| c.height) + self.bottom - self.top
            + self.tail
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

/// The least height of a row's rest below a cut that Hancom for Mac
/// carries to the next page when the row holds no more lines: probes show a
/// rest of 1,281 HU dropped and one of 1,286 HU carried, whatever the cells'
/// padding.
const CARRY_REST: i32 = 1282;

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
    if pl.line_facts(para).any_content() {
        flow.unheld_table_page = None;
    }
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
    let caption = tl
        .caption
        .as_ref()
        .filter(|c| c.side == 3)
        .map_or(0, |c| c.gap + c.height);
    // The caption's frame reserves room before admitting either a whole
    // grid or a fragment. Ordinary outer margins keep their row-cut rule.
    let gap = tl.caption.as_ref().filter(|c| c.side == 3).map_or(0, |c| c.gap);
    let fits = tl.height + mb + caption <= avail
        || fragment_end(t, tl, mode, 0, avail - gap, flow.body.h - mt)
            .is_some_and(|c| c < tl.height);
    if !fits {
        flow.next_column();
    }
}

/// True when a line holds anything besides controls and the paragraph end.
/// True when a line holds something besides floating controls and the
/// paragraph end.
pub fn has_content(para: &Paragraph, line: &crate::para::Line) -> bool {
    para.units()
        .take_while(|&(pos, _)| pos < line.end)
        .any(|(pos, u)| pos >= line.start && unit_is_content(para, u))
}

/// [`has_content`] and [`line_controls`] of every line of one paragraph
/// layout, from one pass over the paragraph's units. Placing a long
/// paragraph asks for them line by line; scanning the units from the start
/// for each line made that quadratic in the paragraph's length.
#[derive(Debug, Default)]
pub struct LineFacts {
    content: Vec<bool>,
    controls: Vec<Vec<u32>>,
    any_content: bool,
}

impl LineFacts {
    /// Build once for the immutable paragraph and line membership.
    pub(crate) fn reset(&mut self, para: &Paragraph, lines: &[crate::para::Line]) {
        // Lines of a layout follow each other; others take the per-line scan.
        let ordered = lines.windows(2).all(|w| w[0].end <= w[1].start);
        if !ordered {
            self.content = lines.iter().map(|l| has_content(para, l)).collect();
            self.controls = lines.iter().map(|l| line_controls(para, l)).collect();
            self.any_content = self.content.iter().any(|&c| c);
            return;
        }
        self.content = vec![false; lines.len()];
        self.controls = vec![Vec::new(); lines.len()];
        let mut li = 0;
        for (pos, u) in para.units() {
            while li < lines.len() && pos >= lines[li].end {
                li += 1;
            }
            let Some(line) = lines.get(li) else {
                break;
            };
            if pos < line.start {
                continue;
            }
            if let hm_model::Unit::Extended { index, .. } = u {
                self.controls[li].push(index);
            }
            if !self.content[li] && unit_is_content(para, u) {
                self.content[li] = true;
            }
        }
        self.any_content = self.content.iter().any(|&c| c);
    }

    /// Whether line `li` of the last reset layout has content, as [`has_content`].
    pub(crate) fn content(&self, li: usize) -> bool {
        self.content[li]
    }

    /// Whether any line of the last reset layout has content.
    pub(crate) fn any_content(&self) -> bool {
        self.any_content
    }

    /// The extended controls in line `li`, as [`line_controls`].
    pub(crate) fn controls(&self, li: usize) -> &[u32] {
        &self.controls[li]
    }
}

fn unit_is_content(para: &Paragraph, u: hm_model::Unit) -> bool {
    match u {
        hm_model::Unit::Extended { index, .. } => para
            .controls
            .get(index as usize)
            .and_then(|c| c.object_common())
            .is_some_and(|o| o.treat_as_char()),
        hm_model::Unit::CharControl(13) => false,
        _ => true,
    }
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
    if !pl.line_facts(para).content(li) {
        return;
    }
    let column = flow.column().1;
    let mut waiting: Vec<u32> = std::mem::take(&mut flow.deferred.controls);
    let in_line = pl.line_facts(para).controls(li);
    waiting.extend(in_line.iter().copied().filter(|&k| {
        para.controls
            .get(k as usize)
            .is_some_and(|c| flows_in(c, column))
    }));
    let line_anchor = li > 0
        && waiting
            .iter()
            .all(|&k| crate::objects::anchor_line(para, pl, k as usize) == li);
    // The line goes below the tables it reaches, and from there may reach
    // more: each round places those and moves the line below them.
    let (cx, cw) = flow.column();
    let mut at = clear_bands(flow, None, top, (cx, cx + cw));
    let anchor_at = (flow.pages.len(), flow.col);
    loop {
        // Tables hang from the paragraph's top while it is on the same
        // column; a line that ends above one leaves it waiting.
        let same = flow.deferred.at == (flow.pages.len(), flow.col);
        let anchor_top = if same && !line_anchor {
            flow.deferred.top
        } else {
            top
        };
        let (wait, ready): (Vec<u32>, Vec<u32>) = waiting.iter().partition(|&&k| {
            let o = para.controls[k as usize]
                .object_common()
                .unwrap_or_default();
            same && at + line.height <= anchor_top + o.y_offset.max(0)
        });
        waiting = wait;
        if ready.is_empty() {
            break;
        }
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
    // A content-less host line whose own table hangs from its top or below
    // it, within its height, goes below the table when Hancom stored the
    // line across the table: one it kept beside the table it stored at no
    // width or clear of it.
    let cx = flow.column().0;
    if let Some(line) = pl.lines.get(li)
        && !pl.line_facts(para).content(li)
        && let Some(seg) = para.line_segs.get(li).filter(|s| s.width > 0)
        && let Some(bottom) = flow
            .bands
            .get(bands..)
            .unwrap_or(&[])
            .iter()
            .filter(|b| b.top >= flow.page_y(top) && b.top < flow.page_y(top + line.height))
            .filter(|b| b.left < cx + seg.col_start + seg.width && cx + seg.col_start < b.right)
            .map(|b| b.bottom)
            .max()
    {
        // Tables stacked under that one by Hancom's flow push the line on.
        let mut stacked: Vec<(i32, i32)> = flow
            .bands
            .get(bands..)
            .unwrap_or(&[])
            .iter()
            .map(|b| (b.top, b.bottom))
            .collect();
        stacked.sort_unstable();
        let bottom = stacked.iter().fold(
            bottom,
            |acc, &(t, b)| if t <= acc { acc.max(b) } else { acc },
        );
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
        .take_while(|&(pos, _)| pos < line.end)
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
    if li >= pl.lines.len() {
        return;
    }
    let content = pl.line_facts(para).content(li);
    // Tables hang from the paragraph's top, above its space before.
    let top = if li == 0 {
        (top - pl.before).max(0)
    } else {
        top
    };
    let column = flow.column().1;
    let chosen: Vec<u32> = pl
        .line_facts(para)
        .controls(li)
        .iter()
        .copied()
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
    let unheld = !flow.hold_table_anchors && !pl.line_facts(para).any_content();
    let original_page = if unheld {
        flow.unheld_table_page.unwrap_or(flow.pages.len())
    } else {
        flow.pages.len()
    };
    let mut anchor_at = (flow.pages.len(), flow.col);
    let mut top = top;
    // A paragraph whose top lies inside an earlier frame: its tables clear
    // that frame too, while those hanging below it keep their offset.
    let mut from = if flow.deferred.at == anchor_at && flow.deferred.top > top {
        None
    } else {
        Some(top)
    };
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
            paper_y: None,
            flow_top: None,
            body: flow.body,
            column: flow.column(),
            para_top: flow.page_y(top),
            para_left,
            para_right: pl.right,
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
                    tail: 0,
                    tail_lines: false,
                    projected: None,
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
            from = Some(top);
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
        // A table stacked below an earlier one of its line that does not
        // fit whole below it starts the next page, as one carried past its
        // anchor does.
        let natural = (top + o.y_offset).max(top);
        let start = clear_bands(flow, Some(top), natural.max(below), span);
        let stacked_below = start > natural
            && (flow.pages.len(), flow.col) == anchor_at
            && start + mt + tl.outer_size().1 + mb > flow.region_height();
        let passed_anchor = unheld && original_page < flow.pages.len();
        let fresh = (flow.pages.len(), flow.col) != anchor_at || stacked_below || passed_anchor;
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
                // A table still waiting for the next page goes there first;
                // this one starts the page after its last fragment.
                flow.flush_tables();
                flow.next_column();
                stacked.clear();
            }
            (if passed_anchor { 0 } else { o.y_offset.max(0) }) + mt + gy
        } else {
            clear_bands(flow, from, (top + o.y_offset).max(below), span) + mt + gy
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
            fresh_cuts: false,
            projected: None,
            spent: Vec::new(),
            start: (flow.pages.len(), flow.col),
            passes: 0,
            bound: 4 * (tl.cells.iter().map(|c| c.lines.len() + 1).sum::<usize>() + tl.row_y.len()),
        };
        flow.unheld_table_page = unheld.then_some(original_page);
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
    /// Refresh subsequent cuts after a recomposed imported cell fragment.
    fresh_cuts: bool,
    projected: Option<Arc<super::TableLayout>>,
    spent: Vec<i32>,
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
    base: &super::TableLayout,
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
    let mut projected = st.projected.clone();
    let square = o.text_wrap() == 0;
    // The rest waits for the text to move on to the next column.
    macro_rules! wait {
        () => {{
            st.y = mt;
            st.from = from;
            st.repeat = if from > 0 { header } else { 0 };
            st.carry = carry;
            st.projected = projected.clone();
            return true;
        }};
    }
    loop {
        let tl = projected.as_deref().unwrap_or(base);
        st.passes += 1;
        if st.passes > bound {
            return false;
        }
        // Every phase of a fragment, including carried rows, keeps the
        // bottom caption's frame room. Its text appears only at the end.
        let avail = flow.region_height() - y - repeat - cap_below;
        // Cells carried over from a cut through their row come first,
        // split again when even they do not fit.
        let mut carried = None;
        if let Some(cs) = carry.take_if(|cs| cs.spanning) {
            from = cs.until;
            carried = Some(cs);
        }
        // Carried cells with no lines left close their row: the rest of
        // its height, when it fits.
        if let Some(cs) = carry.take_if(|cs| cs.cells.iter().all(|c| c.1.is_none())) {
            let rest = cs.until - cs.end;
            if rest > 0 && rest <= avail - mb {
                carried = Some(Carried { height: rest, ..cs });
            }
            from = cs.until;
        }
        if let Some(cs) = carry.take() {
            let cell = |k: u32| &tl.cells[k as usize];
            let full = cs
                .cells
                .iter()
                .filter(|c| c.1.is_some())
                .map(|&(k, f)| carried_height(cell(k), &carried_spans(tl, k as usize, cs.until), f))
                .max()
                .unwrap_or(0);
            let room = avail - mb;
            // A row kept taller than its cells' content keeps its height:
            // the rest goes on as what the cut left of the row below the
            // fragment before. Top-aligned cells keep it only when more
            // of it stands empty than Hancom drops of a row's rest.
            let roomy = cs.cells.iter().all(|&(k, _)| {
                let cl = cell(k);
                cl.h > cl.padding[2] + cl.content + cl.padding[3]
            });
            let aligned = cs.cells.iter().all(|&(k, _)| {
                t.cells
                    .get(k as usize)
                    .is_some_and(|c| c.vertical_align() != 0)
            });
            let slack = roomy && (aligned || cs.until - cs.end - full > CARRY_REST) && cs.end > 0;
            let full = if slack && full.max(cs.until - cs.end) <= room {
                full.max(cs.until - cs.end)
            } else {
                full
            };
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
                            carried_break(
                                cell(k),
                                &carried_spans(tl, k as usize, cs.until),
                                f,
                                room,
                                force,
                            ),
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
                // The minimum keeps filling fragments while it crosses
                // the cap. Once it fits, content sets the fragment height.
                let cap = (room - CUT_ROOM).max(0);
                let kept = cs
                    .cells
                    .iter()
                    .zip(&next)
                    .filter_map(|(&(k, f), &(_, nb))| {
                        let (cl, f) = (cell(k), f?);
                        let bottom = carried_spans(tl, k as usize, until)
                            .iter()
                            .filter(|&&(t, _)| t >= f && nb.is_none_or(|nb| t < nb))
                            .map(|&(_, b)| b)
                            .max()?;
                        Some(cl.padding[2] + bottom - f + cl.padding[3])
                    })
                    .max()
                    .unwrap_or(room)
                    .max(if cs.minimum >= cap { cap } else { 0 })
                    .min(room.max(0));
                let minimum = (cs.minimum - kept).max(0);
                push(
                    flow,
                    PlacedTable {
                        para: pi,
                        control: k,
                        x: x_in(flow, st),
                        y: flow.page_y(y),
                        top: until,
                        bottom: until,
                        tail: 0,
                        tail_lines: false,
                        projected: projected.clone(),
                        header: repeat,
                        caption: false,
                        carried: Some(Arc::new(Carried {
                            height: kept,
                            next: next.clone(),
                            ..cs
                        })),
                        cut: None,
                    },
                );
                flow.has_content = true;
                if next.iter().any(|c| c.1.is_some()) {
                    carry = Some(Carried {
                        cells: next,
                        until,
                        minimum,
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
        let cap_ink = tl.caption.as_ref().filter(|c| c.side == 3).map_or(0, |c| c.height);
        // The table's bottom margin counts where it ends; a fragment cut
        // at a row edge may reach the page end. The cell spacing below the
        // last row may run past it.
        let rows_end = tl.row_y.last().copied().unwrap_or(tl.height).max(from);
        // A continued table whose last line fits ends here: the bottom
        // padding and margin below it may run past the page end.
        let content_end = (0..tl.cells.len())
            .filter_map(|k| tl.cell_spans(k).iter().map(|s| s.1).max())
            .max()
            .unwrap_or(0);
        let c = if rows_end - from + mb <= avail
            || (from > 0 && content_end > from && content_end - from <= avail)
        {
            Some(tl.height)
        } else {
            // A cut between rows keeps the last row only when it ends,
            // with the bottom margin, more than CUT_ROOM above the page end.
            let end = |avail: i32| {
                // Above a bottom caption the table's bottom margin stays on
                // the page with the caption's gap (hwpspec table 328 p50:
                // first frame 9739 in 13190 HU, margin 1984, gap 852).
                let room = if mode == SplitMode::Anywhere {
                    avail - if cap_below > 0 { mb } else { 0 }
                } else {
                    avail - mb - CUT_ROOM - 1
                };
                fragment_end(t, tl, mode, from, room, flow.body.h - mt)
            };
            // A fragment that does not end the table keeps only its bottom
            // caption's gap; the caption's text needs room with the last one.
            match end(avail + cap_ink) {
                Some(c) if c >= tl.height => end(avail),
                c => c,
            }
        };
        // Repeated title rows go whole: a first fragment ends below them.
        let c = c.filter(|&c| from > 0 || c >= header);
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
        let mut limit = from + avail - mb + if last { 0 } else { cap_ink };
        let mut cut_at = None;
        let mut tail_lines = false;
        let mut partial = (!last && mode == SplitMode::Anywhere)
            .then(|| cut_cells(t, tl, from, limit))
            .flatten();
        // A cut that leaves nothing of the table ends it here, so the
        // caption's text needs its room too.
        if !last
            && cap_ink > 0
            && partial
                .as_ref()
                .is_some_and(|p| p.cells.iter().all(|c| c.1.is_none()) && p.until >= tl.height)
        {
            limit -= cap_ink;
            partial = cut_cells(t, tl, from, limit);
        }
        st.fresh_cuts |= limit <= c && partial.as_ref().is_some_and(|p| reflowed_break(t, tl, p));
        let admitted_start = partial.is_some()
            && tl.cells.iter().enumerate().any(|(k, cl)| {
                tl.cell_spans(k).first().is_some_and(|&(a, b)| {
                    !stays(cl, b, limit) && tl.first_line_stays(k, from, a, b, limit)
                })
            });
        let rejected_start = partial.is_none()
            && tl
                .row_y
                .iter()
                .rev()
                .find(|&&edge| edge <= limit - CUT_ROOM.max(tl.bottom_room))
                .is_some_and(|&row_top| {
                    tl.cells.iter().enumerate().any(|(k, cl)| {
                        cl.y == row_top + tl.spacing
                            && cl.y >= from
                            && tl.cell_spans(k).first().is_some_and(|&(a, b)| {
                                a < c && b + cl.padding[3] > limit - tl.bottom_room
                            })
                    })
                });
        let reserved =
            tl.bottom_room > 0 && c > limit - tl.bottom_room && tl.row_y.binary_search(&c).is_ok();
        // Hancom's own cache shows the cell resuming on the next page there.
        let restarted = partial
            .as_ref()
            .is_some_and(|p| stored_page_restart(t, tl, p, limit));
        // An imported merged-cell line restarted by Hancom at the next row
        // edge resumes there (aift table 901 p68/69: native resumes at row
        // edge 32583 and the following anchor is 8774).
        let spanning = (!last
            && mode == SplitMode::Anywhere
            && partial.is_none()
            && flow.table_notes_mut().is_empty())
        .then(|| {
            spanning_break(
                t,
                tl,
                from,
                c,
                limit,
                flow.region_height() - mt - mb - header - cap_below,
            )
        })
        .flatten();
        let (c, resume, next_carry) = match (last, mode) {
            (false, SplitMode::Anywhere) if spanning.is_some() => {
                let cs = spanning.unwrap();
                cut_at = Some(limit);
                (cs.until, cs.until, Some(cs))
            }
            // Recomputed cell fragments invalidate the cached row's
            // admission at the bottom margin, even when its edge fits
            // before the physical page end.
            (false, SplitMode::Anywhere)
                if limit > c
                    || st.fresh_cuts
                    || reserved
                    || rejected_start
                    || admitted_start
                    || restarted =>
            {
                match partial {
                    // Nothing left in the cut row: go on with the next,
                    // unless more of the row than CARRY_REST stands below
                    // the cut, which Hancom carries on empty.
                    Some(cut) if cut.cells.iter().all(|c| c.1.is_none()) => {
                        cut_at = Some(limit);
                        if cut.until - cut.end > CARRY_REST {
                            (cut.end, cut.end, Some(cut))
                        } else {
                            (cut.end, cut.until, None)
                        }
                    }
                    Some(cut) => {
                        cut_at = Some(limit);
                        (cut.end, cut.end, Some(cut))
                    }
                    // The row moves on whole when it can: the fragment
                    // ends at the last cut above it.
                    None => {
                        let cap = limit - CUT_ROOM.max(tl.bottom_room);
                        let row_top = tl.row_y.iter().rev().find(|&&y| y <= cap).copied();
                        if let Some(cs) = row_top
                            .filter(|&top| carried.is_none() && top > from && c > top)
                            .and_then(|top| row_tail_break(t, tl, from, top, limit))
                        {
                            cut_at = Some(limit);
                            tail_lines = true;
                            (cs.until, cs.until, Some(cs))
                        } else {
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
            }
            (false, SplitMode::Anywhere) => (c, resume_after(tl, c), None),
            _ => (c, c, None),
        };
        // A cut through the last row that leaves nothing of it ends the
        // table here.
        let last = last || (next_carry.is_none() && resume >= tl.height);
        // When the remaining text fits, a continued table closes at the
        // page end instead of carrying its unused stored height past it.
        let c = if last && from > 0 {
            c.min((from + avail - mb).max(content_end))
        } else {
            c
        };
        // A merged cell continuing past a row edge closes at the physical
        // cut, the bottom room left, when the next row cannot start. Keep
        // its source band: only the last admitted row's boxes gain the
        // unused room, also after cells carried to the fragment's top.
        // A bottom caption's frame holds the page end, so the rows above
        // it keep their own height.
        let edge = !last
            && cap_below == 0
            && mode == SplitMode::Anywhere
            && c > from.max(header)
            && tl.row_y.binary_search(&c).is_ok()
            && t.cells
                .iter()
                .zip(&tl.cells)
                .any(|(cell, cl)| cell.row_span > 1 && cl.y < c && c < cl.y + cl.h)
            && tl.cells.iter().enumerate().any(|(k, cl)| {
                cl.y == c + tl.spacing
                    && tl
                        .cell_spans(k)
                        .first()
                        .is_some_and(|&(_, b)| !stays(cl, b, limit))
            })
            && tl
                .cells
                .iter()
                .all(|cl| !cl.vertical || cl.y >= c || cl.y + cl.h < c);
        let tail = if edge {
            // Rows grown by their content may already fulfill a spanning
            // cell's authored minimum before that cell's final row.
            let remaining = t
                .cells
                .iter()
                .zip(&tl.cells)
                .enumerate()
                .filter(|(_, (cell, cl))| cell.row_span > 1 && cl.y < c && c < cl.y + cl.h)
                .map(|(k, (cell, cl))| {
                    (cell.height - st.spent.get(k).copied().unwrap_or(0) - (c - cl.y)).max(0)
                })
                .max()
                .unwrap_or(0);
            (limit - tl.bottom_room - c).max(0).min(remaining)
        } else {
            0
        };
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
                tail,
                tail_lines,
                projected: projected.clone(),
                header: repeat,
                caption,
                carried: carried.map(Arc::new),
                cut: cut_at,
            },
        );
        flow.has_content = true;
        let end = y + repeat + lead + (c - from) + tail + if last { cap_below } else { 0 };
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
            } else if (flow.pages.len(), flow.col) == start || (from > 0 && cap_below > 0) {
                // A bottom caption's paragraphs block their frame, consuming
                // following spacing that overlaps it. A continuation's
                // padding and margin past the page end take no extra room.
                let below = if (flow.pages.len(), flow.col) == start {
                    below
                } else {
                    below.min(flow.region_height())
                };
                flow.bands.push(Band {
                    top: flow.page_y(y - mt),
                    bottom: flow.page_y(below),
                    left: span.0,
                    right: span.1,
                    ..Band::default()
                });
            } else {
                // A table continued without a bottom caption, or deferred
                // whole from its anchor, advances the paragraph cursor.
                let col = flow.col;
                let text = flow.fill[col];
                flow.fill[col] = text.max(below.min(flow.region_height()));
                flow.continued_floor = Some((flow.pages.len(), col, text, flow.fill[col]));
            }
            return false;
        }
        // A row-edge tail counts toward every merged minimum crossing it.
        // A later row must not ask for that physical room a second time.
        // Ordinary carried rows and future notes retain their own accounting.
        if tail > 0
            && next_carry.as_ref().is_none_or(|cs| cs.spanning)
            && flow.table_notes_mut().iter().all(|n| n.0 != k)
            && tl
                .cells
                .iter()
                .filter(|cl| cl.y < c && c < cl.y + cl.h)
                .all(|cl| cl.tables.is_empty() && cl.objects.is_empty() && !cl.vertical)
        {
            st.spent.resize(t.cells.len(), 0);
            for ((wire, cl), spent) in t.cells.iter().zip(&tl.cells).zip(&mut st.spent) {
                if wire.row_span > 1 && cl.y < c && c < cl.y + cl.h {
                    *spent = (*spent + tail).min(wire.height.max(0));
                }
            }
            if let Some(next) = base.with_spent_minima(t, &st.spent) {
                projected = Some(Arc::new(next));
            }
        }
        // A cut row's rest dropped at the page end is no physical room: a
        // merged cell running on past it still asks its full minimum.
        if resume > c && next_carry.is_none() && flow.table_notes_mut().iter().all(|n| n.0 != k) {
            let grid = projected.as_deref().unwrap_or(base);
            let crossing: Vec<usize> = (0..t.cells.len())
                .filter(|&k| {
                    let cl = &grid.cells[k];
                    t.cells[k].row_span > 1
                        && cl.y < c
                        && resume < cl.y + cl.h
                        && cl.tables.is_empty()
                        && cl.objects.is_empty()
                        && !cl.vertical
                })
                .collect();
            if !crossing.is_empty() {
                st.spent.resize(t.cells.len(), 0);
                for k in crossing {
                    st.spent[k] -= resume - c;
                }
                if let Some(next) = base.with_spent_minima(t, &st.spent) {
                    projected = Some(Arc::new(next));
                }
            }
        }
        // What a merged cell's minimum still asks beyond its natural rows
        // below the edge goes on empty at the next fragment's top, as the
        // rest of the row cut at the page end, instead of growing the
        // merged cell's last row.
        let next_carry = match next_carry {
            None if edge && flow.table_notes_mut().iter().all(|n| n.0 != k) => {
                let grid = projected.as_deref().unwrap_or(base);
                let crossing: Vec<usize> = (0..t.cells.len())
                    .filter(|&k| {
                        let cl = &grid.cells[k];
                        t.cells[k].row_span > 1 && cl.y < c && c < cl.y + cl.h
                    })
                    .collect();
                let plain = crossing.iter().all(|&k| {
                    let cl = &grid.cells[k];
                    cl.tables.is_empty() && cl.objects.is_empty() && !cl.vertical
                });
                let mut free = st.spent.clone();
                free.resize(t.cells.len(), 0);
                for &k in &crossing {
                    free[k] = t.cells[k].height.max(0);
                }
                let natural = base.with_spent_minima(t, &free);
                let natural = natural.as_ref().unwrap_or(base);
                let rows = natural.row_y.len() - 1;
                let extra = match grid.row_y.binary_search(&c) {
                    Ok(rc) if plain => crossing
                        .iter()
                        .map(|&k| {
                            let (cell, cl) = (&t.cells[k], &grid.cells[k]);
                            let r1 = (cell.end_row() as usize).clamp(rc, rows);
                            let rest =
                                cell.height - st.spent.get(k).copied().unwrap_or(0) - (c - cl.y);
                            rest - (natural.row_y[r1] - natural.row_y[rc])
                        })
                        .max()
                        .unwrap_or(0),
                    _ => 0,
                };
                if extra > 0 {
                    let cells = (0..t.cells.len())
                        .filter(|&k| {
                            let cl = &grid.cells[k];
                            cl.y < c && c <= cl.y + cl.h
                        })
                        .map(|k| (k as u32, None))
                        .collect();
                    st.spent.resize(t.cells.len(), 0);
                    for &k in &crossing {
                        st.spent[k] = (st.spent[k] + extra).min(t.cells[k].height.max(0));
                    }
                    if let Some(next) = base.with_spent_minima(t, &st.spent) {
                        projected = Some(Arc::new(next));
                    }
                    Some(Carried {
                        cells,
                        until: c,
                        end: c - extra,
                        ..Carried::default()
                    })
                } else {
                    None
                }
            }
            other => other,
        };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memoized_layouts_share_lazy_facts_but_changed_controls_and_text_do_not() {
        use crate::para::{ParaEnv, format};
        let table = |inline| {
            let mut table = hm_model::Table::default();
            table.header.common.attr = u32::from(inline);
            hm_model::Control {
                kind: hm_model::ControlKind::Table(Box::new(table)),
                src: Some(vec![1].into()),
            }
        };
        let mut para = Paragraph {
            text: vec![11, 0, 0, 0, 0, 0, 0, 11],
            controls: vec![table(false), table(true)],
            src: Some(vec![2].into()),
            ..Default::default()
        };
        para.text.extend("가😀".encode_utf16());
        para.text.push(10);
        para.text.extend([11, 0, 0, 0, 0, 0, 0, 11, 13]);
        let (info, fonts) = (hm_model::DocInfo::default(), hm_fonts::FontSystem::new());
        let mut styles = crate::style::Styles::new(&info, &fonts);
        styles.memo = Some(crate::incr::Memo::default());
        let env = ParaEnv {
            column_width: 100_000,
            default_tab: 4000,
        };
        let first = format(&para, &info, &mut styles, env, None);
        assert!(
            first.line_facts.get().is_none(),
            "formatting alone does not scan facts"
        );
        let facts = first.line_facts(&para);
        let cached = format(&para, &info, &mut styles, env, None);
        assert!(
            std::ptr::eq(facts, cached.line_facts(&para)),
            "the memo's earlier clone shares later initialization"
        );
        let matches = |para: &Paragraph, layout: &ParaLayout| {
            let facts = layout.line_facts(para);
            for (i, line) in layout.lines.iter().enumerate() {
                assert_eq!(facts.content(i), has_content(para, line));
                assert_eq!(facts.controls(i), line_controls(para, line));
            }
        };
        matches(&para, &first);
        assert!(facts.content(first.lines.len() - 1));
        let hm_model::ControlKind::Table(table) = &mut para.controls[1].kind else {
            unreachable!()
        };
        table.header.common.attr &= !1;
        para.controls[1].src = None;
        let changed = format(&para, &info, &mut styles, env, None);
        matches(&para, &changed);
        assert!(!changed.line_facts(&para).content(changed.lines.len() - 1));
        assert!(
            facts.content(first.lines.len() - 1),
            "another layout cannot overwrite the memoized facts"
        );
        let at = changed.lines.last().unwrap().start as usize;
        para.text.insert(at, 'A' as u16);
        para.mark_all_dirty();
        let changed = format(&para, &info, &mut styles, env, None);
        matches(&para, &changed);
        assert!(changed.line_facts(&para).content(changed.lines.len() - 1));
    }

    #[test]
    fn line_facts_match_wire_scans_after_paragraph_and_layout_changes() {
        let table = |inline| {
            let mut table = hm_model::Table::default();
            table.header.common.attr = u32::from(inline);
            hm_model::Control {
                kind: hm_model::ControlKind::Table(Box::new(table)),
                src: None,
            }
        };
        let mut para = Paragraph {
            text: vec![11, 0, 0, 0, 0, 0, 0, 11],
            controls: vec![table(false), table(true)],
            ..Default::default()
        };
        para.text.extend("가😀".encode_utf16());
        para.text.extend([9, 0, 0, 0, 0, 0, 0, 9]);
        para.text.extend([11, 0, 0, 0, 0, 0, 0, 11, 13]);
        let line = |(start, end)| crate::para::Line {
            start,
            end,
            ..Default::default()
        };
        let original: Vec<_> = [(0, 8), (8, 11), (11, 19), (19, 27), (27, 28)]
            .into_iter()
            .map(line)
            .collect();
        let mut facts = LineFacts::default();
        facts.reset(&para, &original);
        assert_eq!(facts.content, [false, true, true, true, false]);
        assert_eq!(facts.controls, [vec![0], vec![], vec![], vec![1], vec![]]);

        let layouts = [
            original.clone(),
            vec![line((0, 28))],
            vec![line((0, 19)), line((8, 27)), line((27, 28))],
            original.into_iter().rev().collect(),
            vec![line((8, 8)), line((19, 27))],
            vec![],
        ];
        // Reuse one facts object across reflows and another paragraph.
        for para in [
            para,
            Paragraph {
                text: vec![13],
                ..Default::default()
            },
        ] {
            for lines in &layouts {
                facts.reset(&para, lines);
                for (li, line) in lines.iter().enumerate() {
                    assert_eq!(facts.content(li), has_content(&para, line));
                    assert_eq!(facts.controls(li), line_controls(&para, line));
                }
                assert_eq!(
                    facts.any_content(),
                    lines.iter().any(|l| has_content(&para, l))
                );
            }
        }
    }
}
