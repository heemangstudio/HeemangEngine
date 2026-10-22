//! Floating objects that push text: where they sit and which bands of the
//! page they block.
//!
//! Only top-and-bottom wrapping is handled so far: such an object blocks the
//! full width of every column it overlaps. Square, tight and through
//! wrapping (text beside the object) belong to the wrap pass.

use hm_model::{ControlKind, ObjectCommon, Paragraph};

use crate::paginate::Rect;

/// A vertical stretch of the page that lines must not overlap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Band {
    pub top: i32,
    pub bottom: i32,
    pub left: i32,
    pub right: i32,
}

impl Band {
    fn overlaps_x(&self, left: i32, right: i32) -> bool {
        self.left < right && left < self.right
    }
}

/// Where an object is anchored on its page.
pub struct Anchor {
    pub paper: (i32, i32),
    pub body: Rect,
    /// The column the anchoring paragraph is in (page coordinates).
    pub column: (i32, i32),
    /// Top of the anchoring paragraph (page coordinates).
    pub para_top: i32,
    pub para_left: i32,
}

/// Places `extent` inside `[start, start + size)` by Hancom's alignment code:
/// 0 start, 1 center, 2 end; offsets count inward from the aligned edge.
fn align(start: i32, size: i32, extent: i32, code: u32, offset: i32) -> i32 {
    match code {
        1 => start + (size - extent) / 2 + offset,
        2 => start + size - extent - offset,
        _ => start + offset,
    }
}

/// The page rectangle `(x, y, w, h)` of an object, margins excluded.
pub fn frame(o: &ObjectCommon, a: &Anchor) -> (i32, i32, i32, i32) {
    let (vs, vh) = match o.vert_rel_to() {
        0 => (0, a.paper.1),
        1 => (a.body.y, a.body.h),
        _ => (a.para_top, a.body.y + a.body.h - a.para_top),
    };
    let (hs, hw) = match (o.attr >> 8) & 3 {
        0 => (0, a.paper.0),
        1 => (a.body.x, a.body.w),
        2 => a.column,
        _ => (a.para_left, a.column.0 + a.column.1 - a.para_left),
    };
    let valign = if o.vert_rel_to() == 2 {
        0
    } else {
        (o.attr >> 5) & 7
    };
    let y = align(vs, vh, o.height, valign, o.y_offset);
    let x = align(hs, hw, o.width, (o.attr >> 10) & 7, o.x_offset);
    (x, y, o.width, o.height)
}

/// Bands blocked by the paragraph's top-and-bottom objects.
pub fn bands(para: &Paragraph, a: &Anchor, out: &mut Vec<Band>) {
    for c in &para.controls {
        let ControlKind::Raw(raw) = &c.kind else {
            continue;
        };
        let Some(o) = raw.object_common() else {
            continue;
        };
        if o.treat_as_char() || o.text_wrap() != 1 {
            continue;
        }
        let (x, y, w, h) = frame(&o, a);
        let [ml, mr, mt, mb] = o.margins.map(i32::from);
        out.push(Band {
            top: y - mt,
            bottom: y + h + mb,
            left: x - ml,
            right: x + w + mr,
        });
    }
}

/// Moves `top` below every band that a line `[top, top + height)` spanning
/// `[left, right)` would overlap.
pub fn clear(bands: &[Band], mut top: i32, height: i32, left: i32, right: i32) -> i32 {
    loop {
        let hit = bands
            .iter()
            .filter(|b| b.overlaps_x(left, right) && b.top < top + height.max(1) && top < b.bottom)
            .map(|b| b.bottom)
            .max();
        match hit {
            Some(bottom) if bottom > top => top = bottom,
            _ => return top,
        }
    }
}
