//! Pages and columns.

use hm_model::units::unpack_distance;
use hm_model::{ColumnDef, Document, Section, break_kind};

use crate::objects::{self, Anchor, Band};
use crate::para::{ParaEnv, ParaLayout, format};
use crate::style::Styles;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// A line placed on a page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlacedLine {
    /// Paragraph index in the section.
    pub para: u32,
    pub line: u32,
    /// Left edge of the column, page coordinates.
    pub x: i32,
    /// Top of the line, page coordinates.
    pub y: i32,
    /// Top of the line from the top of its column region, as Hancom stores `vpos`.
    pub vpos: i32,
}

#[derive(Clone, Debug, Default)]
pub struct Page {
    pub section: u32,
    /// Paper size.
    pub width: i32,
    pub height: i32,
    pub body: Rect,
    /// Column boxes of every region on the page.
    pub columns: Vec<Rect>,
    pub lines: Vec<PlacedLine>,
}

/// The formatted paragraphs of one section.
#[derive(Clone, Debug, Default)]
pub struct SectionLayout {
    pub paras: Vec<ParaLayout>,
}

/// Default tab interval when a section carries none (40 pt).
const DEFAULT_TAB: i32 = 4000;

struct Flow {
    pages: Vec<Page>,
    section: u32,
    paper: (i32, i32),
    body: Rect,
    /// Columns of the current region: `(x, width)` relative to the body.
    cols: Vec<(i32, i32)>,
    col: usize,
    /// Top of the current region from the body top.
    region_top: i32,
    /// Fill of each column of the region, from the region top.
    fill: Vec<i32>,
    has_content: bool,
    /// Bands blocked by objects on the current page (page coordinates).
    bands: Vec<Band>,
}

impl Flow {
    fn new_page(&mut self) {
        let page = Page {
            section: self.section,
            width: self.paper.0,
            height: self.paper.1,
            body: self.body,
            columns: Vec::new(),
            lines: Vec::new(),
        };
        self.pages.push(page);
        self.region_top = 0;
        self.col = 0;
        self.fill = vec![0; self.cols.len()];
        self.has_content = false;
        self.bands.clear();
        self.push_columns();
    }

    /// Page y of a region-relative y.
    fn page_y(&self, y: i32) -> i32 {
        self.body.y + self.region_top + y
    }

    /// The current column as page `(x, width)`.
    fn column(&self) -> (i32, i32) {
        let (x, w) = self.cols[self.col];
        (self.body.x + x, w)
    }

    fn push_columns(&mut self) {
        let (body, top) = (self.body, self.region_top);
        let page = self.pages.last_mut().unwrap();
        for &(x, w) in &self.cols {
            page.columns.push(Rect {
                x: body.x + x,
                y: body.y + top,
                w,
                h: body.h - top,
            });
        }
    }

    fn region_height(&self) -> i32 {
        self.body.h - self.region_top
    }

    /// Moves to the next column, or the next page after the last one.
    fn next_column(&mut self) {
        if self.col + 1 < self.cols.len() {
            self.col += 1;
            self.has_content = false;
        } else {
            self.new_page();
        }
    }

    /// Starts a new column region below everything placed in the current one.
    fn new_region(&mut self, cols: Vec<(i32, i32)>) {
        let bottom = self.fill.iter().copied().max().unwrap_or(0);
        self.region_top += bottom;
        self.cols = cols;
        self.col = 0;
        self.fill = vec![0; self.cols.len()];
        self.has_content = false;
        if self.region_top >= self.body.h {
            self.new_page();
        } else {
            self.push_columns();
        }
    }
}

/// Column `(x, width)` pairs. Hancom's formatter works in 4-HU units, so a
/// column's width drops to a multiple of 4.
fn column_layout(def: Option<&ColumnDef>, width: i32) -> Vec<(i32, i32)> {
    let cols = def.map_or_else(|| vec![(0, width)], |c| c.layout(width));
    cols.into_iter().map(|(x, w)| (x, w & !3)).collect()
}

