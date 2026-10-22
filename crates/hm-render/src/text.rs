//! Drawing laid-out lines: glyph runs, paragraph heads, highlights,
//! shading, underlines and strikes, and objects set as characters.
//!
//! Body lines, cell lines and every other paragraph list draw through
//! [`draw_line`], given the line's column origin.

use hm_fonts::FontSystem;
use hm_layout::{Kind, Line, ParaLayout, Placed, Style, place};
use hm_model::{Document, Paragraph, Unit};

use crate::{DisplayList, GlyphRun, Item};

/// What line drawing reads besides the line itself.
pub(crate) struct Ctx<'a> {
    pub doc: &'a Document,
    pub styles: &'a [Style],
    pub fonts: &'a FontSystem,
    /// The page being drawn, for objects anchored to the paper or page.
    pub page: Option<&'a hm_layout::Page>,
}

/// Draws line `li` of a paragraph whose column starts at `x` and whose
/// line top is at `y` (page coordinates). `placed` is scratch space.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_line(
    dl: &mut DisplayList,
    ctx: &Ctx<'_>,
    para: &Paragraph,
    pal: &ParaLayout,
    li: usize,
    x: i32,
    y: i32,
    placed: &mut Vec<Placed>,
) {
    let Some(line) = pal.lines.get(li) else {
        return;
    };
    place(pal, line, placed);
    highlights(dl, para, line, x, y, placed);
    shades(dl, ctx, pal, line, x, y, placed);
    glyphs(dl, ctx, pal, line, x, y, placed);
    if li == 0 {
        head(dl, ctx, pal, line, x, y);
    }
    decorations(dl, ctx, pal, line, x, y, placed);
    inline_objects(dl, ctx, para, pal, line, x, y, placed);
}

fn glyphs(
    dl: &mut DisplayList,
    ctx: &Ctx<'_>,
    pal: &ParaLayout,
    line: &Line,
    x: i32,
    y: i32,
    placed: &[Placed],
) {
    let m = &pal.measured;
    let mut run: Option<(u32, GlyphRun)> = None;
    for p in placed {
        let Kind::Char(ch) = p.kind else { continue };
        let si = m.styles[p.pos as usize];
        let style = &ctx.styles[si as usize];
        let Some(face_id) = style.face else { continue };
        let Some(glyph) = ctx.fonts.face(face_id).and_then(|f| f.glyph(ch)) else {
            continue;
        };
        let baseline = y + pal.run_baseline(line, style.line_size) + style.offset;
        let fresh = run
            .as_ref()
            .is_none_or(|(s, r)| *s != si || r.baseline != baseline);
        if fresh {
            if let Some((_, r)) = run.take() {
                dl.items.push(Item::Glyphs(r));
            }
            run = Some((
                si,
                GlyphRun {
                    face: face_id,
                    size: style.size,
                    ratio: style.ratio,
                    color: style.color,
                    fake_bold: style.fake_bold,
                    italic: style.italic,
                    baseline,
                    glyphs: Vec::new(),
                },
            ));
        }
        run.as_mut().unwrap().1.glyphs.push((glyph.glyph, x + p.x));
    }
    if let Some((_, r)) = run.take() {
        dl.items.push(Item::Glyphs(r));
    }
}

/// The paragraph head before the first line.
fn head(dl: &mut DisplayList, ctx: &Ctx<'_>, pal: &ParaLayout, line: &Line, x: i32, y: i32) {
    let Some(head) = &pal.head else { return };
    let origin = x + line.x - head.width;
    for &(ch, si, gx) in &head.glyphs {
        let style = &ctx.styles[si as usize];
        let Some(face_id) = style.face else { continue };
        let Some(glyph) = ctx.fonts.face(face_id).and_then(|f| f.glyph(ch)) else {
            continue;
        };
        dl.items.push(Item::Glyphs(GlyphRun {
            face: face_id,
            size: style.size,
            ratio: style.ratio,
            color: style.color,
            fake_bold: style.fake_bold,
            italic: style.italic,
            baseline: y + pal.run_baseline(line, style.line_size) + style.offset,
            glyphs: vec![(glyph.glyph, origin + gx)],
        }));
    }
}

