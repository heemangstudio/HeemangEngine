//! A page's editable lines, for hit testing, the caret, selections and
//! movement.
//!
//! The packet's `LINE` block carries the same index:
//!
//! ```text
//! u32 lists, per list: u32 section, u32 steps, steps x (u32 para,
//!     u32 control, u32 index)
//! u32 lines, per line: u32 list, u32 para, u32 line, i32 y, i32 height,
//!     i32 baseline, u32 stops, stops x (u32 position, i32 x)
//! ```
//!
//! Lines go in drawing order. A line's stops are its items' left edges in
//! wire order, then its end position at the right edge of its last item.

use std::collections::{HashMap, HashSet};

use hm_edit::{ListPath, Pos, Step};
use hm_model::{Document, Paragraph};
use hm_render::LineBox;

use crate::packet::W;
use crate::types::Affinity;

/// Paragraph addresses to their list and index, for every list the
/// document reaches through controls.
pub struct Paths(HashMap<usize, (ListPath, u32)>);

impl Paths {
    pub fn new(doc: &Document) -> Paths {
        let mut map = HashMap::new();
        for (si, sec) in doc.sections.iter().enumerate() {
            let list = ListPath {
                section: si as u32,
                steps: Vec::new(),
            };
            walk(&sec.paras, &list, &mut map);
        }
        Paths(map)
    }

    /// The paths of what page `page` shows: its body paragraphs, tables and
    /// objects, the paragraphs holding its frames' lists, and everything
    /// in their controls. Also the paragraph addresses of frames the
    /// document's paragraphs do not reach (page masters), which
    /// [`Paths::new`] does not hold either.
    pub fn of_page(doc: &Document, page: &hm_layout::Page) -> (Paths, HashSet<usize>) {
        use hm_layout::ListRef;
        let mut roots: Vec<(u32, u32)> = page
            .lines
            .iter()
            .map(|l| l.para)
            .chain(page.tables.iter().map(|t| t.para))
            .chain(page.objects.iter().map(|o| o.para))
            .map(|q| (page.section, q))
            .collect();
        let mut unreached = HashMap::new();
        for f in &page.frames {
            match f.list {
                ListRef::Control { section, para, .. } | ListRef::Cell { section, para, .. } => {
                    roots.push((section, para))
                }
                // A section's master pages are children of its definition,
                // in its first paragraph.
                ListRef::MasterPage { section, index } if index != hm_layout::LAST_PAGE_MASTER => {
                    roots.push((section, 0))
                }
                ListRef::MasterPage { .. } | ListRef::PageMaster { .. } => {
                    if let Some(paras) = f.list.paras(doc) {
                        walk(paras, &ListPath::default(), &mut unreached);
                    }
                }
            }
        }
        roots.sort_unstable();
        roots.dedup();
        let mut map = HashMap::new();
        for (section, q) in roots {
            if let Some(p) = doc
                .sections
                .get(section as usize)
                .and_then(|s| s.paras.get(q as usize))
            {
                let list = ListPath {
                    section,
                    steps: Vec::new(),
                };
                walk_one(p, q, &list, &mut map);
            }
        }
        (Paths(map), unreached.into_keys().collect())
    }

    pub fn contains(&self, para: usize) -> bool {
        self.0.contains_key(&para)
    }
}

fn walk(paras: &[Paragraph], list: &ListPath, map: &mut HashMap<usize, (ListPath, u32)>) {
    for (pi, p) in paras.iter().enumerate() {
        walk_one(p, pi as u32, list, map);
    }
}

fn walk_one(p: &Paragraph, pi: u32, list: &ListPath, map: &mut HashMap<usize, (ListPath, u32)>) {
    map.insert(p as *const Paragraph as usize, (list.clone(), pi));
    for (ci, c) in p.controls.iter().enumerate() {
        for (k, inner) in hm_edit::ops::control_lists(c).into_iter().enumerate() {
            let mut path = list.clone();
            path.steps.push(Step {
                para: pi,
                control: ci as u32,
                index: k as u32,
            });
            walk(inner, &path, map);
        }
    }
}

