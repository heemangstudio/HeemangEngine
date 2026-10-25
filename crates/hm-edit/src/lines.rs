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

/// Converts an output copy and materializes the line membership selected by
/// the source document's current layout. The source and its caches stay intact.
/// A list without a complete, unambiguous layout cannot be converted here.
pub fn prepare_conversion(
    source: &Document,
    layout: &DocLayout,
    target: hm_model::Format,
) -> Result<Document, String> {
    prepare_conversion_with_fonts(source, layout, target, None)
}

/// Also measures header/footer variants that have no page of their parity.
pub fn prepare_conversion_with_fonts(
    source: &Document,
    layout: &DocLayout,
    target: hm_model::Format,
    fonts: Option<&hm_fonts::FontSystem>,
) -> Result<Document, String> {
    if source.meta.format == target {
        return Ok(source.clone());
    }
    if source.sections.iter().any(|s| s.last_page_master.is_some()) {
        return Err("conversion cannot encode a last-page master in the target format yet".into());
    }
    let mut selected = ConversionLines::default();
    if source.sections.len() != layout.sections.len() {
        return Err("conversion layout has a different section count".into());
    }
    for (si, (sec, sl)) in source.sections.iter().zip(&layout.sections).enumerate() {
        selected.list(&source.info, &sec.paras, &sl.paras, &|pi, li| {
            layout
                .pages
                .iter()
                .filter(|p| p.section == si as u32)
                .flat_map(|p| &p.lines)
                .find(|l| l.para == pi as u32 && l.line == li as u32)
                .map(|l| l.vpos)
        })?;
    }
    for page in &layout.pages {
        for frame in &page.frames {
            let paras = frame
                .list
                .paras(source)
                .ok_or_else(|| format!("conversion frame has no source list: {:?}", frame.list))?;
            selected.list(&source.info, paras, &frame.paras, &|pi, li| {
                frame
                    .lines
                    .iter()
                    .find(|l| l.para == pi as u32 && l.line == li as u32)
                    .map(|l| {
                        l.y - if matches!(
                            frame.kind,
                            hm_layout::FrameKind::Footnotes | hm_layout::FrameKind::Endnotes
                        ) {
                            page.body.y
                        } else {
                            frame.rect.y
                        }
                    })
            })?;
        }
    }
    if let Some(fonts) = fonts {
        let mut styles = hm_layout::style::Styles::new(&source.info, fonts);
        styles.total_pages = layout.pages.len() as u32;
        for (si, sec) in source.sections.iter().enumerate() {
            let page = layout
                .pages
                .iter()
                .find(|p| p.section == si as u32)
                .or_else(|| layout.pages.first());
            let Some(page) = page else { continue };
            styles.page_number = page.number;
            for control in sec.paras.iter().flat_map(|p| &p.controls) {
                let Some(hf) = control.as_header_footer() else {
                    continue;
                };
                if hf.list.paras.iter().all(|p| {
                    selected
                        .paras
                        .contains_key(&(p as *const Paragraph as usize))
                }) {
                    continue;
                }
                let (paras, lines, _) = hm_layout::table::stack(
                    &hf.list.paras,
                    &source.info,
                    &mut styles,
                    hm_layout::para::ParaEnv {
                        column_width: hf
                            .list
                            .text_size
                            .map(|s| s.0)
                            .filter(|w| *w > 0)
                            .unwrap_or(page.body.w),
                        default_tab: sec
                            .definition()
                            .map(|d| d.default_tab as i32 / 2)
                            .unwrap_or(4000),
                    },
                    &mut hm_layout::heads::Heads::default(),
                );
                selected.list(&source.info, &hf.list.paras, &paras, &|pi, li| {
                    cell_top(&lines, pi, li)
                })?;
            }
        }
    }
    let mut pending = Vec::new();
    for (si, sec) in source.sections.iter().enumerate() {
        selected.finish(
            &source.info,
            &sec.paras,
            &format!("section {si} body"),
            &mut pending,
        )?;
    }
    // Source spans must be cleared before the new records become the output's
    // membership. The old records were used above only through matched layout.
    let mut output = source.clone();
    output.convert_to(target);
    let mut pending = pending.into_iter();
    for sec in &mut output.sections {
        materialize(&mut sec.paras, &mut pending);
    }
    Ok(output)
}

