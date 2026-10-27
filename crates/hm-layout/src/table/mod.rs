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
pub use flow::LineFacts;
mod held;
mod split;

pub(crate) use flow::{Deferred, Pending, after_line, before_host, before_line, resume};
pub use flow::{PlacedTable, flows, flows_in, has_content};
pub use held::{CellObject, escapes_cell, para_left};
pub use split::{CUT_ROOM, Carried, Cuts, SplitMode, cut, first_cut, header_height};

use std::sync::OnceLock;

use hm_model::{Cell, DocInfo, Paragraph, Table};

use crate::heads::Heads;
use crate::para::{ParaEnv, ParaLayout, format};
use crate::style::Styles;

/// The narrowest width a cell's text is laid out in.
const MIN_TEXT_WIDTH: i32 = 1440;

/// The width a cell's text is laid out in from the room its padding leaves.
fn text_width(room: i32) -> i32 {
    room.max(MIN_TEXT_WIDTH) & !3
}

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
    pub caption: Option<CaptionLayout>,
    /// Where the table may be cut, found on first use.
    cuts: OnceLock<Cuts>,
    /// Each cell's line and flowing-table spans, by top, found on first use.
    spans: OnceLock<Vec<Vec<(i32, i32)>>>,
}

/// A laid-out caption beside a table.
#[derive(Clone, Debug, Default)]
pub struct CaptionLayout {
    /// 0 left, 1 right, 2 top, 3 bottom.
    pub side: u32,
    /// Gap between caption and table.
    pub gap: i32,
    /// Width the caption text is laid out in, and its content height.
    pub width: i32,
    pub height: i32,
    pub paras: Vec<ParaLayout>,
    pub lines: Vec<CellLine>,
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
    /// Non-inline tables in the cell's paragraphs, including overlays,
    /// with their place and pagination pieces in the content box.
    pub tables: Vec<CellTable>,
    /// Floating objects that take room in the content.
    pub objects: Vec<CellObject>,
    /// Content height: the last line's bottom.
    pub content: i32,
    /// The compatibility profile retains the last paragraph's spacing.
    pub keep_end_spacing: bool,
    /// Shift of the content for vertical alignment.
    pub offset: i32,
    /// The text runs down the cell: its lines are laid out as long as the
    /// cell is high and stack across it from the right.
    pub vertical: bool,
}

/// A table placed in a cell's content: its paragraph and control index and
/// the top left of its grid from the content's top left.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CellTable {
    pub para: u32,
    pub control: u32,
    pub x: i32,
    pub y: i32,
    /// Height of the grid.
    pub height: i32,
    /// Heights the table may be cut at across pages, from its grid top.
    pub cuts: std::sync::Arc<[i32]>,
}

impl CellTable {
    /// The part of the table between grid heights `from` and `to` that
    /// whole pieces between its cuts fill, when there is one.
    pub fn band(&self, from: i32, to: i32) -> Option<(i32, i32)> {
        let edges = || {
            std::iter::once(0)
                .chain(self.cuts.iter().copied())
                .chain([self.height])
        };
        let top = edges().find(|&e| e >= from)?;
        let bottom = edges().rev().find(|&e| e <= to)?;
        (bottom > top).then_some((top, bottom))
    }
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

    /// Top of the content where splitting places it: lines flow from the
    /// cell's top before its vertical alignment, and each part of a cell cut
    /// across pages aligns what it shows in its own height.
    pub fn split_y(&self) -> i32 {
        self.y + self.padding[2]
    }
}

impl TableLayout {
    /// The table's legal cuts.
    pub fn cuts(&self, t: &Table) -> &Cuts {
        self.cuts.get_or_init(|| Cuts::new(t, self))
    }

    /// The heights table `t` may be cut at by its split mode.
    pub fn legal_cuts(&self, t: &Table) -> Vec<i32> {
        self.cuts(t).of(SplitMode::of(t)).to_vec()
    }

    /// What cell `k`'s content occupies in table heights, sorted by top.
    /// Splitting a long cell over many pages asks for it at every cut.
    pub fn cell_spans(&self, k: usize) -> &[(i32, i32)] {
        let all = self
            .spans
            .get_or_init(|| self.cells.iter().map(split::sorted_spans).collect());
        all.get(k).map_or(&[], Vec::as_slice)
    }

