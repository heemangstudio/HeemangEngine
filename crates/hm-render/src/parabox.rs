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

use hm_layout::{DocLayout, Page, ParaLayout};
use hm_model::{BorderFill, Document, ParaShape, Paragraph};

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
    /// Paragraph and line of the last line in the box.
    last: (u32, u32),
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

/// One paragraph list as placed: its paragraphs, their layouts and the
/// placed lines as `(paragraph, line, x, y)`.
pub(crate) struct List<'a, 'l> {
    pub doc: &'a Document,
    pub paras: &'a [Paragraph],
    pub layout: &'l dyn Fn(u32) -> Option<&'l ParaLayout>,
}

impl<'a> List<'a, '_> {
    fn shape_border(&self, para: usize) -> Option<(&'a ParaShape, &'a BorderFill)> {
        let shape = self
            .doc
            .info
            .para_shape(self.paras.get(para)?.header.para_shape as u32)?;
        Some((shape, border_of(self.doc, shape)?))
    }
}

/// Draws the paragraph borders and fills of a page's body.
pub fn draw(dl: &mut DisplayList, doc: &Document, layout: &DocLayout, page: &Page) {
    let layout_of = |p: u32| Some(layout.para(page.section, p));
    let list = List {
        doc,
        paras: &doc.sections[page.section as usize].paras,
        layout: &layout_of,
    };
    draw_list(
        dl,
        &list,
        page.lines.iter().map(|l| (l.para, l.line, l.x, l.y)),
    );
}

/// Draws the paragraph borders and fills of the placed lines of a list.
pub(crate) fn draw_list(
    dl: &mut DisplayList,
    list: &List<'_, '_>,
    lines: impl Iterator<Item = (u32, u32, i32, i32)>,
) {
    let mut run: Option<Run<'_>> = None;
    for (para, li, lx, ly) in lines {
        let (Some((shape, bf)), Some(pal)) =
            (list.shape_border(para as usize), (list.layout)(para))
        else {
            flush(dl, list, run.take());
            continue;
        };
        let Some(line) = pal.lines.get(li as usize) else {
            continue;
        };
        let [ol, or, _, _] = shape.border_offsets.map(i32::from);
        let (x, w) = if shape.border_ignores_margin() {
            (lx + line.seg_x - ol, line.seg_width + ol + or)
        } else {
            (lx - ol, pal.column_width + ol + or)
        };
        let bottom = ly
            + line.height
            + if shape.border_ignores_margin() {
                0
            } else {
                line.spacing
            };
        let continues = run.as_ref().is_some_and(|r| {
            let same_para = r.last.0 == para;
            let next_para = r.last.0 + 1 == para && joins((r.shape, r.bf), (shape, bf));
            r.column_x == lx && (same_para || next_para)
        });
        if continues {
            let r = run.as_mut().unwrap();
            r.bottom = bottom;
            r.last = (para, li);
            r.shape = shape;
            r.bf = bf;
            continue;
        }
        flush(dl, list, run.take());
        // A box opens its group unless it continues the previous
        // paragraph's border from another column or page.
        let opens = li == 0
            && (para == 0
                || list
                    .shape_border(para as usize - 1)
                    .is_none_or(|prev| !joins(prev, (shape, bf))));
        run = Some(Run {
            bf,
            shape,
            x,
            w,
            top: ly,
            bottom,
            opens,
            last: (para, li),
            column_x: lx,
        });
    }
    flush(dl, list, run.take());
}

/// True when the box's last line ends its border group.
fn closes(list: &List<'_, '_>, r: &Run<'_>) -> bool {
    let (para, line) = r.last;
    if (list.layout)(para).is_some_and(|pal| (line as usize) + 1 < pal.lines.len()) {
        return false;
    }
    list.shape_border(para as usize + 1)
        .is_none_or(|n| !joins((r.shape, r.bf), n))
}

fn flush(dl: &mut DisplayList, list: &List<'_, '_>, run: Option<Run<'_>>) {
    let Some(r) = run else { return };
    let [_, _, ot, ob] = r.shape.border_offsets.map(i32::from);
    let closes = closes(list, &r);
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
