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
    let pending = selected_lines(source, layout, fonts)?;
    // Source spans must be cleared before the new records become the output's
    // membership. The old records were used above only through matched layout.
    let mut output = source.clone();
    output.convert_to(target);
    put_lines(&mut output, pending, |_| true);
    Ok(output)
}

/// Writes every changed paragraph's lines, in every list (headers, notes,
/// text boxes and master pages too), as a conversion selects them: a save
/// in the document's own format stores what a converted one does. A layout
/// that does not match the document (some paragraph's lines do not cover
/// its text) is an error, after [`fill`]'s lines for body and cells: the
/// caller lays the document out again.
pub fn fill_lists(
    doc: &mut Document,
    layout: &DocLayout,
    fonts: Option<&hm_fonts::FontSystem>,
) -> Result<(), String> {
    fill(doc, layout);
    let pending = selected_lines(doc, layout, fonts)?;
    put_lines(doc, pending, |p| !p.is_clean());
    Ok(())
}

/// Each paragraph's lines in [`materialize`] order, as the layout selects
/// them for `source`.
fn selected_lines(
    source: &Document,
    layout: &DocLayout,
    fonts: Option<&hm_fonts::FontSystem>,
) -> Result<Vec<Lines>, String> {
    let mut selected = ConversionLines::default();
    if source.sections.len() != layout.sections.len() {
        return Err("conversion layout has a different section count".into());
    }
    selected.note_splits(source, layout);
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
            // Lists no page shows: header/footer variants of the other
            // parity, master pages of pages the section lacks, notes and
            // other lists inside them, laid out at their own text width.
            let mut lists = Vec::new();
            unshown(&sec.paras, &mut lists);
            for list in sec
                .last_page_master
                .iter()
                .chain(sec.page_masters.iter().map(|(_, l)| l))
            {
                lists.push((list.paras.as_slice(), list.text_size));
                unshown(&list.paras, &mut lists);
            }
            for (paras, size) in lists {
                // Unchanged paragraphs keep their stored lines without one.
                if paras.iter().all(|p| {
                    p.is_clean()
                        || selected
                            .paras
                            .contains_key(&(p as *const Paragraph as usize))
                }) {
                    continue;
                }
                let (layouts, lines, _) = hm_layout::table::stack(
                    paras,
                    &source.info,
                    &mut styles,
                    hm_layout::para::ParaEnv {
                        column_width: size.map(|s| s.0).filter(|w| *w > 0).unwrap_or(page.body.w),
                        default_tab: sec
                            .definition()
                            .map(|d| d.default_tab as i32 / 2)
                            .unwrap_or(4000),
                    },
                    &mut hm_layout::heads::Heads::default(),
                );
                selected.list(&source.info, paras, &layouts, &|pi, li| {
                    cell_top(&lines, pi, li)
                })?;
            }
        }
    }
    // A document whose lines are composed fresh on opening, or a layout
    // made fresh, stores the layout's lines: the target format replays them.
    let keep = if layout.stored_lines {
        Keep::Stored
    } else {
        Keep::Fresh
    };
    let mut pending = Vec::new();
    for (si, sec) in source.sections.iter().enumerate() {
        selected.finish(
            &source.info,
            &sec.paras,
            &format!("section {si} body"),
            keep,
            &mut pending,
        )?;
        for list in sec
            .last_page_master
            .iter()
            .chain(sec.page_masters.iter().map(|(_, l)| l))
        {
            selected.finish(
                &source.info,
                &list.paras,
                &format!("section {si} master"),
                keep,
                &mut pending,
            )?;
        }
    }
    Ok(pending)
}

/// Gives the paragraphs `take` accepts their lines from `pending`.
fn put_lines(doc: &mut Document, pending: Vec<Lines>, take: impl Fn(&Paragraph) -> bool + Copy) {
    let mut pending = pending.into_iter();
    for sec in &mut doc.sections {
        materialize(&mut sec.paras, &mut pending, take);
        for list in sec
            .last_page_master
            .iter_mut()
            .chain(sec.page_masters.iter_mut().map(|(_, l)| l))
        {
            materialize(&mut list.paras, &mut pending, take);
        }
    }
}

