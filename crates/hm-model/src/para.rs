//! Paragraphs in Hancom's wire position space.

use crate::{Control, RawRecord, SharedBytes};

/// The paragraph end mark, always the last unit of the text.
pub const PARA_END: u16 = 0x000D;

/// How many wire units a control code occupies: 8 for inline and extended
/// controls, 1 for character controls.
pub fn control_width(code: u16) -> u32 {
    match code {
        1..=9 | 11 | 12 | 14..=23 => 8,
        _ => 1,
    }
}

/// True for codes whose 8-unit block owns a CTRL_HEADER record.
pub fn is_extended_control(code: u16) -> bool {
    matches!(code, 1..=3 | 11 | 12 | 14..=18 | 21..=23)
}

/// One element of paragraph text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    /// A character (a surrogate pair is one `Char` spanning two units).
    Char(char),
    /// A one-unit control: line break (10), paragraph end (13), soft hyphen
    /// (24), no-break space (30), fixed-width space (31) and reserved codes.
    CharControl(u16),
    /// An 8-unit inline control without a record: tab (9), field end (4),
    /// title mark (8) and reserved codes.
    Inline(u16),
    /// An 8-unit extended control; `index` is its position in
    /// [`Paragraph::controls`].
    Extended { code: u16, index: u32 },
}

/// Hancom's paragraph break kinds (bits of the header byte).
pub mod break_kind {
    pub const SECTION: u8 = 0x01;
    pub const MULTI_COLUMN: u8 = 0x02;
    pub const PAGE: u8 = 0x04;
    pub const COLUMN: u8 = 0x08;
}

/// Paragraph header fields that are not derived from the content.
#[derive(Clone, Debug, Default)]
pub struct ParaHeader {
    pub para_shape: u16,
    pub style: u8,
    /// See [`break_kind`].
    pub break_kind: u8,
    pub instance_id: u32,
    /// Track-change merge flag, present from 5.0.3.2.
    pub merge_flag: Option<u16>,
    pub tail: SharedBytes,
    /// The control mask as read. Writers regenerate it for dirty paragraphs.
    pub control_mask: u32,
}

/// Start of a character shape run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CharRun {
    pub pos: u32,
    pub shape: u32,
}

/// One stored line segment: Hancom's last layout of a line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LineSeg {
    pub text_start: u32,
    /// Top of the line from the top of the flow (section body, cell, text box).
    pub vpos: i32,
    pub height: i32,
    pub text_height: i32,
    pub baseline: i32,
    /// Space after the line; may be negative.
    pub spacing: i32,
    /// Start from the column (or cell) content start.
    pub col_start: i32,
    pub width: i32,
    pub flags: u32,
}

impl LineSeg {
    pub const FIRST_IN_PAGE: u32 = 1;
    pub const FIRST_IN_COLUMN: u32 = 1 << 1;
    pub const EMPTY: u32 = 1 << 16;
    pub const FIRST_SEGMENT: u32 = 1 << 17;
    pub const LAST_SEGMENT: u32 = 1 << 18;
    pub const INDENTED: u32 = 1 << 20;
    pub const HEAD: u32 = 1 << 21;
}

/// A text range mark (highlighter and others): kind in the top byte.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RangeTag {
    pub start: u32,
    pub end: u32,
    pub tag: u32,
}

/// A record among a paragraph's children that the model does not interpret,
/// with its index among the paragraph's direct child records.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtraRecord {
    pub slot: u32,
    pub record: RawRecord,
}

#[derive(Clone, Debug, Default)]
pub struct Paragraph {
    pub header: ParaHeader,
    /// Wire units, ending with [`PARA_END`].
    pub text: Vec<u16>,
    pub runs: Vec<CharRun>,
    pub line_segs: Vec<LineSeg>,
    pub range_tags: Vec<RangeTag>,
    /// One per extended control block in `text`, in order.
    pub controls: Vec<Control>,
    pub extra: Vec<ExtraRecord>,
    /// Tracked-change marks in position order.
    pub tracks: Vec<crate::TrackMark>,
    /// The record subtree as read; `None` once this paragraph changed.
    pub src: Option<SharedBytes>,
}

impl Paragraph {
    /// True when this paragraph and all its controls can be copied from source.
    pub fn is_clean(&self) -> bool {
        self.src.is_some() && self.controls.iter().all(Control::is_clean)
    }

