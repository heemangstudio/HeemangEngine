//! Painting objects: inline ones in their lines, floating ones at their
//! placements.
//!
//! Callers that lay out paragraphs in a frame (body, header, footer, note,
//! cell) paint inline objects with [`paint_inline`] while drawing a line and
//! floating ones with [`paint_placement`]. A drawing object is a tree of
//! shapes; each shape maps its own coordinates to its parent's through its
//! rendering matrix, and the top shape's coordinates start at the object's
//! box.

use hm_layout::math::MathLayout;
use hm_layout::objects::{ObjectLayout, Placement, TextFrame};
use hm_layout::{ParaLayout, Rect};
use hm_model::{
    Arrow, Control, CurveGeom, EllipseGeom, LineStyle, Matrix, Paragraph, Picture, Shape, ShapePart,
};

use crate::draw::{ImageItem, Outline, Path, PathItem};
use crate::image::Effects;
use crate::paint::{Dash, brushes};
use crate::text::Ctx;
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
/// paragraph, its box's top left (inside the outer margins) at `(x, y)`, on
/// a line whose baseline is at `baseline` (page coordinates).
pub(crate) fn paint_inline(
    dl: &mut DisplayList,
    ctx: &Ctx<'_>,
    para: &Paragraph,
    layout: &ParaLayout,
    control: u32,
    (x, y): (i32, i32),
    baseline: i32,
) {
    let Some(c) = para.controls.get(control as usize) else {
        return;
    };
    let inner = inner(layout, control);
    let (dx, dy) = inner.map_or((0, 0), ObjectLayout::object_offset);
    let Some(o) = c.object_common() else { return };
    let paper = ctx.page.map_or((0, 0), |p| (p.width, p.height));
    let o = hm_layout::objects::on_paper(&o, paper);
    let rect = Rect {
        x: x + dx,
        y: y + dy,
        w: o.width,
        h: o.height,
    };
    crate::record_object(
        ctx,
        c as *const Control as usize,
        (rect.x, rect.y, rect.w, rect.h),
    );
    match inner.and_then(|i| i.equation.as_ref()) {
        // The equation's baseline is the line's.
        Some(m) => paint_math(dl, ctx.fonts, m, rect.x, baseline),
        None => paint_object(dl, ctx, c, rect, inner),
    }
    paint_caption(dl, ctx, c, inner, rect);
}

/// Paints an object's caption beside its box.
fn paint_caption(
    dl: &mut DisplayList,
    ctx: &Ctx<'_>,
    c: &Control,
    inner: Option<&ObjectLayout>,
    rect: Rect,
) {
    let (Some(inner), Some(cap)) = (inner, c.as_object().and_then(|o| o.caption())) else {
        return;
    };
    let (Some((cx, cy)), Some(cl)) = (inner.caption_offset((rect.w, rect.h)), &inner.caption)
    else {
        return;
    };
    let mut placed = Vec::new();
    for l in &cl.lines {
        let (Some(para), Some(pal)) = (
            cap.paras.get(l.para as usize),
            cl.paras.get(l.para as usize),
        ) else {
            continue;
        };
        crate::text::draw_line(
            dl,
            ctx,
            para,
            pal,
            l.line as usize,
            rect.x + cx,
            rect.y + cy + l.y,
            &mut placed,
        );
    }
}

/// Paints a floating object at its placement.
pub(crate) fn paint_placement(
    dl: &mut DisplayList,
    ctx: &Ctx<'_>,
    para: &Paragraph,
    layout: &ParaLayout,
    p: &Placement,
) {
    let Some(c) = para.controls.get(p.control as usize) else {
        return;
    };
    let inner = inner(layout, p.control);
    let r = p.rect;
    crate::record_object(ctx, c as *const Control as usize, (r.x, r.y, r.w, r.h));
    match inner.and_then(|i| i.equation.as_ref()) {
        Some(m) => {
            let baseline = p.rect.y + p.rect.h.min(m.ascent).max(0);
            paint_math(dl, ctx.fonts, m, p.rect.x, baseline);
        }
        None => paint_object(dl, ctx, c, p.rect, inner),
    }
    paint_caption(dl, ctx, c, inner, p.rect);
}

/// Paints a drawing object into its box.
fn paint_object(
    dl: &mut DisplayList,
    ctx: &Ctx<'_>,
    c: &Control,
    rect: Rect,
    inner: Option<&ObjectLayout>,
) {
    if let Some(form) = c.as_form() {
        let text = inner.and_then(|i| i.form.as_ref());
        crate::forms::paint(dl, ctx, form, rect, text);
        return;
    }
    let Some(shape) = c.as_object().and_then(|o| o.shape()) else {
        return;
    };
    let origin = Matrix([1.0, 0.0, rect.x as f64, 0.0, 1.0, rect.y as f64]);
    // An OLE object fills the object's box; its own sizes are placeholders.
    if let Some(ShapePart::Ole(o)) = shape.kind() {
        ole(dl, ctx, &origin, o, (rect.w, rect.h));
        return;
    }
    let frames = inner.map_or(&[][..], |i| i.frames.as_slice());
    // A text box set as a character takes the height its text lays out
    // to from its set height, whatever height another layout saved it at.
    let origin = match (shape.text(), frames.first(), c.object_common()) {
        (Some(t), Some(f), Some(o)) if o.treat_as_char() && shape.children().next().is_none() => {
            let [_, _, tt, tb] = t.margins.unwrap_or([0; 4]).map(i32::from);
            let h = o.height.max(f.content + tt + tb);
            let saved = shape.component.size.1 as i32;
            if saved > 0 && h > 0 && h != saved {
                origin.mul(&Matrix([1.0, 0.0, 0.0, 0.0, h as f64 / saved as f64, 0.0]))
            } else {
                origin
            }
        }
        _ => origin,
    };
    // A shape set as a character whose box was resolved from the paper is
    // drawn from its stored size scaled to that box.
    let origin = match c.object_common() {
        Some(o)
            if o.treat_as_char()
                && shape.text().is_none()
                && o.width > 0
                && o.height > 0
                && (o.width, o.height) != (rect.w, rect.h) =>
        {
            let (sx, sy) = (
                rect.w as f64 / o.width as f64,
                rect.h as f64 / o.height as f64,
            );
            origin.mul(&Matrix([sx, 0.0, 0.0, 0.0, sy, 0.0]))
        }
        _ => origin,
    };
    let mut next = 0;
    paint_shape(dl, ctx, shape, (&origin, &origin, 0), frames, &mut next);
}

fn pt(m: &Matrix, p: (i32, i32)) -> (i32, i32) {
    let (x, y) = m.apply(p.0 as f64, p.1 as f64);
    (x.round() as i32, y.round() as i32)
}

