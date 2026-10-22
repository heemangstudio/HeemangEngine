//! Objects: their inner layouts, their boxes as characters, where floating
//! ones sit and which bands of the page they block.
//!
//! An object treated as a character is measured into an [`InlineBox`]; a
//! floating one is placed by [`place`] from an [`Anchor`] that callers fill
//! for the body, a header or footer frame, a note or a cell alike. Inner
//! layouts (equations, text inside shapes) ride in the paragraph's
//! [`crate::Measured::objects`].
//!
//! Top-and-bottom wrapping blocks the full width of every column the object
//! overlaps. Square, tight and through wrapping (text beside the object)
//! belong to the wrap pass.

use hm_model::units::mul_div;
use hm_model::{Control, ObjectCommon, Paragraph, Shape, TextBox};

use crate::heads::Heads;
use crate::math::{self, MathLayout};
use crate::measure::InlineBox;
use crate::paginate::Rect;
use crate::para::{ParaEnv, ParaLayout};
use crate::style::Styles;
use crate::table::CellLine;

/// What an object holds inside its box, laid out.
#[derive(Clone, Debug)]
pub enum ObjectLayout {
    Equation(MathLayout),
    /// The text of a drawing object's shapes, one frame per text box in
    /// the order the shape tree is walked (depth first).
    Shape(Vec<TextFrame>),
}

/// The paragraphs of one text box, stacked from the top of its content.
#[derive(Clone, Debug, Default)]
pub struct TextFrame {
    pub paras: Vec<ParaLayout>,
    pub lines: Vec<CellLine>,
    /// The last line's bottom.
    pub content: i32,
    /// The width the paragraphs were formatted for.
    pub width: i32,
}

/// Section default tab interval used inside shapes.
const TEXT_TAB: i32 = 4000;

/// Lays out every text box of a shape tree, depth first.
fn shape_text(shape: &Shape, styles: &mut Styles<'_>, out: &mut Vec<TextFrame>) {
    if let Some(t) = shape.text() {
        out.push(text_frame(t, shape.component.size.0 as i32, styles));
    }
    for child in shape.children() {
        shape_text(child, styles, out);
    }
}

/// The paragraphs of a text box formatted for its width less its side margins.
pub fn text_frame(t: &TextBox, shape_width: i32, styles: &mut Styles<'_>) -> TextFrame {
    let [ml, mr, _, _] = t.margins.unwrap_or([0; 4]).map(i32::from);
    let width = (t.max_width.unwrap_or(shape_width) - ml - mr).max(0);
    let info = styles.info();
    let env = ParaEnv {
        column_width: width,
        default_tab: TEXT_TAB,
    };
    let mut heads = Heads::default();
    let (paras, lines, content) = crate::table::stack(&t.paras, info, styles, env, &mut heads);
    TextFrame {
        paras,
        lines,
        content,
        width,
    }
}

/// Measures an object control: its box as a character when it is treated
/// as one, and its inner layout. `None` for controls that are not objects.
pub fn measure_object(
    c: &Control,
    styles: &mut Styles<'_>,
) -> Option<(Option<InlineBox>, Option<ObjectLayout>)> {
    let fonts = styles.fonts();
    let o = c.object_common()?;
    let [ml, mr, mt, mb] = o.margins.map(i32::from);
    let mut width = o.width;
    let mut height = o.height;
    let mut percent = 85;
    let mut inner = None;
    if let Some(eq) = c.as_object().and_then(|x| x.equation()) {
        // Hancom recomputes an equation's width from its script on open; the
        // stored height and baseline are kept unless missing.
        let ml_layout = math::layout(eq, fonts);
        if !eq.script.is_empty() {
            width = ml_layout.width;
        }
        if height <= 0 {
            height = ml_layout.height();
            percent = ml_layout.baseline_percent();
        } else {
            percent = eq.baseline as i32;
        }
        inner = Some(ObjectLayout::Equation(ml_layout));
    } else if let Some(shape) = c.as_object().and_then(|x| x.shape()) {
        let mut frames = Vec::new();
        shape_text(shape, styles, &mut frames);
        if !frames.is_empty() {
            inner = Some(ObjectLayout::Shape(frames));
        }
    }
    let inline = o.treat_as_char().then(|| {
        let h = height + mt + mb;
        InlineBox {
            pos: 0,
            control: 0,
            width: width + ml + mr,
            height: h,
            baseline: mul_div(h, percent, 100),
        }
    });
    Some((inline, inner))
}

/// How a floating object stacks with the text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    /// Behind the text.
    Behind,
    /// Beside or between the text (square, tight, through, top and bottom).
    Text,
    /// In front of the text.
    Front,
}

/// A floating object placed on a page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    /// Paragraph index in its list.
    pub para: u32,
    /// Index into the paragraph's controls.
    pub control: u32,
    /// The object's box (size without outer margins), page coordinates.
    pub rect: Rect,
    pub z: i32,
    pub layer: Layer,
}

