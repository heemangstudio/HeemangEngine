//! Document-level tables: properties, BinData entries, faces, shapes, styles
//! and compatibility settings.
//!
//! Each entry keeps `src`, the bytes of its record as read (header included),
//! and `tail`, the payload bytes after the fields it models. Optional trailing
//! fields are `Option`s, present exactly when the source carried them.

use crate::{BorderFill, HwpString, RawRecord, Script, SharedBytes, units};

/// Document properties: section count, start numbers and the saved caret.
#[derive(Clone, Debug, Default)]
pub struct DocProperties {
    pub section_count: u16,
    pub page_start: u16,
    pub footnote_start: u16,
    pub endnote_start: u16,
    pub figure_start: u16,
    pub table_start: u16,
    pub equation_start: u16,
    pub caret_list: u32,
    pub caret_para: u32,
    pub caret_pos: u32,
    pub tail: SharedBytes,
    pub src: Option<SharedBytes>,
}

/// The counts record that precedes the tables. Counts for tables the model
/// holds are regenerated on write; the rest are kept as read.
#[derive(Clone, Debug, Default)]
pub struct IdMappings {
    pub counts: Vec<i32>,
    pub tail: SharedBytes,
    pub src: Option<SharedBytes>,
}

impl IdMappings {
    pub const BIN_DATA: usize = 0;
    pub const FACES: usize = 1;
    pub const BORDER_FILLS: usize = 8;
    pub const CHAR_SHAPES: usize = 9;
    pub const TAB_DEFS: usize = 10;
    pub const NUMBERINGS: usize = 11;
    pub const BULLETS: usize = 12;
    pub const PARA_SHAPES: usize = 13;
    pub const STYLES: usize = 14;

    pub fn count(&self, index: usize) -> usize {
        self.counts.get(index).copied().unwrap_or(0).max(0) as usize
    }
}

/// A BinData entry. Pictures refer to entries by 1-based position.
#[derive(Clone, Debug, Default)]
pub struct BinDataEntry {
    pub attr: u16,
    pub link_absolute: Option<HwpString>,
    pub link_relative: Option<HwpString>,
    pub storage_id: Option<u16>,
    pub extension: Option<HwpString>,
    pub tail: SharedBytes,
    pub src: Option<SharedBytes>,
}

impl BinDataEntry {
    /// 0 link, 1 embedding, 2 OLE storage.
    pub fn kind(&self) -> u16 {
        self.attr & 0xF
    }

    /// The container stream holding the payload, `BIN%04X.ext`.
    pub fn stream_name(&self) -> Option<String> {
        let id = self.storage_id?;
        let ext = self
            .extension
            .as_ref()
            .map(|e| e.to_string_lossy())
            .unwrap_or_default();
        Some(format!("BIN{id:04X}.{ext}"))
    }
}

/// A font face name record of one script slot.
#[derive(Clone, Debug, Default)]
pub struct FaceName {
    pub attr: u8,
    pub name: HwpString,
    /// Substitute face: kind (0 unknown, 1 TTF, 2 HFT) and name.
    pub substitute: Option<(u8, HwpString)>,
    pub panose: Option<[u8; 10]>,
    pub default_name: Option<HwpString>,
    pub tail: SharedBytes,
    pub src: Option<SharedBytes>,
}

/// A character shape.
#[derive(Clone, Debug, Default)]
pub struct CharShape {
    /// Face id per script slot, an index into that slot's face list.
    pub faces: [u16; 7],
    /// Width ratio percent per slot (50 to 200).
    pub ratios: [u8; 7],
    /// Spacing percent per slot (-50 to 50).
    pub spacings: [i8; 7],
    /// Relative size percent per slot (10 to 250; 0 reads as 100).
    pub rel_sizes: [u8; 7],
    /// Vertical offset percent per slot (-100 to 100, positive moves down).
    pub offsets: [i8; 7],
    /// Base size in HU (1000 = 10 pt).
    pub base_size: i32,
    pub attr: u32,
    pub shadow_offset: [i8; 2],
    pub color: u32,
    pub underline_color: u32,
    pub shade_color: u32,
    pub shadow_color: u32,
    pub border_fill: Option<u16>,
    pub strike_color: Option<u32>,
    pub tail: SharedBytes,
    pub src: Option<SharedBytes>,
}