/// Paints one shape and its group members; `parent` maps the shape's
/// parent coordinates to the page, `top` the top shape's parent
/// coordinates, and `parent_pairs` counts the parent's transform pairs.
fn paint_shape(
    dl: &mut DisplayList,
    ctx: &Ctx<'_>,
    shape: &Shape,
    (top, parent, parent_pairs): (&Matrix, &Matrix, usize),
    frames: &[TextFrame],
    next: &mut usize,
) {
    // A group member's matrix carries a (scale, rotation) pair for every
    // level above it, so it maps straight to the top shape's parent; one
    // with no more pairs than its group is local to the group.
    let pairs = shape.component.transforms.len();
    let m = if pairs > parent_pairs {
        top.mul(&shape.component.matrix())
    } else {
        parent.mul(&shape.component.matrix())
    };
    let style = shape.style();
    let mut path = Path::default();
    let mut closed = true;
    match shape.kind() {
        Some(ShapePart::Rect(r)) => rect_path(&mut path, &m, r.corners, r.round),
        Some(ShapePart::Ellipse(e)) => {
            closed = ellipse_path(&mut path, &m, e);
        }
        Some(ShapePart::Arc(a)) => {
            closed = arc_path(&mut path, &m, (a.center, a.axis1, a.axis2), a.attr & 0xFF);
        }
        Some(ShapePart::Line(l)) => {
            path.move_to(pt(&m, l.start));
            if let Some(c) = &l.connector {
                if l.attr.unwrap_or(0) >= 6 && c.points.len() >= 4 {
                    path.cubic_to(
                        pt(&m, (c.points[1].0, c.points[1].1)),
                        pt(&m, (c.points[2].0, c.points[2].1)),
                        pt(&m, l.end),
                    );
                } else {
                    // Routes include their cached endpoints. Use only the
                    // interior points: rounding the cached end past l.end
                    // would add a reverse segment and flip its arrowhead.
                    for &(x, y, _) in c
                        .points
                        .iter()
                        .skip(1)
                        .take(c.points.len().saturating_sub(2))
                    {
                        path.line_to(pt(&m, (x, y)));
                    }
                    path.line_to(pt(&m, l.end));
                }
            } else {
                path.line_to(pt(&m, l.end));
            }
            closed = false;
        }
        Some(ShapePart::Polygon(p)) => {
            for (k, &q) in p.points.iter().enumerate() {
                if k == 0 {
                    path.move_to(pt(&m, q));
                } else {
                    path.line_to(pt(&m, q));
                }
            }
            closed = p.points.len() > 2 && p.points.first() == p.points.last();
            if closed {
                path.close();
            }
        }
        Some(ShapePart::Curve(c)) => {
            closed = curve_path(&mut path, &m, c);
        }
        Some(ShapePart::Picture(p)) => picture(dl, ctx, &m, p, &shape.component),

        _ => {}
    }
    if !path.is_empty() {
        let fill = match style {
            Some(s) if closed => brushes(&s.fill),
            _ => Vec::new(),
        };
        let mut outline = style.and_then(|s| outline(&s.line));
        let fill_alpha = style.map_or(0, |s| s.fill.alpha.first().copied().unwrap_or(0));
        let ends = if closed { None } else { path_ends(&path) };
        // Double and triple lines: parallel strokes splitting the width.
        let bands = style
            .zip(outline)
            .map(|(s, o)| multi_line(&path, s.line.kind(), o))
            .unwrap_or_default();
        if !bands.is_empty() {
            outline = None;
        }
        // Hancom fills a closed shape out to the outer edge of its outline:
        // the fill reaches half the line's width past the path, under the
        // outline's outer half.
        if let (&[crate::paint::Brush::Solid(color)], Some(o), 0) =
            (fill.as_slice(), outline.as_ref(), fill_alpha)
        {
            dl.items.push(Item::Path(Box::new(PathItem {
                path: path.clone(),
                fill: Vec::new(),
                outline: Some(Outline {
                    color,
                    dash: Dash::Solid,
                    ..o.clone()
                }),
                fill_alpha: 0,
            })));
        }
        if !fill.is_empty() || outline.is_some() {
            dl.items.push(Item::Path(Box::new(PathItem {
                path,
                fill,
                outline,
                fill_alpha,
            })));
        }
        for item in bands {
            dl.items.push(Item::Path(Box::new(item)));
        }
        if let (Some(s), Some((start, end)), Some(_)) = (style, ends, outline) {
            let (head, head_filled) = s.line.head();
            let (tail, tail_filled) = s.line.tail();
            arrowhead(dl, &s.line, start, head, head_filled, s.line.head_size());
            arrowhead(dl, &s.line, end, tail, tail_filled, s.line.tail_size());
        }
    }
    if let Some(t) = shape.text() {
        if let Some(frame) = frames.get(*next) {
            paint_text(dl, ctx, shape, t, &m, frame);
        }
        *next += 1;
    }
    for child in shape.children() {
        paint_shape(dl, ctx, child, (top, &m, pairs), frames, next);
    }
}

/// Paints the text of a shape inside its box less the text margins,
/// aligned vertically.
fn paint_text(
    dl: &mut DisplayList,
    ctx: &Ctx<'_>,
    shape: &Shape,
    t: &hm_model::TextBox,
    m: &Matrix,
    frame: &TextFrame,
) {
    let (w, h) = (
        shape.component.initial_size.0 as i32,
        shape.component.initial_size.1 as i32,
    );
    let corners = [pt(m, (0, 0)), pt(m, (w, 0)), pt(m, (w, h)), pt(m, (0, h))];
    let x0 = corners.iter().map(|p| p.0).min().unwrap_or(0);
    let x1 = corners.iter().map(|p| p.0).max().unwrap_or(0);
    let y0 = corners.iter().map(|p| p.1).min().unwrap_or(0);
    let y1 = corners.iter().map(|p| p.1).max().unwrap_or(0);
    if t.vertical() {
        vertical_text(dl, ctx, t, frame, (x0, y0, x1, y1));
        return;
    }
    let [ml, _, mt, mb] = t.margins.unwrap_or([0; 4]).map(i32::from);
    let room = (y1 - y0) - mt - mb;
    // Text taller than the box starts at its top whatever the alignment.
    let offset = match t.vertical_align() {
        1 => (room - frame.content) / 2,
        2 => room - frame.content,
        _ => 0,
    }
    .max(0);
    let mut placed = Vec::new();
    // Hancom clips the text to the shape's box, reaching two of its 1/600
    // inch drawing units (24 HU) below the box.
    let clip = (x0, y0, x1 - x0, y1 - y0 + 24);
    crate::clipped(dl, clip, |dl| {
        for l in &frame.lines {
            let (Some(para), Some(pal)) = (
                frame
                    .borrowed
                    .as_deref()
                    .unwrap_or(&t.paras)
                    .get(l.para as usize),
                frame.paras.get(l.para as usize),
            ) else {
                continue;
            };
            crate::text::draw_line(
                dl,
                ctx,
                para,
                pal,
                l.line as usize,
                x0 + ml,
                y0 + mt + offset + l.y,
                &mut placed,
            );
        }
        text_box_objects(dl, ctx, t, frame, (x0 + ml, y0 + mt + offset));
    });
}