    pub fn mark_all_dirty(&mut self) {
        self.src = None;
        self.controls.iter_mut().for_each(Control::mark_all_dirty);
    }

    /// Text length in wire units, the paragraph end included.
    pub fn len(&self) -> u32 {
        self.text.len() as u32
    }

    pub fn is_empty(&self) -> bool {
        self.text.len() <= 1
    }

    /// The character shape id at wire position `pos`.
    pub fn shape_at(&self, pos: u32) -> u32 {
        match self.runs.partition_point(|r| r.pos <= pos) {
            0 => self.runs.first().map_or(0, |r| r.shape),
            i => self.runs[i - 1].shape,
        }
    }

    /// Walks the text as units with their wire positions.
    pub fn units(&self) -> Units<'_> {
        Units {
            text: &self.text,
            pos: 0,
            ext: 0,
        }
    }

    /// Plain text: characters, tabs as `\t`, line breaks as `\n`, controls dropped.
    pub fn plain_text(&self) -> String {
        let mut s = String::new();
        for (_, u) in self.units() {
            match u {
                Unit::Char(c) => s.push(c),
                Unit::Inline(9) => s.push('\t'),
                Unit::CharControl(10) => s.push('\n'),
                Unit::CharControl(30) => s.push('\u{a0}'),
                Unit::CharControl(31) => s.push('\u{2007}'),
                _ => {}
            }
        }
        s
    }
}

/// Iterator over `(position, unit)` pairs of a paragraph's text.
pub struct Units<'a> {
    text: &'a [u16],
    pos: usize,
    ext: u32,
}

impl Iterator for Units<'_> {
    type Item = (u32, Unit);

    fn next(&mut self) -> Option<(u32, Unit)> {
        let pos = self.pos;
        let u = *self.text.get(pos)?;
        let unit = if u < 32 {
            let w = control_width(u) as usize;
            self.pos = (pos + w).min(self.text.len());
            if w == 1 {
                Unit::CharControl(u)
            } else if is_extended_control(u) {
                self.ext += 1;
                Unit::Extended {
                    code: u,
                    index: self.ext - 1,
                }
            } else {
                Unit::Inline(u)
            }
        } else if (0xD800..0xDC00).contains(&u) {
            match self.text.get(pos + 1) {
                Some(&lo) if (0xDC00..0xE000).contains(&lo) => {
                    self.pos = pos + 2;
                    let c = 0x10000 + (((u as u32) - 0xD800) << 10) + (lo as u32 - 0xDC00);
                    Unit::Char(char::from_u32(c).unwrap_or('\u{fffd}'))
                }
                _ => {
                    self.pos = pos + 1;
                    Unit::Char('\u{fffd}')
                }
            }
        } else {
            self.pos = pos + 1;
            Unit::Char(char::from_u32(u as u32).unwrap_or('\u{fffd}'))
        };
        Some((pos as u32, unit))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn para(text: &[u16]) -> Paragraph {
        Paragraph {
            text: text.to_vec(),
            ..Default::default()
        }
    }

    #[test]
    fn units_follow_wire_widths() {
        let mut t = vec![0x41];
        t.extend([9, 0, 0, 0, 0, 0, 0, 9]);
        t.extend([0xD83D, 0xDE00]);
        t.extend([11, 0x6c74, 0x2062, 0, 0, 0, 0, 11]);
        t.push(13);
        let p = para(&t);
        let units: Vec<_> = p.units().collect();
        assert_eq!(units[0], (0, Unit::Char('A')));
        assert_eq!(units[1], (1, Unit::Inline(9)));
        assert_eq!(units[2], (9, Unit::Char('😀')));
        assert_eq!(units[3], (11, Unit::Extended { code: 11, index: 0 }));
        assert_eq!(units[4], (19, Unit::CharControl(13)));
        assert_eq!(p.plain_text(), "A\t😀");
    }

    #[test]
    fn shape_at_uses_last_run_at_or_before() {
        let mut p = para(&[0x41, 0x42, 0x43, 13]);
        p.runs = vec![CharRun { pos: 0, shape: 3 }, CharRun { pos: 2, shape: 7 }];
        assert_eq!(p.shape_at(0), 3);
        assert_eq!(p.shape_at(1), 3);
        assert_eq!(p.shape_at(2), 7);
        assert_eq!(p.shape_at(9), 7);
    }
}
