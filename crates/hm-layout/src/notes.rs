//! Footnotes at the foot of their column.
//!
//! A footnote goes on the page of the line that refers to it. The notes of
//! a column stack up from the body's bottom edge, the last note's last line
//! ending there, with the note shape's gap between notes. Above the first
//! note the separator line sits by the gap below it, and body text keeps the
//! gap above it clear. Every note taken into a column lowers the limit the
//! column's lines must stay under.

use hm_model::{BorderLine, NoteShape};

use crate::furniture::{Frame, Rule};
use crate::paginate::{Page, Rect};

/// The separator length Hancom draws for a stored length of -1: 5 cm.
const DEFAULT_SEPARATOR: i32 = 14173;

/// Notes waiting at the foot of one column.
#[derive(Default)]
struct Bucket {
    x: i32,
    w: i32,
    frames: Vec<Frame>,
}

/// The footnote areas of the page being filled.
#[derive(Default)]
pub(crate) struct NoteArea {
    shape: NoteShape,
    buckets: Vec<Bucket>,
}

impl NoteArea {
    pub(crate) fn new(shape: Option<&NoteShape>) -> Self {
        Self {
            shape: shape.cloned().unwrap_or_else(|| NoteShape {
                separator_length: -1,
                space_above: 850,
                space_below: 567,
                space_between: 283,
                line_kind: 1,
                line_width: 1,
                ..Default::default()
            }),
            buckets: Vec::new(),
        }
    }

    /// Height the notes of `frames` take, with the separator gaps.
    fn height_of(&self, frames: &[&Frame]) -> i32 {
        if frames.is_empty() {
            return 0;
        }
        let s = &self.shape;
        let notes: i32 = frames.iter().map(|f| f.content).sum();
        s.space_above as i32
            + s.space_below as i32
            + notes
            + s.space_between as i32 * (frames.len() as i32 - 1)
    }

    fn bucket(&self, x: i32) -> Option<&Bucket> {
        self.buckets.iter().find(|b| b.x == x)
    }

    /// Height reserved at the foot of the column at page x `x`.
    pub(crate) fn reserve(&self, x: i32) -> i32 {
        self.bucket(x)
            .map_or(0, |b| self.height_of(&b.frames.iter().collect::<Vec<_>>()))
    }

    /// How much more the column at `x` reserves once `more` join it.
    pub(crate) fn extra(&self, x: i32, more: &[Frame]) -> i32 {
        if more.is_empty() {
            return 0;
        }
        let mut all: Vec<&Frame> = self
            .bucket(x)
            .map_or(Vec::new(), |b| b.frames.iter().collect());
        let before = self.height_of(&all);
        all.extend(more.iter());
        self.height_of(&all) - before
    }

    pub(crate) fn add(&mut self, x: i32, w: i32, more: Vec<Frame>) {
        if more.is_empty() {
            return;
        }
        match self.buckets.iter_mut().find(|b| b.x == x) {
            Some(b) => b.frames.extend(more),
            None => self.buckets.push(Bucket { x, w, frames: more }),
        }
    }

    /// Places the waiting notes on `page`, whose body is `body`, and empties
    /// the area.
    pub(crate) fn flush(&mut self, page: &mut Page, body: Rect) {
        let s = self.shape.clone();
        for b in self.buckets.drain(..) {
            if b.frames.is_empty() {
                continue;
            }
            let notes: i32 = b.frames.iter().map(|f| f.content).sum::<i32>()
                + s.space_between as i32 * (b.frames.len() as i32 - 1);
            let first = body.y + body.h - notes;
            if s.line_kind != 0 {
                let len = match s.separator_length {
                    l if l < 0 => DEFAULT_SEPARATOR,
                    l => l.min(b.w),
                };
                let y = first - s.space_below as i32;
                page.rules.push(Rule {
                    x1: b.x,
                    y1: y,
                    x2: b.x + len.min(b.w),
                    y2: y,
                    line: BorderLine {
                        kind: s.line_kind,
                        width: s.line_width,
                        color: s.line_color,
                    },
                });
            }
            let mut top = first;
            for mut f in b.frames {
                let h = f.content;
                f.shift(top - f.rect.y);
                f.rect.h = h;
                top += h + s.space_between as i32;
                page.frames.push(f);
            }
        }
    }
}
