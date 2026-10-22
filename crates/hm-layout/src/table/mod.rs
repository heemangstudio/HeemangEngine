//! Table layout: the column and row grid, cell paragraphs and their
//! vertical alignment.
//!
//! Column edges are cumulative maxima: an edge lies at the largest end of
//! the cells that end there, each cell starting at the edge before it plus
//! the cell spacing. Rows work the same way with each cell asking for the
//! larger of its stored height and its content plus vertical padding (the
//! content ends at the last line's bottom, without its line spacing).
//! Size-protected tables keep their stored heights.

mod flow;
mod split;

pub use flow::{PlacedTable, flows};
pub(crate) use flow::{after_line, before_host, before_line};
pub use split::{SplitMode, cut, header_height};

use hm_model::{Cell, DocInfo, Paragraph, Table};

use crate::heads::Heads;
use crate::para::{ParaEnv, ParaLayout, format};
use crate::style::Styles;

/// Width an edge advances when no cell ends at it.
const MIN_TRACK: i32 = 100;

/// A laid-out table. Coordinates are relative to the table's top left,
/// outer margins excluded.
#[derive(Clone, Debug, Default)]
pub struct TableLayout {
    /// Column edges, `cols + 1` entries from 0; a cell spans from
    /// `col_x[col] + spacing` to `col_x[col + span]`.
    pub col_x: Vec<i32>,
    /// Row edges likewise.
    pub row_y: Vec<i32>,
    pub spacing: i32,
    pub width: i32,
    pub height: i32,
    /// One per table cell, in the table's order.
    pub cells: Vec<CellLayout>,
}

/// A laid-out cell.
#[derive(Clone, Debug, Default)]
pub struct CellLayout {
    /// The cell box relative to the table.
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    /// Padding as applied: left, right, top, bottom.
    pub padding: [i32; 4],
    pub paras: Vec<ParaLayout>,
    /// Every line with its top from the content top.
    pub lines: Vec<CellLine>,
    /// Content height: the last line's bottom.
    pub content: i32,
    /// Shift of the content for vertical alignment.
    pub offset: i32,
}

/// A line of a cell paragraph placed in the cell.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CellLine {
    pub para: u32,
    pub line: u32,
    /// Top of the line from the content top, alignment not applied.
    pub y: i32,
}

impl CellLayout {
    /// Left edge of the content from the table's left.
    pub fn content_x(&self) -> i32 {
        self.x + self.padding[0]
    }

    /// Top of the content from the table's top, alignment applied.
    pub fn content_y(&self) -> i32 {
        self.y + self.padding[2] + self.offset
    }
}

impl TableLayout {
    /// Bottom edge of row `r` (exclusive end of the row band).
    pub fn row_bottom(&self, r: usize) -> i32 {
        self.row_y[r + 1]
    }
}

/// Lays out the paragraphs of a list (a cell, a caption) for `width`,
/// stacking them from 0: returns the paragraphs, their placed lines and the
/// content height.
pub fn stack(
    paras: &[Paragraph],
    info: &DocInfo,
    styles: &mut Styles<'_>,
    env: ParaEnv,
    heads: &mut Heads,
) -> (Vec<ParaLayout>, Vec<CellLine>, i32) {
    stack_lines(paras, info, styles, env, heads, false)
}

/// Width a one-line paragraph is formatted at before it is condensed.
const UNBOUNDED: i32 = 1 << 28;

/// Keeps each line of a paragraph laid out at an unbounded width within
/// `width`: the line keeps its content and condenses it (negative slack)
/// when it overflows.
fn condense_to(pl: &mut ParaLayout, width: i32) {
    let wide = pl.column_width;
    for l in &mut pl.lines {
        l.seg_width = (l.seg_width - (wide - width)).max(0);
        l.width = (l.width - (wide - width)).max(0);
    }
    pl.column_width = width;
}

/// [`stack`], optionally keeping every paragraph on one line by condensing
/// its spacing (a cell's 한 줄로 입력).
fn stack_lines(
    paras: &[Paragraph],
    info: &DocInfo,
    styles: &mut Styles<'_>,
    env: ParaEnv,
    heads: &mut Heads,
    one_line: bool,
) -> (Vec<ParaLayout>, Vec<CellLine>, i32) {
    let mut layouts = Vec::with_capacity(paras.len());
    let mut lines = Vec::new();
    let mut y = 0;
    let mut content = 0;
    for (pi, p) in paras.iter().enumerate() {
        let head = info
            .para_shape(p.header.para_shape as u32)
            .and_then(|s| heads.next(s, info, 0));
        let pl = if one_line {
            let wide = ParaEnv {
                column_width: UNBOUNDED,
                ..env
            };
            let mut pl = format(p, info, styles, wide, head.as_ref());
            condense_to(&mut pl, env.column_width);
            pl
        } else {
            format(p, info, styles, env, head.as_ref())
        };
        y += pl.before;
        for (li, l) in pl.lines.iter().enumerate() {
            lines.push(CellLine {
                para: pi as u32,
                line: li as u32,
                y,
            });
            content = y + l.height;
            y += l.advance();
        }
        y += pl.after;
        layouts.push(pl);
    }
    (layouts, lines, content)
}

