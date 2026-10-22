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

use super::split::{SplitMode, cut, first_cut, header_height};
use crate::objects::{Anchor, Band, frame};
use crate::paginate::Flow;
use crate::para::ParaLayout;

/// A table fragment on a page.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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
}

impl PlacedTable {
    /// Height the fragment takes on the page.
    pub fn height(&self) -> i32 {
        self.header + self.bottom - self.top
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
    let fits = tl.height + mb <= avail
        || fragment_end(t, tl, mode, 0, avail, flow.body.h - mt).is_some_and(|c| c < tl.height);
    if !fits {
        flow.next_column();
    }
}

/// True when a line holds anything besides controls and the paragraph end.
fn has_content(para: &Paragraph, line: &crate::para::Line) -> bool {
    para.units().any(|(pos, u)| {
        pos >= line.start
            && pos < line.end
            && !matches!(
                u,
                hm_model::Unit::Extended { .. } | hm_model::Unit::CharControl(13)
            )
    })
}

/// Before line `li` of a paragraph is placed, its natural top at region y
/// `top`: a line with content makes way for the flowing tables anchored in
/// it, so those are placed first and the line goes below them.
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
    if pl.lines.get(li).is_some_and(|l| has_content(para, l)) {
        place_line_tables(flow, pi, para, pl, li, top, para_left, true);
    }
}

/// After line `li` of a paragraph is placed at region y `top`: places the
/// other tables anchored in it. A line of nothing but controls stays where
/// it is, under its flowing tables.
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
    place_line_tables(flow, pi, para, pl, li, top, para_left, false);
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
    let in_line: Vec<u32> = para
        .units()
        .filter_map(|(pos, u)| match u {
            hm_model::Unit::Extended { index, .. } if pos >= line.start && pos < line.end => {
                Some(index)
            }
            _ => None,
        })
        .collect();
    let mut below = top;
    for (k, c) in para.controls.iter().enumerate() {
        if !in_line.contains(&(k as u32)) {
            continue;
        }
        let (Some(t), Some(tl)) = (c.as_table(), pl.table(k as u32)) else {
            continue;
        };
        let o = t.header.common;
        let flowing = flows_in(c, flow.column().1);
        if o.treat_as_char() || (flowing && content != flowing_only) || (flowing_only && !flowing) {
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
                    control: k as u32,
                    x: x + gx,
                    y: y + mt + gy,
                    top: 0,
                    bottom: tl.height,
                    header: 0,
                    caption: tl.caption.is_some(),
                },
            );
            continue;
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
        let mut y = (top + o.y_offset).max(below) + mt + gy;
        let mut from = 0;
        let mut repeat = 0;
        let start = (flow.pages.len(), flow.col);
        loop {
            let avail = flow.region_height() - y - repeat;
            let c = if tl.height - from + mb <= avail {
                Some(tl.height)
            } else {
                fragment_end(t, tl, mode, from, avail, flow.body.h - mt)
            };
            let c = match c {
                Some(c) => c,
                // Nothing fits here: an empty column takes the smallest
                // piece it can, anything else hands over to the next one.
                None if !flow.has_content => first_cut(t, tl, mode, from),
                None => {
                    flow.next_column();
                    y = mt;
                    repeat = if from > 0 { header } else { 0 };
                    continue;
                }
            };
            let last = c >= tl.height;
            let caption = tl
                .caption
                .as_ref()
                .is_some_and(|cap| if cap.side == 3 { last } else { from == 0 });
            push(
                flow,
                PlacedTable {
                    para: pi,
                    control: k as u32,
                    x,
                    y: flow.page_y(y),
                    top: from,
                    bottom: c,
                    header: repeat,
                    caption,
                },
            );
            flow.has_content = true;
            let end = y + repeat + (c - from) + if last { cap_below } else { 0 };
            if last {
                below = end + mb;
                if (flow.pages.len(), flow.col) == start {
                    // On the anchor's page the table blocks its band: later
                    // lines that overlap it, the host's own included, move
                    // below it.
                    let [ml, mr, _, _] = o.margins.map(i32::from);
                    flow.bands.push(Band {
                        top: flow.page_y(y - mt),
                        bottom: flow.page_y(below),
                        left: x - ml,
                        right: x + tl.width + mr,
                        square: false,
                    });
                } else {
                    let col = flow.col;
                    flow.fill[col] = flow.fill[col].max(below);
                }
                break;
            }
            from = c;
            flow.next_column();
            y = mt;
            repeat = header;
        }
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

fn push(flow: &mut Flow, at: PlacedTable) {
    flow.take_table_notes(at.control, at.top, at.bottom);
    if let Some(page) = flow.pages.last_mut() {
        page.tables.push(at);
    }
}
