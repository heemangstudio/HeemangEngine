//! Page furniture controls: headers and footers, footnotes and endnotes,
//! automatic and new numbers, page number position, page hiding and the
//! odd/even page adjustment, and the paragraph lists they carry.

use crate::{Paragraph, SharedBytes};

/// A paragraph list with its list header (HWP 5 table 65, OWPML
/// `subList`): stored paragraph count, list attributes, the text area of
/// headers, footers and master pages, and whatever follows.
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ParaList {
    pub count: u32,
    /// Bits 0-2 text direction, 3-4 line wrap, 5-6 vertical alignment
    /// (top, center, bottom).
    pub attr: u32,
    /// Text area width and height (headers, footers, master pages).
    pub text_size: Option<(i32, i32)>,
    /// Bit sets of the levels whose text and numbers the list refers to.
    pub refs: Option<(u8, u8)>,
    pub tail: SharedBytes,
    pub paras: Vec<Paragraph>,
}

impl ParaList {
    /// 0 top, 1 center, 2 bottom.
    pub fn vertical_align(&self) -> u32 {
        (self.attr >> 5) & 3
    }
}

/// Which pages a header, footer or master page applies to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum PageSide {
    Both,
    Even,
    Odd,
}

impl PageSide {
    pub fn from_bits(v: u32) -> PageSide {
        match v & 3 {
            1 => PageSide::Even,
            2 => PageSide::Odd,
            _ => PageSide::Both,
        }
    }

    /// True when the side covers page number `n` (1-based).
    pub fn covers(self, n: u32) -> bool {
        match self {
            PageSide::Both => true,
            PageSide::Even => n % 2 == 0,
            PageSide::Odd => n % 2 == 1,
        }
    }
}

/// A header (`head`) or footer (`foot`).
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct HeaderFooter {
    pub footer: bool,
    /// Bits 0-1: both, even or odd pages.
    pub attr: u32,
    /// The rest of the control header (Hancom stores an instance number).
    pub tail: SharedBytes,
    pub list: ParaList,
}

impl HeaderFooter {
    pub fn side(&self) -> PageSide {
        PageSide::from_bits(self.attr)
    }
}

/// A footnote (`fn  `) or endnote (`en  `).
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Note {
    pub endnote: bool,
    /// The number as Hancom last assigned it (page-local when footnotes
    /// restart on every page).
    pub number: u32,
    /// Character the reference mark repeats for the user-character format
    /// (0 when none).
    pub user_char: u16,
    /// Prefix and suffix characters of the reference mark (0 when none).
    pub prefix: u16,
    pub suffix: u16,
    /// The note shape Hancom mirrors into the control: bits 0-7 the number
    /// format code, bit 8 footnotes restarting on every page. Other bits are
    /// kept as read.
    pub flag: u32,
    /// Hancom's instance id, when the file stores one.
    pub inst_id: Option<u32>,
    pub tail: SharedBytes,
    pub list: ParaList,
}

/// What an automatic or new number counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum NumberKind {
    Page,
    Footnote,
    Endnote,
    Figure,
    Table,
    Equation,
    Other(u32),
}

impl NumberKind {
    pub fn from_bits(v: u32) -> NumberKind {
        match v & 0xF {
            0 => NumberKind::Page,
            1 => NumberKind::Footnote,
            2 => NumberKind::Endnote,
            3 => NumberKind::Figure,
            4 => NumberKind::Table,
            5 => NumberKind::Equation,
            k => NumberKind::Other(k),
        }
    }
}

/// An automatic number (`atno`): the current page, note, figure, table or
/// equation number drawn in text.
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AutoNumber {
    /// Bits 0-3 kind, 4-11 number shape, 12 superscript.
    pub attr: u32,
    pub number: u16,
    pub user_char: u16,
    pub prefix: u16,
    pub suffix: u16,
    pub tail: SharedBytes,
}

impl AutoNumber {
    pub fn kind(&self) -> NumberKind {
        NumberKind::from_bits(self.attr)
    }
    pub fn shape(&self) -> u32 {
        (self.attr >> 4) & 0xFF
    }
    pub fn superscript(&self) -> bool {
        self.attr & (1 << 12) != 0
    }
}

/// A new number (`nwno`): restarts a counter at `number`.
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct NewNumber {
    pub attr: u32,
    pub number: u16,
    pub tail: SharedBytes,
}

impl NewNumber {
    pub fn kind(&self) -> NumberKind {
        NumberKind::from_bits(self.attr)
    }
}

/// Page number position (`pgnp`).
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PageNumberPos {
    /// Bits 0-7 number shape, 8-11 position (0 none, 1 top left, 2 top
    /// center, 3 top right, 4 bottom left, 5 bottom center, 6 bottom right,
    /// 7 outside top, 8 outside bottom, 9 inside top, 10 inside bottom).
    pub attr: u32,
    pub user_char: u16,
    pub prefix: u16,
    pub suffix: u16,
    /// The side decoration drawn around the number (Hancom writes '-').
    pub side: u16,
    pub tail: SharedBytes,
}

impl PageNumberPos {
    pub fn shape(&self) -> u32 {
        self.attr & 0xFF
    }
    pub fn position(&self) -> u32 {
        (self.attr >> 8) & 0xF
    }
}

/// Page hiding (`pghd`) for the page holding the control.
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PageHide {
    /// Bits: 0 header, 1 footer, 2 master page, 3 border, 4 fill, 5 page number.
    pub attr: u32,
    pub tail: SharedBytes,
}

/// The odd/even page adjustment (`pgct`): bits 0-1 both, even or odd.
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PageParity {
    pub attr: u32,
    pub tail: SharedBytes,
}

/// What a page hides, as a bit set over [`PageHide::attr`].
pub mod hide {
    pub const HEADER: u32 = 1;
    pub const FOOTER: u32 = 1 << 1;
    pub const MASTER: u32 = 1 << 2;
    pub const BORDER: u32 = 1 << 3;
    pub const FILL: u32 = 1 << 4;
    pub const PAGE_NUMBER: u32 = 1 << 5;
}