#[derive(Default)]
struct ConversionLines {
    // References identify the actual source paragraph, including lists repeated
    // or split among page frames. They never escape this preparation call.
    paras: std::collections::HashMap<usize, SelectedLines>,
    /// Where each page after the first takes up a split table, by table:
    /// per cell, the table heights its lines resume from there.
    resumes: std::collections::HashMap<usize, Vec<Vec<i32>>>,
}

impl ConversionLines {
    /// The resumption points of the tables the body's pages split.
    fn note_splits(&mut self, source: &Document, layout: &DocLayout) {
        for page in &layout.pages {
            let Some(sec) = source.sections.get(page.section as usize) else {
                continue;
            };
            for pt in page.tables.iter().filter(|pt| pt.top > 0) {
                let Some(ControlKind::Table(t)) = sec
                    .paras
                    .get(pt.para as usize)
                    .and_then(|p| p.controls.get(pt.control as usize))
                    .map(|c| &c.kind)
                else {
                    continue;
                };
                let cells = self
                    .resumes
                    .entry(&**t as *const hm_model::Table as usize)
                    .or_insert_with(|| vec![Vec::new(); t.cells.len()]);
                for (k, at) in cells.iter_mut().enumerate() {
                    // A cell cut at the page end resumes at its first
                    // carried line; one running through the band at its top.
                    let carried = pt.carried.as_ref().and_then(|c| {
                        c.cells
                            .iter()
                            .find(|&&(c, _)| c as usize == k)
                            .map(|&(_, first)| first)
                    });
                    match carried {
                        Some(Some(first)) => at.push(first),
                        Some(None) => {}
                        None => at.push(pt.top),
                    }
                }
            }
        }
    }
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
            self.para(p, pl, &|li| top(pi, li));
            self.children(info, p, pl)?;
        }
        Ok(())
    }

    /// One laid-out instance of `p`. A list shown in several places (a
    /// header on every page, a master page, a note split across pages)
    /// gives one instance per place; they differ only where the places do
    /// (page widths, page number fields). The file keeps one set of lines:
    /// the first instance that replayed stored lines, else the first one.
    /// Places it does not fit lay the paragraph out again on reopening, as
    /// they did here.
    fn para(&mut self, p: &Paragraph, pl: &ParaLayout, top: &dyn Fn(usize) -> Option<i32>) {
        let selected = || SelectedLines {
            layout: pl.clone(),
            lines: pl
                .lines
                .iter()
                .enumerate()
                .map(|(li, l)| (li, (*l, top(li))))
                .collect(),
        };
        let key = p as *const Paragraph as usize;
        let Some(entry) = self.paras.get_mut(&key) else {
            self.paras.insert(key, selected());
            return;
        };
        // Repeated page fields can change glyph count and natural width
        // without changing the wire membership or stored line geometry.
        let same = entry.lines.len() == pl.lines.len()
            && entry.lines.values().zip(&pl.lines).all(|((old, _), line)| {
                *old == hm_layout::Line {
                    first_item: old.first_item,
                    end_item: old.end_item,
                    used: old.used,
                    ..*line
                }
            });
        let stored = |mut ls: std::slice::Iter<'_, hm_layout::Line>| ls.all(|l| l.stored);
        if same {
            for (li, (_, y)) in entry.lines.iter_mut() {
                if y.is_none() {
                    *y = top(*li);
                }
            }
        } else if stored(pl.lines.iter()) && !stored(entry.layout.lines.iter()) {
            *entry = selected();
        }
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
                // A cell's lines on a later page start its vertical
                // positions again from where the cell resumes there, as
                // Hancom stores them: the split replays where it fell.
                let resumes = self
                    .resumes
                    .get(&(&**t as *const hm_model::Table as usize))
                    .cloned()
                    .unwrap_or_default();
                for (k, (cell, cl)) in t.cells.iter().zip(&tl.cells).enumerate() {
                    let resumes = resumes.get(k).map(Vec::as_slice).unwrap_or(&[]);
                    self.list(info, &cell.paras, &cl.paras, &|pi, li| {
                        let y = cell_top(&cl.lines, pi, li)?;
                        let at = cl.split_y() + y;
                        Some(
                            resumes
                                .iter()
                                .copied()
                                .filter(|&r| r > cl.split_y() && r <= at)
                                .max()
                                .map_or(y, |r| at - r),
                        )
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
        keep: Keep,
        out: &mut Vec<Lines>,
    ) -> Result<(), String> {
        for (pi, p) in paras.iter().enumerate() {
            let path = format!("{path}/paragraph {pi}");
            out.push(self.segments(info, p, &path, keep)?);
            for (ci, c) in p.controls.iter().enumerate() {
                for (li, list) in crate::ops::control_lists(c).iter().enumerate() {
                    self.finish(
                        info,
                        list,
                        &format!("{path}/control {ci}/list {li}"),
                        keep,
                        out,
                    )?;
                }
            }
        }
        Ok(())
    }

    /// The lines the output stores for `p`.
    fn segments(
        &self,
        info: &DocInfo,
        p: &Paragraph,
        path: &str,
        keep: Keep,
    ) -> Result<Lines, String> {
        // An unchanged paragraph keeps its stored lines, or their absence:
        // the other format replays or refits them as this one did.
        if keep == Keep::Stored && p.is_clean() {
            return Ok((p.line_segs.clone(), Vec::new()));
        }
        // A list no layout reached keeps the lines its paragraphs have, as
        // a save in the document's own format does.
        let Some(entry) = self.paras.get(&(p as *const Paragraph as usize)) else {
            return Ok((p.line_segs.clone(), Vec::new()));
        };
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
                // A changed paragraph keeps where its stored lines sit across
                // the column, as `fill` does for a save in its own format.
                if line.stored || flow_host || (keep == Keep::Stored && p.src.is_some()) {
                    seg.col_start = old.col_start;
                    seg.width = old.width;
                    seg.flags = old.flags;
                }
            }
        }
        Ok((segs, tab_widths(p, &entry.layout)))
    }
}

