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
#[derive(Clone, Copy)]
pub(crate) struct Ctx<'a> {
    pub doc: &'a Document,
    pub styles: &'a [Style],
    pub fonts: &'a FontSystem,
    /// The page being drawn, for objects anchored to the paper or page.
    pub page: Option<&'a hm_layout::Page>,
    /// Enclosing body paragraph for drawings free of an inline table's cell.
    pub body_anchor: Option<hm_layout::objects::Anchor>,
    /// Where drawn lines are recorded, when wanted.
    pub lines: Option<&'a std::cell::RefCell<Vec<crate::LineBox>>>,
    /// Visible cell boxes, collected only for table geometry queries.
    pub cells: Option<&'a std::cell::RefCell<Vec<crate::CellBox>>>,
    /// Object and table boxes, collected only for object queries.
    pub objects: Option<&'a std::cell::RefCell<Vec<crate::ObjectBox>>>,
    /// Only lines are wanted: text and decorations are skipped.
    pub index_only: bool,
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
    if let Some(sink) = ctx.lines {
        sink.borrow_mut()
            .push(line_box(para, pal, li, x, y, placed));
    }
    if ctx.index_only {
        // Objects set in the line may hold lines of their own.
        inline_objects(dl, ctx, para, pal, line, x, y, placed);
        return;
    }
    highlights(dl, para, line, x, y, placed);
    shades(dl, ctx, pal, line, x, y, placed);
    crate::proof::draw(dl, ctx, para, pal, line, x, y, placed, true);
    glyphs(dl, ctx, pal, line, x, y, placed);
    crate::tabfill::draw(dl, ctx, para, pal, line, x, y, placed);
    if li == 0 {
        head(dl, ctx, pal, line, x, y);
    }
    crate::decor::draw(dl, ctx, pal, line, x, y, placed);
    crate::decor::emphasis(dl, ctx, pal, line, x, y, placed);
    crate::proof::draw(dl, ctx, para, pal, line, x, y, placed, false);
    inline_objects(dl, ctx, para, pal, line, x, y, placed);
    // Hancom boxes bordered runs in every list that draws lines.
    borders(dl, ctx, pal, line, x, y, placed);
}

