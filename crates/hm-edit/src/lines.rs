//! Line segments for edited paragraphs, from our layout.
//!
//! Hancom replays the line segments a file stores when the first one's
//! span (`horzpos`, `horzsize`) equals the span it computes, taking line
//! membership from the starts and the vertical advance from `textheight`,
//! `baseline` and `spacing`. Writing our own lines there makes Hancom show
//! the lines we show. Flags: every line is one whole segment (bits 17 and
//! 18); bit 20 marks lines the indent applies to (the first line of a
//! positive indent, the later lines of a hanging one); Hancom's page and
//! column state bits are left clear, since it does not read them.

use hm_layout::{DocLayout, ParaLayout};
use hm_model::{ControlKind, DocInfo, Document, LineSeg, Paragraph};

const WHOLE_LINE: u32 = 1 << 17 | 1 << 18;
const INDENTED: u32 = 1 << 20;

/// Fills the line segments of every paragraph that changed (no source
/// bytes) and has none, from `layout`.
pub fn fill(doc: &mut Document, layout: &DocLayout) {
    let info = &doc.info;
    for (si, sec) in doc.sections.iter_mut().enumerate() {
        let Some(sl) = layout.sections.get(si) else {
            continue;
        };
        // Body lines' vpos: top from the column region, as placed.
        let mut vpos = std::collections::HashMap::new();
        for page in layout.pages.iter().filter(|p| p.section == si as u32) {
            for pl in &page.lines {
                vpos.entry((pl.para, pl.line)).or_insert(pl.vpos);
            }
        }
        for (pi, para) in sec.paras.iter_mut().enumerate() {
            let Some(pal) = sl.paras.get(pi) else {
                continue;
            };
            let top = |li: usize| vpos.get(&(pi as u32, li as u32)).copied();
            fill_para(info, para, pal, &top);
        }
    }
}

fn fill_para(
    info: &DocInfo,
    para: &mut Paragraph,
    pal: &ParaLayout,
    top: &dyn Fn(usize) -> Option<i32>,
) {
    if para.src.is_none() && para.line_segs.is_empty() {
        para.line_segs = segments(info, para, pal, top);
    }
    // Cell paragraphs of tables in this paragraph.
    for (ci, c) in para.controls.iter_mut().enumerate() {
        let ControlKind::Table(t) = &mut c.kind else {
            continue;
        };
        let Some(tl) = pal.table(ci as u32) else {
            continue;
        };
        for (cell, cl) in t.cells.iter_mut().zip(&tl.cells) {
            for (pi, (p, pl)) in cell.paras.iter_mut().zip(&cl.paras).enumerate() {
                let tops: Vec<(u32, i32)> = cl
                    .lines
                    .iter()
                    .filter(|l| l.para == pi as u32)
                    .map(|l| (l.line, l.y))
                    .collect();
                let top = |li: usize| tops.iter().find(|(l, _)| *l == li as u32).map(|&(_, y)| y);
                fill_para(info, p, pl, &top);
            }
        }
    }
}

/// The segments of one laid-out paragraph.
pub fn segments(
    info: &DocInfo,
    para: &Paragraph,
    pal: &ParaLayout,
    top: &dyn Fn(usize) -> Option<i32>,
) -> Vec<LineSeg> {
    let indent = info
        .para_shape(para.header.para_shape as u32)
        .map_or(0, |s| s.first_indent(1000).signum());
    let mut y = 0;
    pal.lines
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let vpos = top(i).unwrap_or(y);
            y = vpos + l.advance();
            let indented = (indent > 0 && i == 0) || (indent < 0 && i > 0);
            LineSeg {
                text_start: l.start,
                vpos,
                height: l.height,
                text_height: l.text_height,
                baseline: l.baseline,
                spacing: l.spacing,
                col_start: l.seg_x,
                width: l.seg_width,
                flags: WHOLE_LINE | if indented { INDENTED } else { 0 },
            }
        })
        .collect()
}