impl CharShape {
    pub fn italic(&self) -> bool {
        self.attr & 1 != 0
    }
    pub fn bold(&self) -> bool {
        self.attr & 2 != 0
    }
    /// 0 none, 1 below, 3 above.
    pub fn underline_kind(&self) -> u32 {
        (self.attr >> 2) & 3
    }
    pub fn underline_shape(&self) -> u32 {
        (self.attr >> 4) & 0xF
    }
    pub fn superscript(&self) -> bool {
        self.attr & (1 << 15) != 0
    }
    pub fn subscript(&self) -> bool {
        self.attr & (1 << 16) != 0
    }
    pub fn strike(&self) -> u32 {
        (self.attr >> 18) & 7
    }
    /// Strikeout line shape (border line kinds, 0 solid).
    pub fn strike_shape(&self) -> u32 {
        (self.attr >> 26) & 0xF
    }
    /// True when a strikeout line is drawn. Hancom marks some char shapes
    /// as struck with shape 15, which OWPML writes as `3D` or `NONE` and
    /// which draws nothing.
    pub fn draws_strike(&self) -> bool {
        self.strike() != 0 && self.strike_shape() != 15
    }
    pub fn face(&self, s: Script) -> u16 {
        self.faces[s.index()]
    }
    pub fn ratio(&self, s: Script) -> i32 {
        match self.ratios[s.index()] {
            0 => 100,
            r => r as i32,
        }
    }
    pub fn spacing(&self, s: Script) -> i32 {
        self.spacings[s.index()] as i32
    }
    pub fn rel_size(&self, s: Script) -> i32 {
        match self.rel_sizes[s.index()] {
            0 => 100,
            r => r as i32,
        }
    }
    pub fn offset(&self, s: Script) -> i32 {
        self.offsets[s.index()] as i32
    }
}

/// One tab stop. `position` is a packed distance (see [`units::unpack_distance`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TabStop {
    pub position: i32,
    /// 0 left, 1 right, 2 center, 3 decimal.
    pub kind: u8,
    pub fill: u8,
    pub reserved: u16,
}

#[derive(Clone, Debug, Default)]
pub struct TabDef {
    /// Bit 0: automatic tab at the left end (hanging indent); bit 1: at the right end.
    pub attr: u32,
    pub tabs: Vec<TabStop>,
    pub tail: SharedBytes,
    pub src: Option<SharedBytes>,
}

/// Paragraph head information shared by numberings and bullets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ParaHead {
    pub attr: u32,
    pub width_adjust: i16,
    pub text_distance: i16,
    pub char_shape: u32,
}

#[derive(Clone, Debug, Default)]
pub struct NumberingLevel {
    pub head: ParaHead,
    /// Format string with `^n` level placeholders.
    pub format: HwpString,
}

#[derive(Clone, Debug, Default)]
pub struct Numbering {
    pub levels: Vec<NumberingLevel>,
    pub start: Option<u16>,
    pub level_starts: Option<[u32; 7]>,
    pub tail: SharedBytes,
    pub src: Option<SharedBytes>,
}

#[derive(Clone, Debug, Default)]
pub struct Bullet {
    pub head: ParaHead,
    pub bullet_char: u16,
    pub image: Option<i32>,
    pub image_info: Option<[u8; 4]>,
    pub check_char: Option<u16>,
    pub tail: SharedBytes,
    pub src: Option<SharedBytes>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Justify,
    Left,
    Right,
    Center,
    Distribute,
    Divide,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineSpacingKind {
    /// Percent of the line's character height.
    Percent,
    /// The line advance is the value.
    Fixed,
    /// The value is added after the line.
    Added,
    /// At least the value.
    AtLeast,
}

/// A paragraph shape. Margins, indent and spacing are packed distances.
#[derive(Clone, Debug, Default)]
pub struct ParaShape {
    pub attr1: u32,
    pub margin_left: i32,
    pub margin_right: i32,
    pub indent: i32,
    pub space_before: i32,
    pub space_after: i32,
    pub line_spacing_legacy: i32,
    pub tab_def: u16,
    pub numbering: u16,
    pub border_fill: u16,
    /// Left, right, top, bottom.
    pub border_offsets: [i16; 4],
    pub attr2: Option<u32>,
    pub attr3: Option<u32>,
    pub line_spacing: Option<u32>,
    pub tail: SharedBytes,
    pub src: Option<SharedBytes>,
}

impl ParaShape {
    pub fn align(&self) -> Align {
        match (self.attr1 >> 2) & 7 {
            1 => Align::Left,
            2 => Align::Right,
            3 => Align::Center,
            4 => Align::Distribute,
            5 => Align::Divide,
            _ => Align::Justify,
        }
    }

