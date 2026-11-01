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

pub(crate) use flow::{
    Deferred, Pending, after_line, before_host, before_line, host_takes_room, resume, resume_queued,
};
pub use flow::{PlacedTable, flows, flows_in, has_content};
pub use held::{CellObject, HeldFloor, escapes_cell, para_left};
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
    /// Physical room kept by a table whose size may be adjusted.
    pub bottom_room: i32,
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
    /// Paragraphs whose top-and-bottom objects push what follows below them.
    pub floors: Vec<HeldFloor>,
    /// Content height: the last line's bottom.
    pub content: i32,
    /// The compatibility profile retains the last paragraph's spacing.
    pub keep_end_spacing: bool,
    /// Shift of the content for vertical alignment.
    pub offset: i32,
    /// The part of `offset` splitting keeps above the content.
    pub split_shift: i32,
    /// The text runs down the cell: its lines are laid out as long as the
    /// cell is high and stack across it from the right.
    pub vertical: bool,
    /// The table's size is not adjusted to its page: a line cut at the page
    /// end stays when it ends, with the cell's padding, above the bottom
    /// margin, without the cut room.
    pub unadjusted: bool,
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
    /// For each cut, the room the fragment above it keeps below it, and how
    /// far below the cut the rest starts at the top of the next fragment.
    pub below: std::sync::Arc<[i32]>,
    pub above: std::sync::Arc<[i32]>,
    /// For each cut, when the piece ending there holds nothing but the rest
    /// of a row cut above it: the bottom padding of the cells that cut passes
    /// through; otherwise `None`.
    pub rests: std::sync::Arc<[Option<i32>]>,
    /// The table's top margin: it goes with the table to the next part.
    pub margin: i32,
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
        self.y + self.padding[2] + self.split_shift
    }

    /// The room a line keeps above a cut through this cell, besides the
    /// cell's padding: none in a table whose size is not adjusted (probes
    /// cnm40-na, cnm80-na: a line ending 60 and 20 HU above the margin line
    /// with its padding stays, cnm120-na 20 below it goes on; 1342000 p137,
    /// p361; adjusted, the -adj probes, all three go on).
    pub fn cut_room(&self) -> i32 {
        if self.unadjusted { 0 } else { split::CUT_ROOM }
    }

    /// Whether a line of this cell, or of a table nested in it, lies between
    /// table heights `from` and `to`.
    fn holds_lines(&self, from: i32, to: i32) -> bool {
        let top = self.split_y();
        let line = self.lines.iter().any(|l| {
            let h = self
                .paras
                .get(l.para as usize)
                .and_then(|p| p.lines.get(l.line as usize))
                .map_or(0, |ln| ln.height);
            top + l.y < to && from < top + l.y + h
        });
        line || self.tables.iter().any(|ct| {
            let origin = top + ct.y;
            self.paras
                .get(ct.para as usize)
                .and_then(|p| p.table(ct.control))
                .is_some_and(|ntl| {
                    ntl.cells
                        .iter()
                        .any(|ncl| ncl.holds_lines(from - origin, to - origin))
                })
        })
    }

    /// Where this cell goes on when the first of its spans not staying starts
    /// at table height `t`, spans reaching no lower than `end` staying: past
    /// the rest of a row of a nested table holding nothing, when no more of
    /// the row than Hancom drops stands below the page end (probes
    /// nf2797p1000: a rest of 1,105 HU is dropped; nf2797p1400: 1,513 HU go on).
    pub(crate) fn past_row_rest(&self, t: i32, end: i32) -> i32 {
        let top = self.split_y();
        for ct in &self.tables {
            let origin = top + ct.y;
            let Some(i) = ct.cuts.iter().position(|&c| origin + c == t) else {
                continue;
            };
            let (Some(&Some(pad)), Some(&edge), Some(&below)) =
                (ct.rests.get(i + 1), ct.cuts.get(i + 1), ct.below.get(i))
            else {
                continue;
            };
            // The piece's part ends where the room it keeps, its cells'
            // padding aside, starts.
            if origin + edge - (end - below + pad) <= flow::CARRY_REST {
                return origin + edge;
            }
        }
        t
    }

    /// The room kept below a cut through this cell at table height `at`
    /// where a piece of a table nested in it ends.
    pub fn room_below(&self, at: i32) -> i32 {
        self.nested_cut(at).map_or(0, |(ct, i)| ct.below[i])
    }

    /// How far below table height `first` the rest of this cell starts when
    /// it goes on there at the top of a fragment: a table nested in it that
    /// is cut there starts again below its top margin.
    pub fn resume_offset(&self, first: i32) -> i32 {
        self.nested_cut(first).map_or(0, |(ct, i)| ct.above[i])
    }

    /// Whether the piece of a nested table ending at table height `at` is cut
    /// where its content lies aligned in its cell's whole height.
    fn cut_aligned(&self, at: i32) -> bool {
        let Some((ct, _)) = self.nested_cut(at) else {
            return false;
        };
        let origin = self.split_y() + ct.y;
        self.paras
            .get(ct.para as usize)
            .and_then(|p| p.table(ct.control))
            .is_some_and(|ntl| {
                ntl.cells.iter().any(|ncl| {
                    ncl.y < at - origin
                        && at - origin < ncl.y + ncl.h
                        && (ncl.split_shift != 0 || ncl.cut_aligned(at - origin))
                })
            })
    }

    fn nested_cut(&self, at: i32) -> Option<(&CellTable, usize)> {
        let top = self.split_y();
        self.tables.iter().find_map(|ct| {
            let i = ct.cuts.iter().position(|&c| top + ct.y + c == at)?;
            (i < ct.below.len() && i < ct.above.len()).then_some((ct, i))
        })
    }
}

