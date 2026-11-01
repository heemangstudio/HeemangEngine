//! Object controls: drawing objects (`gso `) and equations (`eqed`).
//!
//! An object control keeps its child records in source order as
//! [`ObjectPart`]s, so a dirty control re-encodes in the order it was read.
//! A drawing object is a tree of [`Shape`]s: each shape component carries
//! its geometry and style, and its own parts (kind record, text box, nested
//! shapes of a group).

use crate::{CtrlId, Fill, HwpString, ObjectCommon, Paragraph, RawRecord, SharedBytes};

/// The object header after the control id (HWP 5 table 69).
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ObjectHeader {
    pub common: ObjectCommon,
    pub instance_id: Option<u32>,
    pub prevent_split: Option<i32>,
    /// Alternative text.
    pub description: Option<HwpString>,
    pub tail: SharedBytes,
}

/// A drawing object or an equation.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ObjectControl {
    pub id: CtrlId,
    pub header: ObjectHeader,
    pub parts: Vec<ObjectPart>,
}

/// A child of an object control.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ObjectPart {
    Caption(Caption),
    Shape(Box<Shape>),
    Equation(Equation),
    /// A record the model does not interpret, level relative to the control.
    Raw(RawRecord),
}

impl ObjectControl {
    pub fn shape(&self) -> Option<&Shape> {
        self.parts.iter().find_map(|p| match p {
            ObjectPart::Shape(s) => Some(&**s),
            _ => None,
        })
    }

    pub fn equation(&self) -> Option<&Equation> {
        self.parts.iter().find_map(|p| match p {
            ObjectPart::Equation(e) => Some(e),
            _ => None,
        })
    }

    pub fn caption(&self) -> Option<&Caption> {
        self.parts.iter().find_map(|p| match p {
            ObjectPart::Caption(c) => Some(c),
            _ => None,
        })
    }
}

/// A caption: a paragraph list beside the object (tables 71 to 73).
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Caption {
    /// Paragraph count as stored.
    pub count: u32,
    pub list_attr: u32,
    /// Bits 0-1 side (left, right, top, bottom); bit 2 includes the margin
    /// in the width of a side caption.
    pub attr: Option<u32>,
    /// Width of a side caption.
    pub width: Option<i32>,
    /// Gap between caption and object.
    pub gap: Option<i16>,
    /// Text width of a top or bottom caption (the object's width).
    pub max_width: Option<i32>,
    pub tail: SharedBytes,
    pub paras: Vec<Paragraph>,
}

impl Caption {
    /// 0 left, 1 right, 2 top, 3 bottom.
    pub fn side(&self) -> u32 {
        self.attr.unwrap_or(3) & 3
    }
}

/// A 2x3 affine matrix `[a, b, tx, c, d, ty]`: `x' = a x + b y + tx`,
/// `y' = c x + d y + ty`.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Matrix(pub [f64; 6]);

impl Default for Matrix {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Matrix {
    pub const IDENTITY: Matrix = Matrix([1.0, 0.0, 0.0, 0.0, 1.0, 0.0]);

    /// The product `self × other`: `other` applied first, then `self`.
    pub fn mul(&self, other: &Matrix) -> Matrix {
        let [a, b, c, d, e, f] = other.0;
        let [g, h, i, j, k, l] = self.0;
        Matrix([
            g * a + h * d,
            g * b + h * e,
            g * c + h * f + i,
            j * a + k * d,
            j * b + k * e,
            j * c + k * f + l,
        ])
    }

    pub fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        let [a, b, c, d, e, f] = self.0;
        (a * x + b * y + c, d * x + e * y + f)
    }
}

/// The style block of a drawing shape component: outline, fill and the
/// trailing shadow fields newer files carry.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ShapeStyle {
    pub line: LineStyle,
    pub fill: Fill,
    pub shadow: Option<Shadow>,
    pub instance_id: Option<u32>,
    /// A byte Hancom leaves 0 except for some imported shapes.
    pub reserved: Option<u8>,
    pub shadow_alpha: Option<u8>,
}

