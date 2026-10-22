//! Painting objects: inline ones in their lines, floating ones at their
//! placements.
//!
//! Callers that lay out paragraphs in a frame (body, header, footer, note,
//! cell) paint inline objects with [`paint_inline`] while drawing a line and
//! floating ones with [`paint_placement`].

use hm_fonts::FontSystem;
use hm_layout::ParaLayout;
use hm_layout::math::MathLayout;
use hm_layout::objects::{ObjectLayout, Placement};
use hm_model::Paragraph;

use crate::{DisplayList, GlyphRun, Item};

/// The inner layout of control `control` of a paragraph.
fn inner(layout: &ParaLayout, control: u32) -> Option<&ObjectLayout> {
    layout
        .measured
        .objects
        .iter()
        .find(|(k, _)| *k == control)
        .map(|(_, o)| &**o)
}

/// Paints an object treated as a character: control `control` of a
/// paragraph, its box's left edge (after the outer margin) at `x`, on a
/// line whose baseline is at `baseline` (page coordinates).
pub fn paint_inline(
    dl: &mut DisplayList,
    fonts: &FontSystem,
    layout: &ParaLayout,
    control: u32,
    x: i32,
    baseline: i32,
) {
    match inner(layout, control) {
        Some(ObjectLayout::Equation(m)) => paint_math(dl, fonts, m, x, baseline),
        None => {}
    }
}

/// Paints a floating object at its placement.
pub fn paint_placement(
    dl: &mut DisplayList,
    fonts: &FontSystem,
    _para: &Paragraph,
    layout: &ParaLayout,
    p: &Placement,
) {
    match inner(layout, p.control) {
        Some(ObjectLayout::Equation(m)) => {
            let baseline = p.rect.y + p.rect.h.min(m.ascent).max(0);
            paint_math(dl, fonts, m, p.rect.x, baseline);
        }
        None => {}
    }
}

/// Paints an equation with its origin at `x` and its baseline at `y`.
pub fn paint_math(dl: &mut DisplayList, fonts: &FontSystem, m: &MathLayout, x: i32, y: i32) {
    use hm_layout::math::MathItem;
    for it in &m.items {
        match *it {
            MathItem::Glyph {
                face,
                glyph,
                size,
                x: gx,
                y: gy,
                x_scale,
                bold,
                italic,
                color,
                ..
            } => {
                if fonts.face(face).is_none() {
                    continue;
                }
                dl.items.push(Item::Glyphs(GlyphRun {
                    face,
                    size,
                    ratio: x_scale,
                    color,
                    fake_bold: bold,
                    italic,
                    baseline: y + gy,
                    glyphs: vec![(glyph, x + gx)],
                }));
            }
            MathItem::Rule {
                x: rx,
                y: ry,
                w,
                h,
                color,
            } => dl.items.push(Item::Rect {
                x: x + rx,
                y: y + ry,
                w,
                h,
                color,
            }),
        }
    }
}
