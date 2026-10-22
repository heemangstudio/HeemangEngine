//! Tables: the `tbl ` control with its grid, cells and caption.

use crate::{Caption, ExtraRecord, ObjectHeader, Paragraph, SharedBytes};

/// A table control.
#[derive(Clone, Debug, Default)]
pub struct Table {
    /// Position, size, wrap, outer margins and the rest of the header.
    pub header: ObjectHeader,
    pub caption: Option<Caption>,
    /// Bits 0-1 split at a page boundary (0 never, 1 anywhere, 2 between
    /// rows; OWPML `NONE`, `TABLE`, `CELL`), bit 2 repeat header rows, bit 3
    /// no automatic size adjustment.
    pub attr: u32,
    pub rows: u16,
    pub cols: u16,
    pub spacing: i16,
    /// Default cell padding: left, right, top, bottom.
    pub padding: [i16; 4],
    /// Cells starting in each row, as stored.
    pub row_cells: Vec<u16>,
    pub border_fill: u16,
    /// Border fills applied to cell blocks; `None` when the record has no
    /// zone count.
    pub zones: Option<Vec<CellZone>>,
    /// Table record bytes after the zones.
    pub tail: SharedBytes,
    /// Cells in stored order (row by row).
    pub cells: Vec<Cell>,
    /// Records of the control the model does not interpret. A slot counts
    /// the caption, the table record and the cells before the record.
    pub extra: Vec<ExtraRecord>,
}

impl Table {
    /// 0 never split, 1 split between rows (HWPX `TABLE`), 2 split inside
    /// cells too (HWPX `CELL`).
    pub fn split(&self) -> u32 {
        self.attr & 3
    }

    pub fn repeat_header(&self) -> bool {
        self.attr & 4 != 0
    }

    /// The padding of `cell`: its own when it says so, the table's otherwise.
    pub fn cell_padding(&self, cell: &Cell) -> [i32; 4] {
        if cell.own_padding() {
            cell.padding.map(i32::from)
        } else {
            self.padding.map(i32::from)
        }
    }
}

/// A block of cells drawn with one border fill.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CellZone {
    pub start_col: u16,
    pub start_row: u16,
    pub end_col: u16,
    pub end_row: u16,
    pub border_fill: u16,
}

/// A table cell: its list header fields and paragraphs.
#[derive(Clone, Debug, Default)]
pub struct Cell {
    /// List attribute: bits 0-2 text direction (1 vertical), 3-4 line
    /// wrap, 5-6 vertical alignment, 16 own padding, 17 protected, 18
    /// header cell, 19 editable in form mode, the rest as stored.
    pub list_attr: u32,
    pub col: u16,
    pub row: u16,
    pub col_span: u16,
    pub row_span: u16,
    pub width: i32,
    pub height: i32,
    /// Left, right, top, bottom.
    pub padding: [i16; 4],
    pub border_fill: u16,
    /// List header bytes after the border fill (text width, field name and
    /// newer fields).
    pub tail: SharedBytes,
    pub paras: Vec<Paragraph>,
}

impl Cell {
    /// 0 top, 1 center, 2 bottom.
    pub fn vertical_align(&self) -> u32 {
        (self.list_attr >> 5) & 3
    }

    pub fn vertical_text(&self) -> bool {
        self.list_attr & 7 == 1
    }

    /// 0 wrap, 1 keep one line by condensing, 2 widen with the content.
    pub fn line_wrap(&self) -> u32 {
        (self.list_attr >> 3) & 3
    }

    /// The cell's padding overrides the table's.
    pub fn own_padding(&self) -> bool {
        self.list_attr & (1 << 16) != 0
    }

    /// Protected from editing.
    pub fn protected(&self) -> bool {
        self.list_attr & (1 << 17) != 0
    }

    /// A header cell (제목 셀); rows of header cells repeat on each page.
    pub fn header(&self) -> bool {
        self.list_attr & (1 << 18) != 0
    }

    pub fn end_col(&self) -> u32 {
        self.col as u32 + self.col_span.max(1) as u32
    }

    pub fn end_row(&self) -> u32 {
        self.row as u32 + self.row_span.max(1) as u32
    }
}