/// Column edges by cumulative maxima.
fn edges(
    n: usize,
    spacing: i32,
    cells: &[Cell],
    span: impl Fn(&Cell) -> (usize, usize),
    extent: impl Fn(usize) -> i32,
) -> Vec<i32> {
    let mut e = vec![0i32; n + 1];
    for i in 1..=n {
        let mut best: Option<i32> = None;
        for (k, c) in cells.iter().enumerate() {
            let (start, end) = span(c);
            if end == i && start < i {
                let v = e[start].saturating_add(extent(k)).saturating_add(spacing);
                best = Some(best.map_or(v, |b| b.max(v)));
            }
        }
        e[i] = best.unwrap_or(e[i - 1] + MIN_TRACK);
    }
    e
}

fn col_span(c: &Cell, cols: usize) -> (usize, usize) {
    let start = (c.col as usize).min(cols.saturating_sub(1));
    (start, (c.end_col() as usize).clamp(start + 1, cols))
}

fn row_span(c: &Cell, rows: usize) -> (usize, usize) {
    let start = (c.row as usize).min(rows.saturating_sub(1));
    (start, (c.end_row() as usize).clamp(start + 1, rows))
}

/// Lays out a table. `default_tab` is the section's tab interval.
pub fn layout(t: &Table, info: &DocInfo, styles: &mut Styles<'_>, default_tab: i32) -> TableLayout {
    let cols = (t.cols as usize).max(1);
    let rows = (t.rows as usize).max(1);
    let sp = t.spacing as i32;
    let col_x = edges(
        cols,
        sp,
        &t.cells,
        |c| col_span(c, cols),
        |k| t.cells[k].width,
    );
    let mut heads = Heads::default();
    let mut cells: Vec<CellLayout> = t
        .cells
        .iter()
        .map(|c| {
            let (c0, c1) = col_span(c, cols);
            let x = col_x[c0] + sp;
            let w = col_x[c1] - x;
            let padding = t.cell_padding(c);
            let inner = (w - padding[0] - padding[1]).max(0) & !3;
            let env = ParaEnv {
                column_width: inner,
                default_tab,
            };
            let (paras, lines, content) =
                stack_lines(&c.paras, info, styles, env, &mut heads, c.line_wrap() == 1);
            CellLayout {
                x,
                w,
                padding,
                paras,
                lines,
                content,
                ..CellLayout::default()
            }
        })
        .collect();
    let protected = t.header.common.attr & (1 << 20) != 0;
    let row_y = edges(
        rows,
        sp,
        &t.cells,
        |c| row_span(c, rows),
        |k| {
            let (c, l) = (&t.cells[k], &cells[k]);
            if protected {
                c.height
            } else {
                c.height.max(l.content + l.padding[2] + l.padding[3])
            }
        },
    );
    for (c, l) in t.cells.iter().zip(cells.iter_mut()) {
        let (r0, r1) = row_span(c, rows);
        l.y = row_y[r0] + sp;
        l.h = row_y[r1] - l.y;
        let slack = l.h - l.padding[2] - l.padding[3] - l.content;
        l.offset = match c.vertical_align() {
            1 if slack > 0 => slack / 2,
            2 if slack > 0 => slack,
            _ => 0,
        };
    }
    TableLayout {
        width: col_x[cols] + sp,
        height: row_y[rows] + sp,
        col_x,
        row_y,
        spacing: sp,
        cells,
    }
}

/// Lays out the tables among a paragraph's controls, as
/// `(control index, layout)`.
pub fn layout_controls(
    para: &Paragraph,
    info: &DocInfo,
    styles: &mut Styles<'_>,
    default_tab: i32,
) -> Vec<(u32, TableLayout)> {
    para.controls
        .iter()
        .enumerate()
        .filter_map(|(k, c)| {
            c.as_table()
                .map(|t| (k as u32, layout(t, info, styles, default_tab)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(row: u16, col: u16, rs: u16, cs: u16, w: i32, h: i32) -> Cell {
        Cell {
            row,
            col,
            row_span: rs,
            col_span: cs,
            width: w,
            height: h,
            ..Cell::default()
        }
    }

    #[test]
    fn merged_widths_extend_the_trailing_column() {
        // A merged cell wider than the columns below it widens the last
        // column it spans; smaller conflicting widths lose.
        let cells = [
            cell(0, 0, 1, 2, 200, 10),
            cell(1, 0, 1, 1, 80, 10),
            cell(1, 1, 1, 1, 100, 10),
            cell(2, 0, 1, 1, 60, 10),
        ];
        let e = edges(2, 0, &cells, |c| col_span(c, 2), |k| cells[k].width);
        assert_eq!(e, vec![0, 80, 200]);
        // Spacing separates cells and both table edges.
        let e = edges(2, 5, &cells, |c| col_span(c, 2), |k| cells[k].width);
        assert_eq!(e, vec![0, 85, 205]);
    }

    #[test]
    fn rows_take_the_largest_request() {
        let cells = [
            cell(0, 0, 2, 1, 50, 900),
            cell(0, 1, 1, 1, 50, 300),
            cell(1, 1, 1, 1, 50, 400),
        ];
        let e = edges(2, 0, &cells, |c| row_span(c, 2), |k| cells[k].height);
        // The spanning cell pushes the second row down to 900.
        assert_eq!(e, vec![0, 300, 900]);
    }
}