#[derive(Default)]
struct ConversionLines {
    // References identify the actual source paragraph, including lists repeated
    // or split among page frames. They never escape this preparation call.
    paras: std::collections::HashMap<usize, SelectedLines>,
}

struct SelectedLines {
    layout: ParaLayout,
    lines: std::collections::BTreeMap<usize, (hm_layout::Line, Option<i32>)>,
}

impl ConversionLines {
    fn list(
        &mut self,
        info: &DocInfo,
        paras: &[Paragraph],
        layouts: &[ParaLayout],
        top: &dyn Fn(usize, usize) -> Option<i32>,
    ) -> Result<(), String> {
        if paras.len() != layouts.len() {
            return Err("conversion list layout has a different paragraph count".into());
        }
        for (pi, (p, pl)) in paras.iter().zip(layouts).enumerate() {
            for (li, line) in pl.lines.iter().enumerate() {
                self.line(p, pl, li, *line, top(pi, li))?;
            }
            self.children(info, p, pl)?;
        }
        Ok(())
    }

    fn line(
        &mut self,
        p: &Paragraph,
        pl: &ParaLayout,
        li: usize,
        line: hm_layout::Line,
        top: Option<i32>,
    ) -> Result<(), String> {
        let entry = self
            .paras
            .entry(p as *const Paragraph as usize)
            .or_insert_with(|| SelectedLines {
                layout: pl.clone(),
                lines: Default::default(),
            });
        if let Some((old, y)) = entry.lines.get_mut(&li) {
            if *old != line {
                return Err(format!(
                    "conversion has conflicting layouts at logical line {li}, source start {}",
                    line.start
                ));
            }
            if y.is_none() {
                *y = top;
            }
        } else {
            entry.lines.insert(li, (line, top));
        }
        Ok(())
    }

    fn children(&mut self, info: &DocInfo, p: &Paragraph, pl: &ParaLayout) -> Result<(), String> {
        for (ci, c) in p.controls.iter().enumerate() {
            if let ControlKind::Table(t) = &c.kind {
                let tl = pl
                    .table(ci as u32)
                    .ok_or_else(|| format!("conversion has no layout for table control {ci}"))?;
                if t.cells.len() != tl.cells.len() {
                    return Err(format!(
                        "conversion table control {ci} has a different cell count"
                    ));
                }
                for (cell, cl) in t.cells.iter().zip(&tl.cells) {
                    self.list(info, &cell.paras, &cl.paras, &|pi, li| {
                        cell_top(&cl.lines, pi, li)
                    })?;
                }
                if let Some(cap) = &t.caption {
                    let cl = tl.caption.as_ref().ok_or_else(|| {
                        format!("conversion has no layout for table caption {ci}")
                    })?;
                    self.list(info, &cap.paras, &cl.paras, &|pi, li| {
                        cell_top(&cl.lines, pi, li)
                    })?;
                }
            }
        }
        self.objects(info, p, pl)
    }