pub struct Line {
    pub list: ListPath,
    pub para: u32,
    pub line: u32,
    pub y: i32,
    pub height: i32,
    pub baseline: i32,
    /// The x of the column or box the line is set in.
    pub origin: i32,
    /// `(position, x)`, the last one the line's end.
    pub stops: Vec<(u32, i32)>,
}

impl Line {
    pub fn left(&self) -> i32 {
        self.stops.first().map_or(0, |s| s.1)
    }

    pub fn right(&self) -> i32 {
        self.stops.last().map_or(0, |s| s.1)
    }

    pub fn start(&self) -> u32 {
        self.stops.first().map_or(0, |s| s.0)
    }

    pub fn end(&self) -> u32 {
        self.stops.last().map_or(0, |s| s.0)
    }

    /// The x of the last stop at or before `offset`.
    pub fn x_of(&self, offset: u32) -> i32 {
        self.stops
            .iter()
            .take_while(|s| s.0 <= offset)
            .last()
            .map_or(self.left(), |s| s.1)
    }

    /// Distance from a point, vertical misses weighing more.
    fn distance(&self, x: i32, y: i32) -> i64 {
        let dy = if y < self.y {
            self.y - y
        } else {
            (y - (self.y + self.height)).max(0)
        };
        let dx = if x < self.left() {
            self.left() - x
        } else {
            (x - self.right()).max(0)
        };
        dy as i64 * 4 + dx as i64
    }
}

/// The editable lines of one page, in drawing order.
pub struct PageIndex {
    pub lines: Vec<Line>,
    pub cells: Vec<hm_render::CellBox>,
}

/// Where the caret may stand in a paragraph: from after its leading
/// section and column controls and floating object anchors to its end mark.
pub fn caret_range(doc: &Document, list: &ListPath, para: u32) -> Option<(u32, u32)> {
    let p = hm_edit::ops::list(doc, list)?.get(para as usize)?;
    Some((hm_edit::ops::caret_lead(p), p.len().saturating_sub(1)))
}

impl PageIndex {
    pub fn new(boxes: Vec<LineBox>, cells: Vec<hm_render::CellBox>, paths: &Paths) -> PageIndex {
        let lines = boxes
            .into_iter()
            .filter_map(|b| {
                let (list, para) = paths.0.get(&b.para)?.clone();
                Some(Line {
                    list,
                    para,
                    line: b.line,
                    y: b.y,
                    height: b.height,
                    baseline: b.baseline,
                    origin: b.x,
                    stops: b.stops,
                })
            })
            .collect();
        PageIndex { lines, cells }
    }

    /// The `LINE` block.
    pub fn block(&self) -> Vec<u8> {
        let mut lists: Vec<&ListPath> = Vec::new();
        let mut ids = Vec::with_capacity(self.lines.len());
        for l in &self.lines {
            let id = match lists.iter().position(|p| **p == l.list) {
                Some(i) => i,
                None => {
                    lists.push(&l.list);
                    lists.len() - 1
                }
            };
            ids.push(id as u32);
        }
        let mut w = W(Vec::new());
        w.u32(lists.len() as u32);
        for p in lists {
            w.u32(p.section);
            w.u32(p.steps.len() as u32);
            for s in &p.steps {
                w.u32(s.para);
                w.u32(s.control);
                w.u32(s.index);
            }
        }
        w.u32(self.lines.len() as u32);
        for (l, id) in self.lines.iter().zip(ids) {
            w.u32(id);
            w.u32(l.para);
            w.u32(l.line);
            w.i32(l.y);
            w.i32(l.height);
            w.i32(l.baseline);
            w.u32(l.stops.len() as u32);
            for &(p, x) in &l.stops {
                w.u32(p);
                w.i32(x);
            }
        }
        w.0
    }