/// A shape component (table 83) with the data its kind keeps inline.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ShapeComponent {
    /// The kind (`$rec`, `$pic`, `$con`, ...).
    pub kind: CtrlId,
    /// Top-level components write the kind twice.
    pub doubled: bool,
    /// Offset inside the group.
    pub offset: (i32, i32),
    pub group_level: u16,
    pub version: u16,
    pub initial_size: (u32, u32),
    pub size: (u32, u32),
    /// Bit 0 horizontal flip, bit 1 vertical flip.
    pub flip: u32,
    /// Degrees.
    pub rotation: i16,
    pub rotation_center: (i32, i32),
    pub translation: Matrix,
    /// `(scale, rotation)` pairs, one per grouping level.
    pub transforms: Vec<(Matrix, Matrix)>,
    /// Drawing kinds (lines, rectangles, ellipses, arcs, polygons, curves).
    pub style: Option<ShapeStyle>,
    /// Groups: the kinds of their members.
    pub members: Option<Vec<CtrlId>>,
    /// Groups: their instance id after the member list.
    pub group_instance: Option<u32>,
    pub tail: SharedBytes,
}

impl ShapeComponent {
    /// The rendering matrix from this component's own coordinates to its
    /// parent's: the translation, then each (rotation, scale) pair, the
    /// last pair applied first.
    pub fn matrix(&self) -> Matrix {
        let mut m = self.translation;
        for (scale, rot) in &self.transforms {
            m = m.mul(rot).mul(scale);
        }
        m
    }

    pub fn flip_horizontal(&self) -> bool {
        self.flip & 1 != 0
    }

    pub fn flip_vertical(&self) -> bool {
        self.flip & 2 != 0
    }
}

/// One shape: its component and its parts in source order.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Shape {
    pub component: ShapeComponent,
    pub parts: Vec<ShapePart>,
}

impl Shape {
    pub fn kind(&self) -> Option<&ShapePart> {
        self.parts.iter().find(|p| {
            !matches!(
                p,
                ShapePart::Text(_) | ShapePart::Child(_) | ShapePart::Raw(_)
            )
        })
    }

    pub fn text(&self) -> Option<&TextBox> {
        self.parts.iter().find_map(|p| match p {
            ShapePart::Text(t) => Some(t),
            _ => None,
        })
    }

    /// Group members.
    pub fn children(&self) -> impl Iterator<Item = &Shape> {
        self.parts.iter().filter_map(|p| match p {
            ShapePart::Child(c) => Some(&**c),
            _ => None,
        })
    }

    pub fn style(&self) -> Option<&ShapeStyle> {
        self.component.style.as_ref()
    }
}

/// A child of a shape component.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ShapePart {
    Line(LineGeom),
    Rect(RectGeom),
    Ellipse(EllipseGeom),
    Arc(ArcGeom),
    Polygon(PolyGeom),
    Curve(CurveGeom),
    Picture(Picture),
    Ole(Ole),
    /// Text inside the shape.
    Text(TextBox),
    /// A group member.
    Child(Box<Shape>),
    /// A record the model does not interpret, level relative to the shape
    /// component.
    Raw(RawRecord),
}

/// A straight line (table 92).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LineGeom {
    /// Attachment subjects and route points of a connector.
    pub connector: Option<Connector>,
    pub start: (i32, i32),
    pub end: (i32, i32),
    /// Set when Hancom corrected the direction of a horizontal or vertical line.
    pub attr: Option<u16>,
    pub tail: SharedBytes,
}

/// Connector subjects use drawing instance IDs and clockwise edge indices.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Connector {
    pub start: (u32, u32),
    pub end: (u32, u32),
    pub points: Vec<(i32, i32, u16)>,
}