    /// Bottom edge of row `r` (exclusive end of the row band).
    pub fn row_bottom(&self, r: usize) -> i32 {
        self.row_y[r + 1]
    }

    /// Size of the table with its caption.
    pub fn outer_size(&self) -> (i32, i32) {
        match &self.caption {
            Some(c) if c.side >= 2 => (self.width.max(c.width), self.height + c.gap + c.height),
            Some(c) => (self.width + c.gap + c.width, self.height.max(c.height)),
            None => (self.width, self.height),
        }
    }

    /// Where the grid sits inside the outer box.
    pub fn grid_offset(&self) -> (i32, i32) {
        match &self.caption {
            Some(c) if c.side == 2 => (0, c.height + c.gap),
            Some(c) if c.side == 0 => (c.width + c.gap, 0),
            _ => (0, 0),
        }
    }

    /// Where the caption sits inside the outer box.
    pub fn caption_offset(&self) -> (i32, i32) {
        match &self.caption {
            Some(c) if c.side == 3 => (0, self.height + c.gap),
            Some(c) if c.side == 1 => (self.width + c.gap, 0),
            _ => (0, 0),
        }
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
    let (layouts, lines, content, ..) =
        stack_lines(paras, info, styles, env, heads, false, false, false);
    (layouts, lines, content)
}

/// Stacks endnotes whose cached line membership is independent of the
/// vertical coordinates assigned when they flow through columns.
pub(crate) fn stack_endnote(
    paras: &[Paragraph],
    info: &DocInfo,
    styles: &mut Styles<'_>,
    env: ParaEnv,
    heads: &mut Heads,
) -> (Vec<ParaLayout>, Vec<CellLine>, i32, Vec<CellTable>) {
    let (layouts, lines, content, tables, _) =
        stack_lines(paras, info, styles, env, heads, false, true, false);
    (layouts, lines, content, tables)
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
#[allow(clippy::too_many_arguments)]
fn stack_lines(
    paras: &[Paragraph],
    info: &DocInfo,
    styles: &mut Styles<'_>,
    env: ParaEnv,
    heads: &mut Heads,
    one_line: bool,
    endnote: bool,
    inline_table: bool,
) -> (
    Vec<ParaLayout>,
    Vec<CellLine>,
    i32,
    Vec<CellTable>,
    Vec<CellObject>,
) {
    let mut layouts = Vec::with_capacity(paras.len());
    let mut lines = Vec::new();
    let mut tables = Vec::new();
    let mut objects = Vec::new();
    let mut table_bands = Vec::new();
    let mut y = 0;
    let mut content = 0;
    for (pi, p) in paras.iter().enumerate() {
        let head = info.para_shape(p.header.para_shape as u32).and_then(|s| {
            heads.next_with_restart(s, info, styles.outline_numbering, p.list_restart)
        });
        let pl = if one_line {
            let wide = ParaEnv {
                column_width: UNBOUNDED,
                ..env
            };
            // The one line holds everything, whatever was stored.
            let stored = std::mem::replace(&mut styles.stored_lines, false);
            let mut pl = format(p, info, styles, wide, head.as_ref());
            styles.stored_lines = stored;
            condense_to(&mut pl, env.column_width);
            pl
        } else if endnote {
            crate::para::format_endnote(p, info, styles, env, head.as_ref())
        } else {
            format(p, info, styles, env, head.as_ref())
        };
        let para_top = y;
        y += pl.before;
        let (mut text_end, mut tables_end) = (y, y);
        for (li, l) in pl.lines.iter().enumerate() {
            // Tables flowing with the text hang from the top of the line
            // holding them, by their vertical offset; the line stays above
            // them unless it has content they would cover, and what
            // follows goes below them.
            let line_top = y;
            let anchor_top = if li == 0 { para_top } else { line_top };
            let earlier_bands = table_bands.len();
            let mut below = line_top;
            let mut covers = false;
            for (k, t, tl, o) in flowing_in_line(p, &pl, l, env.column_width) {
                let [_, _, mt, mb] = o.margins.map(i32::from);
                let (gx, gy) = tl.grid_offset();
                let (_, h) = tl.outer_size();
                if matches!(o.text_wrap(), 0 | 3) {
                    // Anchored tables keep their paragraph-relative frame.
                    // An overlay contributes pagination pieces without
                    // advancing the text cursor or displacing its host line.
                    let left = held::para_left(info, p);
                    let outer_top = para_top + o.y_offset.max(0);
                    tables.push(CellTable {
                        para: pi as u32,
                        control: k,
                        x: cell_table_x(&o, left, pl.right, env.column_width) + gx,
                        y: outer_top + mt + gy,
                        height: tl.height,
                        cuts: tl.legal_cuts(t).into(),
                    });
                    content = content.max(outer_top + mt + h);
                    continue;
                }
                // Only an indivisible table has one known exclusion band.
                // Split tables keep their fragment cursor until nested cell
                // continuations can supply each page's exclusion bands.
                let indivisible = SplitMode::of(t) == SplitMode::Never;
                let natural = if indivisible {
                    (anchor_top + o.y_offset).max(anchor_top)
                } else {
                    (line_top + o.y_offset).max(below)
                };
                let outer_top =
                    crate::objects::clear(&table_bands, natural, mt + h + mb, 0, env.column_width);
                tables.push(CellTable {
                    para: pi as u32,
                    control: k,
                    x: cell_table_x(&o, l.seg_x.min(pl.column_width), pl.right, env.column_width)
                        + gx,
                    y: outer_top + mt + gy,
                    height: tl.height,
                    cuts: tl.legal_cuts(t).into(),
                });
                let bottom = outer_top + mt + h + mb;
                if indivisible {
                    table_bands.push(crate::objects::Band {
                        top: outer_top,
                        bottom,
                        right: env.column_width,
                        ..Default::default()
                    });
                } else {
                    covers |= outer_top < line_top + l.height;
                    below = bottom;
                }
                content = content.max(bottom - mb);
            }
            let has_content = flow::has_content(p, l);
            let bands = if has_content {
                &table_bands[..]
            } else {
                &table_bands[..earlier_bands]
            };
            let top = if covers && has_content {
                below
            } else {
                line_top
            };
            let at = crate::objects::clear(bands, top, l.height, 0, env.column_width);
            lines.push(CellLine {
                para: pi as u32,
                line: li as u32,
                y: at,
            });
            content = content.max(at + l.height);
            text_end = at + l.advance();
            tables_end = below;
            y = text_end.max(tables_end);
        }
        // Paragraph spacing advances the text cursor before a later line
        // clears a floating table. The clearance can consume that spacing.
        y = (text_end + pl.after).max(tables_end);
        // Held objects hang from the paragraph's top by their offset; what
        // follows goes below them.
        let (floor, reach) = held::hold(
            info,
            p,
            pi as u32,
            &pl,
            para_top,
            env.column_width,
            inline_table,
            &mut objects,
        );
        content = content.max(reach);
        y = y.max(floor);
        layouts.push(pl);
    }
    if crate::compat::effective(info)[1] & (1 << 7) != 0 {
        let bottom = table_bands.iter().map(|b| b.bottom).max().unwrap_or(0);
        content = content.max(y).max(bottom);
    }
    (layouts, lines, content, tables, objects)
}

/// The tables anchored in line `l` that flow with a cell's text, stand
/// square beside it or overlay it, with their layouts and attributes.
fn flowing_in_line<'a>(
    p: &'a Paragraph,
    pl: &'a ParaLayout,
    l: &crate::para::Line,
    width: i32,
) -> Vec<(u32, &'a Table, &'a TableLayout, hm_model::ObjectCommon)> {
    p.units()
        .filter_map(|(pos, u)| match u {
            hm_model::Unit::Extended { index, .. } if pos >= l.start && pos < l.end => Some(index),
            _ => None,
        })
        .filter_map(|k| {
            let c = p.controls.get(k as usize)?;
            let t = c.as_table()?;
            let o = t.header.common;
            // Anchored square and overlay tables retain their frame while
            // contributing content and legal cuts to the enclosing cell.
            let anchored =
                !o.treat_as_char() && o.vert_rel_to() == 2 && matches!(o.text_wrap(), 0 | 3);
            if !anchored && !flows_in(c, width) {
                return None;
            }
            Some((k, t, pl.table(k)?, o))
        })
        .collect()
}

/// The x of a flowing table's box in a cell's content (margins excluded):
/// its horizontal offset and alignment in the content width.
fn cell_table_x(o: &hm_model::ObjectCommon, para_left: i32, para_right: i32, width: i32) -> i32 {
    let [ml, mr, mt, mb] = o.margins.map(i32::from);
    let outer = hm_model::ObjectCommon {
        width: ml + o.width + mr,
        height: mt + o.height + mb,
        ..*o
    };
    let big = 1 << 28;
    let anchor = crate::objects::Anchor {
        paper: (width, big),
        paper_x: None,
        paper_y: None,
        flow_top: None,
        body: crate::paginate::Rect {
            x: 0,
            y: 0,
            w: width,
            h: big,
        },
        column: (0, width),
        para_top: 0,
        para_left,
        para_right,
    };
    crate::objects::frame(&outer, &anchor).0 + ml
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
            // Hancom lays a cell's text out at least 1,440 HU wide, however
            // its padding squeezes it.
            let inner = text_width(w - padding[0] - padding[1]);
            let env = ParaEnv {
                column_width: inner,
                default_tab,
            };
            let env = if c.vertical_text() {
                ParaEnv {
                    column_width: text_width(c.height - padding[2] - padding[3]),
                    ..env
                }
            } else {
                env
            };
            let (paras, lines, content, tables, objects) = stack_lines(
                &c.paras,
                info,
                styles,
                env,
                &mut heads,
                c.line_wrap() == 1,
                false,
                t.header.common.treat_as_char(),
            );
            CellLayout {
                x,
                w,
                padding,
                paras,
                lines,
                tables,
                objects,
                content,
                keep_end_spacing: crate::compat::effective(info)[1] & (1 << 7) != 0,
                vertical: c.vertical_text(),
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
            // Vertical text runs along the cell's height, which it keeps.
            if protected || l.vertical {
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
        if l.vertical {
            // Lines run the cell's laid-out height; Hancom stores them so.
            let env = ParaEnv {
                column_width: text_width(l.h - l.padding[2] - l.padding[3]),
                default_tab,
            };
            let (paras, lines, content, ..) = stack_lines(
                &c.paras,
                info,
                styles,
                env,
                &mut heads,
                false,
                false,
                t.header.common.treat_as_char(),
            );
            (l.paras, l.lines, l.content) = (paras, lines, content);
            l.tables.clear();
            l.objects.clear();
            continue;
        }
        let slack = l.h - l.padding[2] - l.padding[3] - held::alignment_reach(c, l);
        l.offset = match c.vertical_align() {
            1 if slack > 0 => slack / 2,
            2 if slack > 0 => slack,
            _ => 0,
        };
    }
    let width = col_x[cols] + sp;
    let caption = t.caption.as_ref().map(|c| {
        let side = c.side();
        let w = if side >= 2 {
            c.max_width.filter(|&m| m > 0).unwrap_or(width)
        } else {
            c.width.unwrap_or(0)
        };
        let env = ParaEnv {
            column_width: w.max(0) & !3,
            default_tab,
        };
        let (paras, lines, height) = stack(&c.paras, info, styles, env, &mut heads);
        CaptionLayout {
            side,
            gap: c.gap.unwrap_or(0) as i32,
            width: w,
            height,
            paras,
            lines,
        }
    });
    TableLayout {
        width,
        height: row_y[rows] + sp,
        col_x,
        row_y,
        spacing: sp,
        cells,
        caption,
        cuts: OnceLock::new(),
        spans: OnceLock::new(),
    }
}

/// Lays out the tables among a paragraph's controls, as
/// `(control index, layout)`.
pub fn layout_controls(
    para: &Paragraph,
    info: &DocInfo,
    styles: &mut Styles<'_>,
    default_tab: i32,
) -> Vec<(u32, std::sync::Arc<TableLayout>)> {
    para.controls
        .iter()
        .enumerate()
        .filter_map(|(k, c)| {
            c.as_table().map(|t| {
                (
                    k as u32,
                    std::sync::Arc::new(layout(t, info, styles, default_tab)),
                )
            })
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

    #[test]
    fn cell_alignment_includes_overlays_without_growing_the_row() {
        use hm_fonts::FontSystem;
        use hm_model::{Control, ControlKind, CtrlId, ObjectCommon, ObjectControl, ObjectHeader};
        let info = DocInfo::default();
        let fonts = FontSystem::new();
        for (height, offset, expected) in [(4000, 0, 0), (1000, 0, 1500), (4000, -3000, 1500)] {
            let mut c = cell(0, 0, 1, 1, 20000, 4000);
            c.list_attr = 1 << 5;
            c.paras.push(Paragraph {
                text: [vec![11, 0, 0, 0, 0, 0, 0, 11], vec!['A' as u16, 13]].concat(),
                controls: vec![Control {
                    kind: ControlKind::Object(Box::new(ObjectControl {
                        id: CtrlId::SHAPE,
                        header: ObjectHeader {
                            common: ObjectCommon {
                                attr: (2 << 3) | (3 << 21),
                                width: 1000,
                                height,
                                y_offset: offset,
                                ..Default::default()
                            },
                            ..Default::default()
                        },
                        parts: Vec::new(),
                    })),
                    src: None,
                }],
                ..Default::default()
            });
            let t = Table {
                rows: 1,
                cols: 1,
                cells: vec![c],
                ..Default::default()
            };
            let l = layout(&t, &info, &mut Styles::new(&info, &fonts), 4000);
            assert_eq!(l.height, 4000);
            assert_eq!(l.cells[0].content, 1000);
            assert_eq!(l.cells[0].offset, expected);
        }
    }

    #[test]
    fn paragraph_spacing_is_consumed_by_an_indivisible_tables_clearance() {
        use hm_model::{
            CharRun, CharShape, Control, ControlKind, ObjectCommon, ObjectHeader, ParaHeader,
            ParaShape, units::pack_distance,
        };

        let nested = Table {
            header: ObjectHeader {
                common: ObjectCommon {
                    attr: (2 << 3) | (1 << 21),
                    y_offset: 100,
                    width: 6000,
                    height: 3200,
                    ..Default::default()
                },
                ..Default::default()
            },
            rows: 1,
            cols: 1,
            cells: vec![cell(0, 0, 1, 1, 6000, 3200)],
            ..Default::default()
        };
        let host = Paragraph {
            text: vec![11, 0, 0, 0, 0, 0, 0, 11, 13],
            runs: vec![CharRun { pos: 0, shape: 0 }],
            controls: vec![Control {
                kind: ControlKind::Table(Box::new(nested)),
                src: None,
            }],
            ..Default::default()
        };
        let fonts = hm_fonts::FontSystem::new();
        for blank in [false, true] {
            let next = Paragraph {
                header: ParaHeader {
                    para_shape: 1,
                    ..Default::default()
                },
                text: if blank {
                    vec![13]
                } else {
                    "NEXT\r".encode_utf16().collect()
                },
                runs: vec![CharRun { pos: 0, shape: 1 }],
                ..Default::default()
            };
            for host_before in [0, 600] {
                for next_before in [0, 1000, 1800, 2400] {
                    let info = DocInfo {
                        char_shapes: [1000, 1000]
                            .map(|base_size| CharShape {
                                base_size,
                                ..Default::default()
                            })
                            .into(),
                        para_shapes: [host_before, next_before]
                            .map(|before| ParaShape {
                                space_before: pack_distance(before),
                                line_spacing_legacy: 130,
                                ..Default::default()
                            })
                            .into(),
                        ..Default::default()
                    };
                    let (_, lines, content, tables, _) = stack_lines(
                        &[host.clone(), next.clone()],
                        &info,
                        &mut Styles::new(&info, &fonts),
                        ParaEnv {
                            column_width: 6000,
                            default_tab: 4000,
                        },
                        &mut Heads::default(),
                        false,
                        false,
                        false,
                    );
                    assert_eq!(tables[0].y, 100);
                    assert_eq!(lines[0].y, host_before);
                    let next_top = 3300.max(host_before + 1300 + next_before);
                    assert_eq!(lines[1].y, next_top);
                    assert_eq!(content, next_top + 1000);
                }
            }
        }
    }
}