/// Paints vertical text: the lines laid out across the box run down it as
/// columns from the right edge, glyphs upright, and the box's alignment
/// places the columns across it. `(x0, y0, x1, y1)` is the box on the page.
fn vertical_text(
    dl: &mut DisplayList,
    ctx: &Ctx<'_>,
    t: &hm_model::TextBox,
    frame: &TextFrame,
    (x0, y0, x1, y1): (i32, i32, i32, i32),
) {
    let [ml, mr, mt, _] = t.margins.unwrap_or([0; 4]).map(i32::from);
    let room = (x1 - x0) - ml - mr;
    let content = frame.content;
    // Text taller than the box starts at its top whatever the alignment.
    let offset = match t.vertical_align() {
        1 => (room - content) / 2,
        2 => room - content,
        _ => 0,
    }
    .max(0);
    // Each line drawn across, then turned: a point `(u, v)` along and
    // across the lines lands at `(right - v, top + u)`.
    let (right, top) = (x1 - mr - offset, y0 + mt);
    // Every character upright (text direction 2) takes a full-width cell
    // down the column, blanks keeping their width; otherwise characters
    // keep their place along the line.
    let em_cells = t.attr & 7 == 2;
    let clip = (x0, y0, x1 - x0, y1 - y0 + 24);
    let mut placed = Vec::new();
    crate::clipped(dl, clip, |dl| {
        for l in &frame.lines {
            let (Some(para), Some(pal)) = (
                frame
                    .borrowed
                    .as_deref()
                    .unwrap_or(&t.paras)
                    .get(l.para as usize),
                frame.paras.get(l.para as usize),
            ) else {
                continue;
            };
            let Some(line) = pal.lines.get(l.line as usize) else {
                continue;
            };
            let mut flat = DisplayList::default();
            crate::text::draw_line(
                &mut flat,
                ctx,
                para,
                pal,
                l.line as usize,
                0,
                l.y,
                &mut placed,
            );
            // The line's characters in order with their cell and advance:
            // glyphs come out in the same order.
            let m = &pal.measured;
            // Each character with the characters and the blank width before
            // it on the line, and its own advance.
            let items = &m.items[line.first_item as usize..line.end_item as usize];
            let adv = |pos: u32| m.advances.get(pos as usize).copied().unwrap_or(0);
            // An upright character takes the cell of the widest full-width
            // character on the line, or its own advance when wider.
            let cell = items
                .iter()
                .filter_map(|&(pos, kind)| match kind {
                    hm_layout::measure::Kind::Char(c) if c as u32 >= 0x1100 => Some(adv(pos)),
                    _ => None,
                })
                .max()
                .unwrap_or(0);
            // Each character with the room narrower characters before it
            // gained by taking a whole cell.
            let mut cells = Vec::new();
            let mut extra = 0;
            for &(pos, kind) in items {
                if let hm_layout::measure::Kind::Char(_) = kind {
                    cells.push((extra, adv(pos)));
                    extra += (cell - adv(pos)).max(0);
                }
            }
            let mut glyphs: Vec<(i32, i32, u16, usize)> = Vec::new();
            let mut runs: Vec<GlyphRun> = Vec::new();
            for item in flat.items {
                match item {
                    Item::Glyphs(run) => {
                        for &(g, u) in &run.glyphs {
                            glyphs.push((u, run.baseline, g, runs.len()));
                        }
                        runs.push(run);
                    }
                    Item::Rect { x, y, w, h, color } => dl.items.push(Item::Rect {
                        x: right - y - h,
                        y: top + x,
                        w: h,
                        h: w,
                        color,
                    }),
                    _ => {}
                }
            }
            glyphs.sort_by_key(|g| g.0);
            for (k, &(u, _, g, r)) in glyphs.iter().enumerate() {
                let run = &runs[r];
                let (at, advance) = match cells.get(k) {
                    Some(&(extra, adv)) if em_cells => (u + extra, adv),
                    Some(&(_, adv)) => (u, adv),
                    None => (u, run.size * run.ratio / 100),
                };
                // An upright glyph sits on the column's middle (the middle of
                // the line laid across), its em box from its place along the
                // line.
                let mid = l.y + line.height / 2;
                dl.items.push(Item::Glyphs(GlyphRun {
                    baseline: top + at + run.size * 85 / 100,
                    glyphs: vec![(g, right - mid - advance / 2)],
                    ..run.clone()
                }));
            }
        }
    });
}

/// Floating objects anchored in a text box's paragraphs, placed with the
/// text area as their column; `(x, y)` is the page position of its top left.
fn text_box_objects(
    dl: &mut DisplayList,
    ctx: &Ctx<'_>,
    t: &hm_model::TextBox,
    frame: &TextFrame,
    (x, y): (i32, i32),
) {
    let Some(page) = ctx.page else { return };
    let paras = frame.borrowed.as_deref().unwrap_or(&t.paras);
    for (pi, (para, pal)) in paras.iter().zip(&frame.paras).enumerate() {
        if !para.controls.iter().any(|c| {
            c.as_object()
                .is_some_and(|o| !o.header.common.treat_as_char())
        }) {
            continue;
        }
        let Some(first) = frame.lines.iter().find(|l| l.para == pi as u32) else {
            continue;
        };
        let left = pal.lines.first().map_or(0, |l| l.seg_x);
        let anchor = hm_layout::objects::Anchor {
            paper: (page.width, page.height),
            paper_x: None,
            paper_y: None,
            flow_top: None,
            body: page.body,
            column: (x, frame.width),
            // Objects hang from the paragraph's top above its space before.
            para_top: y + first.y - pal.before,
            para_left: x + left,
            para_right: pal.right,
        };
        let mut list = hm_layout::objects::place(para, pi as u32, &anchor);
        list.sort_by_key(|p| (p.layer, p.z));
        for p in &list {
            paint_placement(dl, ctx, para, pal, p);
        }
    }
}

/// The outline of a drawing object, when it draws one. Drawing objects
/// count line kinds from 1 for solid.
fn outline(l: &LineStyle) -> Option<Outline> {
    let dash = match l.kind() {
        0 => return None,
        2 => Dash::LongDash,
        3 => Dash::Dot,
        4 => Dash::DashDot,
        5 => Dash::DashDotDot,
        6 => Dash::Dash,
        7 => Dash::Circle,
        _ => Dash::Solid,
    };
    Some(Outline {
        // Hancom rounds drawing strokes to 600-dpi units, with one unit
        // for a hairline. Quantize before any display backend scales it.
        width: hm_model::units::mul_div(l.width, 1, 12)
            .max(1)
            .saturating_mul(12),
        color: l.color & 0x00FF_FFFF,
        dash,
        round_cap: l.cap() == 0,
    })
}

/// An open path's end points with the direction pointing out of the path
/// at each: `((point, direction), (point, direction))`.
#[allow(clippy::type_complexity)]
fn path_ends(path: &Path) -> Option<(((f64, f64), (f64, f64)), ((f64, f64), (f64, f64)))> {
    use crate::draw::PathEl;
    let mut pts: Vec<(f64, f64)> = Vec::new();
    for e in &path.els {
        match *e {
            PathEl::Move(x, y) | PathEl::Line(x, y) => pts.push((x as f64, y as f64)),
            PathEl::Cubic(a, b, c, d, x, y) => {
                pts.push((a as f64, b as f64));
                pts.push((c as f64, d as f64));
                pts.push((x as f64, y as f64));
            }
            PathEl::Close => {}
        }
    }
    let first = *pts.first()?;
    let last = *pts.last()?;
    // The nearest distinct points give the directions.
    let after = pts.iter().find(|p| **p != first)?;
    let before = pts.iter().rev().find(|p| **p != last)?;
    let dir = |from: (f64, f64), to: (f64, f64)| {
        let (dx, dy) = (to.0 - from.0, to.1 - from.1);
        let l = dx.hypot(dy).max(1e-9);
        (dx / l, dy / l)
    };
    Some(((first, dir(*after, first)), (last, dir(*before, last))))
}

/// An arrowhead at an end of a line pointing along `dir`. Hancom draws
/// the smallest class about 576 HU long and wide whatever the line width,
/// larger classes and thick lines bigger.
fn arrowhead(
    dl: &mut DisplayList,
    line: &LineStyle,
    ((x, y), (dx, dy)): ((f64, f64), (f64, f64)),
    kind: Arrow,
    filled: bool,
    size: u32,
) {
    if matches!(kind, Arrow::None) {
        return;
    }
    let base = 576f64.max(4.0 * line.width as f64);
    let class = |c: u32| [1.0, 1.5, 2.0][c.min(2) as usize];
    let (w, l) = (base * class(size / 3), base * class(size % 3));
    // Along the line (u) and across it (v).
    let at = |u: f64, v: f64| {
        (
            (x + dx * u - dy * v).round() as i32,
            (y + dy * u + dx * v).round() as i32,
        )
    };
    let mut path = Path::default();
    let mut solid = filled;
    match kind {
        Arrow::Arrow => {
            path.move_to(at(0.0, 0.0));
            path.line_to(at(-l, -w / 2.0));
            path.line_to(at(-l, w / 2.0));
            path.close();
            solid = true;
        }
        Arrow::Spear => {
            path.move_to(at(0.0, 0.0));
            path.line_to(at(-l, -w / 2.0));
            path.line_to(at(-l * 0.75, 0.0));
            path.line_to(at(-l, w / 2.0));
            path.close();
            solid = true;
        }
        Arrow::ConcaveArrow => {
            path.move_to(at(0.0, 0.0));
            path.line_to(at(-l, -w / 2.0));
            path.line_to(at(-l * 0.6, 0.0));
            path.line_to(at(-l, w / 2.0));
            path.close();
            solid = true;
        }
        Arrow::Diamond => {
            path.move_to(at(l / 2.0, 0.0));
            path.line_to(at(0.0, -w / 2.0));
            path.line_to(at(-l / 2.0, 0.0));
            path.line_to(at(0.0, w / 2.0));
            path.close();
        }
        Arrow::Square => {
            path.move_to(at(l / 2.0, -w / 2.0));
            path.line_to(at(l / 2.0, w / 2.0));
            path.line_to(at(-l / 2.0, w / 2.0));
            path.line_to(at(-l / 2.0, -w / 2.0));
            path.close();
        }
        Arrow::Circle => {
            const K: f64 = 0.552_284_8;
            let (a, b) = (l / 2.0, w / 2.0);
            path.move_to(at(a, 0.0));
            path.cubic_to(at(a, b * K), at(a * K, b), at(0.0, b));
            path.cubic_to(at(-a * K, b), at(-a, b * K), at(-a, 0.0));
            path.cubic_to(at(-a, -b * K), at(-a * K, -b), at(0.0, -b));
            path.cubic_to(at(a * K, -b), at(a, -b * K), at(a, 0.0));
            path.close();
        }
        Arrow::None => return,
    }
    let color = line.color & 0x00FF_FFFF;
    dl.items.push(Item::Path(Box::new(PathItem {
        path,
        fill: if solid {
            vec![crate::paint::Brush::Solid(color)]
        } else {
            vec![crate::paint::Brush::Solid(0x00FF_FFFF)]
        },
        outline: Some(Outline {
            width: line.width.max(0),
            color,
            dash: Dash::Solid,
            round_cap: false,
        }),
        fill_alpha: 0,
    })));
}

