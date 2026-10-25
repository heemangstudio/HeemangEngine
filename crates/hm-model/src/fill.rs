//! Borders and fills: border lines, border-fill definitions and the fill
//! description shared by border fills and drawing objects.

use crate::SharedBytes;

/// One border line as stored: kind, width index and COLORREF.
///
/// Kinds count from 0 for no line: 1 solid, 2 long dash, 3 dot, 4 dash dot,
/// 5 dash dot dot, 6 dash, 7 circle dots, 8 double, 9 thin thick, 10 thick
/// thin, 11 thin thick thin, 12 wave, 13 double wave, 14 thick 3D, 15 thick
/// 3D inverted, 16 3D, 17 3D inverted. The width index selects one of 16
/// widths from 0.1 mm to 5 mm.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BorderLine {
    pub kind: u8,
    pub width: u8,
    pub color: u32,
}

/// Border widths in HU by width index (0.1, 0.12, 0.15, 0.2, 0.25, 0.3, 0.4,
/// 0.5, 0.6, 0.7, 1, 1.5, 2, 3, 4 and 5 mm), as Hancom rounds them.
pub const BORDER_WIDTHS: [i32; 16] = [
    28, 33, 42, 56, 70, 84, 113, 141, 169, 198, 283, 424, 567, 850, 1134, 1417,
];

impl BorderLine {
    pub const NONE: u8 = 0;
    pub const SOLID: u8 = 1;

    /// True when the line draws anything.
    pub fn visible(&self) -> bool {
        self.kind != Self::NONE
    }

    /// Width in HU; indexes past the table draw nothing.
    pub fn width_hu(&self) -> i32 {
        BORDER_WIDTHS.get(self.width as usize).copied().unwrap_or(0)
    }
}

/// Solid color with an optional hatch pattern.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SolidFill {
    /// COLORREF; `0xFFFFFFFF` means no background.
    pub background: u32,
    pub pattern_color: u32,
    /// Hatch: -1 or 0 none, 1 horizontal, 2 vertical, 3 backward diagonal,
    /// 4 forward diagonal, 5 cross, 6 diagonal cross.
    pub pattern: i32,
}

impl SolidFill {
    pub const NO_COLOR: u32 = 0xFFFF_FFFF;
}

/// A gradient. Positions are given per color only with more than two colors.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Gradient {
    /// 1 linear, 2 radial, 3 conical, 4 square.
    pub kind: u8,
    /// Degrees.
    pub angle: i32,
    /// Center in percent of the box.
    pub center_x: i32,
    pub center_y: i32,
    /// Number of color steps (0 to 255).
    pub step: i32,
    pub positions: Vec<i32>,
    pub colors: Vec<u32>,
    /// Where the steps concentrate (0 to 100), from the extra attributes.
    pub step_center: Option<u8>,
}

/// A picture fill.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ImageFill {
    /// 0-4 tile variants, 5 fit, 6-14 placed by position, 15 none.
    pub mode: u8,
    /// Stored before the brightness.
    pub contrast: i8,
    pub brightness: i8,
    /// 0 picture, 1 grayscale, 2 black and white, 4 pattern.
    pub effect: u8,
    /// 1-based BinData id.
    pub bin_id: u16,
}

/// A fill description (HWP 5 table 28).
///
/// The parts are stored in this order: kind, solid, gradient, image, the
/// length-prefixed extra attributes and one alpha byte per present part.
/// Each part is `Some` exactly when it was read; a reader stops at the first
/// part it cannot read and keeps the rest in the owner's tail.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Fill {
    /// Bits: 1 solid, 2 image, 4 gradient.
    pub kind: u32,
    pub solid: Option<SolidFill>,
    pub gradient: Option<Gradient>,
    pub image: Option<ImageFill>,
    /// The extra attributes after their length word.
    pub extra: Option<SharedBytes>,
    /// Alpha (0 opaque) of the solid, gradient and image parts, in that order.
    pub alpha: Vec<u8>,
}

impl Fill {
    pub const SOLID: u32 = 1;
    pub const IMAGE: u32 = 2;
    pub const GRADIENT: u32 = 4;

    /// The solid background color, when the fill paints one. A color whose
    /// top byte is all ones is transparent (OWPML `#FF000000`), as is
    /// [`SolidFill::NO_COLOR`].
    pub fn background(&self) -> Option<u32> {
        self.solid
            .filter(|s| s.background >> 24 != 0xFF)
            .map(|s| s.background & 0x00FF_FFFF)
    }

    /// True when the fill paints nothing.
    pub fn is_empty(&self) -> bool {
        self.background().is_none()
            && self.solid.is_none_or(|s| s.pattern <= 0)
            && self.gradient.as_ref().is_none_or(|g| g.colors.is_empty())
            && self.image.is_none_or(|i| i.bin_id == 0)
    }
}

/// A border and fill definition (`BORDER_FILL` in HWP 5, `borderFill` in
/// OWPML), referenced 1-based by cells, paragraphs, pages and characters.
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BorderFill {
    /// Bit 0 3D, bit 1 shadow, bits 2-4 slash, 5-7 backslash, 8-9 crooked
    /// slash/backslash (or center-line directions when bit 13 is set),
    /// 10 omit split-cell edges, 11-12 rotated diagonals, 13 center line.
    pub attr: u16,
    /// Left, right, top, bottom.
    pub sides: [BorderLine; 4],
    pub diagonal: BorderLine,
    /// `None` when the record ends before the fill.
    pub fill: Option<Fill>,
    pub tail: SharedBytes,
    #[cfg_attr(feature = "serde", serde(skip))]
    pub src: Option<SharedBytes>,
}

impl BorderFill {
    pub const LEFT: usize = 0;
    pub const RIGHT: usize = 1;
    pub const TOP: usize = 2;
    pub const BOTTOM: usize = 3;

    /// Slash diagonal shape bits (2 slash, 3 to the bottom edge, 6 to the
    /// right edge, 7 to both).
    pub fn slash(&self) -> u16 {
        (self.attr >> 2) & 7
    }

    /// Backslash diagonal shape bits, as [`BorderFill::slash`].
    pub fn backslash(&self) -> u16 {
        (self.attr >> 5) & 7
    }

    pub fn center_line(&self) -> bool {
        self.attr & (1 << 13) != 0
    }

    /// Native `breakCellSeparateLine`: omit the synthetic edges where a
    /// cell continues across a page boundary.
    pub fn break_cell_separate_line(&self) -> bool {
        self.attr & (1 << 10) != 0
    }

    pub fn shadow(&self) -> bool {
        self.attr & 2 != 0
    }

    pub fn three_d(&self) -> bool {
        self.attr & 1 != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(background: u32) -> Fill {
        Fill {
            kind: Fill::SOLID,
            solid: Some(SolidFill {
                background,
                pattern_color: 0,
                pattern: -1,
            }),
            ..Default::default()
        }
    }

    #[test]
    fn a_fully_transparent_face_paints_nothing() {
        assert_eq!(solid(0xFF00_0000).background(), None);
        assert!(solid(0xFF00_0000).is_empty());
        assert_eq!(solid(SolidFill::NO_COLOR).background(), None);
        assert_eq!(solid(0x0012_3456).background(), Some(0x0012_3456));
    }
}