impl TableLayout {
    /// A physical tail already occupied part of a merged cell's minimum.
    /// Recompose its remaining grid without changing paragraph or line data.
    pub(crate) fn with_spent_minima(&self, t: &Table, spent: &[i32]) -> Option<Self> {
        if t.header.common.attr & (1 << 20) != 0 {
            return None;
        }
        let rows = self.row_y.len().saturating_sub(1);
        let row_y = edges(
            rows,
            self.spacing,
            &t.cells,
            |c| row_span(c, rows),
            |k| {
                let (c, cl) = (&t.cells[k], &self.cells[k]);
                let minimum = (c.height - spent.get(k).copied().unwrap_or(0)).max(0);
                if cl.vertical {
                    // Its line breaking depends on this vertical text width.
                    cl.h
                } else {
                    minimum.max(cl.content + cl.padding[2] + cl.padding[3])
                }
            },
        );
        if row_y == self.row_y {
            return None;
        }
        if t.cells.iter().zip(&self.cells).any(|(cell, cl)| {
            let (r0, r1) = row_span(cell, rows);
            cl.vertical && row_y[r1] - row_y[r0] - self.spacing != cl.h
        }) {
            return None;
        }
        let mut out = self.clone();
        out.row_y = row_y;
        out.height = out.row_y[rows] + out.spacing;
        out.cuts = OnceLock::new();
        out.spans = OnceLock::new();
        for (c, cl) in t.cells.iter().zip(&mut out.cells) {
            let (r0, r1) = row_span(c, rows);
            cl.y = out.row_y[r0] + out.spacing;
            cl.h = out.row_y[r1] - cl.y;
            if cl.vertical {
                continue;
            }
            let slack = cl.h - cl.padding[2] - cl.padding[3] - held::alignment_reach(c, cl);
            cl.offset = match c.vertical_align() {
                1 => slack.max(0) / 2,
                2 => slack.max(0),
                _ => 0,
            };
        }
        Some(out)
    }

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