/// Formats and paginates one section, appending its pages.
pub(crate) fn section(
    doc: &Document,
    index: usize,
    sec: &Section,
    styles: &mut Styles<'_>,
    pages: &mut Vec<Page>,
) -> SectionLayout {
    let def = sec.definition();
    let page_def = def.and_then(|d| d.page_def()).cloned().unwrap_or_default();
    let paper = page_def.paper();
    let (bx, by, bw, bh) = page_def.body();
    let default_tab = def
        .map(|d| unpack_distance(d.default_tab, || 1000))
        .filter(|t| *t > 0)
        .unwrap_or(DEFAULT_TAB);
    let first_cols = sec
        .paras
        .first()
        .and_then(|p| p.controls.iter().find_map(|c| c.as_columns()));
    let mut flow = Flow {
        pages: std::mem::take(pages),
        section: index as u32,
        paper,
        body: Rect {
            x: bx,
            y: by,
            w: bw,
            h: bh,
        },
        cols: column_layout(first_cols, bw),
        col: 0,
        region_top: 0,
        fill: Vec::new(),
        has_content: false,
        bands: Vec::new(),
    };
    flow.new_page();
    let mut out = SectionLayout {
        paras: Vec::with_capacity(sec.paras.len()),
    };
    for (pi, para) in sec.paras.iter().enumerate() {
        let shape = doc.info.para_shape(para.header.para_shape as u32);
        if pi > 0 {
            let page_break = para.header.break_kind & break_kind::PAGE != 0
                || shape.is_some_and(|s| s.page_break_before());
            if let Some(cd) = para.controls.iter().find_map(|c| c.as_columns()) {
                if page_break {
                    flow.cols = column_layout(Some(cd), bw);
                    flow.new_page();
                } else {
                    flow.new_region(column_layout(Some(cd), bw));
                }
            } else if page_break && (flow.has_content || flow.col > 0 || flow.region_top > 0) {
                flow.new_page();
            } else if para.header.break_kind & break_kind::COLUMN != 0 && flow.has_content {
                flow.next_column();
            }
        }
        let env = |w| ParaEnv {
            column_width: w,
            default_tab,
        };
        let mut layout = format(para, &doc.info, styles, env(flow.cols[flow.col].1));
        let left = doc
            .info
            .para_shape(para.header.para_shape as u32)
            .map_or(0, |s| s.left(1000));
        let anchor = |flow: &Flow, layout: &ParaLayout| Anchor {
            paper: flow.paper,
            body: flow.body,
            column: flow.column(),
            para_top: flow.page_y(flow.fill[flow.col] + layout.before),
            para_left: flow.column().0 + left,
        };
        objects::bands(para, &anchor(&flow, &layout), &mut flow.bands);
        for li in 0..layout.lines.len() {
            let before = if li == 0 { layout.before } else { 0 };
            let mut moved = false;
            let vpos = loop {
                let line = &layout.lines[li];
                let want = flow.fill[flow.col] + before;
                let (cx, cw) = flow.column();
                let top = objects::clear(&flow.bands, flow.page_y(want), line.height, cx, cx + cw);
                let y = top - flow.page_y(0);
                // An empty column takes the line even when it does not fit,
                // so a line taller than the page cannot loop forever.
                let too_tall = want + line.height > flow.region_height();
                if y + line.height <= flow.region_height()
                    || (!flow.has_content && (too_tall || moved))
                {
                    break y;
                }
                moved = true;
                let page = flow.pages.len();
                flow.next_column();
                // A column of another width reflows the paragraph from scratch.
                if flow.cols[flow.col].1 != layout.column_width && li == 0 {
                    layout = format(para, &doc.info, styles, env(flow.cols[flow.col].1));
                }
                if li == 0 && flow.pages.len() != page {
                    objects::bands(para, &anchor(&flow, &layout), &mut flow.bands);
                }
            };
            let line = layout.lines[li];
            let x = flow.column().0;
            let y = flow.page_y(vpos);
            let vpos = vpos + flow.region_top;
            flow.pages.last_mut().unwrap().lines.push(PlacedLine {
                para: pi as u32,
                line: li as u32,
                x,
                y,
                vpos,
            });
            flow.fill[flow.col] = vpos - flow.region_top + line.advance();
            flow.has_content = true;
        }
        flow.fill[flow.col] += layout.after;
        out.paras.push(layout);
    }
    *pages = flow.pages;
    out
}