/// A rectangle (table 94).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RectGeom {
    /// Corner rounding in percent: 0 square, 20 rounded, 50 half circle.
    pub round: u8,
    /// Corners clockwise from the top left.
    pub corners: [(i32, i32); 4],
    pub tail: SharedBytes,
}

/// An ellipse, possibly turned into an arc (table 96).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct EllipseGeom {
    /// Bit 0 interval needs recomputing, bit 1 is an arc, bits 2-9 arc kind.
    pub attr: u32,
    pub center: (i32, i32),
    pub axis1: (i32, i32),
    pub axis2: (i32, i32),
    pub start: (i32, i32),
    pub end: (i32, i32),
    pub start2: (i32, i32),
    pub end2: (i32, i32),
    pub tail: SharedBytes,
}

impl EllipseGeom {
    pub fn is_arc(&self) -> bool {
        self.attr & 2 != 0
    }

    /// 0 arc, 1 pie, 2 chord.
    pub fn arc_kind(&self) -> u32 {
        (self.attr >> 2) & 0xFF
    }
}

/// An arc (table 101).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ArcGeom {
    /// Arc kind in bits 0-7: 0 arc, 1 pie, 2 chord.
    pub attr: u32,
    pub center: (i32, i32),
    pub axis1: (i32, i32),
    pub axis2: (i32, i32),
    pub tail: SharedBytes,
}

/// A polygon (table 99).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PolyGeom {
    pub points: Vec<(i32, i32)>,
    pub tail: SharedBytes,
}

/// A curve (table 103): points and one segment kind per gap, 0 straight
/// and 1 a Bezier segment whose control points follow.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CurveGeom {
    pub points: Vec<(i32, i32)>,
    pub segments: Vec<u8>,
    pub tail: SharedBytes,
}

/// A picture (table 107).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Picture {
    pub border: LineStyle,
    /// The image rectangle corners as first inserted.
    pub corners: [(i32, i32); 4],
    /// The visible part of the image in image units: left, top, right, bottom.
    pub crop: [i32; 4],
    /// Left, right, top, bottom.
    pub margins: [i16; 4],
    pub image: ImageRef,
    pub border_alpha: u8,
    pub instance_id: u32,
    /// Effects (shadow, glow, soft edge, reflection) and the additional
    /// properties (original size, alpha), undecoded.
    pub tail: SharedBytes,
}

impl Picture {
    /// The image transparency, 0 opaque to 255 transparent, from the
    /// additional properties at the end of the record.
    pub fn alpha(&self) -> u8 {
        if self.tail.len() >= 13 {
            self.tail[self.tail.len() - 1]
        } else {
            0
        }
    }

    /// The image's original size in HU, from the additional properties.
    pub fn original_size(&self) -> Option<(i32, i32)> {
        let t = &self.tail[..];
        if t.len() < 13 {
            return None;
        }
        let at = t.len() - 9;
        let w = i32::from_le_bytes(t[at..at + 4].try_into().ok()?);
        let h = i32::from_le_bytes(t[at + 4..at + 8].try_into().ok()?);
        Some((w, h))
    }

