//! Line and fill styles of drawing objects and pictures (HWP 5 tables 28,
//! 86 and 87), shared with border fills.

use crate::{SharedBytes, SolidFill};

/// An outline: color, width and the packed attribute word of table 87, plus
/// the outline placement byte drawing objects carry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LineStyle {
    /// COLORREF (`0x00BBGGRR`).
    pub color: u32,
    /// Width in HU; 0 draws a hairline.
    pub width: i32,
    pub attr: u32,
    /// 0 normal, 1 outer, 2 inner. Pictures and OLE objects store no such byte.
    pub outline: Option<u8>,
}

/// Arrowhead shapes (table 87).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arrow {
    None,
    Arrow,
    Spear,
    ConcaveArrow,
    Diamond,
    Circle,
    Square,
}

impl LineStyle {
    /// Line kind: 0 none, then solid, dash, dot, dash-dot, dash-dot-dot,
    /// long dash, big dots, double, thin-thick, thick-thin,
    /// thin-thick-thin, wave, double wave, thick 3D, thick 3D lit, 3D, 3D lit.
    ///
    /// Drawing objects store kinds one above the border fill table, so 0
    /// means no line here.
    pub fn kind(&self) -> u32 {
        self.attr & 0x3F
    }

    /// 0 round, 1 flat.
    pub fn cap(&self) -> u32 {
        (self.attr >> 6) & 0xF
    }

    fn arrow(code: u32) -> Arrow {
        match code {
            1 => Arrow::Arrow,
            2 => Arrow::Spear,
            3 => Arrow::ConcaveArrow,
            4 | 7 => Arrow::Diamond,
            5 | 8 => Arrow::Circle,
            6 | 9 => Arrow::Square,
            _ => Arrow::None,
        }
    }

    /// The arrowhead at the start and whether it is filled. Codes follow
    /// OWPML's order (none, arrow, spear, concave arrow, then empty and
    /// filled diamond, circle and box); bit 30 also fills it.
    pub fn head(&self) -> (Arrow, bool) {
        let code = (self.attr >> 10) & 0x3F;
        (Self::arrow(code), code >= 7 || self.attr & (1 << 30) != 0)
    }

    /// The arrowhead at the end; bit 31 fills it.
    pub fn tail(&self) -> (Arrow, bool) {
        let code = (self.attr >> 16) & 0x3F;
        (Self::arrow(code), code >= 7 || self.attr & (1 << 31) != 0)
    }

    /// Arrowhead sizes, 0 to 8: width class (small, medium, large) times 3
    /// plus length class.
    pub fn head_size(&self) -> u32 {
        (self.attr >> 22) & 0xF
    }

    pub fn tail_size(&self) -> u32 {
        (self.attr >> 26) & 0xF
    }
}

/// A gradient fill (table 28; Hancom stores the kind as a byte and the
/// numbers as 32-bit words).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Gradient {
    /// 1 linear, 2 radial, 3 conical, 4 square.
    pub kind: u8,
    pub angle: i32,
    /// Center in percent of the box.
    pub center: (i32, i32),
    /// Blur in percent.
    pub blur: i32,
    /// Stop positions, stored only when there are more than two colors.
    pub positions: Vec<i32>,
    pub colors: Vec<u32>,
}

/// A picture reference with its effects (tables 27 and 32).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImageRef {
    /// Percent, -100 to 100. Hancom writes contrast before brightness.
    pub contrast: i8,
    pub brightness: i8,
    /// 0 real picture, 1 grayscale, 2 black and white, 4 8x8 pattern.
    pub effect: u8,
    /// 1-based BinData id.
    pub bin_id: u16,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImageFill {
    /// Table 31: 0-4 tiled, 5 fit, 6-14 placed at an edge or the center, 15 none.
    pub mode: u8,
    pub image: ImageRef,
}

/// A fill (table 28): any combination of solid, gradient and image.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fill {
    /// Bit 0 solid, bit 1 image, bit 2 gradient.
    pub kind: u32,
    pub solid: Option<SolidFill>,
    pub gradient: Option<Gradient>,
    pub image: Option<ImageFill>,
    /// Additional properties after the length word; a gradient keeps its
    /// blur center here.
    pub extra: SharedBytes,
    /// One alpha byte per present kind (solid, gradient, image), when stored.
    pub alpha: Vec<u8>,
}

impl Fill {
    /// The gradient's blur center (percent), from the additional properties.
    pub fn gradient_center(&self) -> Option<u8> {
        self.gradient.as_ref()?;
        self.extra.first().copied()
    }

    /// Fill alpha, 0 opaque to 255 transparent.
    pub fn transparency(&self) -> u8 {
        self.alpha.first().copied().unwrap_or(0)
    }
}

/// A drawing object's shadow.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Shadow {
    /// 0 none; other values pick a direction or a perspective.
    pub kind: u32,
    pub color: u32,
    pub offset: (i32, i32),
}