    /// Latin break unit: 0 word, 1 hyphenation, 2 character.
    pub fn latin_break(&self) -> u32 {
        (self.attr1 >> 5) & 3
    }

    /// Korean break unit: false word (어절), true syllable (글자).
    pub fn korean_break_by_char(&self) -> bool {
        self.attr1 & (1 << 7) != 0
    }

    pub fn snap_to_grid(&self) -> bool {
        self.attr1 & (1 << 8) != 0
    }

    /// Minimum space width percent (0 to 75) for justification.
    pub fn min_space(&self) -> u32 {
        (self.attr1 >> 9) & 0x7F
    }

    pub fn widow_orphan(&self) -> bool {
        self.attr1 & (1 << 16) != 0
    }
    pub fn keep_with_next(&self) -> bool {
        self.attr1 & (1 << 17) != 0
    }
    pub fn keep_lines(&self) -> bool {
        self.attr1 & (1 << 18) != 0
    }
    pub fn page_break_before(&self) -> bool {
        self.attr1 & (1 << 19) != 0
    }
    /// 0 baseline, 1 top, 2 center, 3 bottom.
    pub fn vertical_align(&self) -> u32 {
        (self.attr1 >> 20) & 3
    }
    /// 0 none, 1 outline, 2 numbering, 3 bullet.
    pub fn head_kind(&self) -> u32 {
        (self.attr1 >> 23) & 3
    }
    pub fn level(&self) -> u32 {
        (self.attr1 >> 25) & 7
    }
    /// Joins this paragraph's border with the next one's when they are equal.
    pub fn connect_border(&self) -> bool {
        self.attr1 & (1 << 28) != 0
    }
    /// Draws the border at the paragraph margins instead of the column edges.
    pub fn border_ignores_margin(&self) -> bool {
        self.attr1 & (1 << 29) != 0
    }

    /// Line spacing kind and raw value. Files from 5.0.2.5 on carry the
    /// current fields; older ones only the legacy pair.
    pub fn line_spacing(&self) -> (LineSpacingKind, i32) {
        match (self.attr3, self.line_spacing) {
            (Some(a3), Some(v)) => {
                let kind = match a3 & 0x1F {
                    1 => LineSpacingKind::Fixed,
                    2 => LineSpacingKind::Added,
                    3 => LineSpacingKind::AtLeast,
                    _ => LineSpacingKind::Percent,
                };
                (kind, v as i32)
            }
            _ => {
                let kind = match self.attr1 & 3 {
                    1 => LineSpacingKind::Fixed,
                    2 => LineSpacingKind::Added,
                    _ => LineSpacingKind::Percent,
                };
                (kind, self.line_spacing_legacy)
            }
        }
    }