    /// The effect flags word: bit 0 shadow, 1 glow, 2 soft edge, 3 reflection.
    pub fn effects(&self) -> u32 {
        self.tail
            .get(0..4)
            .map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap()))
    }

    /// The effect flags word and the effects it lists, as stored: the tail
    /// up to the additional properties.
    pub fn effect_bytes(&self) -> &[u8] {
        let flags = self.effects();
        let t = &self.tail[..];
        let i32at = |o: usize| -> Option<i32> {
            Some(i32::from_le_bytes(t.get(o..o + 4)?.try_into().ok()?))
        };
        // A color: kind, a value of 4 or 12 bytes, then counted modifiers.
        let color = |o: usize| -> Option<usize> {
            let len = if matches!(i32at(o)?, 5 | 6) { 12 } else { 4 };
            let n = i32at(o + 4 + len)?.clamp(0, 64) as usize;
            Some(4 + len + 4 + 8 * n)
        };
        let mut at = 4;
        if flags & 1 != 0 {
            at += 44 + color(at + 44).unwrap_or(0);
        }
        if flags & 2 != 0 {
            at += 8 + color(at + 8).unwrap_or(0);
        }
        if flags & 4 != 0 {
            at += 4;
        }
        if flags & 8 != 0 {
            at += 56;
        }
        &t[..at.min(t.len())]
    }

    /// The picture effects (tables 108 to 113), in flag order.
    pub fn effect_list(&self) -> PictureEffects {
        let flags = self.effects();
        let t = &self.tail[..];
        let f32at = |o: usize| -> Option<f32> {
            Some(f32::from_le_bytes(t.get(o..o + 4)?.try_into().ok()?))
        };
        let i32at = |o: usize| -> Option<i32> {
            Some(i32::from_le_bytes(t.get(o..o + 4)?.try_into().ok()?))
        };
        // A color: kind, a value of 4 or 12 bytes, then counted modifiers.
        let color = |o: usize| -> Option<(u32, usize)> {
            let kind = i32at(o)?;
            let (rgb, len) = match kind {
                5 | 6 => (0, 12),
                0 => (i32at(o + 4)? as u32 & 0x00FF_FFFF, 4),
                _ => (0, 4),
            };
            let n = i32at(o + 4 + len)?.clamp(0, 64) as usize;
            // 0x00rrggbb to COLORREF.
            let c = ((rgb & 0xFF) << 16) | (rgb & 0xFF00) | ((rgb >> 16) & 0xFF);
            Some((c, 4 + len + 4 + 8 * n))
        };
        let mut out = PictureEffects::default();
        let mut at = 4;
        if flags & 1 != 0 {
            let Some((c, len)) = color(at + 44) else {
                return out;
            };
            out.shadow = Some(PictureShadow {
                transparency: f32at(at + 4).unwrap_or(0.0),
                blur: f32at(at + 8).unwrap_or(0.0),
                direction: f32at(at + 12).unwrap_or(0.0),
                distance: f32at(at + 16).unwrap_or(0.0),
                scale: (f32at(at + 32).unwrap_or(1.0), f32at(at + 36).unwrap_or(1.0)),
                color: c,
            });
            at += 44 + len;
        }
        if flags & 2 != 0 {
            let Some((c, len)) = color(at + 8) else {
                return out;
            };
            out.glow = Some(PictureGlow {
                transparency: f32at(at).unwrap_or(0.0),
                radius: f32at(at + 4).unwrap_or(0.0),
                color: c,
            });
            at += 8 + len;
        }
        if flags & 4 != 0 {
            out.soft_edge = f32at(at);
            at += 4;
        }
        if flags & 8 != 0 {
            out.reflection = Some(PictureReflection {
                radius: f32at(at + 4).unwrap_or(0.0),
                distance: f32at(at + 12).unwrap_or(0.0),
                start_alpha: f32at(at + 36).unwrap_or(0.5),
                start: f32at(at + 40).unwrap_or(0.0),
                end_alpha: f32at(at + 44).unwrap_or(1.0),
                end: f32at(at + 48).unwrap_or(0.5),
            });
        }
        out
    }
}

/// A picture's shadow: transparency 0 to 1, blur, distance in HU,
/// direction in degrees (0 right, 90 down), scale and COLORREF color.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PictureShadow {
    pub transparency: f32,
    pub blur: f32,
    pub direction: f32,
    pub distance: f32,
    pub scale: (f32, f32),
    pub color: u32,
}

/// A glow around a picture.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PictureGlow {
    pub transparency: f32,
    pub radius: f32,
    pub color: u32,
}

