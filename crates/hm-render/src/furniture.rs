//! Drawing page furniture: the page background and border, master pages,
//! headers and footers, notes, separators and the page number.

use hm_fonts::FontSystem;
use hm_layout::objects::Layer;
use hm_layout::{DocLayout, Frame, FrameKind, Mark, Page, Placed};

use crate::paint::{self, Sides};
use crate::text::{Ctx, draw_line};
use crate::{DisplayList, GlyphRun, Item};

fn frame(dl: &mut DisplayList, ctx: &Ctx<'_>, f: &Frame, placed: &mut Vec<Placed>) {
    let Some(paras) = f.list.paras(ctx.doc) else {
        return;
    };
    let objects = |dl: &mut DisplayList, front: bool| {
        let mut list: Vec<_> = f
            .objects
            .iter()
            .filter(|p| (p.layer == Layer::Behind) != front)
            .collect();
        list.sort_by_key(|p| p.z);
        for p in list {
            if let (Some(para), Some(pal)) =
                (paras.get(p.para as usize), f.paras.get(p.para as usize))
            {
                crate::objects::paint_placement(dl, ctx, para, pal, p);
            }
        }
    };
    objects(dl, false);
    let layout_of = |p: u32| f.paras.get(p as usize);
    let list = crate::parabox::List {
        doc: ctx.doc,
        paras,
        layout: &layout_of,
    };
    crate::parabox::draw_list(
        dl,
        &list,
        f.lines.iter().map(|l| (l.para, l.line, l.x, l.y)),
    );
    for pt in &f.tables {
        let (Some(para), Some(pal)) = (paras.get(pt.para as usize), f.paras.get(pt.para as usize))
        else {
            continue;
        };
        if let (Some(t), Some(tl)) = (
            para.controls
                .get(pt.control as usize)
                .and_then(|c| c.as_table()),
            pal.table(pt.control),
        ) {
            crate::table::draw_with_caption(dl, ctx, t, tl, pt.x, pt.y);
        }
    }
    for fl in &f.lines {
        let (Some(para), Some(pal)) = (paras.get(fl.para as usize), f.paras.get(fl.para as usize))
        else {
            continue;
        };
        draw_line(dl, ctx, para, pal, fl.line as usize, fl.x, fl.y, placed);
    }
    objects(dl, true);
}

fn mark(dl: &mut DisplayList, ctx: &Ctx<'_>, fonts: &FontSystem, m: &Mark) {
    for &(ch, si, x) in &m.glyphs {
        let style = &ctx.styles[si as usize];
        let Some(face_id) = style.face else { continue };
        let Some(glyph) = fonts.face(face_id).and_then(|f| f.glyph(ch)) else {
            continue;
        };
        dl.items.push(Item::Glyphs(GlyphRun {
            face: face_id,
            size: style.size,
            ratio: style.ratio,
            color: style.color,
            fake_bold: style.fake_bold,
            italic: style.italic,
            baseline: m.baseline + style.offset,
            glyphs: vec![(glyph.glyph, x)],
        }));
    }
}

/// What lies under the body text: the background, master pages, headers,
/// footers and separators.
pub(crate) fn draw_under(
    dl: &mut DisplayList,
    ctx: &Ctx<'_>,
    _layout: &DocLayout,
    page: &Page,
    placed: &mut Vec<Placed>,
) {
    if let Some(b) = page.border.filter(|b| b.background)
        && let Some(bf) = ctx
            .doc
            .info
            .border_fills
            .get((b.border_fill as usize).wrapping_sub(1))
        && let Some(fill) = &bf.fill
    {
        paint::fill_rect(dl, b.fill.x, b.fill.y, b.fill.w, b.fill.h, fill);
    }
    for f in page.frames.iter().filter(|f| {
        matches!(
            f.kind,
            FrameKind::MasterPage | FrameKind::Header | FrameKind::Footer
        )
    }) {
        frame(dl, ctx, f, placed);
    }
    // Column and note separators lie under the body: tables and objects
    // over a separator hide it.
    for r in &page.rules {
        paint::border_line(dl, r.x1, r.y1, r.x2, r.y2, &r.line);
    }
}

/// What lies over the body text: notes, the page number and the border
/// lines.
pub(crate) fn draw_over(
    dl: &mut DisplayList,
    ctx: &Ctx<'_>,
    page: &Page,
    placed: &mut Vec<Placed>,
) {
    for f in page
        .frames
        .iter()
        .filter(|f| matches!(f.kind, FrameKind::Footnotes | FrameKind::Endnotes))
    {
        frame(dl, ctx, f, placed);
    }
    for m in &page.marks {
        mark(dl, ctx, ctx.fonts, m);
    }
    if let Some(b) = page.border.filter(|b| b.lines)
        && let Some(bf) = ctx
            .doc
            .info
            .border_fills
            .get((b.border_fill as usize).wrapping_sub(1))
    {
        let r = b.rect;
        paint::diagonals(dl, r.x, r.y, r.w, r.h, bf);
        paint::border_sides(dl, r.x, r.y, r.w, r.h, bf, Sides::ALL);
    }
}