    fn objects(&mut self, info: &DocInfo, p: &Paragraph, pl: &ParaLayout) -> Result<(), String> {
        let mut boxes = Vec::new();
        for (ci, c) in p.controls.iter().enumerate() {
            let Some(o) = c.as_object() else { continue };
            if o.parts
                .iter()
                .filter(|p| matches!(p, hm_model::ObjectPart::Shape(_)))
                .count()
                > 1
                || o.parts
                    .iter()
                    .filter(|p| matches!(p, hm_model::ObjectPart::Caption(_)))
                    .count()
                    > 1
            {
                return Err(format!(
                    "conversion object control {ci} has multiple shape or caption roots"
                ));
            }
            let ol = pl
                .measured
                .objects
                .iter()
                .find(|(k, _)| *k == ci as u32)
                .map(|(_, l)| l);
            let mut texts = Vec::new();
            if let Some(shape) = o.shape() {
                text_boxes(shape, &mut texts);
            }
            if !texts.is_empty() || o.caption().is_some() {
                let ol = ol.ok_or_else(|| {
                    format!("conversion has no inner layout for object control {ci}")
                })?;
                if texts.len() != ol.frames.len() {
                    return Err(format!(
                        "conversion object control {ci} has a different text-box count"
                    ));
                }
                boxes.extend(texts.into_iter().zip(&ol.frames));
                if let Some(cap) = o.caption() {
                    let cl = ol.caption.as_ref().ok_or_else(|| {
                        format!("conversion has no layout for object caption {ci}")
                    })?;
                    self.list(info, &cap.paras, &cl.paras, &|pi, li| {
                        cell_top(&cl.lines, pi, li)
                    })?;
                }
            }
        }
        for (text, frame) in boxes {
            // Existing writers do not encode typed link IDs, and convert_to
            // clears the HWP 5 tail that can carry them. Reject before output.
            if text.links() != (0, 0) || frame.borrowed.is_some() {
                return Err(format!(
                    "conversion cannot encode linked text-box IDs {:?} in the target format yet",
                    text.links()
                ));
            }
            self.list(info, &text.paras, &frame.paras, &|pi, li| {
                cell_top(&frame.lines, pi, li)
            })?;
        }
        Ok(())
    }

    fn finish(
        &self,
        info: &DocInfo,
        paras: &[Paragraph],
        path: &str,
        out: &mut Vec<Vec<LineSeg>>,
    ) -> Result<(), String> {
        for (pi, p) in paras.iter().enumerate() {
            let path = format!("{path}/paragraph {pi}");
            let entry = self
                .paras
                .get(&(p as *const Paragraph as usize))
                .ok_or_else(|| format!("conversion has no current layout for {path}"))?;
            let mut pl = entry.layout.clone();
            pl.lines = entry.lines.values().map(|(l, _)| *l).collect();
            if entry.lines.keys().copied().ne(0..pl.lines.len())
                || pl.lines.first().is_none_or(|l| l.start != 0)
                || pl.lines.last().is_none_or(|l| l.end != p.len())
                || pl.lines.windows(2).any(|ls| ls[0].end != ls[1].start)
            {
                return Err(format!(
                    "conversion has incomplete logical membership for {path}"
                ));
            }
            let mut segs = segments(info, p, &pl, &|li| {
                entry.lines.get(&li).and_then(|(_, y)| *y)
            });
            let same_lines = segs.len() == p.line_segs.len()
                && segs
                    .iter()
                    .zip(&p.line_segs)
                    .all(|(a, b)| a.text_start == b.text_start);
            if same_lines {
                for ((seg, old), line) in segs.iter_mut().zip(&p.line_segs).zip(&pl.lines) {
                    // after_line consumes a content-less flowing host's raw
                    // horizontal segment even when its membership was fitted.
                    let flow_host = p
                        .controls
                        .iter()
                        .any(|c| c.object_common().is_some_and(|o| !o.treat_as_char()))
                        && !hm_layout::table::has_content(p, line);
                    if line.stored || flow_host {
                        seg.col_start = old.col_start;
                        seg.width = old.width;
                        seg.flags = old.flags;
                    }
                }
            }
            out.push(segs);
            for (ci, c) in p.controls.iter().enumerate() {
                for (li, list) in crate::ops::control_lists(c).iter().enumerate() {
                    self.finish(info, list, &format!("{path}/control {ci}/list {li}"), out)?;
                }
            }
        }
        Ok(())
    }
}

fn cell_top(lines: &[hm_layout::CellLine], pi: usize, li: usize) -> Option<i32> {
    lines
        .iter()
        .find(|l| l.para == pi as u32 && l.line == li as u32)
        .map(|l| l.y)
}

