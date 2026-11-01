//! Tab leaders: the dots or dashes that fill a tab's gap.
//!
//! Hancom sets a leader as repeated characters in the tab's style, a
//! quarter of the size apart. The last one starts two steps before the
//! tab's end and the rest step back from there while they stay inside the
//! tab.
//! The leader kind is the low byte of the tab's third data unit, as Hancom
//! stores it with the tab.

use hm_layout::{Kind, Line, ParaLayout, Placed};
use hm_model::Paragraph;

use crate::text::Ctx;
use crate::{DisplayList, GlyphRun, Item};

/// Draws the leaders of the tabs on a line placed at `(x, y)`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw(
    dl: &mut DisplayList,
    ctx: &Ctx<'_>,
    para: &Paragraph,
    pal: &ParaLayout,
    line: &Line,
    x: i32,
    y: i32,
    placed: &[Placed],
) {
    for p in placed {
        if p.kind != Kind::Tab || p.advance <= 0 {
            continue;
        }
        let pos = p.pos as usize;
        let Some(&data) = para.text.get(pos + 3) else {
            continue;
        };
        let Some(ch) = hm_layout::measure::tab_leader_char(data & 0xFF) else {
            continue;
        };
        let si = pal.measured.styles[pos];
        let style = &ctx.styles[si as usize];
        let Some(face_id) = style.face else { continue };
        let Some(face) = ctx.fonts.face(face_id) else {
            continue;
        };
        let Some(glyph) = face.glyph(ch) else {
            continue;
        };
        // A quarter of the size, in Hancom's 4-HU steps, whatever the
        // character's own advance.
        let pitch = (style.size + 15) / 16 * 4;
        if pitch <= 0 {
            continue;
        }
        let (start, end) = (x + p.x, x + p.x + p.advance);
        let mut glyphs = Vec::new();
        let mut at = end - 2 * pitch;
        while at >= start {
            glyphs.push((glyph.glyph, at));
            at -= pitch;
        }
        if glyphs.is_empty() {
            continue;
        }
        glyphs.reverse();
        dl.items.push(Item::Glyphs(GlyphRun {
            face: face_id,
            size: style.size,
            ratio: style.ratio,
            color: style.color,
            fake_bold: style.fake_bold,
            italic: style.italic,
            baseline: y + pal.run_baseline(line, style.line_size) + style.offset,
            glyphs,
        }));
    }
}