/// A mirrored copy below a picture, fading from `start_alpha` (a
/// transparency) at `start` to `end_alpha` at `end` (fractions of its
/// height).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PictureReflection {
    pub radius: f32,
    pub distance: f32,
    pub start_alpha: f32,
    pub start: f32,
    pub end_alpha: f32,
    pub end: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PictureEffects {
    pub shadow: Option<PictureShadow>,
    pub glow: Option<PictureGlow>,
    /// Soft edge radius in HU.
    pub soft_edge: Option<f32>,
    pub reflection: Option<PictureReflection>,
}

/// An OLE object (tables 118 and 119).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Ole {
    /// HWPX drawing identity (`instid`), distinct from the common XML object `id`.
    /// HWP 5 has only the common instance-ID slot and reads it into this field.
    pub instance_id: Option<u32>,
    /// Bits 0-7 draw aspect, 8 moniker, 9-15 baseline, 16-21 object type.
    pub attr: u32,
    pub extent: (i32, i32),
    /// BinData id of the storage.
    pub bin_id: u16,
    pub border: Option<LineStyle>,
    pub tail: SharedBytes,
}

impl Ole {
    /// 0 unknown, 1 embedded, 2 link, 3 static, 4 equation.
    pub fn object_type(&self) -> u32 {
        (self.attr >> 16) & 0x3F
    }
}

/// Text inside a shape (tables 65, 89 and 90).
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TextBox {
    pub count: u32,
    /// Bits 0-2 text direction (1 vertical, 2 vertical all upright), 3-4
    /// line wrap, 5-6 vertical alignment (top, center, bottom).
    pub attr: u32,
    /// Left, right, top, bottom.
    pub margins: Option<[i16; 4]>,
    pub max_width: Option<i32>,
    pub tail: SharedBytes,
    pub paras: Vec<Paragraph>,
    /// This list's link id and the id of the box its text flows on into
    /// (0 for none), when the format stores them apart from [`Self::tail`].
    pub link: Option<(u32, u32)>,
}

impl TextBox {
    /// This box's link id and the id of the box its text flows on into
    /// (0 for none): HWP 5 keeps them as the first two words after the
    /// maximum width.
    pub fn links(&self) -> (u32, u32) {
        self.link.unwrap_or_else(|| {
            let w = |o: usize| {
                self.tail
                    .get(o..o + 4)
                    .map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap()))
            };
            (w(0), w(4))
        })
    }

    /// Text runs down the box: 1 with Latin turned, 2 with every
    /// character upright.
    pub fn vertical(&self) -> bool {
        matches!(self.attr & 7, 1 | 2)
    }

    /// 0 break lines, 1 condense to keep one line, 2 widen with the content.
    pub fn line_wrap(&self) -> u32 {
        (self.attr >> 3) & 3
    }

    /// 0 top, 1 center, 2 bottom.
    pub fn vertical_align(&self) -> u32 {
        (self.attr >> 5) & 3
    }
}

/// An equation (table 105).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Equation {
    /// Bit 0: the script spans lines rather than characters.
    pub attr: u32,
    pub script: HwpString,
    /// Base size in HU.
    pub size: u32,
    pub color: u32,
    /// Baseline as a percentage of the object height.
    pub baseline: i16,
    /// Two bytes after the baseline that Hancom writes as zero.
    pub reserved: Option<u16>,
    pub version: Option<HwpString>,
    pub font: Option<HwpString>,
    pub tail: SharedBytes,
}

/// An outline: color, width and the packed attribute word of table 87, plus
/// the outline placement byte drawing objects carry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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

/// A picture reference with its effects (tables 27 and 32).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ImageRef {
    /// Percent, -100 to 100. Hancom writes contrast before brightness.
    pub contrast: i8,
    pub brightness: i8,
    /// 0 real picture, 1 grayscale, 2 black and white, 4 8x8 pattern.
    pub effect: u8,
    /// 1-based BinData id.
    pub bin_id: u16,
}

/// A drawing object's shadow.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Shadow {
    /// 0 none; other values pick a direction or a perspective.
    pub kind: u32,
    pub color: u32,
    pub offset: (i32, i32),
}