/// The parallel strokes of a multi-line outline kind (double, thin-thick,
/// thick-thin, thin-thick-thin, double wave) along a polygonal path, each
/// band offset from the path toward the right of travel and the bands
/// together as wide as the line. Empty for single lines and curved paths.
fn multi_line(path: &Path, kind: u32, o: Outline) -> Vec<PathItem> {
    use crate::draw::PathEl;
    let parts: &[i32] = match kind {
        8 | 13 => &[1, -1, 1],
        9 => &[1, -1, 2],
        10 => &[2, -1, 1],
        11 => &[1, -1, 2, -1, 1],
        _ => return Vec::new(),
    };
    // Polylines only, split into subpaths.
    let mut subs: Vec<(Vec<(f64, f64)>, bool)> = Vec::new();
    for e in &path.els {
        match *e {
            PathEl::Move(x, y) => subs.push((vec![(x as f64, y as f64)], false)),
            PathEl::Line(x, y) => match subs.last_mut() {
                Some(s) => s.0.push((x as f64, y as f64)),
                None => return Vec::new(),
            },
            PathEl::Close => {
                if let Some(s) = subs.last_mut() {
                    s.1 = true;
                }
            }
            PathEl::Cubic(..) => return Vec::new(),
        }
    }
    // A hairline still shows two strokes.
    let w = (o.width as f64).max(30.0);
    let total: i32 = parts.iter().map(|p| p.abs()).sum();
    let mut out = Vec::new();
    let mut at = 0.0;
    for &p in parts {
        let span = p.abs() as f64 * w / total as f64;
        if p > 0 {
            let off = at + span / 2.0 - w / 2.0;
            let mut band = Path::default();
            for (pts, closed) in &subs {
                let moved = offset_polyline(pts, *closed, off);
                for (k, q) in moved.iter().enumerate() {
                    let q = (q.0.round() as i32, q.1.round() as i32);
                    if k == 0 {
                        band.move_to(q);
                    } else {
                        band.line_to(q);
                    }
                }
                if *closed {
                    band.close();
                }
            }
            out.push(PathItem {
                path: band,
                fill: Vec::new(),
                outline: Some(Outline {
                    width: span.round().max(1.0) as i32,
                    ..o
                }),
                fill_alpha: 0,
            });
        }
        at += span;
    }
    out
}

/// A polyline moved `d` to the right of its direction of travel, corners
/// mitered.
fn offset_polyline(pts: &[(f64, f64)], closed: bool, d: f64) -> Vec<(f64, f64)> {
    let mut pts: Vec<(f64, f64)> = pts.to_vec();
    pts.dedup();
    if closed && pts.len() > 2 && pts.first() == pts.last() {
        pts.pop();
    }
    let n = pts.len();
    if n < 2 {
        return pts;
    }
    let normal = |a: (f64, f64), b: (f64, f64)| {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let l = dx.hypot(dy).max(1e-9);
        (-dy / l, dx / l)
    };
    (0..n)
        .map(|i| {
            let prev = if i > 0 {
                Some(pts[i - 1])
            } else if closed {
                Some(pts[n - 1])
            } else {
                None
            };
            let next = if i + 1 < n {
                Some(pts[i + 1])
            } else if closed {
                Some(pts[0])
            } else {
                None
            };
            let p = pts[i];
            match (prev, next) {
                (Some(a), Some(b)) => {
                    let (n1, n2) = (normal(a, p), normal(p, b));
                    let (mx, my) = (n1.0 + n2.0, n1.1 + n2.1);
                    let dot = 1.0 + n1.0 * n2.0 + n1.1 * n2.1;
                    if dot < 0.1 {
                        (p.0 + n1.0 * d, p.1 + n1.1 * d)
                    } else {
                        (p.0 + mx * d / dot, p.1 + my * d / dot)
                    }
                }
                (None, Some(b)) => {
                    let n1 = normal(p, b);
                    (p.0 + n1.0 * d, p.1 + n1.1 * d)
                }
                (Some(a), None) => {
                    let n1 = normal(a, p);
                    (p.0 + n1.0 * d, p.1 + n1.1 * d)
                }
                (None, None) => p,
            }
        })
        .collect()
}

/// A rectangle with corners rounded by `round` percent of its shorter side
/// as drawn: Hancom takes the radius from the shape's size on the page, the
/// same along both sides, whatever the shape's own scale (36388711 p1: a
/// rectangle of 8504 x 8504 scaled 4.6 x 0.36 has corners of 3 % of its
/// drawn height, not quarter ellipses of 3 % of 8504 stretched).
fn rect_path(path: &mut Path, m: &Matrix, c: [(i32, i32); 4], round: u8) {
    let (x0, y0) = c[0];
    let (x1, y1) = c[2];
    let [a, b, _, d, e, _] = m.0;
    // Page lengths of a unit along the shape's own axes.
    let (sx, sy) = (a.hypot(d), b.hypot(e));
    let w = f64::from((x1 - x0).abs()) * sx;
    let h = f64::from((y1 - y0).abs()) * sy;
    let r = w.min(h) * f64::from(round.min(50)) / 100.0;
    if r < 1.0 || sx <= 0.0 || sy <= 0.0 {
        path.move_to(pt(m, c[0]));
        for &q in &c[1..] {
            path.line_to(pt(m, q));
        }
        path.close();
        return;
    }
    let (rx, ry) = ((r / sx).round() as i32, (r / sy).round() as i32);
    let kx = (f64::from(rx) * 0.552_284_75).round() as i32;
    let ky = (f64::from(ry) * 0.552_284_75).round() as i32;
    let (l, t, rt, b) = (x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1));
    path.move_to(pt(m, (l + rx, t)));
    path.line_to(pt(m, (rt - rx, t)));
    path.cubic_to(
        pt(m, (rt - rx + kx, t)),
        pt(m, (rt, t + ry - ky)),
        pt(m, (rt, t + ry)),
    );
    path.line_to(pt(m, (rt, b - ry)));
    path.cubic_to(
        pt(m, (rt, b - ry + ky)),
        pt(m, (rt - rx + kx, b)),
        pt(m, (rt - rx, b)),
    );
    path.line_to(pt(m, (l + rx, b)));
    path.cubic_to(
        pt(m, (l + rx - kx, b)),
        pt(m, (l, b - ry + ky)),
        pt(m, (l, b - ry)),
    );
    path.line_to(pt(m, (l, t + ry)));
    path.cubic_to(
        pt(m, (l, t + ry - ky)),
        pt(m, (l + rx - kx, t)),
        pt(m, (l + rx, t)),
    );
    path.close();
}