/// A paragraph's line segments and its tabs' laid-out advances.
type Lines = (Vec<LineSeg>, Vec<(usize, i32)>);

/// Each tab's laid-out advance, by its wire position.
fn tab_widths(p: &Paragraph, pl: &ParaLayout) -> Vec<(usize, i32)> {
    let advances = &pl.measured.advances;
    if advances.len() != p.text.len() {
        return Vec::new();
    }
    p.units()
        .filter(|(_, u)| *u == hm_model::Unit::Inline(9))
        .map(|(pos, _)| (pos as usize, advances[pos as usize]))
        .collect()
}

/// Stores each tab's advance as its width (units 1 and 2 of the tab
/// control), as Hancom does: a reopened paragraph whose stored lines replay
/// takes its tabs' advances from there, and a tab saved without its width
/// closes up.
fn put_tab_widths(p: &mut Paragraph, tabs: &[(usize, i32)]) {
    for &(pos, w) in tabs {
        if let Some(units) = p.text.get_mut(pos + 1..pos + 3) {
            let w = w.max(0) as u32;
            units[0] = w as u16;
            units[1] = (w >> 16) as u16;
        }
    }
}

/// What a conversion stores as each paragraph's lines.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Keep {
    /// The layout broke every line fresh: its lines, for every paragraph.
    Fresh,
    /// Unchanged paragraphs keep their stored lines; changed ones take the
    /// layout's.
    Stored,
}

fn cell_top(lines: &[hm_layout::CellLine], pi: usize, li: usize) -> Option<i32> {
    lines
        .iter()
        .find(|l| l.para == pi as u32 && l.line == li as u32)
        .map(|l| l.y)
}

