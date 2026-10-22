//! Vector paths and pictures in the display list.

use hm_model::SharedBytes;

use crate::image::Effects;
use crate::paint::{Brush, Dash};

/// One path command in HU.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathEl {
    Move(i32, i32),
    Line(i32, i32),
    Cubic(i32, i32, i32, i32, i32, i32),
    Close,
}

/// A path: one or more subpaths.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Path {
    pub els: Vec<PathEl>,
}

impl Path {
    pub fn move_to(&mut self, p: (i32, i32)) {
        self.els.push(PathEl::Move(p.0, p.1));
    }

    pub fn line_to(&mut self, p: (i32, i32)) {
        self.els.push(PathEl::Line(p.0, p.1));
    }

    pub fn cubic_to(&mut self, a: (i32, i32), b: (i32, i32), p: (i32, i32)) {
        self.els.push(PathEl::Cubic(a.0, a.1, b.0, b.1, p.0, p.1));
    }

    pub fn close(&mut self) {
        self.els.push(PathEl::Close);
    }

    pub fn is_empty(&self) -> bool {
        self.els.is_empty()
    }

    /// The bounding box of the control points: `(x, y, w, h)`.
    pub fn bounds(&self) -> (i32, i32, i32, i32) {
        let mut b = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        let mut add = |x: i32, y: i32| {
            b.0 = b.0.min(x);
            b.1 = b.1.min(y);
            b.2 = b.2.max(x);
            b.3 = b.3.max(y);
        };
        for e in &self.els {
            match *e {
                PathEl::Move(x, y) | PathEl::Line(x, y) => add(x, y),
                PathEl::Cubic(a, b2, c, d, x, y) => {
                    add(a, b2);
                    add(c, d);
                    add(x, y);
                }
                PathEl::Close => {}
            }
        }
        if b.0 > b.2 {
            return (0, 0, 0, 0);
        }
        (b.0, b.1, b.2 - b.0, b.3 - b.1)
    }
}

/// How a shape's outline is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outline {
    /// HU; 0 is a hairline.
    pub width: i32,
    pub color: u32,
    pub dash: Dash,
    pub round_cap: bool,
}

/// A filled and stroked path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathItem {
    pub path: Path,
    /// Brushes painted inside, in order.
    pub fill: Vec<Brush>,
    pub outline: Option<Outline>,
    /// 0 opaque to 255 transparent, for the fill.
    pub fill_alpha: u8,
}

/// A picture mapped onto the page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageItem {
    pub data: SharedBytes,
    /// The page position of the picture's unit square: an affine map
    /// `[a, b, c, d, e, f]` taking `(u, v)` in 0..1 to
    /// `(a u + b v + c, d u + e v + f)` in HU.
    pub frame: [i32; 6],
    /// The visible part of the picture in Hancom's crop units (pixels
    /// times 75): left, top, right, bottom. All zero shows the whole picture.
    pub crop: [i32; 4],
    pub effects: Effects,
    /// The picture's original size in HU (0 when unknown): the crop's
    /// reference for metafiles, which have no pixel size.
    pub natural: (i32, i32),
}
