//! Tables: the `tbl ` control with its grid, cells and caption.

use crate::{ExtraRecord, ObjectCommon, Paragraph, SharedBytes};

/// A table control.
#[derive(Clone, Debug, Default)]
pub struct Table {
    /// Position, size, wrap and outer margins.
    pub common: ObjectCommon,
    /// Control header bytes after the common fields (instance id, page
    /// break prevention, description and anything newer).
    pub header_tail: SharedBytes,
    pub caption: Option<Caption>,
    /// Bits 0-1 split at a page boundary (0 never, 1 between rows, 2
    /// anywhere), bit 2 repeat header rows.
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
    /// 0 never split, 1 split between rows, 2 split anywhere.
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
    /// wrap, 5-6 vertical alignment, 16 own padding, the rest as stored.
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

    pub fn end_col(&self) -> u32 {
        self.col as u32 + self.col_span.max(1) as u32
    }

    pub fn end_row(&self) -> u32 {
        self.row as u32 + self.row_span.max(1) as u32
    }
}

/// An object caption: list header fields, caption attributes and paragraphs.
#[derive(Clone, Debug, Default)]
pub struct Caption {
    pub list_attr: u32,
    /// Bits 0-1 side (left, right, top, bottom), bit 2 width includes the
    /// object margin.
    pub attr: u32,
    /// Width of a side caption.
    pub width: i32,
    /// Gap between caption and object.
    pub gap: i16,
    /// Longest text width (the object width).
    pub max_width: i32,
    pub tail: SharedBytes,
    pub paras: Vec<Paragraph>,
}

impl Caption {
    /// 0 left, 1 right, 2 top, 3 bottom.
    pub fn side(&self) -> u32 {
        self.attr & 3
    }
}
