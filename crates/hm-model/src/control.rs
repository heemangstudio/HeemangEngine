//! Extended controls anchored in paragraph text.

use crate::{
    AutoNumber, CtrlId, HeaderFooter, NewNumber, Note, ObjectControl, PageHide, PageNumberPos,
    PageParity, PageSide, ParaList, RawRecord, SharedBytes,
};

/// An extended control. Kinds the model does not interpret yet are
/// [`ControlKind::Raw`] and save byte for byte.
#[derive(Clone, Debug)]
pub struct Control {
    pub kind: ControlKind,
    /// The CTRL_HEADER record and its subtree as read; `None` once changed.
    pub src: Option<SharedBytes>,
}

#[derive(Clone, Debug)]
pub enum ControlKind {
    Section(Box<SectionDef>),
    Columns(ColumnDef),
    /// Drawing objects and equations.
    Object(Box<ObjectControl>),
    HeaderFooter(Box<HeaderFooter>),
    Note(Box<Note>),
    AutoNumber(AutoNumber),
    NewNumber(NewNumber),
    PageNumber(PageNumberPos),
    PageHide(PageHide),
    PageParity(PageParity),
    Raw(RawControl),
}

impl Control {
    pub fn id(&self) -> CtrlId {
        match &self.kind {
            ControlKind::Section(_) => CtrlId::SECTION,
            ControlKind::Columns(_) => CtrlId::COLUMNS,
            ControlKind::Object(o) => o.id,
            ControlKind::HeaderFooter(h) if h.footer => CtrlId::FOOTER,
            ControlKind::HeaderFooter(_) => CtrlId::HEADER,
            ControlKind::Note(n) if n.endnote => CtrlId::ENDNOTE,
            ControlKind::Note(_) => CtrlId::FOOTNOTE,
            ControlKind::AutoNumber(_) => CtrlId::AUTO_NUMBER,
            ControlKind::NewNumber(_) => CtrlId::NEW_NUMBER,
            ControlKind::PageNumber(_) => CtrlId::PAGE_NUMBER_POS,
            ControlKind::PageHide(_) => CtrlId::PAGE_HIDE,
            ControlKind::PageParity(_) => CtrlId::PAGE_PARITY,
            ControlKind::Raw(r) => r.id,
        }
    }

    pub fn is_clean(&self) -> bool {
        self.src.is_some()
    }

    pub fn mark_all_dirty(&mut self) {
        self.src = None;
    }