/// An ellipse given by its center and two conjugate semi-axis vectors.
#[derive(Clone, Copy)]
struct Ellipse {
    c: (f64, f64),
    u: (f64, f64),
    v: (f64, f64),
}

impl Ellipse {
    fn new(center: (i32, i32), a1: (i32, i32), a2: (i32, i32)) -> Ellipse {
        let c = (center.0 as f64, center.1 as f64);
        Ellipse {
            c,
            u: (a1.0 as f64 - c.0, a1.1 as f64 - c.1),
            v: (a2.0 as f64 - c.0, a2.1 as f64 - c.1),
        }
    }

    fn at(&self, t: f64) -> (f64, f64) {
        (
            self.c.0 + self.u.0 * t.cos() + self.v.0 * t.sin(),
            self.c.1 + self.u.1 * t.cos() + self.v.1 * t.sin(),
        )
    }

    fn tangent(&self, t: f64) -> (f64, f64) {
        (
            -self.u.0 * t.sin() + self.v.0 * t.cos(),
            -self.u.1 * t.sin() + self.v.1 * t.cos(),
        )
    }

    /// The parameter of point `p`.
    fn param(&self, p: (i32, i32)) -> f64 {
        let (px, py) = (p.0 as f64 - self.c.0, p.1 as f64 - self.c.1);
        let (u, v) = (self.u, self.v);
        let det = u.0 * v.1 - u.1 * v.0;
        if det.abs() < 1e-9 {
            return 0.0;
        }
        let cos = (px * v.1 - py * v.0) / det;
        let sin = (u.0 * py - u.1 * px) / det;
        sin.atan2(cos)
    }

    /// Appends the arc from parameter `a0` to `a1` as cubic pieces.
    fn arc(&self, path: &mut Path, m: &Matrix, (a0, a1): (f64, f64), start: bool) {
        let p = |q: (f64, f64)| pt(m, (q.0.round() as i32, q.1.round() as i32));
        let n = (((a1 - a0).abs() / std::f64::consts::FRAC_PI_2).ceil() as usize).max(1);
        let step = (a1 - a0) / n as f64;
        let k = 4.0 / 3.0 * (step / 4.0).tan();
        if start {
            path.move_to(p(self.at(a0)));
        } else {
            path.line_to(p(self.at(a0)));
        }
        for i in 0..n {
            let (t0, t1) = (a0 + step * i as f64, a0 + step * (i + 1) as f64);
            let (p0, p1) = (self.at(t0), self.at(t1));
            let (d0, d1) = (self.tangent(t0), self.tangent(t1));
            path.cubic_to(
                p((p0.0 + k * d0.0, p0.1 + k * d0.1)),
                p((p1.0 - k * d1.0, p1.1 - k * d1.1)),
                p(p1),
            );
        }
    }

    /// An arc closed as a pie (1), a chord (2) or left open (0). Returns
    /// whether the path closes.
    fn finish(&self, path: &mut Path, m: &Matrix, span: (f64, f64), kind: u32) -> bool {
        match kind {
            1 => {
                path.move_to(pt(m, (self.c.0.round() as i32, self.c.1.round() as i32)));
                self.arc(path, m, span, false);
                path.close();
                true
            }
            2 => {
                self.arc(path, m, span, true);
                path.close();
                true
            }
            _ => {
                self.arc(path, m, span, true);
                false
            }
        }
    }
}

/// An ellipse, or the arc it was turned into. Returns whether it closes.
fn ellipse_path(path: &mut Path, m: &Matrix, e: &EllipseGeom) -> bool {
    let el = Ellipse::new(e.center, e.axis1, e.axis2);
    if !e.is_arc() {
        el.arc(path, m, (0.0, std::f64::consts::TAU), true);
        path.close();
        return true;
    }
    let a0 = el.param(e.start);
    let mut a1 = el.param(e.end);
    if a1 <= a0 {
        a1 += std::f64::consts::TAU;
    }
    el.finish(path, m, (a0, a1), e.arc_kind())
}

/// An arc object: the quarter of the ellipse from the first axis end to
/// the second.
fn arc_path(
    path: &mut Path,
    m: &Matrix,
    (center, a1, a2): ((i32, i32), (i32, i32), (i32, i32)),
    kind: u32,
) -> bool {
    Ellipse::new(center, a1, a2).finish(path, m, (0.0, std::f64::consts::FRAC_PI_2), kind)
}

/// A curve: straight segments, and cubic segments whose two control points
/// follow their start point. Returns whether it closes.
fn curve_path(path: &mut Path, m: &Matrix, c: &CurveGeom) -> bool {
    let pts = &c.points;
    if pts.is_empty() {
        return false;
    }
    path.move_to(pt(m, pts[0]));
    let mut i = 0;
    while i + 1 < pts.len() {
        if c.segments.get(i) == Some(&1) && i + 3 < pts.len() {
            path.cubic_to(pt(m, pts[i + 1]), pt(m, pts[i + 2]), pt(m, pts[i + 3]));
            i += 3;
        } else {
            path.line_to(pt(m, pts[i + 1]));
            i += 1;
        }
    }
    let closed = pts.len() > 2 && pts.first() == pts.last();
    if closed {
        path.close();
    }
    closed
}

#[cfg(test)]
mod rect_corner_tests {
    use super::*;
    use crate::draw::PathEl;

    /// 36388711 p1 (Hancom PDF outline): a rectangle of 8504 x 8504 with
    /// 4 % round corners, scaled to 39699 x 3685, is outlined with corners
    /// of radius 147 along both sides (4 % of its drawn height).
    #[test]
    fn scaled_round_corners_take_their_radius_from_the_drawn_size() {
        let m = Matrix([4.668274, 0.0, 0.0, 0.0, 0.433325, 0.0]);
        let mut path = Path::default();
        rect_path(
            &mut path,
            &m,
            [(0, 0), (8504, 0), (8504, 8504), (0, 8504)],
            4,
        );
        let PathEl::Move(x, y) = path.els[0] else {
            panic!("{:?}", path.els[0])
        };
        assert_eq!(y, 0);
        assert!((x - 147).abs() <= 3, "corner starts {x} along the top");
        let Some(&PathEl::Cubic(_, _, _, _, ex, ey)) = path.els.get(2) else {
            panic!("{:?}", path.els)
        };
        assert_eq!(ex, 39699);
        assert!((ey - 147).abs() <= 3, "corner ends {ey} down the side");
    }
}

#[cfg(test)]
mod connector_path_tests {
    use super::*;
    use hm_model::{
        Connector, DocMeta, Document, Format, LineGeom, PolyGeom, ShapeComponent, ShapeStyle,
    };

    fn pixels(part: ShapePart) -> Vec<u8> {
        let fonts = hm_fonts::FontSystem::new();
        let doc = Document {
            meta: DocMeta {
                format: Format::Hwp5,
                version: Default::default(),
                flags: 0,
            },
            info: Default::default(),
            sections: Vec::new(),
            source: None,
            bins: None,
            diagnostics: Vec::new(),
        };
        let ctx = Ctx {
            doc: &doc,
            styles: &[],
            fonts: &fonts,
            page: None,
            body_anchor: None,
            lines: None,
            cells: None,
            objects: None,
            index_only: false,
        };
        let shape = Shape {
            component: ShapeComponent {
                kind: hm_model::CtrlId::from_chars(b"$col"),
                doubled: false,
                offset: (0, 0),
                group_level: 0,
                version: 1,
                initial_size: (10000, 10000),
                size: (10000, 10000),
                flip: 0,
                rotation: 0,
                rotation_center: (0, 0),
                translation: Matrix::IDENTITY,
                transforms: Vec::new(),
                members: None,
                group_instance: None,
                tail: Default::default(),
                style: Some(ShapeStyle {
                    line: LineStyle {
                        color: 0,
                        width: 100,
                        attr: 1 | (1 << 6) | (1 << 16) | (4 << 26) | (1 << 31),
                        outline: None,
                    },
                    ..Default::default()
                }),
            },
            parts: vec![part],
        };
        let mut dl = DisplayList {
            width: 10000,
            height: 10000,
            items: Vec::new(),
        };
        paint_shape(
            &mut dl,
            &ctx,
            &shape,
            (&Matrix::IDENTITY, &Matrix::IDENTITY, 0),
            &[],
            &mut 0,
        );
        crate::raster::render(&dl, &fonts, 144.0)
            .unwrap()
            .data()
            .to_vec()
    }

