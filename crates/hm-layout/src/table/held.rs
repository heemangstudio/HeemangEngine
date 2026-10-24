//! Floating objects held in a cell's text.
//!
//! An object anchored to a cell paragraph with top-and-bottom or square
//! wrapping takes room in the cell: the cell's content reaches down to its
//! outer bottom (margins and a caption above or below included). An object
//! kept inside the page that may not overlap others moves down below every
//! earlier one in the cell it would cover, so a pile of pictures stored at nearly the same
//! offset stands as a column, one under the other, and the table splits
//! between them.

use hm_model::{DocInfo, ObjectCommon, Paragraph};

use crate::objects::{Anchor, place_one};
use crate::paginate::Rect;
use crate::para::ParaLayout;

/// An object placed in a cell's content.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CellObject {
    pub para: u32,
    pub control: u32,
    /// Outer box (margins and caption included), from the content's top left.
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    /// Wrapped square rather than top and bottom.
    pub square: bool,
    /// How far the object moved down from its stored place to clear the
    /// objects above it.
    pub shift: i32,
}

impl CellObject {
    fn covers(&self, x: i32, y: i32, w: i32, h: i32) -> bool {
        self.x < x + w && x < self.x + self.w && self.y < y + h && y < self.y + self.h
    }
}

/// Places the held objects of cell paragraph `p` (index `pi`, its top at
/// `para_top`, laid out `width` wide), after the cell's earlier ones in
/// `placed`. Returns the lowest outer bottoms among them, `para_top` when
/// there are none: of the top-and-bottom ones, which what follows goes
/// below, and of all, which the content reaches down to.
#[allow(clippy::too_many_arguments)]
pub(super) fn hold(
    info: &DocInfo,
    p: &Paragraph,
    pi: u32,
    pl: &ParaLayout,
    para_top: i32,
    width: i32,
    inline_table: bool,
    placed: &mut Vec<CellObject>,
) -> (i32, i32) {
    let (mut floor, mut reach) = (para_top, para_top);
    let big = 1 << 28;
    let anchor = Anchor {
        paper: (width, big),
        paper_x: None,
        body: Rect {
            x: 0,
            y: 0,
            w: width,
            h: big,
        },
        column: (0, width),
        para_top,
        para_left: para_left(info, p),
    };
    for (k, c) in p.controls.iter().enumerate() {
        let Some(o) = c.object_common() else {
            continue;
        };
        if !held(&o) || c.as_object().is_none() || (inline_table && escapes_cell(&o)) {
            continue;
        }
        let Some(at) = place_one(p, pi, k as u32, &anchor) else {
            continue;
        };
        let [ml, mr, mt, mb] = o.margins.map(i32::from);
        // A height kept relative to the page is not known here; the stored
        // one stands in.
        let h = if (o.attr >> 18) & 3 >= 2 {
            at.rect.h
        } else {
            o.height
        };
        let (above, below) = pl
            .measured
            .objects
            .iter()
            .find(|(i, _)| *i as usize == k)
            .and_then(|(_, l)| l.caption.as_ref())
            .map_or((0, 0), |cap| match cap.side {
                2 => (cap.height + cap.gap, 0),
                3 => (0, cap.height + cap.gap),
                _ => (0, 0),
            });
        let (x, w) = (at.rect.x - ml, ml + at.rect.w + mr);
        let top = at.rect.y - mt - above;
        let h = above + mt + h + mb + below;
        let mut y = top;
        if super::flow::avoids(&o) {
            while let Some(b) = placed
                .iter()
                .filter(|q| q.covers(x, y, w, h))
                .map(|q| q.y + q.h)
                .max()
            {
                y = b;
            }
        }
        placed.push(CellObject {
            para: pi,
            control: k as u32,
            x,
            y,
            w,
            h,
            square: o.text_wrap() == 0,
            shift: y - top,
        });
        reach = reach.max(y + h);
        if o.text_wrap() == 1 {
            floor = floor.max(y + h);
        }
    }
    (floor, reach)
}

/// The left edge paragraph-relative objects measure from: the paragraph's
/// left margin. Its lines may start further right, beside square objects.
pub fn para_left(info: &DocInfo, p: &Paragraph) -> i32 {
    info.para_shape(p.header.para_shape as u32)
        .map_or(0, |s| s.left(1000))
}

/// Overlays take part in the cell's alignment extent without growing its
/// row. Their paragraph anchor precedes the cell's vertical alignment.
pub(super) fn alignment_reach(c: &hm_model::Cell, cl: &super::CellLayout) -> i32 {
    let mut reach = cl.content;
    for (pi, (p, pl)) in c.paras.iter().zip(&cl.paras).enumerate() {
        let Some(first) = cl.lines.iter().find(|l| l.para == pi as u32) else {
            continue;
        };
        for o in p.controls.iter().filter_map(|c| c.object_common()) {
            if !o.treat_as_char() && o.vert_rel_to() == 2 && matches!(o.text_wrap(), 2 | 3) {
                reach = reach.max(first.y - pl.before + o.y_offset + o.height);
            }
        }
    }
    reach
}

/// True for an object that takes room in a cell: anchored to its paragraph,
/// not set as a character, with top-and-bottom or square wrapping.
fn held(o: &ObjectCommon) -> bool {
    !o.treat_as_char() && o.vert_rel_to() == 2 && matches!(o.text_wrap(), 0 | 1)
}

/// A top-and-bottom drawing free of an inline table's cell wraps the
/// enclosing paragraph instead. Bit 13 confines paragraph-relative objects
/// to their text area; without it, the cell does not grow around the drawing.
pub fn escapes_cell(o: &ObjectCommon) -> bool {
    !o.treat_as_char() && o.vert_rel_to() == 2 && o.text_wrap() == 1 && o.attr & (1 << 13) == 0
}