/// Lists of the controls in `paras` and in their lists, outer ones first,
/// with the text size the list stores.
fn unshown<'a>(paras: &'a [Paragraph], out: &mut Vec<(&'a [Paragraph], Option<(i32, i32)>)>) {
    for p in paras {
        for c in &p.controls {
            let sized: Vec<&hm_model::ParaList> = match &c.kind {
                ControlKind::HeaderFooter(h) => vec![&h.list],
                ControlKind::Note(n) => vec![&n.list],
                ControlKind::Section(s) => s
                    .children
                    .iter()
                    .filter_map(|k| match k {
                        hm_model::SectionChild::MasterPage(l) => Some(l),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            };
            if sized.is_empty() {
                for list in crate::ops::control_lists(c) {
                    out.push((list, None));
                    unshown(list, out);
                }
            } else {
                for list in sized {
                    out.push((&list.paras, list.text_size));
                    unshown(&list.paras, out);
                }
            }
        }
    }
}

fn text_boxes<'a>(s: &'a hm_model::Shape, out: &mut Vec<&'a hm_model::TextBox>) {
    out.extend(s.text());
    for c in s.children() {
        text_boxes(c, out);
    }
}

// Same list order as ops::control_lists, after convert_to has cleared sources.
fn materialize(
    paras: &mut [Paragraph],
    pending: &mut impl Iterator<Item = Lines>,
    take: impl Fn(&Paragraph) -> bool + Copy,
) {
    use hm_model::{ObjectPart, SectionChild, ShapePart};
    fn shape(
        s: &mut hm_model::Shape,
        pending: &mut impl Iterator<Item = Lines>,
        take: impl Fn(&Paragraph) -> bool + Copy,
    ) {
        for part in &mut s.parts {
            match part {
                ShapePart::Text(t) => materialize(&mut t.paras, pending, take),
                ShapePart::Child(s) => shape(s, pending, take),
                _ => {}
            }
        }
    }
    for p in paras {
        let (lines, tabs) = pending.next().expect("validated conversion paragraph");
        if take(p) {
            p.line_segs = lines;
            put_tab_widths(p, &tabs);
        }
        for c in &mut p.controls {
            match &mut c.kind {
                ControlKind::Table(t) => {
                    for cell in &mut t.cells {
                        materialize(&mut cell.paras, pending, take);
                    }
                    if let Some(cap) = &mut t.caption {
                        materialize(&mut cap.paras, pending, take);
                    }
                }
                ControlKind::Object(o) => {
                    if let Some(s) = o.parts.iter_mut().find_map(|part| match part {
                        ObjectPart::Shape(s) => Some(s),
                        _ => None,
                    }) {
                        shape(s, pending, take);
                    }
                    if let Some(c) = o.parts.iter_mut().find_map(|part| match part {
                        ObjectPart::Caption(c) => Some(c),
                        _ => None,
                    }) {
                        materialize(&mut c.paras, pending, take);
                    }
                }
                ControlKind::Section(s) => {
                    for child in &mut s.children {
                        if let SectionChild::MasterPage(l) = child {
                            materialize(&mut l.paras, pending, take);
                        }
                    }
                }
                ControlKind::HeaderFooter(h) => materialize(&mut h.list.paras, pending, take),
                ControlKind::Note(n) => materialize(&mut n.list.paras, pending, take),
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
    if para.src.is_none() || !para.is_clean() {
        put_tab_widths(para, &tab_widths(para, pal));
    }
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
        let edited = c.src.is_none();
        let ControlKind::Table(t) = &mut c.kind else {
            continue;
        };
        let Some(tl) = pal.table(ci as u32) else {
            continue;
        };
        if edited {
            fit_height(t, tl);
        }
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

/// An edited table saves the height it is laid out at, as Hancom does when
/// typing grows or shrinks a row; cells keep their stored minimum heights.
/// A height relative to the paper or page stays.
fn fit_height(t: &mut hm_model::Table, tl: &hm_layout::table::TableLayout) {
    if (t.header.common.attr >> 18) & 3 >= 2 {
        t.header.common.height = tl.height;
    }
}

/// [`fit_height`] for a converted copy `out` of `source`, whose edited
/// tables (no source bytes) take their laid-out heights, cells included.
pub fn fit_edited_tables(out: &mut Document, source: &Document, layout: &DocLayout) {
    fn walk(out: &mut [Paragraph], src: &[Paragraph], pals: &[ParaLayout]) {
        for ((o, s), pal) in out.iter_mut().zip(src).zip(pals) {
            for (ci, (oc, sc)) in o.controls.iter_mut().zip(&s.controls).enumerate() {
                let (ControlKind::Table(ot), ControlKind::Table(st)) = (&mut oc.kind, &sc.kind)
                else {
                    continue;
                };
                let Some(tl) = pal.table(ci as u32) else {
                    continue;
                };
                if sc.src.is_none() {
                    fit_height(ot, tl);
                }
                for ((ocell, scell), cl) in ot.cells.iter_mut().zip(&st.cells).zip(&tl.cells) {
                    walk(&mut ocell.paras, &scell.paras, &cl.paras);
                }
            }
        }
    }
    for ((o, s), sl) in out
        .sections
        .iter_mut()
        .zip(&source.sections)
        .zip(&layout.sections)
    {
        walk(&mut o.paras, &s.paras, &sl.paras);
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