    /// A new row keeps its first source line by the current physical cut.
    pub fn first_line_stays(
        &self,
        cell: usize,
        from: i32,
        top: i32,
        bottom: i32,
        limit: i32,
    ) -> bool {
        let Some(cl) = self.cells.get(cell) else {
            return false;
        };
        let Some(first) = cl.lines.first() else {
            return false;
        };
        let end = limit - self.bottom_room;
        if cl.vertical
            || !cl.objects.is_empty()
            || !cl.tables.is_empty()
            || cl.y < from
            || first.para != 0
            || first.line != 0
            || top != cl.split_y() + first.y
            || bottom + cl.padding[3] > end
        {
            return false;
        }
        let height = cl
            .paras
            .first()
            .and_then(|p| p.lines.first())
            .map_or(0, |l| l.height);
        if bottom != top + height {
            return false;
        }
        let Some(r) = self
            .row_y
            .partition_point(|&y| y <= limit - CUT_ROOM.max(self.bottom_room))
            .checked_sub(1)
        else {
            return false;
        };
        let (row_top, Some(&row_end)) = (self.row_y[r], self.row_y.get(r + 1)) else {
            return false;
        };
        if cl.y != row_top + self.spacing {
            return false;
        }
        row_end <= end
            || self.cells.iter().enumerate().any(|(k, other)| {
                self.cell_spans(k)
                    .iter()
                    .any(|&(a, b)| row_top <= a && a < row_end && b + other.padding[3] > end)
            })
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

/// [`stack`] with the tables flowing in the list's text, placed in it.
pub(crate) fn stack_tables(
    paras: &[Paragraph],
    info: &DocInfo,
    styles: &mut Styles<'_>,
    env: ParaEnv,
    heads: &mut Heads,
) -> (Vec<ParaLayout>, Vec<CellLine>, i32, Vec<CellTable>) {
    let (layouts, lines, content, tables, ..) =
        stack_lines(paras, info, styles, env, heads, false, false, false);
    (layouts, lines, content, tables)
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
    let (layouts, lines, content, tables, ..) =
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
    Vec<HeldFloor>,
) {
    let mut layouts = Vec::with_capacity(paras.len());
    let mut lines = Vec::new();
    let mut tables = Vec::new();
    let mut objects = Vec::new();
    let mut floors = Vec::new();
    let mut table_bands = Vec::new();
    let mut y = 0;
    // Bottom of the tables that flow in the text of earlier paragraphs: the
    // text cursor may stay above it, and the next paragraph's spacing
    // before is consumed by clearing it.
    let mut floor = 0;
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
        let pl = nested_shift(p, pl);
        let para_top = y.max(floor);
        y = (y + pl.before).max(floor);
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
            // Tables that may split hang from the same top as the others
            // and stack below one another.
            let mut stacked = anchor_top;
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
                        below: cut_room(t, tl, mb, true),
                        above: cut_room(t, tl, mt, false),
                        rests: row_rests(t, tl),
                        margin: mt,
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
                    (anchor_top + o.y_offset).max(stacked)
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
                    below: cut_room(t, tl, mb, true),
                    above: cut_room(t, tl, mt, false),
                    rests: row_rests(t, tl),
                    margin: mt,
                });
                let bottom = outer_top + mt + h + mb;
                // Either way the table's outer box is a band the later lines
                // clear: a line of its paragraph that ends above the table's
                // outer top stays there (59043 p36: the host's second line
                // above a table hung 3407 HU below the first).
                table_bands.push(crate::objects::Band {
                    top: outer_top,
                    bottom,
                    right: env.column_width,
                    ..Default::default()
                });
                if !indivisible {
                    covers |= outer_top < line_top + l.height;
                    below = bottom;
                    stacked = bottom;
                }
                // The cell's content reaches the table's bottom outer
                // margin, as Hancom sizes a row ending in such a table.
                content = content.max(bottom);
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
            tables_end = tables_end.max(below);
            y = text_end;
        }
        // Paragraph spacing advances the text cursor before a later line
        // clears a floating table. The clearance can consume that spacing.
        y = text_end + pl.after;
        floor = floor.max(tables_end);
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
        if floor > y {
            floors.push(HeldFloor {
                para: pi as u32,
                text_end: y,
                floor,
            });
        }
        y = y.max(floor);
        layouts.push(pl);
    }
    if crate::compat::effective(info)[1] & (1 << 7) != 0 {
        let bottom = table_bands.iter().map(|b| b.bottom).max().unwrap_or(0);
        content = content.max(y).max(bottom);
    }
    (layouts, lines, content, tables, objects, floors)
}