/// Highlighter range tags (kind 2, end exclusive) behind the glyphs.
fn highlights(
    dl: &mut DisplayList,
    para: &Paragraph,
    line: &Line,
    x: i32,
    y: i32,
    placed: &[Placed],
) {
    for tag in para.range_tags.iter().filter(|t| t.tag >> 24 == 2) {
        for p in placed
            .iter()
            .filter(|p| p.pos >= tag.start && p.pos < tag.end)
        {
            dl.items.push(Item::Rect {
                x: x + p.x,
                y,
                w: p.advance,
                h: line.height,
                color: tag.tag & 0x00FF_FFFF,
            });
        }
    }
}

/// Character shading behind the glyphs, one rectangle per character.
fn shades(
    dl: &mut DisplayList,
    ctx: &Ctx<'_>,
    pal: &ParaLayout,
    line: &Line,
    x: i32,
    y: i32,
    placed: &[Placed],
) {
    let m = &pal.measured;
    for p in placed {
        if !matches!(p.kind, Kind::Char(_) | Kind::Space) {
            continue;
        }
        if let Some(color) = ctx.styles[m.styles[p.pos as usize] as usize].shade {
            dl.items.push(Item::Rect {
                x: x + p.x,
                y,
                w: p.advance,
                h: line.height,
                color,
            });
        }
    }
}

/// Underlines and strikes, one rectangle per styled stretch.
fn decorations(
    dl: &mut DisplayList,
    ctx: &Ctx<'_>,
    pal: &ParaLayout,
    line: &Line,
    x: i32,
    y: i32,
    placed: &[Placed],
) {
    let m = &pal.measured;
    for p in placed {
        if !matches!(p.kind, Kind::Char(_) | Kind::Space) {
            continue;
        }
        let style = &ctx.styles[m.styles[p.pos as usize] as usize];
        let thickness = (style.size / 15).max(7);
        let baseline = y + pal.run_baseline(line, style.line_size);
        if style.underline == 1 {
            dl.items.push(Item::Rect {
                x: x + p.x,
                y: baseline + style.size / 10,
                w: p.advance,
                h: thickness,
                color: style.underline_color,
            });
        }
        if style.strike {
            dl.items.push(Item::Rect {
                x: x + p.x,
                y: baseline - style.size * 3 / 10,
                w: p.advance,
                h: thickness,
                color: style.color,
            });
        }
    }
}

/// The control index of the extended control at wire position `pos`.
fn control_at(para: &Paragraph, pos: u32) -> Option<u32> {
    para.units().find_map(|(p, u)| match u {
        Unit::Extended { index, .. } if p == pos => Some(index),
        _ => None,
    })
}

/// Objects set as characters: the box sits on the line's baseline, its
/// content inside the outer margins.
#[allow(clippy::too_many_arguments)]
fn inline_objects(
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
        let Kind::Object(i) = p.kind else { continue };
        let b = pal.measured.boxes[i as usize];
        let Some(k) = control_at(para, b.pos) else {
            continue;
        };
        let Some(control) = para.controls.get(k as usize) else {
            continue;
        };
        let Some(o) = control.object_common() else {
            continue;
        };
        let top = y + line.baseline - b.baseline;
        let (ox, oy) = (x + p.x + o.margins[0] as i32, top + o.margins[2] as i32);
        if let (Some(t), Some(tl)) = (control.as_table(), pal.table(k)) {
            crate::table::draw(dl, ctx, t, tl, ox, oy, None);
        } else if control.as_object().is_some() {
            crate::objects::paint_inline(dl, ctx.fonts, pal, k, ox, y + line.baseline);
        }
    }
}