    /// Effective left margin in HU; `basis` resolves character-relative values.
    pub fn left(&self, basis: i32) -> i32 {
        units::unpack_distance(self.margin_left, || basis)
    }
    pub fn right(&self, basis: i32) -> i32 {
        units::unpack_distance(self.margin_right, || basis)
    }
    pub fn first_indent(&self, basis: i32) -> i32 {
        units::unpack_distance(self.indent, || basis)
    }
    pub fn before(&self, basis: i32) -> i32 {
        units::unpack_distance(self.space_before, || basis)
    }
    pub fn after(&self, basis: i32) -> i32 {
        units::unpack_distance(self.space_after, || basis)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Style {
    pub local_name: HwpString,
    pub english_name: HwpString,
    /// Bits 0-2: 0 paragraph style, 1 character style.
    pub attr: u8,
    pub next: u8,
    pub language: i16,
    pub para_shape: u16,
    pub char_shape: u16,
    pub tail: SharedBytes,
    pub src: Option<SharedBytes>,
}

/// Target program: 0 current Hancom, 1 Hangul 2007, 2 MS Word.
#[derive(Clone, Debug, Default)]
pub struct CompatibleDocument {
    pub target: u32,
    pub tail: SharedBytes,
    pub src: Option<SharedBytes>,
}

/// The five layout compatibility flag words: character, paragraph, section,
/// object and field level.
#[derive(Clone, Debug, Default)]
pub struct LayoutCompatibility {
    pub flags: [u32; 5],
    pub tail: SharedBytes,
    pub src: Option<SharedBytes>,
}

/// What one DocInfo record of the source was, in source order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InfoItem {
    Properties,
    IdMappings,
    BinData(u32),
    Face(Script, u32),
    BorderFill(u32),
    CharShape(u32),
    TabDef(u32),
    Numbering(u32),
    Bullet(u32),
    ParaShape(u32),
    Style(u32),
    Compatible,
    LayoutCompat,
    /// An index into [`DocInfo::raw`].
    Raw(u32),
}

/// One slot of the record order: the item and the record level it had.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InfoSlot {
    pub item: InfoItem,
    pub level: u16,
}

/// The document tables (`DocInfo` in HWP 5, `head` in OWPML).
#[derive(Clone, Debug, Default)]
pub struct DocInfo {
    pub properties: DocProperties,
    pub id_mappings: IdMappings,
    pub bin_data: Vec<BinDataEntry>,
    /// Face lists per script slot.
    pub faces: [Vec<FaceName>; 7],
    pub border_fills: Vec<BorderFill>,
    pub char_shapes: Vec<CharShape>,
    pub tab_defs: Vec<TabDef>,
    pub numberings: Vec<Numbering>,
    pub bullets: Vec<Bullet>,
    pub para_shapes: Vec<ParaShape>,
    pub styles: Vec<Style>,
    pub compatible: Option<CompatibleDocument>,
    pub layout_compat: Option<LayoutCompatibility>,
    /// Records the model does not interpret, each with its absolute level.
    pub raw: Vec<RawRecord>,
    /// The source record order. Writers follow it and append new entries
    /// after the last entry of their kind.
    pub order: Vec<InfoSlot>,
}

impl DocInfo {
    pub fn char_shape(&self, id: u32) -> Option<&CharShape> {
        self.char_shapes.get(id as usize)
    }

    pub fn para_shape(&self, id: u32) -> Option<&ParaShape> {
        self.para_shapes.get(id as usize)
    }

    pub fn face(&self, script: Script, id: u16) -> Option<&FaceName> {
        self.faces[script.index()].get(id as usize)
    }

    /// True when every record can be copied from its source bytes.
    pub fn is_clean(&self) -> bool {
        fn all<T>(v: &[T], f: impl Fn(&T) -> bool) -> bool {
            v.iter().all(f)
        }
        self.properties.src.is_some()
            && self.id_mappings.src.is_some()
            && all(&self.bin_data, |e| e.src.is_some())
            && self.faces.iter().all(|l| all(l, |e| e.src.is_some()))
            && all(&self.border_fills, |e| e.src.is_some())
            && all(&self.char_shapes, |e| e.src.is_some())
            && all(&self.tab_defs, |e| e.src.is_some())
            && all(&self.numberings, |e| e.src.is_some())
            && all(&self.bullets, |e| e.src.is_some())
            && all(&self.para_shapes, |e| e.src.is_some())
            && all(&self.styles, |e| e.src.is_some())
            && self.compatible.as_ref().is_none_or(|e| e.src.is_some())
            && self.layout_compat.as_ref().is_none_or(|e| e.src.is_some())
    }

    /// Drops every source span so writers re-encode all records.
    pub fn mark_all_dirty(&mut self) {
        self.properties.src = None;
        self.id_mappings.src = None;
        self.bin_data.iter_mut().for_each(|e| e.src = None);
        self.faces.iter_mut().flatten().for_each(|e| e.src = None);
        self.border_fills.iter_mut().for_each(|e| e.src = None);
        self.char_shapes.iter_mut().for_each(|e| e.src = None);
        self.tab_defs.iter_mut().for_each(|e| e.src = None);
        self.numberings.iter_mut().for_each(|e| e.src = None);
        self.bullets.iter_mut().for_each(|e| e.src = None);
        self.para_shapes.iter_mut().for_each(|e| e.src = None);
        self.styles.iter_mut().for_each(|e| e.src = None);
        if let Some(e) = &mut self.compatible {
            e.src = None;
        }
        if let Some(e) = &mut self.layout_compat {
            e.src = None;
        }
    }
}