/// A one-cell table flowing in a cell's text, cut across pages, is cut
/// where its content lies after its vertical alignment in the cell's whole
/// height: Hancom cuts it as aligned and aligns each part again.
fn nested_shift(p: &Paragraph, mut pl: ParaLayout) -> ParaLayout {
    for (k, tl) in &mut pl.tables {
        let one = p
            .controls
            .get(*k as usize)
            .and_then(|c| c.as_table())
            .is_some_and(|t| t.rows == 1 && t.cols == 1 && !t.header.common.treat_as_char());
        if one && tl.cells.len() == 1 && tl.cells[0].offset != 0 {
            let tl = std::sync::Arc::make_mut(tl);
            tl.cells[0].split_shift = tl.cells[0].offset;
            tl.cuts = OnceLock::new();
            tl.spans = OnceLock::new();
        }
    }
    pl
}

/// What the fragments of table `t` nested in a cell keep at each of its
/// cuts. Hancom keeps the table's outer margins on both sides of a cut: with
/// `below`, the room the fragment above the cut keeps under it, the bottom
/// margin `margin` and the bottom padding and cut room of the cells a cut
/// inside a row passes through, each nesting level having its own (probes
/// nf1565p100, nf1565p200: a nested row's line stays 82 HU above that and
/// goes on with its table 17 HU below it), with what a table nested in them
/// keeps; otherwise how
/// far below the cut the rest starts on the next page, the top margin
/// `margin`, the header rows the table repeats (Hancom repeats a nested
/// table's header rows at the top of each page too:
/// table-giant-cell-overfill p43-p46) and, inside a row, the cut cells'
/// lines going on below their top padding instead of below the gap left at
/// the cut.
/// [`CellTable::rests`] for nested table `t`.
fn row_rests(t: &Table, tl: &TableLayout) -> std::sync::Arc<[Option<i32>]> {
    let cuts = tl.legal_cuts(t);
    let mut from = 0;
    cuts.iter()
        .map(|&c| {
            let start = std::mem::replace(&mut from, c);
            (start > 0
                && tl.row_y.binary_search(&start).is_err()
                && tl.row_y.binary_search(&c).is_ok()
                && tl.cells.iter().all(|cl| !cl.holds_lines(start, c)))
            .then(|| {
                tl.cells
                    .iter()
                    .filter(|cl| cl.y < start && start < cl.y + cl.h)
                    .map(|cl| cl.padding[3].max(0))
                    .max()
                    .unwrap_or(0)
            })
        })
        .collect()
}