    pub fn as_section(&self) -> Option<&SectionDef> {
        match &self.kind {
            ControlKind::Section(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&ObjectControl> {
        match &self.kind {
            ControlKind::Object(o) => Some(o),
            _ => None,
        }
    }

    /// The common object attributes of any object control.
    pub fn object_common(&self) -> Option<ObjectCommon> {
        match &self.kind {
            ControlKind::Object(o) => Some(o.header.common),
            ControlKind::Raw(r) => r.object_common(),
            _ => None,
        }
    }

    pub fn as_header_footer(&self) -> Option<&HeaderFooter> {
        match &self.kind {
            ControlKind::HeaderFooter(h) => Some(h),
            _ => None,
        }
    }

    pub fn as_note(&self) -> Option<&Note> {
        match &self.kind {
            ControlKind::Note(n) => Some(n),
            _ => None,
        }
    }

    pub fn as_columns(&self) -> Option<&ColumnDef> {
        match &self.kind {
            ControlKind::Columns(c) => Some(c),
            _ => None,
        }
    }
}

/// A control kept as records.
#[derive(Clone, Debug)]
pub struct RawControl {
    pub id: CtrlId,
    /// The CTRL_HEADER payload, starting with the control id.
    pub header: SharedBytes,
    /// Descendant records in order, levels relative to the control.
    pub children: Vec<RawRecord>,
    /// The OWPML element the control was read from, for HWPX documents.
    /// HWPX writers copy it; `header` then holds the HWP 5 header fields it
    /// maps to, so readers of the common object fields see them.
    pub xml: Option<SharedBytes>,
}

impl RawControl {
    /// The common object attributes at the start of an object control's
    /// header (`tbl `, `gso `, `eqed`, ...), when present.
    pub fn object_common(&self) -> Option<ObjectCommon> {
        ObjectCommon::parse(self.header.get(4..)?)
    }
}

/// The leading fields of the common object attributes (HWP 5 table 69).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ObjectCommon {
    pub attr: u32,
    pub y_offset: i32,
    pub x_offset: i32,
    pub width: i32,
    pub height: i32,
    pub z_order: i32,
    /// Left, right, top, bottom.
    pub margins: [i16; 4],
}

impl ObjectCommon {
    /// Reads the 32 bytes after the control id.
    pub fn parse(b: &[u8]) -> Option<Self> {
        if b.len() < 32 {
            return None;
        }
        let u32_at = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        let i16_at = |o: usize| i16::from_le_bytes(b[o..o + 2].try_into().unwrap());
        Some(Self {
            attr: u32_at(0),
            y_offset: u32_at(4) as i32,
            x_offset: u32_at(8) as i32,
            width: u32_at(12) as i32,
            height: u32_at(16) as i32,
            z_order: u32_at(20) as i32,
            margins: [i16_at(24), i16_at(26), i16_at(28), i16_at(30)],
        })
    }

    pub fn treat_as_char(&self) -> bool {
        self.attr & 1 != 0
    }

    /// 0 paper, 1 page, 2 paragraph.
    pub fn vert_rel_to(&self) -> u32 {
        (self.attr >> 3) & 3
    }

    /// Stored text wrap: 0 square, 1 top and bottom, 2 behind text, 3 in front of text.
    pub fn text_wrap(&self) -> u32 {
        (self.attr >> 21) & 7
    }
}

/// Section definition (`secd`).
#[derive(Clone, Debug, Default)]
pub struct SectionDef {
    pub attr: u32,
    pub column_gap: i16,
    pub vertical_grid: i16,
    pub horizontal_grid: i16,
    /// Packed distance.
    pub default_tab: i32,
    pub outline_numbering: u16,
    pub page_number: u16,
    pub figure_number: u16,
    pub table_number: u16,
    pub equation_number: u16,
    pub language: Option<u16>,
    pub tail: SharedBytes,
    pub children: Vec<SectionChild>,
}

impl SectionDef {
    pub fn page_def(&self) -> Option<&PageDef> {
        self.children.iter().find_map(|c| match c {
            SectionChild::Page(p) => Some(p),
            _ => None,
        })
    }

    /// The first note shape is for footnotes, the second for endnotes.
    pub fn note_shapes(&self) -> impl Iterator<Item = &NoteShape> {
        self.children.iter().filter_map(|c| match c {
            SectionChild::NoteShape(n) => Some(n),
            _ => None,
        })
    }

    /// At most two empty lines at a page top take no height.
    pub fn hide_empty_lines(&self) -> bool {
        self.attr & (1 << 19) != 0
    }

    /// Master pages with the pages they apply to. Attribute bits 29, 30
    /// and 31 mark the both, even and odd master pages present; their lists
    /// follow in that order.
    pub fn master_pages(&self) -> Vec<(PageSide, &ParaList)> {
        let lists: Vec<&ParaList> = self
            .children
            .iter()
            .filter_map(|c| match c {
                SectionChild::MasterPage(l) => Some(l),
                _ => None,
            })
            .collect();
        let mut sides: Vec<PageSide> = [
            (29, PageSide::Both),
            (30, PageSide::Even),
            (31, PageSide::Odd),
        ]
        .iter()
        .filter(|(bit, _)| self.attr & (1 << bit) != 0)
        .map(|&(_, s)| s)
        .collect();
        // Files that do not mark their master pages apply them in order.
        if sides.len() != lists.len() {
            sides = [PageSide::Both, PageSide::Even, PageSide::Odd]
                .into_iter()
                .take(lists.len())
                .collect();
        }
        sides.into_iter().zip(lists).collect()
    }

    /// The page border and fill records: both, even and odd pages.
    pub fn page_borders(&self) -> impl Iterator<Item = &PageBorderFill> {
        self.children.iter().filter_map(|c| match c {
            SectionChild::PageBorder(b) => Some(b),
            _ => None,
        })
    }
}

/// A child record of a section definition.
#[derive(Clone, Debug)]
pub enum SectionChild {
    Page(PageDef),
    NoteShape(NoteShape),
    PageBorder(PageBorderFill),
    /// A master page: in Hancom's order, both pages, then even, then odd
    /// pages, as far as present (see [`SectionDef::master_pages`]).
    MasterPage(ParaList),
    /// Anything else, level relative to the control.
    Raw(RawRecord),
}

/// Paper size and margins in HU.
#[derive(Clone, Debug, Default)]
pub struct PageDef {
    pub width: u32,
    pub height: u32,
    pub margin_left: u32,
    pub margin_right: u32,
    pub margin_top: u32,
    pub margin_bottom: u32,
    pub margin_header: u32,
    pub margin_footer: u32,
    pub gutter: u32,
    /// Bit 0 landscape; bits 1-2 binding.
    pub attr: u32,
    pub tail: SharedBytes,
}

impl PageDef {
    pub fn landscape(&self) -> bool {
        self.attr & 1 != 0
    }

    /// Paper size as laid out: width and height swap in landscape.
    pub fn paper(&self) -> (i32, i32) {
        let (w, h) = (sat(self.width), sat(self.height));
        if self.landscape() { (h, w) } else { (w, h) }
    }

    /// The body area `(x, y, width, height)` on the paper. Damaged files can
    /// carry huge margins, so sums saturate.
    pub fn body(&self) -> (i32, i32, i32, i32) {
        let (pw, ph) = self.paper();
        let left = sat(self.margin_left).saturating_add(sat(self.gutter));
        let top = sat(self.margin_top).saturating_add(sat(self.margin_header));
        let w = pw
            .saturating_sub(left)
            .saturating_sub(sat(self.margin_right));
        let h = ph
            .saturating_sub(top)
            .saturating_sub(sat(self.margin_bottom))
            .saturating_sub(sat(self.margin_footer));
        (left, top, w.max(0), h.max(0))
    }
}

fn sat(v: u32) -> i32 {
    i32::try_from(v).unwrap_or(i32::MAX)
}

/// Footnote or endnote shape (28 bytes in Hancom files).
#[derive(Clone, Debug, Default)]
pub struct NoteShape {
    pub attr: u32,
    pub user_char: u16,
    pub prefix: u16,
    pub suffix: u16,
    pub start: u16,
    pub separator_length: i32,
    pub space_above: i16,
    pub space_below: i16,
    pub space_between: i16,
    pub line_kind: u8,
    pub line_width: u8,
    pub line_color: u32,
    pub tail: SharedBytes,
}

#[derive(Clone, Debug, Default)]
pub struct PageBorderFill {
    pub attr: u32,
    /// Left, right, top, bottom.
    pub offsets: [i16; 4],
    pub border_fill: u16,
    pub tail: SharedBytes,
}

/// Column definition (`cold`).
#[derive(Clone, Debug, Default)]
pub struct ColumnDef {
    /// Bits 0-1 kind (normal, distributed, parallel), 2-9 count,
    /// 10-11 direction, 12 equal widths.
    pub attr: u16,
    pub attr2: u16,
    /// Gap between equal-width columns.
    pub gap: i16,
    /// Proportional `(width, gap)` per column (sum 32768) when widths differ.
    pub columns: Vec<(i16, i16)>,
    pub line_kind: u8,
    pub line_width: u8,
    pub line_color: u32,
    pub tail: SharedBytes,
}

impl ColumnDef {
    pub fn count(&self) -> u32 {
        ((self.attr >> 2) & 0xFF).max(1) as u32
    }

    pub fn equal_widths(&self) -> bool {
        self.attr & (1 << 12) != 0
    }

    /// 0 normal, 1 distributed (balanced), 2 parallel.
    pub fn kind(&self) -> u32 {
        (self.attr & 3) as u32
    }

    /// Column `(x, width)` pairs inside a body of width `body_w`.
    pub fn layout(&self, body_w: i32) -> Vec<(i32, i32)> {
        let n = self.count() as i32;
        if n <= 1 {
            return vec![(0, body_w)];
        }
        if !self.equal_widths() && self.columns.len() == n as usize {
            let total: i64 = self
                .columns
                .iter()
                .map(|&(w, g)| w as u16 as i64 + g as u16 as i64)
                .sum::<i64>()
                .max(1);
            let mut x = 0i64;
            let mut out = Vec::with_capacity(n as usize);
            for &(w, g) in &self.columns {
                let w = w as u16 as i64 * body_w as i64 / total;
                out.push((x as i32, w as i32));
                x += w + g as u16 as i64 * body_w as i64 / total;
            }
            return out;
        }
        let gap = self.gap as i32;
        let w = (body_w - gap * (n - 1)) / n;
        (0..n).map(|i| (i * (w + gap), w)).collect()
    }
}
