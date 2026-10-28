//! Paragraph borders and shading.
//!
//! Consecutive lines of bordered paragraphs form one box per column. A
//! paragraph joins the box of the one before it when both connect their
//! borders and the border lines and horizontal offsets are equal. A box
//! that continues from the previous column leaves out its top side, one
//! that continues in the next column its bottom side.
//!
//! Horizontally the box spans the column, or the paragraph margins when the
//! paragraph ignores them, widened by the border offsets. Vertically it runs
//! from the top of the first paragraph's spacing above to the last line's
//! bottom plus its line spacing and the last paragraph's spacing below (all
//! left out when margins are ignored), widened by the offsets on the sides
//! it draws.

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
        && a.0.border_offsets[..2] == b.0.border_offsets[..2]
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
        let [ol, or, ot, _] = shape.border_offsets.map(i32::from);
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
        // An opening box takes in the paragraph's spacing above it, unless
        // the border ignores the margins and hugs the text.
        let above = if opens && !shape.border_ignores_margin() {
            pal.before
        } else {
            0
        };
        run = Some(Run {
            bf,
            shape,
            x,
            w,
            top: ly - above - if opens { ot } else { 0 },
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
    let [_, _, _, ob] = r.shape.border_offsets.map(i32::from);
    let closes = closes(list, &r);
    // A closing box takes in the last paragraph's spacing below, as an
    // opening one takes in the spacing above, unless it hugs the text.
    let below = match (list.layout)(r.last.0) {
        Some(pal) if closes && !r.shape.border_ignores_margin() => pal.after,
        _ => 0,
    };
    let bottom = if closes {
        r.bottom + below + ob
    } else {
        r.bottom
    };
    let sides = Sides {
        left: true,
        right: true,
        top: r.opens,
        bottom: closes,
    };
    paint::border_box(dl, r.x, r.top, r.w, bottom - r.top, r.bf, sides);
}

#[cfg(test)]
mod tests {
    use super::*;
    use hm_fonts::FontSystem;
    use hm_layout::Line;
    use hm_model::{BorderLine, DocInfo, DocMeta, Format, ParaHeader};

    #[test]
    fn adjacent_connected_paragraphs_keep_one_box_when_only_vertical_offsets_change() {
        for split in [false, true] {
            let left = if split { 500 } else { 0 };
            let doc = Document {
                meta: DocMeta {
                    format: Format::Hwp5,
                    version: Default::default(),
                    flags: 0,
                },
                info: DocInfo {
                    para_shapes: [[0, 0, 1000, 300], [left, 0, 0, 0], [left, 0, 600, 500]]
                        .map(|border_offsets| ParaShape {
                            attr1: 1 << 28,
                            border_fill: 1,
                            border_offsets,
                            ..Default::default()
                        })
                        .to_vec(),
                    border_fills: vec![BorderFill {
                        sides: [BorderLine {
                            kind: 1,
                            width: 10,
                            color: 0,
                        }; 4],
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                sections: Vec::new(),
                source: None,
                bins: None,
                diagnostics: Vec::new(),
            };
            let paras = [0, 1, 2].map(|para_shape| Paragraph {
                header: ParaHeader {
                    para_shape,
                    ..Default::default()
                },
                ..Default::default()
            });
            let layouts = [0, 1, 2].map(|_| ParaLayout {
                column_width: 5000,
                lines: vec![Line {
                    height: 1000,
                    ..Default::default()
                }],
                ..Default::default()
            });
            let layout = |p: u32| layouts.get(p as usize);
            let list = List {
                doc: &doc,
                paras: &paras,
                layout: &layout,
            };
            let mut dl = DisplayList {
                width: 7000,
                height: 6500,
                items: Vec::new(),
            };
            draw_list(
                &mut dl,
                &list,
                (0..3).map(|p| (p, 0, 1000, 2000 + p as i32 * 1000)),
            );
            let pm = crate::raster::render(&dl, &FontSystem::new(), 72.0).unwrap();
            let ink = |y| pm.pixel(30, y).unwrap().red() < 100;
            assert!(ink(10), "the first paragraph's top offset was lost");
            assert_eq!(ink(30), split, "an internal border changed");
            assert!(!ink(40), "the last paragraph opened a second box");
            assert!(ink(55), "the last paragraph's bottom offset was lost");
        }
    }

    #[test]
    fn a_closing_box_takes_in_the_last_paragraphs_spacing_below() {
        // exam-eng p8: Hancom closes a paragraph border under the last
        // paragraph's spacing below, as an opening box takes in the
        // spacing above.
        for ignores_margin in [false, true] {
            let doc = Document {
                meta: DocMeta {
                    format: Format::Hwp5,
                    version: Default::default(),
                    flags: 0,
                },
                info: DocInfo {
                    para_shapes: vec![ParaShape {
                        attr1: (1 << 28) | if ignores_margin { 1 << 29 } else { 0 },
                        border_fill: 1,
                        ..Default::default()
                    }],
                    border_fills: vec![BorderFill {
                        sides: [BorderLine {
                            kind: 1,
                            width: 10,
                            color: 0,
                        }; 4],
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                sections: Vec::new(),
                source: None,
                bins: None,
                diagnostics: Vec::new(),
            };
            let paras = [0, 1].map(|_| Paragraph::default());
            let layouts = [0, 1].map(|p| ParaLayout {
                column_width: 5000,
                lines: vec![Line {
                    height: 1000,
                    seg_width: 5000,
                    ..Default::default()
                }],
                after: if p == 1 { 500 } else { 0 },
                ..Default::default()
            });
            let layout = |p: u32| layouts.get(p as usize);
            let list = List {
                doc: &doc,
                paras: &paras,
                layout: &layout,
            };
            let mut dl = DisplayList {
                width: 7000,
                height: 6500,
                items: Vec::new(),
            };
            draw_list(
                &mut dl,
                &list,
                (0..2).map(|p| (p, 0, 1000, 2000 + p as i32 * 1000)),
            );
            let pm = crate::raster::render(&dl, &FontSystem::new(), 72.0).unwrap();
            let ink = |y| pm.pixel(30, y).unwrap().red() < 100;
            assert_eq!(ink(40), ignores_margin);
            assert_eq!(ink(45), !ignores_margin);
        }
    }
}