fn cut_room(t: &Table, tl: &TableLayout, margin: i32, below: bool) -> std::sync::Arc<[i32]> {
    let margin = margin.max(0);
    let header = if below {
        0
    } else {
        split::header_height(t, tl)
    };
    tl.legal_cuts(t)
        .iter()
        .map(|&c| {
            let crossing = tl
                .cells
                .iter()
                .enumerate()
                .filter(|(_, cl)| cl.y < c && c < cl.y + cl.h);
            let inner = if below {
                crossing
                    .map(|(_, cl)| {
                        // One cut where it lies aligned in its whole height
                        // already keeps what it leaves below (issue2007 p10).
                        let room = if cl.split_shift != 0 || cl.cut_aligned(c) {
                            0
                        } else {
                            cl.cut_room()
                        };
                        cl.padding[3].max(0) + room + cl.room_below(c)
                    })
                    .max()
            } else {
                crossing
                    .filter_map(|(k, cl)| {
                        let nested = cl.resume_offset(c);
                        if cl.split_shift != 0 {
                            return Some(nested);
                        }
                        let &(top, _) = tl.cell_spans(k).iter().find(|s| s.0 >= c)?;
                        Some(cl.padding[2].max(0) - (top - split::lead(cl, top) - c) + nested)
                    })
                    .max()
            };
            let repeat = if header > 0 && c >= header { header } else { 0 };
            margin + repeat + inner.unwrap_or(0)
        })
        .collect()
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
        // A row its ending cells leave short of the edge above still
        // starts there: rows never run backwards.
        e[i] = best.map_or(e[i - 1] + MIN_TRACK, |b| b.max(e[i - 1]));
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
            // A negative stored padding counts as none: the text starts at
            // the cell's top, so cutting the table at a row edge never
            // leaves a line's first HU on the page before.
            let padding = t.cell_padding(c).map(|p| p.max(0));
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
            let (paras, lines, content, tables, objects, floors) = stack_lines(
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
                floors,
                content,
                keep_end_spacing: crate::compat::effective(info)[1] & (1 << 7) != 0,
                vertical: c.vertical_text(),
                unadjusted: t.attr & 8 != 0,
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
            l.floors.clear();
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
        bottom_room: if t.attr & 8 != 0 { 0 } else { CUT_ROOM + 1 },
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

    #[test]
    fn minimum_credit_preserves_vertical_text_width_and_protected_heights() {
        let t = Table {
            rows: 3,
            cols: 3,
            cells: vec![
                cell(0, 0, 2, 1, 1000, 4500),
                cell(1, 1, 2, 1, 1000, 4500),
                cell(0, 2, 1, 1, 1000, 2000),
                cell(1, 2, 1, 1, 1000, 2000),
                cell(2, 2, 1, 1, 1000, 2000),
            ],
            ..Default::default()
        };
        let cl = |y, h| CellLayout {
            y,
            h,
            content: 1000,
            ..Default::default()
        };
        let mut tl = TableLayout {
            row_y: vec![0, 2000, 4500, 6500],
            height: 6500,
            cells: vec![
                cl(0, 4500),
                cl(2000, 4500),
                cl(0, 2000),
                cl(2000, 2500),
                cl(4500, 2000),
            ],
            ..Default::default()
        };
        tl.cells[4].vertical = true;
        tl.cells[4].paras.push(ParaLayout {
            column_width: 2000,
            ..Default::default()
        });
        // The first owner can shrink the middle edge, but the second
        // owner's minimum still holds the far edge. The later vertical
        // column would grow to 2200 without rebreaking its source lines.
        assert!(tl.with_spent_minima(&t, &[200, 0, 0, 0, 0]).is_none());
        let mut protected = t.clone();
        protected.header.common.attr |= 1 << 20;
        tl.cells[4].vertical = false;
        assert!(
            tl.with_spent_minima(&protected, &[200, 0, 0, 0, 0])
                .is_none()
        );
    }

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
    fn rows_ending_only_short_merged_cells_keep_the_edge_above() {
        // Row 1 ends only a merged cell shorter than row 0 above it, as
        // when a merged cell's minimum is lifted to find its natural rows.
        let cells = [cell(0, 0, 1, 1, 1000, 5000), cell(0, 1, 2, 1, 1000, 1000)];
        let e = edges(2, 0, &cells, |c| row_span(c, 2), |k| cells[k].height);
        assert_eq!(e, [0, 5000, 5000]);
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
    fn paragraph_spacing_is_consumed_by_a_cell_tables_clearance() {
        // 86712 p29 (Hancom PDF): tables that may split hang from their
        // paragraph's top as indivisible ones do, and the paragraph after
        // them starts at their bottom margin with its spacing before
        // consumed; we had both 200 HU lower for each of two tables.
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
        let fonts = hm_fonts::FontSystem::new();
        for (split, blank) in [(0, false), (0, true), (1, false), (2, false), (2, true)] {
            let host = Paragraph {
                text: vec![11, 0, 0, 0, 0, 0, 0, 11, 13],
                runs: vec![CharRun { pos: 0, shape: 0 }],
                controls: vec![Control {
                    kind: ControlKind::Table(Box::new(Table {
                        attr: split,
                        ..nested.clone()
                    })),
                    src: None,
                }],
                ..Default::default()
            };
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
                    let (_, lines, content, tables, ..) = stack_lines(
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

    #[test]
    fn host_lines_above_a_splittable_tables_offset_stay_above_it() {
        use hm_model::{
            CharRun, CharShape, Control, ControlKind, ObjectCommon, ObjectHeader, ParaShape,
        };

        // 59043 p36: a cell paragraph opens with a table that may split,
        // wrapped top and bottom 3407 HU below the paragraph's top; its two
        // text lines end above that and stay there in Hancom's PDF and
        // stored lines. We had the second line below the table.
        let fonts = hm_fonts::FontSystem::new();
        for split in [0, 1, 2] {
            let nested = Table {
                attr: split,
                header: ObjectHeader {
                    common: ObjectCommon {
                        attr: (2 << 3) | (1 << 21),
                        y_offset: 2700,
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
            let mut text = vec![11, 0, 0, 0, 0, 0, 0, 11];
            text.extend("WORD WORD WORD WORD WORD WORD WORD\r".encode_utf16());
            let host = Paragraph {
                text,
                runs: vec![CharRun { pos: 0, shape: 0 }],
                controls: vec![Control {
                    kind: ControlKind::Table(Box::new(nested)),
                    src: None,
                }],
                ..Default::default()
            };
            let info = DocInfo {
                char_shapes: vec![CharShape {
                    base_size: 1000,
                    ..Default::default()
                }],
                para_shapes: vec![ParaShape {
                    line_spacing_legacy: 130,
                    ..Default::default()
                }],
                ..Default::default()
            };
            let (_, lines, content, tables, ..) = stack_lines(
                &[host],
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
            // Lines of 1000 HU every 1300: the first two end above the
            // table's top at 2700, the others go below its bottom at 5900.
            let tops: Vec<i32> = lines.iter().map(|l| l.y).collect();
            assert_eq!(tops, [0, 1300, 5900, 7200], "split {split}");
            assert_eq!(tables[0].y, 2700, "split {split}");
            assert_eq!(content, 8200, "split {split}");
        }
    }

    #[test]
    fn negative_cell_padding_keeps_a_row_whole_at_a_page_end() {
        use hm_model::{CharRun, CharShape};

        // 2017 시장구조조사 p146-147: every data cell stores a top and bottom
        // padding of -1. A page end short of a row's line moves that row to
        // the next page with its text, as Hancom prints it.
        let row = |r: u16| {
            let mut c = cell(r, 0, 1, 1, 6000, 1650);
            c.list_attr = (1 << 16) | (1 << 5);
            c.padding = [0, 0, -1, -1];
            c.paras.push(Paragraph {
                text: "A\r".encode_utf16().collect(),
                runs: vec![CharRun { pos: 0, shape: 0 }],
                ..Default::default()
            });
            c
        };
        let t = Table {
            rows: 3,
            cols: 1,
            attr: 2,
            cells: (0..3).map(row).collect(),
            ..Default::default()
        };
        let info = DocInfo {
            char_shapes: vec![CharShape {
                base_size: 700,
                ..Default::default()
            }],
            para_shapes: vec![Default::default()],
            ..Default::default()
        };
        let fonts = hm_fonts::FontSystem::new();
        let tl = layout(&t, &info, &mut Styles::new(&info, &fonts), 4000);
        assert_eq!(tl.row_y, [0, 1650, 3300, 4950]);
        let cl = &tl.cells[1];
        assert!(cl.content > 0 && cl.content + 300 < 1650);
        // A page end inside row 1 above its line's bottom: no part of the
        // row stays, so the row moves on whole instead of being carried
        // empty with its line lost.
        let limit = 1650 + cl.content / 2 + split::CUT_ROOM;
        assert!(split::cut_cells(&t, &tl, 0, limit).is_none());
        assert_eq!(cut(&t, &tl, SplitMode::of(&t), 0, limit), Some(1650));
        // The line stands inside its cell, centred as before.
        assert_eq!(cl.split_y(), cl.y);
        assert_eq!(cl.content_y(), cl.y + (1650 - cl.content) / 2);
    }

    #[test]
    fn a_trailing_flowing_table_keeps_its_bottom_margin_in_the_cell() {
        use hm_model::{CharRun, CharShape, Control, ControlKind, ObjectCommon, ObjectHeader};

        // 곡성 바우처 p2: 칸의 마지막 문단에 떠 있는 표가 있으면 한컴은 표의
        // 아래 바깥 여백까지 칸 내용으로 잡는다.
        for split in [false, true] {
            let nested = Table {
                header: ObjectHeader {
                    common: ObjectCommon {
                        attr: (2 << 3) | (1 << 21),
                        y_offset: 530,
                        width: 6000,
                        height: 3200,
                        margins: [141, 141, 141, 141],
                        ..Default::default()
                    },
                    ..Default::default()
                },
                rows: 1,
                cols: 1,
                cells: vec![cell(0, 0, 1, 1, 6000, 3200)],
                attr: if split { 2 } else { 0 },
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
            let info = DocInfo {
                char_shapes: vec![CharShape {
                    base_size: 600,
                    ..Default::default()
                }],
                para_shapes: vec![Default::default()],
                ..Default::default()
            };
            let fonts = hm_fonts::FontSystem::new();
            let (_, _, content, tables, ..) = stack_lines(
                &[host],
                &info,
                &mut Styles::new(&info, &fonts),
                ParaEnv {
                    column_width: 6400,
                    default_tab: 4000,
                },
                &mut Heads::default(),
                false,
                false,
                false,
            );
            assert_eq!(tables[0].y, 530 + 141);
            assert_eq!(content, 530 + 141 + 3200 + 141, "split {split}");
        }
    }
    #[test]
    fn a_nested_table_cut_across_pages_keeps_its_margins_on_both_sides() {
        use hm_model::{CharRun, CharShape, Control, ControlKind, ObjectCommon, ObjectHeader};

        // 한컴은 칸 안에서 잘린 표의 두 조각 모두에 바깥 여백을 둔다. 위
        // 조각 아래에는 아래 여백과 잘린 칸의 아래 안 여백을, 다음 조각 위에는
        // 위 여백과 위 안 여백을 둔다. 가운데 정렬로 남는 높이가 있는 칸은
        // 정렬된 자리에서 잘리고 각 조각이 다시 정렬하므로 위 여백만 둔다.
        let text = Paragraph {
            text: (0..4)
                .flat_map(|i| [0xAC00, if i == 3 { hm_model::PARA_END } else { 10 }])
                .collect(),
            ..Default::default()
        };
        for (height, centered) in [(282, false), (20000, true)] {
            let mut inner = cell(0, 0, 1, 1, 6000, height);
            inner.paras = vec![text.clone()];
            inner.list_attr = if centered { 1 << 5 } else { 0 };
            let nested = Table {
                header: ObjectHeader {
                    common: ObjectCommon {
                        attr: (2 << 3) | (1 << 21),
                        width: 6000,
                        height,
                        margins: [283, 283, 283, 283],
                        ..Default::default()
                    },
                    ..Default::default()
                },
                rows: 1,
                cols: 1,
                cells: vec![inner],
                attr: 2,
                padding: [510, 510, 141, 141],
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
            let info = DocInfo {
                char_shapes: vec![CharShape {
                    base_size: 1000,
                    ratios: [100; 7],
                    rel_sizes: [100; 7],
                    ..Default::default()
                }],
                para_shapes: vec![hm_model::ParaShape {
                    line_spacing_legacy: 160,
                    ..Default::default()
                }],
                ..Default::default()
            };
            let fonts = hm_fonts::FontSystem::new();
            let (_, _, _, tables, ..) = stack_lines(
                &[host],
                &info,
                &mut Styles::new(&info, &fonts),
                ParaEnv {
                    column_width: 6400,
                    default_tab: 4000,
                },
                &mut Heads::default(),
                false,
                false,
                false,
            );
            let t = &tables[0];
            assert_eq!(t.cuts.len(), 4, "{t:?}");
            // The fragment keeps the margin, the padding and, cut where its
            // lines flow, its own cut room.
            let room = if centered { 0 } else { split::CUT_ROOM };
            assert!(t.below.iter().all(|&b| b == 283 + 141 + room), "{t:?}");
            // Cuts fall at line bottoms, the next line 600 lower: it lands
            // below the top margin and padding of the next fragment.
            for &above in &t.above[..3] {
                if centered {
                    assert_eq!(above, 283, "{t:?}");
                } else {
                    assert_eq!(above + 600, 283 + 141, "{t:?}");
                }
            }
        }
    }
}