fn text_boxes<'a>(s: &'a hm_model::Shape, out: &mut Vec<&'a hm_model::TextBox>) {
    out.extend(s.text());
    for c in s.children() {
        text_boxes(c, out);
    }
}

// Same list order as ops::control_lists, after convert_to has cleared sources.
fn materialize(paras: &mut [Paragraph], pending: &mut impl Iterator<Item = Vec<LineSeg>>) {
    use hm_model::{ObjectPart, SectionChild, ShapePart};
    fn shape(s: &mut hm_model::Shape, pending: &mut impl Iterator<Item = Vec<LineSeg>>) {
        for part in &mut s.parts {
            match part {
                ShapePart::Text(t) => materialize(&mut t.paras, pending),
                ShapePart::Child(s) => shape(s, pending),
                _ => {}
            }
        }
    }
    for p in paras {
        p.line_segs = pending.next().expect("validated conversion paragraph");
        for c in &mut p.controls {
            match &mut c.kind {
                ControlKind::Table(t) => {
                    for cell in &mut t.cells {
                        materialize(&mut cell.paras, pending);
                    }
                    if let Some(cap) = &mut t.caption {
                        materialize(&mut cap.paras, pending);
                    }
                }
                ControlKind::Object(o) => {
                    if let Some(s) = o.parts.iter_mut().find_map(|part| match part {
                        ObjectPart::Shape(s) => Some(s),
                        _ => None,
                    }) {
                        shape(s, pending);
                    }
                    if let Some(c) = o.parts.iter_mut().find_map(|part| match part {
                        ObjectPart::Caption(c) => Some(c),
                        _ => None,
                    }) {
                        materialize(&mut c.paras, pending);
                    }
                }
                ControlKind::Section(s) => {
                    for child in &mut s.children {
                        if let SectionChild::MasterPage(l) = child {
                            materialize(&mut l.paras, pending);
                        }
                    }
                }
                ControlKind::HeaderFooter(h) => materialize(&mut h.list.paras, pending),
                ControlKind::Note(n) => materialize(&mut n.list.paras, pending),
                _ => {}
            }
        }
    }
}

/// Writes the line segments of every paragraph laid out fresh (no source
/// bytes), from `layout`.
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
    // A paragraph laid out fresh (no source bytes) saves the lines it was
    // laid out with: stored lines left from before its edit would be
    // replayed on reopening. One that replayed its stored lines while a
    // list in it changed keeps where each stored line sits across the
    // column, which decides how lines beside floating tables go, and takes
    // the heights and places it was laid out with now.
    if para.src.is_none() {
        para.line_segs = segments(info, para, pal, top);
    } else if !para.is_clean() {
        let mut fresh = segments(info, para, pal, top);
        let same_lines = fresh.len() == para.line_segs.len()
            && fresh
                .iter()
                .zip(&para.line_segs)
                .all(|(a, b)| a.text_start == b.text_start);
        if same_lines {
            for (f, old) in fresh.iter_mut().zip(&para.line_segs) {
                f.col_start = old.col_start;
                f.width = old.width;
                f.flags = old.flags;
            }
        }
        para.line_segs = fresh;
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
            // A line of nothing but a flowing table stays beside the table,
            // as Hancom keeps a line it stored at no width.
            let flowing = para
                .controls
                .iter()
                .any(|c| c.object_common().is_some_and(|o| !o.treat_as_char()));
            let width = if flowing && !hm_layout::table::has_content(para, l) {
                0
            } else {
                l.seg_width
            };
            LineSeg {
                text_start: l.start,
                vpos,
                height: l.height,
                text_height: l.text_height,
                baseline: l.baseline,
                spacing: l.spacing,
                col_start: l.seg_x,
                width,
                flags: WHOLE_LINE | if indented { INDENTED } else { 0 },
            }
        })
        .collect()
}
