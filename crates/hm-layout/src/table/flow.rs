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

use super::split::{SplitMode, cut, header_height};
use crate::objects::{Anchor, frame};
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
}

impl PlacedTable {
    /// Height the fragment takes on the page.
    pub fn height(&self) -> i32 {
        self.header + self.bottom - self.top
    }
}

/// True for a table that flows with the text: not set as a character,
/// anchored to its paragraph, wrapped top and bottom.
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
    let Some((k, c)) = para.controls.iter().enumerate().find(|(_, c)| flows(c)) else {
        return;
    };
    let (Some(t), Some(tl)) = (c.as_table(), pl.table(k as u32)) else {
        return;
    };
    let o = &t.header.common;
    let [_, _, mt, mb] = o.margins.map(i32::from);
    let start = top + o.y_offset.max(0) + mt;
    let avail = flow.region_height() - start;
    let mode = mode_for(t, tl.height, flow.body.h);
    let fits = if tl.height + mb <= avail {
        true
    } else {
        cut(t, tl, mode, 0, avail).is_some_and(|c| c < tl.height)
    };
    if !fits {
        flow.next_column();
    }
}

/// After line `li` of a paragraph is placed at region y `top`: places the
/// tables anchored in that line. Flowing tables start at the line's top and
/// push the flow below their last fragment.
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
    let Some(line) = pl.lines.get(li) else {
        return;
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
        if o.treat_as_char() {
            continue;
        }
        let anchor = Anchor {
            paper: flow.paper,
            body: flow.body,
            column: flow.column(),
            para_top: flow.page_y(top),
            para_left,
        };
        let x = box_x(&o, &anchor);
        let [_, _, mt, mb] = o.margins.map(i32::from);
        if !flows(c) {
            let [ml, mr, _, _] = o.margins.map(i32::from);
            let outer = ObjectCommon {
                width: ml + o.width + mr,
                height: mt + o.height + mb,
                ..o
            };
            let (_, y, _, _) = frame(&outer, &anchor);
            push(flow, pi, k as u32, x, y + mt, 0, tl.height, 0);
            continue;
        }
        let mode = mode_for(t, tl.height, flow.body.h);
        let header = header_height(t, tl);
        let mut y = (top + o.y_offset).max(below) + mt;
        let mut from = 0;
        let mut repeat = 0;
        loop {
            let avail = flow.region_height() - y - repeat;
            let whole = tl.height - from + mb <= avail;
            let c = if whole {
                Some(tl.height)
            } else {
                cut(t, tl, mode, from, from + avail)
            };
            let c = match c {
                Some(c) => c,
                // Nothing fits here: an empty column takes the smallest
                // piece it can, anything else hands over to the next one.
                None if !flow.has_content => smallest_cut(t, tl, mode, from),
                None => {
                    flow.next_column();
                    y = mt;
                    repeat = if from > 0 { header } else { 0 };
                    continue;
                }
            };
            push(flow, pi, k as u32, x, flow.page_y(y), from, c, repeat);
            flow.has_content = true;
            let end = y + repeat + (c - from);
            if c >= tl.height {
                below = end + mb;
                let col = flow.col;
                flow.fill[col] = flow.fill[col].max(below);
                break;
            }
            from = c;
            flow.next_column();
            y = mt;
            repeat = header;
        }
    }
}

/// The first legal cut below `from`, or the table end.
fn smallest_cut(t: &hm_model::Table, tl: &super::TableLayout, mode: SplitMode, from: i32) -> i32 {
    let mut lo = from + 1;
    let mut hi = tl.height;
    // Cuts are monotonic in the limit: find the smallest limit with a cut.
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if cut(t, tl, mode, from, mid).is_some() {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    cut(t, tl, mode, from, lo).unwrap_or(tl.height)
}

#[allow(clippy::too_many_arguments)]
fn push(flow: &mut Flow, pi: u32, k: u32, x: i32, y: i32, top: i32, bottom: i32, header: i32) {
    if let Some(page) = flow.pages.last_mut() {
        page.tables.push(PlacedTable {
            para: pi,
            control: k,
            x,
            y,
            top,
            bottom,
            header,
        });
    }
}