fn line_box(
    para: &Paragraph,
    pal: &ParaLayout,
    li: usize,
    x: i32,
    y: i32,
    placed: &[Placed],
) -> crate::LineBox {
    let line = &pal.lines[li];
    let mut stops: Vec<(u32, i32)> = placed.iter().map(|p| (p.pos, x + p.x)).collect();
    let end = placed.last().map_or(x + line.x, |p| x + p.x + p.advance);
    stops.push((line.end, end));
    crate::LineBox {
        para: para as *const Paragraph as usize,
        line: li as u32,
        x,
        y,
        height: line.height,
        baseline: line.baseline,
        stops,
    }
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
    let mut runs: Vec<(u32, GlyphRun)> = Vec::new();
    for p in placed {
        let Kind::Char(ch) = p.kind else { continue };
        let si = m.styles[p.pos as usize];
        let style = &ctx.styles[si as usize];
        let Some(face_id) = style.face else { continue };
        let glyph = m
            .clusters
            .binary_search_by_key(&p.pos, |c| c.start)
            .ok()
            .map(|i| m.clusters[i].glyph)
            .or_else(|| {
                ctx.fonts
                    .face(face_id)
                    .and_then(|f| f.glyph(ch))
                    .map(|g| g.glyph)
            });
        let Some(glyph) = glyph else {
            continue;
        };
        let baseline = y + pal.run_baseline(line, style.line_size) + style.offset;
        let fresh = run
            .as_ref()
            .is_none_or(|(s, r)| *s != si || r.baseline != baseline);
        if fresh {
            if let Some(r) = run.take() {
                runs.push(r);
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
        run.as_mut().unwrap().1.glyphs.push((glyph, x + p.x));
    }
    if let Some(r) = run.take() {
        runs.push(r);
    }
    // Shadows go behind every glyph of the line.
    for (si, r) in &runs {
        shadow(dl, ctx, *si, r);
    }
    for (si, r) in runs {
        crate::decor::push_run(dl, ctx, si, r);
    }
}

/// Draws the shadow of a glyph run whose character shape has one (attribute
/// bits 11-12: 1 separate, 2 continuous): the glyphs again in the shadow
/// color, offset by the shape's shadow distances, a percentage of the size;
/// a continuous shadow fills the way there.
fn shadow(dl: &mut DisplayList, ctx: &Ctx<'_>, si: u32, r: &GlyphRun) {
    let style = &ctx.styles[si as usize];
    let Some(cs) = ctx.doc.info.char_shapes.get(style.char_shape as usize) else {
        return;
    };
    let kind = (cs.attr >> 11) & 3;
    if kind == 0 {
        return;
    }
    let [ox, oy] = cs.shadow_offset.map(|v| r.size * v as i32 / 100);
    // A continuous shadow repeats the glyphs about every 10 HU of the way.
    let steps = if kind == 2 {
        (ox.abs().max(oy.abs()) / 10).clamp(1, 32)
    } else {
        1
    };
    for k in 1..=steps {
        let (dx, dy) = (ox * k / steps, oy * k / steps);
        dl.items.push(Item::Glyphs(GlyphRun {
            color: cs.shadow_color,
            baseline: r.baseline + dy,
            glyphs: r.glyphs.iter().map(|&(g, gx)| (g, gx + dx)).collect(),
            ..r.clone()
        }));
    }
}

/// The paragraph head before the first line.
fn head(dl: &mut DisplayList, ctx: &Ctx<'_>, pal: &ParaLayout, line: &Line, x: i32, y: i32) {
    let Some(head) = &pal.head else { return };
    let origin = x + line.x - head.width;
    if let Some(image) = head.image
        && let Some(data) = ctx.doc.bins.as_ref().and_then(|bins| bins.get(image.bin))
    {
        let size = head.size;
        let top = y + pal.run_baseline(line, size) - hm_model::units::mul_div(size, 85, 100);
        dl.items.push(Item::Image(Box::new(crate::draw::ImageItem {
            data,
            frame: [size, 0, origin, 0, size, top],
            crop: [0; 4],
            effects: crate::image::Effects {
                contrast: image.contrast,
                brightness: image.brightness,
                effect: image.effect,
                ..Default::default()
            },
            natural: (0, 0),
        })));
    }
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

/// Character borders follow contiguous cells of the same border fill.
/// Their box follows the logical em, independently of a run's raised or
/// lowered glyphs.
pub(crate) fn borders(
    dl: &mut DisplayList,
    ctx: &Ctx<'_>,
    pal: &ParaLayout,
    line: &Line,
    x: i32,
    y: i32,
    placed: &[Placed],
) {
    use hm_layout::paginate::Rect;
    struct Span {
        fill: u16,
        rect: Rect,
        half: [i32; 2],
    }
    let mut open: Option<Span> = None;
    let flush = |dl: &mut DisplayList, span: Span| {
        let Span {
            fill,
            rect: r,
            half: [a, z],
            ..
        } = span;
        crate::paint::border_sides_meeting(
            dl,
            r.x,
            r.y,
            r.w,
            r.h,
            &ctx.doc.info.border_fills[fill as usize - 1],
            crate::paint::Sides::ALL,
            [a, z, a, z],
        );
    };
    for (k, p) in placed.iter().enumerate() {
        let want = if matches!(p.kind, Kind::Char(_) | Kind::Space) {
            let style = &ctx.styles[pal.measured.styles[p.pos as usize] as usize];
            ctx.doc
                .info
                .char_shapes
                .get(style.char_shape as usize)
                .and_then(|c| c.border_fill)
                .filter(|&id| {
                    id > 0
                        && ctx
                            .doc
                            .info
                            .border_fills
                            .get(id as usize - 1)
                            .is_some_and(|b| b.sides.iter().any(|s| s.visible()))
                })
                .map(|id| {
                    let sides = ctx.doc.info.border_fills[id as usize - 1].sides;
                    let half = |side: usize| {
                        if sides[side].visible() {
                            sides[side].width_hu() / 2
                        } else {
                            0
                        }
                    };
                    let (top, bottom) = (half(2), half(3));
                    let from = line.first_item as usize;
                    let end_item = line.end_item as usize;
                    let (left, right) = pal.measured.border_room(from + k, from, end_item);
                    let end = placed.get(k + 1).map_or(p.x + p.advance + right, |q| {
                        let (next_left, _) = pal.measured.border_room(from + k + 1, from, end_item);
                        q.x - next_left
                    });
                    let (a, z) = (left / 2, right / 2);
                    Span {
                        fill: id,
                        rect: Rect {
                            x: x + p.x - a,
                            y: y + pal.run_baseline(line, style.line_size)
                                - hm_model::units::mul_div(style.size, 85, 100)
                                - top,
                            w: end - p.x + a - z,
                            h: style.size + top + bottom,
                        },
                        half: [a, z],
                    }
                })
        } else {
            None
        };
        match (open.as_mut(), want) {
            (Some(a), Some(b))
                if a.fill == b.fill
                    && a.rect.y == b.rect.y
                    && a.rect.h == b.rect.h
                    && a.rect.x + a.rect.w == b.rect.x =>
            {
                a.rect.w += b.rect.w;
                a.half[1] = b.half[1];
            }
            (_, next) => {
                if let Some(previous) = open.take() {
                    flush(dl, previous);
                }
                open = next;
            }
        }
    }
    if let Some(previous) = open {
        flush(dl, previous);
    }
}

/// The control index of the extended control at wire position `pos`.
fn control_at(para: &Paragraph, pos: u32) -> Option<u32> {
    para.units().find_map(|(p, u)| match u {
        Unit::Extended { index, .. } if p == pos => Some(index),
        _ => None,
    })
}

/// Objects set as characters: the box sits on the line's baseline, or
/// aligns like the runs under a paragraph's vertical alignment, its
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
        if crate::annot::draw(dl, ctx, pal, k, x + p.x, y + line.baseline) {
            continue;
        }
        let Some(control) = para.controls.get(k as usize) else {
            continue;
        };
        let Some(o) = control.object_common() else {
            continue;
        };
        let top = y + pal.box_top(line, b.height, b.baseline);
        let (ox, oy) = (x + p.x + o.margins[0] as i32, top + o.margins[2] as i32);
        if let (Some(t), Some(tl)) = (control.as_table(), pal.table(k)) {
            let (gx, gy) = tl.grid_offset();
            crate::table::draw_with_caption(dl, ctx, t, tl, ox + gx, oy + gy);
        } else if control.as_object().is_some() || control.as_form().is_some() {
            crate::objects::paint_inline(dl, ctx, para, pal, k, (ox, oy), top + b.baseline);
        }
    }
}
