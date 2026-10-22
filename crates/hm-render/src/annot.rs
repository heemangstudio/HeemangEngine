//! Overlaid-characters and ruby boxes: their glyphs as laid out.

use hm_layout::{ParaLayout, annot};

use crate::text::Ctx;
use crate::{DisplayList, GlyphRun, Item};

/// Draws the glyphs of control `index` for a box whose left edge is `x`
/// on a line with baseline `baseline`. False when the control has no
/// such layout.
pub(crate) fn draw(
    dl: &mut DisplayList,
    ctx: &Ctx<'_>,
    pal: &ParaLayout,
    index: u32,
    x: i32,
    baseline: i32,
) -> bool {
    let Some(layout) = annot::of(&pal.measured, index) else {
        return false;
    };
    for g in &layout.glyphs {
        let style = &ctx.styles[g.style as usize];
        let Some(face_id) = style.face else { continue };
        let Some(glyph) = ctx.fonts.face(face_id).and_then(|f| f.glyph(g.ch)) else {
            continue;
        };
        dl.items.push(Item::Glyphs(GlyphRun {
            face: face_id,
            size: g.size,
            ratio: g.ratio,
            color: g.color.unwrap_or(style.color),
            fake_bold: style.fake_bold,
            italic: style.italic,
            baseline: baseline + g.dy + style.offset,
            glyphs: vec![(glyph.glyph, x + g.dx)],
        }));
    }
    true
}