fn layer_of(o: &ObjectCommon) -> Layer {
    match o.text_wrap() {
        2 => Layer::Behind,
        3 => Layer::Front,
        _ => Layer::Text,
    }
}

/// Places the floating objects (those not treated as characters) of a
/// paragraph, `para` being its index in its list.
pub fn place(paragraph: &Paragraph, para: u32, a: &Anchor) -> Vec<Placement> {
    let mut out = Vec::new();
    for (k, c) in paragraph.controls.iter().enumerate() {
        let Some(o) = c.object_common() else { continue };
        if o.treat_as_char() || c.as_object().is_none() {
            continue;
        }
        let (x, y, w, h) = frame(&o, a);
        out.push(Placement {
            para,
            control: k as u32,
            rect: Rect { x, y, w, h },
            z: o.z_order,
            layer: layer_of(&o),
        });
    }
    out
}

/// A vertical stretch of the page that lines must not overlap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Band {
    pub top: i32,
    pub bottom: i32,
    pub left: i32,
    pub right: i32,
}

impl Band {
    fn overlaps_x(&self, left: i32, right: i32) -> bool {
        self.left < right && left < self.right
    }
}

/// Where an object is anchored on its page.
pub struct Anchor {
    pub paper: (i32, i32),
    pub body: Rect,
    /// The column the anchoring paragraph is in (page coordinates).
    pub column: (i32, i32),
    /// Top of the anchoring paragraph (page coordinates).
    pub para_top: i32,
    pub para_left: i32,
}

/// Places `extent` inside `[start, start + size)` by Hancom's alignment code:
/// 0 start, 1 center, 2 end; offsets count inward from the aligned edge.
fn align(start: i32, size: i32, extent: i32, code: u32, offset: i32) -> i32 {
    match code {
        1 => start + (size - extent) / 2 + offset,
        2 => start + size - extent - offset,
        _ => start + offset,
    }
}

/// The page rectangle `(x, y, w, h)` of an object, margins excluded.
pub fn frame(o: &ObjectCommon, a: &Anchor) -> (i32, i32, i32, i32) {
    let (vs, vh) = match o.vert_rel_to() {
        0 => (0, a.paper.1),
        1 => (a.body.y, a.body.h),
        _ => (a.para_top, a.body.y + a.body.h - a.para_top),
    };
    let (hs, hw) = match (o.attr >> 8) & 3 {
        0 => (0, a.paper.0),
        1 => (a.body.x, a.body.w),
        2 => a.column,
        _ => (a.para_left, a.column.0 + a.column.1 - a.para_left),
    };
    let valign = if o.vert_rel_to() == 2 {
        0
    } else {
        (o.attr >> 5) & 7
    };
    let y = align(vs, vh, o.height, valign, o.y_offset);
    let x = align(hs, hw, o.width, (o.attr >> 10) & 7, o.x_offset);
    (x, y, o.width, o.height)
}

/// Bands blocked by the paragraph's top-and-bottom objects, split into
/// those that apply to the anchoring paragraph itself (paper- and
/// page-relative objects) and those that start with the next paragraph
/// (paragraph-relative objects, which sit at the paragraph's flow position).
///
/// An object occupies its outer margins plus its size, starting at the
/// anchor offset.
pub fn bands(para: &Paragraph, a: &Anchor, now: &mut Vec<Band>, after: &mut Vec<Band>) {
    for c in &para.controls {
        // Flowing tables push the content themselves, across pages.
        if crate::table::flows(c) {
            continue;
        }
        let Some(o) = c.object_common() else {
            continue;
        };
        if o.treat_as_char() || o.text_wrap() != 1 || o.width <= 0 || o.height <= 0 {
            continue;
        }
        let [ml, mr, mt, mb] = o.margins.map(i32::from);
        let outer = ObjectCommon {
            width: ml + o.width + mr,
            height: mt + o.height + mb,
            ..o
        };
        let (x, y, w, h) = frame(&outer, a);
        let band = Band {
            top: y,
            bottom: y + h,
            left: x,
            right: x + w,
        };
        if o.vert_rel_to() == 2 {
            after.push(band)
        } else {
            now.push(band)
        }
    }
}

/// Moves `top` below every band that a line `[top, top + height)` spanning
/// `[left, right)` would overlap.
pub fn clear(bands: &[Band], mut top: i32, height: i32, left: i32, right: i32) -> i32 {
    loop {
        let hit = bands
            .iter()
            .filter(|b| b.overlaps_x(left, right) && b.top < top + height.max(1) && top < b.bottom)
            .map(|b| b.bottom)
            .max();
        match hit {
            Some(bottom) if bottom > top => top = bottom,
            _ => return top,
        }
    }
}
