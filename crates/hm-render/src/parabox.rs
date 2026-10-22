//! Paragraph borders and shading.
//!
//! Consecutive lines of bordered paragraphs form one box per column. A
//! paragraph joins the box of the one before it when both connect their
//! borders and the border lines and offsets are equal. A box that continues
//! from the previous column leaves out its top side, one that continues in
//! the next column its bottom side.
//!
//! Horizontally the box spans the column, or the paragraph margins when the
//! paragraph ignores them, widened by the border offsets. Vertically it runs
//! from the first line's top to the last line's bottom plus its line spacing
//! (without the spacing when margins are ignored), widened by the offsets
//! on the sides it draws.

use hm_layout::{DocLayout, Page};
use hm_model::{BorderFill, Document, ParaShape};

use crate::DisplayList;
use crate::paint::{self, Sides};

/// A box being collected.
struct Run<'a> {
    bf: &'a BorderFill,
    shape: &'a ParaShape,
    x: i32,
    w: i32,
    top: i32,
    bottom: i32,
    /// The box starts its group (draws the top side).
    opens: bool,
    /// Section, paragraph and line of the last line in the box.
    last: (u32, u32, u32),
    column_x: i32,
}

/// Border definition of a paragraph, when it has a visible one.
fn border_of<'a>(doc: &'a Document, shape: &ParaShape) -> Option<&'a BorderFill> {
    let id = shape.border_fill as usize;
    let bf = doc.info.border_fills.get(id.checked_sub(1)?)?;
    let shows =
        bf.sides.iter().any(|s| s.visible()) || bf.fill.as_ref().is_some_and(|f| !f.is_empty());
    shows.then_some(bf)
}

/// True when paragraph `b` continues the border of paragraph `a`.
fn joins(a: (&ParaShape, &BorderFill), b: (&ParaShape, &BorderFill)) -> bool {
    a.0.connect_border()
        && b.0.connect_border()
        && a.1.sides == b.1.sides
        && a.0.border_offsets == b.0.border_offsets
        && a.0.border_ignores_margin() == b.0.border_ignores_margin()
}

/// Draws the paragraph borders and fills of a page.
pub fn draw(dl: &mut DisplayList, doc: &Document, layout: &DocLayout, page: &Page) {
    let paras = &doc.sections[page.section as usize].paras;
    let mut run: Option<Run<'_>> = None;
    for pl in &page.lines {
        let para = &paras[pl.para as usize];
        let Some(shape) = doc.info.para_shape(para.header.para_shape as u32) else {
            flush(dl, doc, layout, run.take());
            continue;
        };
        let Some(bf) = border_of(doc, shape) else {
            flush(dl, doc, layout, run.take());
            continue;
        };
        let pal = layout.para(page.section, pl.para);
        let line = &pal.lines[pl.line as usize];
        let [ol, or, _, _] = shape.border_offsets.map(i32::from);
        let (x, w) = if shape.border_ignores_margin() {
            (pl.x + line.seg_x - ol, line.seg_width + ol + or)
        } else {
            (pl.x - ol, pal.column_width + ol + or)
        };
        let bottom = pl.y
            + line.height
            + if shape.border_ignores_margin() {
                0
            } else {
                line.spacing
            };
        let continues = run.as_ref().is_some_and(|r| {
            let same_para = r.last.1 == pl.para && r.last.0 == page.section;
            let next_para = r.last.1 + 1 == pl.para && joins((r.shape, r.bf), (shape, bf));
            r.column_x == pl.x && (same_para || next_para)
        });
        if continues {
            let r = run.as_mut().unwrap();
            r.bottom = bottom;
            r.last = (page.section, pl.para, pl.line);
            r.shape = shape;
            r.bf = bf;
            continue;
        }
        flush(dl, doc, layout, run.take());
        // A box opens its group unless it continues the previous
        // paragraph's border from another column or page.
        let opens = pl.line == 0
            && (pl.para == 0
                || paras
                    .get(pl.para as usize - 1)
                    .and_then(|p| doc.info.para_shape(p.header.para_shape as u32))
                    .and_then(|s| border_of(doc, s).map(|b| (s, b)))
                    .is_none_or(|prev| !joins(prev, (shape, bf))));
        run = Some(Run {
            bf,
            shape,
            x,
            w,
            top: pl.y,
            bottom,
            opens,
            last: (page.section, pl.para, pl.line),
            column_x: pl.x,
        });
    }
    flush(dl, doc, layout, run.take());
}

/// True when the box's last line ends its border group.
fn closes(doc: &Document, layout: &DocLayout, r: &Run<'_>) -> bool {
    let (sec, para, line) = r.last;
    let pal = layout.para(sec, para);
    if (line as usize) + 1 < pal.lines.len() {
        return false;
    }
    let next = doc.sections[sec as usize]
        .paras
        .get(para as usize + 1)
        .and_then(|p| doc.info.para_shape(p.header.para_shape as u32))
        .and_then(|s| border_of(doc, s).map(|b| (s, b)));
    next.is_none_or(|n| !joins((r.shape, r.bf), n))
}

fn flush(dl: &mut DisplayList, doc: &Document, layout: &DocLayout, run: Option<Run<'_>>) {
    let Some(r) = run else { return };
    let [_, _, ot, ob] = r.shape.border_offsets.map(i32::from);
    let closes = closes(doc, layout, &r);
    let top = if r.opens { r.top - ot } else { r.top };
    let bottom = if closes { r.bottom + ob } else { r.bottom };
    let sides = Sides {
        left: true,
        right: true,
        top: r.opens,
        bottom: closes,
    };
    paint::border_box(dl, r.x, top, r.w, bottom - top, r.bf, sides);
}