    /// The position nearest `(x, y)`, among lines `keep` accepts.
    pub fn hit(
        &self,
        doc: &Document,
        x: i32,
        y: i32,
        keep: &dyn Fn(&Line) -> bool,
    ) -> Option<(Pos, Affinity, [i32; 3], bool)> {
        let line = self
            .lines
            .iter()
            .filter(|l| keep(l))
            // Lines inside a cell or box win over the line holding it.
            .min_by_key(|l| (l.distance(x, y), std::cmp::Reverse(l.list.steps.len()), l.height))?;
        // In its own column, a point belongs to the last line starting at or
        // above it, however short that line is: a point right of a short
        // line is on that line, not on a longer one nearer in x below.
        let line = self
            .lines
            .iter()
            .filter(|l| keep(l) && l.list == line.list && l.origin == line.origin && l.y <= y)
            .max_by_key(|l| l.y)
            .map(|last| last.y)
            .and_then(|top| {
                self.lines
                    .iter()
                    .filter(|l| {
                        keep(l) && l.list == line.list && l.origin == line.origin && l.y == top
                    })
                    .min_by_key(|l| l.distance(x, y))
            })
            .unwrap_or(line);
        let (lo, hi) = caret_range(doc, &line.list, line.para)?;
        let (k, stop) = line
            .stops
            .iter()
            .enumerate()
            .filter(|(_, s)| s.0 >= lo && s.0 <= hi)
            .min_by_key(|(_, s)| (s.1 - x).abs())?;
        // The end of a wrapped line is the next line's start, shown here.
        let affinity = if k + 1 == line.stops.len() && stop.0 < hi {
            Affinity::Upstream
        } else {
            Affinity::Downstream
        };
        let pos = Pos {
            list: line.list.clone(),
            para: line.para,
            offset: stop.0,
        };
        let inside = line.distance(x, y) == 0;
        Some((pos, affinity, [stop.1, line.y, line.height], inside))
    }

    /// The line showing a position's caret on this page.
    pub fn line_of(&self, pos: &Pos, affinity: Affinity) -> Option<&Line> {
        let mut mine = self
            .lines
            .iter()
            .filter(|l| l.list == pos.list && l.para == pos.para);
        let inside = |l: &&Line| l.start() <= pos.offset && pos.offset < l.end();
        let at_end = |l: &&Line| l.start() <= pos.offset && pos.offset <= l.end();
        match affinity {
            Affinity::Upstream => mine
                .clone()
                .find(|l| l.end() == pos.offset && l.start() < pos.offset)
                .or_else(|| mine.find(inside)),
            Affinity::Downstream => mine.clone().find(inside).or_else(|| mine.rfind(at_end)),
        }
    }

    /// The caret `[x, y, height]` of a position drawn on this page.
    pub fn caret(&self, pos: &Pos, affinity: Affinity) -> Option<[i32; 3]> {
        let line = self.line_of(pos, affinity)?;
        Some([line.x_of(pos.offset), line.y, line.height])
    }

    /// Rectangles covering `[from, to)` on this page, one per line.
    pub fn selection(&self, from: &Pos, to: &Pos) -> Vec<[i32; 4]> {
        if from.list != to.list {
            return Vec::new();
        }
        let mut out = Vec::new();
        for l in self.lines.iter().filter(|l| l.list == from.list) {
            if l.para < from.para || l.para > to.para {
                continue;
            }
            let a = if l.para == from.para {
                from.offset.max(l.start())
            } else {
                l.start()
            };
            let b = if l.para == to.para {
                to.offset.min(l.end())
            } else {
                l.end()
            };
            // A paragraph's end mark selected shows as a sliver.
            let whole_end = l.para < to.para && b == l.end();
            if a > b || (a == b && !whole_end) {
                continue;
            }
            let (x0, x1) = (l.x_of(a), l.x_of(b));
            let w = if whole_end {
                (x1 - x0).max(300)
            } else {
                x1 - x0
            };
            out.push([x0, l.y, w.max(0), l.height]);
        }
        out
    }
}