    /// A closed shape's solid fill reaches the outer half of its outline, as
    /// Hancom's PDFs fill a rectangle half the line width past its stroked
    /// path (36388711 title box): the pixel just outside the path takes the
    /// fill's color under a thin line.
    #[test]
    fn solid_fill_reaches_under_the_outer_half_of_the_outline() {
        let fonts = hm_fonts::FontSystem::new();
        let doc = Document {
            meta: DocMeta {
                format: Format::Hwp5,
                version: Default::default(),
                flags: 0,
            },
            info: Default::default(),
            sections: Vec::new(),
            source: None,
            bins: None,
            diagnostics: Vec::new(),
        };
        let ctx = Ctx {
            doc: &doc,
            styles: &[],
            fonts: &fonts,
            page: None,
            body_anchor: None,
            lines: None,
            cells: None,
            objects: None,
            index_only: false,
        };
        // 36 HU per pixel at 200 dpi: the path's top edge lies on a pixel
        // boundary and the 24 HU line covers a third of the row above it.
        let pixel_above_top = |background: u32| {
            let shape = Shape {
                component: ShapeComponent {
                    kind: hm_model::CtrlId::from_chars(b"$pol"),
                    doubled: false,
                    offset: (0, 0),
                    group_level: 0,
                    version: 1,
                    initial_size: (10000, 10000),
                    size: (10000, 10000),
                    flip: 0,
                    rotation: 0,
                    rotation_center: (0, 0),
                    translation: Matrix::IDENTITY,
                    transforms: Vec::new(),
                    members: None,
                    group_instance: None,
                    tail: Default::default(),
                    style: Some(ShapeStyle {
                        line: LineStyle {
                            color: 0x00B4_B4B4,
                            width: 24,
                            attr: 1,
                            outline: None,
                        },
                        fill: hm_model::Fill {
                            kind: hm_model::Fill::SOLID,
                            solid: Some(hm_model::SolidFill {
                                background,
                                pattern_color: 0,
                                pattern: 0,
                            }),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                },
                parts: vec![ShapePart::Polygon(PolyGeom {
                    points: vec![
                        (1800, 1800),
                        (7200, 1800),
                        (7200, 7200),
                        (1800, 7200),
                        (1800, 1800),
                    ],
                    tail: Default::default(),
                })],
            };
            let mut dl = DisplayList {
                width: 10000,
                height: 10000,
                items: Vec::new(),
            };
            paint_shape(
                &mut dl,
                &ctx,
                &shape,
                (&Matrix::IDENTITY, &Matrix::IDENTITY, 0),
                &[],
                &mut 0,
            );
            let pixmap = crate::raster::render(&dl, &fonts, 200.0).unwrap();
            pixmap.pixel(100, 49).unwrap().green()
        };
        assert!(pixel_above_top(0x00D4_D4D4) < pixel_above_top(0x00FF_FFFF));
    }

    #[test]
    fn cached_connector_end_rounding_does_not_reverse_arrows_or_remove_bends() {
        for route in [
            vec![(2000, 1000), (2000, 8000)],
            vec![(2000, 8000), (2000, 1000)],
            vec![(2000, 1000), (6000, 1000), (6000, 8000)],
        ] {
            let reference = pixels(ShapePart::Polygon(PolyGeom {
                points: route.clone(),
                tail: Default::default(),
            }));
            for rounding in [-1, 0, 1] {
                let mut points: Vec<_> = route
                    .iter()
                    .enumerate()
                    .map(|(i, &(x, y))| {
                        (
                            x,
                            y,
                            if i == 0 {
                                3
                            } else if i + 1 == route.len() {
                                26
                            } else {
                                1
                            },
                        )
                    })
                    .collect();
                points.last_mut().unwrap().1 += rounding;
                let actual = pixels(ShapePart::Line(LineGeom {
                    start: route[0],
                    end: *route.last().unwrap(),
                    attr: Some(if route.len() == 2 { 1 } else { 4 }),
                    connector: Some(Connector {
                        points,
                        ..Default::default()
                    }),
                    ..Default::default()
                }));
                assert!(
                    actual == reference,
                    "rounded cached endpoint {rounding} changed the authored route/arrow: {route:?}"
                );
            }
        }
    }
}

/// A picture: its visible part stretched over the image rectangle, then
/// through the shape's matrix; the border on top.
fn picture(
    dl: &mut DisplayList,
    ctx: &Ctx<'_>,
    m: &Matrix,
    p: &Picture,
    component: &hm_model::ShapeComponent,
) {
    let [c0, c1, _, c3] = p.corners;
    let o = pt(m, c0);
    let xv = pt(m, c1);
    let yv = pt(m, c3);
    let mut frame = [xv.0 - o.0, yv.0 - o.0, o.0, xv.1 - o.1, yv.1 - o.1, o.1];
    // Negative current dimensions reverse the geometry, while the picture
    // content follows its explicit flips. Keep the border's original matrix.
    for (axis, reverse) in [
        (
            0,
            (component.size.0 as i32) < 0 && !component.flip_horizontal(),
        ),
        (
            1,
            (component.size.1 as i32) < 0 && !component.flip_vertical(),
        ),
    ] {
        if reverse {
            frame[2] += frame[axis];
            frame[5] += frame[axis + 3];
            frame[axis] = -frame[axis];
            frame[axis + 3] = -frame[axis + 3];
        }
    }
    // A picture's inner margins lie between its border and its image: the
    // image keeps its size inside the left and top margins, and the border
    // encloses the margins too.
    let [ml, mr, mt, mb] = p.margins.map(i32::from);
    let upright = frame[1] == 0 && frame[3] == 0 && frame[0] > 0 && frame[4] > 0;
    let margined = upright && (ml | mr | mt | mb) != 0;
    let border_box = margined.then(|| {
        let (x, y) = (frame[2], frame[5]);
        let (r, b) = (x + frame[0] + ml + mr, y + frame[4] + mt + mb);
        [(x, y), (r, y), (r, b), (x, b)]
    });
    if margined {
        frame[2] += ml;
        frame[5] += mt;
    }
    let fx = effects_around(dl, p, frame);
    if let Some(data) = ctx.doc.bins.as_ref().and_then(|b| b.get(p.image.bin_id)) {
        dl.items.push(Item::Image(Box::new(ImageItem {
            data,
            frame,
            crop: p.crop,
            effects: Effects {
                effect: p.image.effect,
                brightness: p.image.brightness,
                contrast: p.image.contrast,
                // Hancom's PDF output draws pictures opaque whatever their
                // transparency.
                alpha: 0,
                soft: fx,
            },
            natural: p.original_size().unwrap_or((0, 0)),
        })));
    }
    // A visible border of width 0 is Hancom's one-unit hairline.
    if let Some(ol) = outline(&p.border) {
        let mut path = Path::default();
        match border_box {
            Some(c) => rect_path(&mut path, &Matrix::IDENTITY, c, 0),
            None => rect_path(&mut path, m, p.corners, 0),
        }
        dl.items.push(Item::Path(Box::new(PathItem {
            path,
            fill: Vec::new(),
            outline: Some(ol),
            fill_alpha: 0,
        })));
    }
}

/// Paints a picture's shadow and glow under its frame `[a, b, c, d, e, f]`
/// (unit square to page) and returns its soft edge as thousandths of the
/// frame's width and height.
fn effects_around(dl: &mut DisplayList, p: &Picture, f: [i32; 6]) -> (u16, u16) {
    let fx = p.effect_list();
    let w = (f[0] as f64).hypot(f[3] as f64).max(1.0);
    let h = (f[1] as f64).hypot(f[4] as f64).max(1.0);
    // The frame grown by `e` HU on every side, moved by `(dx, dy)`.
    let quad = |e: f64, dx: f64, dy: f64| {
        let (ux, uy) = (f[0] as f64 / w, f[3] as f64 / w);
        let (vx, vy) = (f[1] as f64 / h, f[4] as f64 / h);
        let at = |s: f64, t: f64| {
            (
                (f[2] as f64 + ux * s + vx * t + dx).round() as i32,
                (f[5] as f64 + uy * s + vy * t + dy).round() as i32,
            )
        };
        let mut path = Path::default();
        path.move_to(at(-e, -e));
        path.line_to(at(w + e, -e));
        path.line_to(at(w + e, h + e));
        path.line_to(at(-e, h + e));
        path.close();
        path
    };
    // A soft-edged rectangle as stacked layers of partial opacity.
    let mut layers = |color: u32, opacity: f64, spread: (f64, f64), shift: (f64, f64)| {
        const N: usize = 8;
        let a = (opacity.clamp(0.0, 1.0) / N as f64 * 255.0).round() as u8;
        if a == 0 {
            return;
        }
        for k in 0..N {
            let e = spread.0 + (spread.1 - spread.0) * k as f64 / (N - 1) as f64;
            dl.items.push(Item::Path(Box::new(PathItem {
                path: quad(e, shift.0, shift.1),
                fill: vec![crate::paint::Brush::Solid(color)],
                outline: None,
                fill_alpha: 255 - a,
            })));
        }
    };
    if let Some(g) = fx.glow {
        let r = g.radius as f64;
        layers(g.color, 1.0 - g.transparency as f64, (0.0, r), (0.0, 0.0));
    }
    if let Some(s) = fx.shadow {
        let t = (s.direction as f64).to_radians();
        let d = s.distance as f64;
        let b = s.blur as f64;
        layers(
            s.color,
            1.0 - s.transparency as f64,
            (-b / 2.0, b / 2.0),
            (d * t.cos(), d * t.sin()),
        );
    }
    fx.soft_edge.map_or((0, 0), |r| {
        let r = r as f64;
        (
            (r * 1000.0 / w).round().clamp(0.0, 500.0) as u16,
            (r * 1000.0 / h).round().clamp(0.0, 500.0) as u16,
        )
    })
}

/// An OLE object: its saved presentation picture stretched over its box,
/// or its chart drawn there.
fn ole(dl: &mut DisplayList, ctx: &Ctx<'_>, m: &Matrix, o: &hm_model::Ole, (w, h): (i32, i32)) {
    let Some(view) = ctx.doc.bins.as_ref().and_then(|b| b.ole(o.bin_id)) else {
        // Hancom for Mac shows an equation object it cannot draw as a
        // dashed box crossed by its diagonals.
        if o.object_type() == 4 {
            let outline = Some(Outline {
                width: 12,
                color: 0,
                dash: Dash::LongDash,
                round_cap: false,
            });
            let mut path = Path::default();
            path.move_to(pt(m, (0, 0)));
            path.line_to(pt(m, (w, 0)));
            path.line_to(pt(m, (w, h)));
            path.line_to(pt(m, (0, h)));
            path.close();
            path.move_to(pt(m, (0, 0)));
            path.line_to(pt(m, (w, h)));
            path.move_to(pt(m, (0, h)));
            path.line_to(pt(m, (w, 0)));
            dl.items.push(Item::Path(Box::new(PathItem {
                path,
                fill: Vec::new(),
                outline,
                fill_alpha: 0,
            })));
        }
        return;
    };
    let p0 = pt(m, (0, 0));
    match &*view {
        hm_model::chart::OleView::Picture(data) => {
            let px = pt(m, (w, 0));
            let py = pt(m, (0, h));
            dl.items.push(Item::Image(Box::new(ImageItem {
                data: data.clone(),
                frame: [
                    px.0 - p0.0,
                    py.0 - p0.0,
                    p0.0,
                    px.1 - p0.1,
                    py.1 - p0.1,
                    p0.1,
                ],
                crop: [0; 4],
                effects: Effects::default(),
                natural: (0, 0),
            })));
        }
        hm_model::chart::OleView::Chart(c) => {
            let rect = Rect {
                x: p0.0,
                y: p0.1,
                w,
                h,
            };
            crate::chart::draw(dl, ctx.fonts, c, rect);
        }
    }
}

/// Paints an equation with its origin at `x` and its baseline at `y`.
pub fn paint_math(
    dl: &mut DisplayList,
    fonts: &hm_fonts::FontSystem,
    m: &MathLayout,
    x: i32,
    y: i32,
) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use hm_fonts::FontSystem;

    #[test]
    fn a_character_picture_resolved_from_the_paper_fills_its_resolved_box() {
        use hm_model::{
            BinStore, Control, ControlKind, CtrlId, DocInfo, DocMeta, Document, Format,
            ObjectCommon, ObjectControl, ObjectHeader, ObjectPart, ShapeComponent, SharedBytes,
        };
        let data = SharedBytes::from(tiny_skia::Pixmap::new(4, 4).unwrap().encode_png().unwrap());
        let doc = Document {
            meta: DocMeta {
                format: Format::Hwp5,
                version: Default::default(),
                flags: 0,
            },
            info: DocInfo::default(),
            sections: Vec::new(),
            source: None,
            bins: Some(BinStore::new(move |_| Some(data.clone()))),
            diagnostics: Vec::new(),
        };
        let fonts = FontSystem::new();
        let ctx = Ctx {
            doc: &doc,
            styles: &[],
            fonts: &fonts,
            page: None,
            body_anchor: None,
            lines: None,
            cells: None,
            objects: None,
            index_only: false,
        };
        // Stored 6000 square; its box resolved from the paper is 12000 by 3000.
        let c = Control {
            kind: ControlKind::Object(Box::new(ObjectControl {
                id: CtrlId::SHAPE,
                header: ObjectHeader {
                    common: ObjectCommon {
                        attr: 1,
                        width: 6000,
                        height: 6000,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                parts: vec![ObjectPart::Shape(Box::new(Shape {
                    component: ShapeComponent {
                        kind: CtrlId::from_chars(b"$pic"),
                        doubled: true,
                        offset: (0, 0),
                        group_level: 0,
                        version: 1,
                        initial_size: (6000, 6000),
                        size: (6000, 6000),
                        flip: 0,
                        rotation: 0,
                        rotation_center: (0, 0),
                        translation: Matrix::IDENTITY,
                        transforms: Vec::new(),
                        style: None,
                        members: None,
                        group_instance: None,
                        tail: SharedBytes::empty(),
                    },
                    parts: vec![ShapePart::Picture(Picture {
                        corners: [(0, 0), (6000, 0), (6000, 6000), (0, 6000)],
                        image: hm_model::ImageRef {
                            bin_id: 1,
                            ..Default::default()
                        },
                        ..Default::default()
                    })],
                }))],
            })),
            src: None,
        };
        let mut dl = DisplayList {
            width: 20000,
            height: 20000,
            items: Vec::new(),
        };
        let rect = Rect {
            x: 1000,
            y: 2000,
            w: 12000,
            h: 3000,
        };
        paint_object(&mut dl, &ctx, &c, rect, None);
        let frames: Vec<[i32; 6]> = dl
            .items
            .iter()
            .filter_map(|i| match i {
                Item::Image(im) => Some(im.frame),
                _ => None,
            })
            .collect();
        assert_eq!(frames, [[12000, 0, 1000, 0, 3000, 2000]]);
    }

    #[test]
    fn zero_width_solid_picture_borders_draw_as_hairlines() {
        use hm_model::{DocInfo, DocMeta, Document, Format, ShapeComponent, SharedBytes};
        let doc = Document {
            meta: DocMeta {
                format: Format::Hwp5,
                version: Default::default(),
                flags: 0,
            },
            info: DocInfo::default(),
            sections: Vec::new(),
            source: None,
            bins: None,
            diagnostics: Vec::new(),
        };
        let fonts = FontSystem::new();
        let ctx = Ctx {
            doc: &doc,
            styles: &[],
            fonts: &fonts,
            page: None,
            body_anchor: None,
            lines: None,
            cells: None,
            objects: None,
            index_only: false,
        };
        for kind in [0, 1] {
            let shape = Shape {
                component: ShapeComponent {
                    kind: hm_model::CtrlId::from_chars(b"$pic"),
                    doubled: true,
                    offset: (0, 0),
                    group_level: 0,
                    version: 1,
                    initial_size: (6000, 6000),
                    size: (6000, 6000),
                    flip: 0,
                    rotation: 0,
                    rotation_center: (0, 0),
                    translation: Matrix([1.0, 0.0, 1000.0, 0.0, 1.0, 1000.0]),
                    transforms: Vec::new(),
                    style: None,
                    members: None,
                    group_instance: None,
                    tail: SharedBytes::empty(),
                },
                parts: vec![ShapePart::Picture(Picture {
                    corners: [(0, 0), (6000, 0), (6000, 6000), (0, 6000)],
                    border: LineStyle {
                        attr: kind,
                        ..Default::default()
                    },
                    ..Default::default()
                })],
            };
            let mut dl = DisplayList {
                width: 8000,
                height: 8000,
                items: Vec::new(),
            };
            paint_shape(
                &mut dl,
                &ctx,
                &shape,
                (&Matrix::IDENTITY, &Matrix::IDENTITY, 0),
                &[],
                &mut 0,
            );
            let pm = crate::raster::render(&dl, &fonts, 600.0).unwrap();
            assert_eq!(pm.pixel(83, 333).unwrap().red() < 200, kind == 1);
        }
    }

    #[test]
    fn negative_picture_sizes_keep_content_upright_without_changing_explicit_or_group_flips() {
        use hm_model::{BinStore, DocInfo, DocMeta, Document, Format, ShapeComponent, SharedBytes};
        let colors = [
            [255, 0, 0, 255],
            [0, 255, 0, 255],
            [0, 0, 255, 255],
            [255, 255, 0, 255],
        ];
        let mut src = tiny_skia::Pixmap::new(8, 8).unwrap();
        for y in 0..8 {
            for x in 0..8 {
                let at = (y * 8 + x) * 4;
                src.data_mut()[at..at + 4].copy_from_slice(&colors[(y / 4) * 2 + x / 4]);
            }
        }
        let data = SharedBytes::from(src.encode_png().unwrap());
        let doc = Document {
            meta: DocMeta {
                format: Format::Hwp5,
                version: Default::default(),
                flags: 0,
            },
            info: DocInfo::default(),
            sections: Vec::new(),
            source: None,
            bins: Some(BinStore::new(move |_| Some(data.clone()))),
            diagnostics: Vec::new(),
        };
        let fonts = FontSystem::new();
        let ctx = Ctx {
            doc: &doc,
            styles: &[],
            fonts: &fonts,
            page: None,
            body_anchor: None,
            lines: None,
            cells: None,
            objects: None,
            index_only: false,
        };
        for parent in [
            Matrix([1.0, 0.0, 7200.0, 0.0, 1.0, 7200.0]),
            Matrix([0.0, -1.0, 14400.0, 1.0, 0.0, 7200.0]),
            Matrix([-1.0, 0.0, 14400.0, 0.0, 1.0, 7200.0]),
        ] {
            for negative in 0..4 {
                for flip in 0..4 {
                    let signs = (negative | flip) as u32;
                    let sx = if signs & 1 != 0 { -1.0 } else { 1.0 };
                    let sy = if signs & 2 != 0 { -1.0 } else { 1.0 };
                    let shape = Shape {
                        component: ShapeComponent {
                            kind: hm_model::CtrlId::from_chars(b"$pic"),
                            doubled: true,
                            offset: (0, 0),
                            group_level: 0,
                            version: 1,
                            initial_size: (7200, 7200),
                            size: (
                                if negative & 1 != 0 {
                                    (-7200i32) as u32
                                } else {
                                    7200
                                },
                                if negative & 2 != 0 {
                                    (-7200i32) as u32
                                } else {
                                    7200
                                },
                            ),
                            flip,
                            rotation: 0,
                            rotation_center: (0, 0),
                            translation: Matrix::IDENTITY,
                            transforms: vec![(
                                Matrix([
                                    sx,
                                    0.0,
                                    if sx < 0.0 { 7200.0 } else { 0.0 },
                                    0.0,
                                    sy,
                                    if sy < 0.0 { 7200.0 } else { 0.0 },
                                ]),
                                Matrix::IDENTITY,
                            )],
                            style: None,
                            members: None,
                            group_instance: None,
                            tail: SharedBytes::empty(),
                        },
                        parts: vec![ShapePart::Picture(Picture {
                            corners: [(0, 0), (7200, 0), (7200, 7200), (0, 7200)],
                            border: LineStyle {
                                width: 720,
                                color: 0xff00ff,
                                attr: 1,
                                outline: None,
                            },
                            ..Default::default()
                        })],
                    };
                    let mut dl = DisplayList {
                        width: 21600,
                        height: 21600,
                        items: Vec::new(),
                    };
                    paint_shape(&mut dl, &ctx, &shape, (&parent, &parent, 0), &[], &mut 0);
                    let pm = crate::raster::render(&dl, &fonts, 72.0).unwrap();
                    for (i, &(x, y)) in [(1800, 1800), (5400, 1800), (1800, 5400), (5400, 5400)]
                        .iter()
                        .enumerate()
                    {
                        let (x, y) = pt(&parent, (x, y));
                        let pixel = pm.pixel(x as u32 / 100, y as u32 / 100).unwrap();
                        assert_eq!(
                            [pixel.red(), pixel.green(), pixel.blue(), pixel.alpha()],
                            colors[i ^ flip as usize],
                            "negative={negative} flip={flip}"
                        );
                    }
                    let (x, y) = pt(&parent, (0, 3600));
                    let pixel = pm.pixel(x as u32 / 100, y as u32 / 100).unwrap();
                    assert_eq!(
                        [pixel.red(), pixel.green(), pixel.blue()],
                        [255, 0, 255],
                        "picture border moved"
                    );
                }
            }
        }
    }

    #[test]
    fn drawing_strokes_render_on_hancom_six_hundred_dpi_widths() {
        let draw = |width| {
            let line = LineStyle {
                width,
                color: 0x2cb62c,
                attr: 1,
                outline: None,
            };
            let mut path = Path::default();
            path.move_to((300, 1000));
            path.line_to((3300, 1000));
            let dl = DisplayList {
                width: 3600,
                height: 1800,
                items: vec![Item::Path(Box::new(PathItem {
                    path,
                    fill: Vec::new(),
                    outline: outline(&line),
                    fill_alpha: 0,
                }))],
            };
            crate::raster::render(&dl, &FontSystem::new(), 600.0)
                .unwrap()
                .data()
                .to_vec()
        };
        // Measured from a single Hancom PDF with independently sized strokes.
        for (source, painted) in [(0, 12), (17, 12), (18, 24), (56, 60), (112, 108)] {
            assert_eq!(draw(source), draw(painted), "source width {source}");
        }
        assert_ne!(draw(12), draw(24));
        assert_ne!(draw(108), draw(120));
    }
}
