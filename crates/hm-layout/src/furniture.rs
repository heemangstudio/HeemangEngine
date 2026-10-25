//! Page furniture: page numbers, master pages, headers and footers, page
//! borders, column separators and the paragraph lists placed outside the
//! body flow.
//!
//! After a section's body is paginated, each page learns its number and
//! which furniture applies. Controls take effect on the page that holds
//! them: a header, footer or page number position stays in force until
//! another of its kind replaces it, a page hiding control affects only its
//! own page, and a new page number restarts counting there.

use hm_model::{
    BorderLine, ControlKind, DocInfo, Document, NumberKind, PageBorderFill, PageDef, PageSide,
    ParaList, Paragraph, Section, SectionDef, hide,
};

use crate::heads::{Heads, number};
use crate::measure::advance;
use crate::paginate::{Page, Rect};
use crate::para::{ParaEnv, ParaLayout};
use crate::script::script_of;
use crate::style::Styles;

/// What a frame holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameKind {
    MasterPage,
    Header,
    Footer,
    Footnotes,
    Endnotes,
}

/// One step from a table into a control of one of its cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellStep {
    /// Cell index in the table's stored order.
    pub cell: u32,
    /// Paragraph in the cell, and control in that paragraph.
    pub para: u32,
    pub control: u32,
}

/// Where a frame's paragraphs live in the document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ListRef {
    /// The list of control `control` in body paragraph `para` of `section`
    /// (a header, footer or note).
    Control {
        section: u32,
        para: u32,
        control: u32,
    },
    /// Master page `index` of a section's definition, in stored order, or
    /// the section's last-page master page ([`LAST_PAGE_MASTER`]).
    MasterPage { section: u32, index: u32 },
    /// An individual-page master, indexed in the section's stored list.
    PageMaster { section: u32, index: u32 },
    /// The list of a control inside table cells or text boxes (a note): the
    /// body control at `(section, para, control)` is a table or a drawing
    /// object, every step but the last leads into a nested one, and the
    /// last names the control.
    Cell {
        section: u32,
        para: u32,
        control: u32,
        steps: Box<[CellStep]>,
    },
}

/// The [`ListRef::MasterPage`] index of a section's last-page master page.
pub const LAST_PAGE_MASTER: u32 = u32::MAX;

impl ListRef {
    /// The paragraphs a reference names.
    pub fn paras<'a>(&self, doc: &'a Document) -> Option<&'a [Paragraph]> {
        match *self {
            ListRef::PageMaster { section, index } => doc
                .sections
                .get(section as usize)?
                .page_masters
                .get(index as usize)
                .map(|(_, l)| l.paras.as_slice()),
            ListRef::MasterPage { section, index } if index == LAST_PAGE_MASTER => doc
                .sections
                .get(section as usize)?
                .last_page_master
                .as_ref()
                .map(|l| l.paras.as_slice()),
            ListRef::MasterPage { section, index } => doc
                .sections
                .get(section as usize)?
                .definition()?
                .children
                .iter()
                .filter_map(|c| match c {
                    hm_model::SectionChild::MasterPage(l) => Some(l),
                    _ => None,
                })
                .nth(index as usize)
                .map(|l| l.paras.as_slice()),
            _ => list_of(doc, self).map(|l| l.paras.as_slice()),
        }
    }
}

/// A line of a frame paragraph placed on the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameLine {
    pub para: u32,
    pub line: u32,
    /// Left edge of the frame's column and top of the line, page coordinates.
    pub x: i32,
    pub y: i32,
}

/// A paragraph list laid out on a page outside the body flow.
#[derive(Clone, Debug)]
pub struct Frame {
    pub kind: FrameKind,
    pub list: ListRef,
    /// The frame's area on the page.
    pub rect: Rect,
    /// Layouts of the list's paragraphs.
    pub paras: Vec<ParaLayout>,
    pub lines: Vec<FrameLine>,
    /// Floating objects of the list's paragraphs (`para` indexes the list).
    pub objects: Vec<crate::objects::Placement>,
    /// Tables of the list's paragraphs that are not set as characters, at
    /// their anchored place (`para` indexes the list).
    pub tables: Vec<crate::table::PlacedTable>,
    /// Height of the content: the last line's bottom from the first line's
    /// top.
    pub content: i32,
}

impl Frame {
    /// Moves the frame and everything in it down by `dy`.
    pub(crate) fn shift(&mut self, dy: i32) {
        self.rect.y += dy;
        self.lines.iter_mut().for_each(|l| l.y += dy);
        self.objects.iter_mut().for_each(|o| o.rect.y += dy);
        self.tables.iter_mut().for_each(|t| t.y += dy);
    }

    /// Moves the frame and everything in it right by `dx`.
    pub(crate) fn shift_x(&mut self, dx: i32) {
        self.rect.x += dx;
        self.lines.iter_mut().for_each(|l| l.x += dx);
        self.objects.iter_mut().for_each(|o| o.rect.x += dx);
        self.tables.iter_mut().for_each(|t| t.x += dx);
    }
}

/// A straight separator: between columns, or above notes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rule {
    pub x1: i32,
    pub y1: i32,
    pub x2: i32,
    pub y2: i32,
    pub line: BorderLine,
}

/// Generated text drawn on the page, such as the page number.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mark {
    /// `(character, style, x)`, x in page coordinates.
    pub glyphs: Vec<(char, u32, i32)>,
    pub baseline: i32,
}

/// The border and background of a page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageBorder {
    /// The border's box.
    pub rect: Rect,
    /// The area the background fills.
    pub fill: Rect,
    /// 1-based border fill id.
    pub border_fill: u16,
    /// Draw the lines and the background (page hiding can drop either).
    pub lines: bool,
    pub background: bool,
}

/// Furniture in force while pages are decorated; it carries from one
/// section to the next.
#[derive(Default)]
pub(crate) struct State {
    /// The last page number given.
    page: u32,
    headers: [Option<ListAt>; 3],
    footers: [Option<ListAt>; 3],
    /// Master pages by side (both, even, odd) as `(section, index)`: a
    /// section's master page goes on in the sections after it until one of
    /// them sets that side again.
    masters: [Option<(u32, u32)>; 3],
    page_number: Option<hm_model::PageNumberPos>,
    /// The last line number given, and whether any section numbered lines.
    line_number: u32,
    numbered_lines: bool,
}

impl State {
    /// The number of the last page decorated, 0 before the first.
    pub(crate) fn last_number(&self) -> u32 {
        self.page
    }
}

#[derive(Clone)]
struct ListAt {
    list: ListRef,
}

fn side_index(s: PageSide) -> usize {
    match s {
        PageSide::Both => 0,
        PageSide::Even => 1,
        PageSide::Odd => 2,
    }
}

/// A control's effect, with the page that holds it.
enum Event<'a> {
    Header(PageSide, ListRef, bool),
    PageNumber(&'a hm_model::PageNumberPos),
    NewPage(u32),
    Hide(u32),
    /// Odd/even page adjustment: 1 even, 2 odd.
    Parity(u32),
}

/// Pages (indices into the section's pages) of every furniture control.
fn events<'a>(
    section: u32,
    sec: &'a Section,
    paras: &[ParaLayout],
    pages: &[Page],
    hwp5: bool,
) -> Vec<Vec<Event<'a>>> {
    let line_page = line_pages(paras, pages);
    let mut out: Vec<Vec<Event<'a>>> = (0..pages.len()).map(|_| Vec::new()).collect();
    for (pi, para) in sec.paras.iter().enumerate() {
        if para.controls.is_empty() {
            continue;
        }
        let layout = &paras[pi];
        for (pos, unit) in para.units() {
            let hm_model::Unit::Extended { index: ci, .. } = unit else {
                continue;
            };
            let Some(c) = para.controls.get(ci as usize) else {
                continue;
            };
            let line = layout
                .lines
                .iter()
                .rposition(|l| l.start <= pos)
                .unwrap_or(0);
            let page = line_page[pi].get(line).copied().unwrap_or(0);
            let list = ListRef::Control {
                section,
                para: pi as u32,
                control: ci,
            };
            // Furniture controls inside a table's cells take effect on the
            // page that shows their cell.
            if let Some(t) = c.as_table() {
                let mut found = Vec::new();
                cell_furniture(t, layout.table(ci), &[], None, &mut found);
                for (steps, y, cc, cp) in found {
                    let page = pages
                        .iter()
                        .position(|pg| {
                            pg.tables.iter().any(|f| {
                                f.para == pi as u32 && f.control == ci && f.top <= y && y < f.bottom
                            })
                        })
                        .unwrap_or(page);
                    let list = ListRef::Cell {
                        section,
                        para: pi as u32,
                        control: ci,
                        steps: steps.into(),
                    };
                    if let Some(ev) = event(cc, cp, list, hwp5)
                        && let Some(v) = out.get_mut(page)
                    {
                        v.push(ev);
                    }
                }
                continue;
            }
            if let Some(ev) = event(c, para, list, hwp5)
                && let Some(v) = out.get_mut(page)
            {
                v.push(ev);
            }
        }
    }
    out
}

/// The effect of a furniture control found in paragraph `para`.
fn event<'a>(
    c: &'a hm_model::Control,
    para: &Paragraph,
    list: ListRef,
    hwp5: bool,
) -> Option<Event<'a>> {
    Some(match &c.kind {
        ControlKind::HeaderFooter(h) => Event::Header(h.side(), list, h.footer),
        ControlKind::PageNumber(p) => Event::PageNumber(p),
        ControlKind::PageHide(h) => Event::Hide(h.attr),
        ControlKind::PageParity(p) => Event::Parity(p.attr & 3),
        _ => Event::NewPage(new_page_number(c, para, hwp5)?),
    })
}

/// A furniture control inside table cells: its path, the table y of its
/// cell, the control and its paragraph.
type CellFurniture<'a> = (Vec<CellStep>, i32, &'a hm_model::Control, &'a Paragraph);

/// The headers, footers, page numbers and page hiding inside the cells of
/// `t`, nested tables included; a nested table's controls count at the y of
/// the outer cell.
fn cell_furniture<'a>(
    t: &'a hm_model::Table,
    tl: Option<&crate::table::TableLayout>,
    prefix: &[CellStep],
    at: Option<i32>,
    out: &mut Vec<CellFurniture<'a>>,
) {
    for (ci, cell) in t.cells.iter().enumerate() {
        let cl = tl.and_then(|tl| tl.cells.get(ci));
        let y = at.unwrap_or_else(|| cl.map_or(0, |cl| cl.y));
        for (pi, p) in cell.paras.iter().enumerate() {
            for (k, c) in p.controls.iter().enumerate() {
                let mut steps = prefix.to_vec();
                steps.push(CellStep {
                    cell: ci as u32,
                    para: pi as u32,
                    control: k as u32,
                });
                match &c.kind {
                    ControlKind::HeaderFooter(_)
                    | ControlKind::PageNumber(_)
                    | ControlKind::PageHide(_)
                    | ControlKind::NewNumber(_) => out.push((steps, y, c, p)),
                    ControlKind::Table(nt) => {
                        let ntl = cl
                            .and_then(|cl| cl.paras.get(pi))
                            .and_then(|pl| pl.table(k as u32));
                        cell_furniture(nt, ntl, &steps, Some(y), out);
                    }
                    _ => {}
                }
            }
        }
    }
}

/// The number of a section's first page: the section's own start number,
/// else the document's start for the first numbered page, else one past
/// the previous page, skipping one more when the section asks to start on
/// a page of the other parity (attribute bits 20-21: 1 even, 2 odd).
pub(crate) fn first_number(def: &SectionDef, info: &DocInfo, previous: u32) -> u32 {
    if def.page_number > 0 {
        def.page_number as u32
    } else if previous == 0 {
        (info.properties.page_start as u32).max(1)
    } else {
        with_parity(previous + 1, (def.attr >> 20) & 3)
    }
}

/// Page number `n` after an odd/even page adjustment (`pgct`, 1 even,
/// 2 odd): Hancom skips one number when the page's number has the other
/// parity, without adding a page.
pub(crate) fn with_parity(n: u32, kind: u32) -> u32 {
    match kind {
        1 if n % 2 == 1 => n + 1,
        2 if n % 2 == 0 => n + 1,
        _ => n,
    }
}

/// The number a new page number control gives the page that holds it.
pub(crate) fn new_page_number(c: &hm_model::Control, para: &Paragraph, hwp5: bool) -> Option<u32> {
    match &c.kind {
        ControlKind::NewNumber(n)
            if n.kind() == NumberKind::Page && (!hwp5 || page_code(c, para)) =>
        {
            Some(n.number as u32)
        }
        _ => None,
    }
}

/// True when control `c` of `para` sits in the text under code 21, the
/// code of page controls. Hancom ignores a new number some other writers
/// store under another code.
fn page_code(c: &hm_model::Control, para: &Paragraph) -> bool {
    para.units().any(|(_, u)| {
        matches!(u, hm_model::Unit::Extended { code: 21, index }
            if para.controls.get(index as usize).is_some_and(|k| std::ptr::eq(k, c)))
    })
}

/// The page (index into the section's pages) of every line of every
/// paragraph.
fn line_pages(paras: &[ParaLayout], pages: &[Page]) -> Vec<Vec<usize>> {
    let mut line_page: Vec<Vec<usize>> = paras.iter().map(|p| vec![0; p.lines.len()]).collect();
    for (k, page) in pages.iter().enumerate() {
        for pl in &page.lines {
            if let Some(slot) = line_page
                .get_mut(pl.para as usize)
                .and_then(|v| v.get_mut(pl.line as usize))
            {
                *slot = k;
            }
        }
    }
    line_page
}

/// True when paragraphs show a page number through an automatic number,
/// in their text or inside their tables and shapes.
pub(crate) fn shows_page_number(paras: &[Paragraph]) -> bool {
    paras
        .iter()
        .flat_map(|p| &p.controls)
        .any(|c| match &c.kind {
            ControlKind::AutoNumber(a) => a.kind() == NumberKind::Page,
            ControlKind::Table(t) => t.cells.iter().any(|cell| shows_page_number(&cell.paras)),
            _ => c
                .as_object()
                .and_then(|o| o.shape())
                .is_some_and(shape_shows_page_number),
        })
}

fn shape_shows_page_number(s: &hm_model::Shape) -> bool {
    s.text().is_some_and(|t| shows_page_number(&t.paras))
        || s.children().any(shape_shows_page_number)
}

/// A frame for a list in `rect`, its content aligned by the list's
/// vertical alignment.
#[allow(clippy::too_many_arguments)]
pub(crate) fn frame(
    kind: FrameKind,
    list: ListRef,
    l: &ParaList,
    rect: Rect,
    page: (i32, i32, Rect),
    info: &DocInfo,
    styles: &mut Styles<'_>,
    env: ParaEnv,
) -> Frame {
    let (paper, body) = ((page.0, page.1), page.2);
    let width = l
        .text_size
        .map(|(w, _)| w)
        .filter(|&w| w > 0)
        .unwrap_or(rect.w);
    let (paras, lines, content) = crate::table::stack(
        &l.paras,
        info,
        styles,
        ParaEnv {
            column_width: width & !3,
            ..env
        },
        &mut Heads::default(),
    );
    let shift = match l.vertical_align() {
        1 => (rect.h - content) / 2,
        2 => rect.h - content,
        _ => 0,
    };
    let lines: Vec<FrameLine> = lines
        .into_iter()
        .map(|l| FrameLine {
            para: l.para,
            line: l.line,
            x: rect.x,
            y: rect.y + shift + l.y,
        })
        .collect();
    // Objects anchor to the frame as their column, paragraph-relative ones
    // at their paragraph's first line.
    let mut objects = Vec::new();
    let mut tables = Vec::new();
    for (pi, p) in l.paras.iter().enumerate() {
        let top = lines
            .iter()
            .find(|fl| fl.para == pi as u32)
            .map_or(rect.y + shift, |fl| fl.y);
        let left = info
            .para_shape(p.header.para_shape as u32)
            .map_or(0, |s| s.left(1000));
        let anchor = crate::objects::Anchor {
            paper,
            // Furniture uses the body span horizontally. A header's
            // top-aligned PAPER objects use the frame top vertically.
            paper_x: matches!(kind, FrameKind::Header | FrameKind::Footer)
                .then_some((body.x, body.w)),
            paper_y: matches!(kind, FrameKind::Header).then_some(rect.y),
            body,
            column: (rect.x, rect.w),
            para_top: top,
            para_left: rect.x + left,
        };
        // A floating table advances the list's text cursor. A later
        // paragraph-relative drawing hangs from that cursor, even when
        // the host line has no printable text and stays above the table.
        let mut cursor = top;
        let mut anchors = vec![top; p.controls.len()];
        for (_, unit) in p.units() {
            let hm_model::Unit::Extended { index, .. } = unit else {
                continue;
            };
            let Some(c) = p.controls.get(index as usize) else {
                continue;
            };
            anchors[index as usize] = cursor;
            if crate::table::flows(c)
                && let (Some(o), Some(tl)) = (c.object_common(), paras[pi].table(index))
            {
                let [_, _, mt, mb] = o.margins.map(i32::from);
                cursor = cursor.max(top + o.y_offset.max(0) + mt + tl.outer_size().1 + mb);
            }
        }
        for (index, top) in anchors.into_iter().enumerate() {
            if let Some(at) = crate::objects::place_one(
                p,
                pi as u32,
                index as u32,
                &crate::objects::Anchor {
                    para_top: top,
                    ..anchor
                },
            ) {
                objects.push(at);
            }
        }
        for (k, c) in p.controls.iter().enumerate() {
            let (Some(t), Some(tl)) = (c.as_table(), paras[pi].table(k as u32)) else {
                continue;
            };
            let o = t.header.common;
            if o.treat_as_char() {
                continue;
            }
            let [ml, mr, mt, mb] = o.margins.map(i32::from);
            let outer = hm_model::ObjectCommon {
                width: ml + o.width + mr,
                height: mt + o.height + mb,
                ..o
            };
            let (x, y, _, _) = crate::objects::frame(&outer, &anchor);
            let (gx, gy) = tl.grid_offset();
            tables.push(crate::table::PlacedTable {
                para: pi as u32,
                control: k as u32,
                x: x + ml + gx,
                y: y + mt + gy,
                top: 0,
                bottom: tl.height,
                header: 0,
                caption: tl.caption.is_some(),
                carried: None,
                cut: None,
            });
        }
    }
    Frame {
        kind,
        list,
        rect,
        paras,
        lines,
        objects,
        tables,
        content,
    }
}

/// The char shape of the style named "Page Number" ("쪽 번호"), which
/// Hancom draws page numbers with.
fn page_number_shape(info: &DocInfo) -> Option<u32> {
    info.styles
        .iter()
        .find(|s| {
            s.english_name.to_string_lossy() == "Page Number"
                || s.local_name.to_string_lossy() == "쪽 번호"
        })
        .map(|s| s.char_shape as u32)
        .filter(|&id| info.char_shape(id).is_some())
}

/// Hancom's built-in page number style, for documents without one:
/// 함초롬돋움 at 10 pt.
const PAGE_NUMBER_FACE: &str = "함초롬돋움";
const PAGE_NUMBER_SIZE: i32 = 1000;

/// The page number text: the side decoration around the number, as
/// `"- 3 -"`, with the prefix and suffix next to the number.
fn page_number_text(p: &hm_model::PageNumberPos, n: u32) -> String {
    let mut s = String::new();
    let ch = |c: u16| char::from_u32(c as u32).filter(|c| *c != '\0');
    let side = ch(p.side);
    if let Some(c) = side {
        s.push(c);
        s.push(' ');
    }
    if let Some(c) = ch(p.prefix) {
        s.push(c);
    }
    s.push_str(&number(n, p.shape()));
    if let Some(c) = ch(p.suffix) {
        s.push(c);
    }
    if let Some(c) = side {
        s.push(' ');
        s.push(c);
    }
    s
}

/// The page number mark. Bottom positions sit on the bottom margin's edge,
/// raised by the face's descent; top positions hang from the top margin's
/// edge by the face's ascent.
fn page_number_mark(
    p: &hm_model::PageNumberPos,
    n: u32,
    page: &Page,
    def: &PageDef,
    info: &DocInfo,
    styles: &mut Styles<'_>,
) -> Option<Mark> {
    let pos = p.position();
    if pos == 0 {
        return None;
    }
    let cs = page_number_shape(info);
    let builtin = hm_model::CharShape {
        base_size: PAGE_NUMBER_SIZE,
        ratios: [100; 7],
        rel_sizes: [100; 7],
        ..Default::default()
    };
    let text = page_number_text(p, n);
    let mut glyphs = Vec::new();
    let mut x = 0;
    let (mut ascent, mut descent) = (0, 0);
    for c in text.chars() {
        let script = script_of(c).unwrap_or(hm_model::Script::Latin);
        let si = match cs {
            Some(cs) => styles.style_for(cs, script, Some(c)),
            None => styles.builtin_style(PAGE_NUMBER_FACE, script, c, PAGE_NUMBER_SIZE),
        };
        let st = styles.get(si).clone();
        let face = styles.face(si).cloned();
        if let Some(f) = &face {
            let em = f.units_per_em.max(1) as i32;
            ascent = ascent.max(hm_model::units::mul_div(st.size, f.ascent, em));
            descent = descent.max(hm_model::units::mul_div(st.size, f.descent, em));
        }
        glyphs.push((c, si, x));
        // Blanks take Hancom's blank width, not the font's space.
        let shape = cs.and_then(|cs| info.char_shape(cs)).unwrap_or(&builtin);
        x += match c {
            ' ' => crate::measure::blank_advance(shape, crate::measure::Blank::Ordinary),
            _ => advance(&st, face.as_deref(), c),
        };
    }
    let width = x;
    let body = page.body;
    let odd = n % 2 == 1;
    let horizontal = match pos {
        1 | 4 => 0,
        2 | 5 => 1,
        3 | 6 => 2,
        7 | 8 => {
            if odd {
                2
            } else {
                0
            }
        }
        _ => {
            if odd {
                0
            } else {
                2
            }
        }
    };
    let x0 = match horizontal {
        0 => body.x,
        1 => body.x + (body.w - width) / 2,
        _ => body.x + body.w - width,
    };
    let top = matches!(pos, 1 | 2 | 3 | 7 | 9);
    let baseline = if top {
        sat(def.margin_top) + ascent
    } else if def.margin_footer == 0 {
        // Without a footer area Hancom sets a bottom number halfway down
        // the bottom margin.
        page.height - sat(def.margin_bottom) / 2 - descent
    } else {
        page.height - sat(def.margin_bottom) - descent
    };
    for g in &mut glyphs {
        g.2 += x0;
    }
    Some(Mark { glyphs, baseline })
}

fn sat(v: u32) -> i32 {
    i32::try_from(v).unwrap_or(i32::MAX)
}

/// The border of a page from the section's page border records (both,
/// even and odd pages in stored order). An even or odd record applies when
/// it draws something; otherwise the page takes the record for both.
fn page_border(
    def: &SectionDef,
    info: &DocInfo,
    page: &Page,
    n: u32,
    page_def: &PageDef,
) -> Option<PageBorder> {
    let borders: Vec<_> = def.page_borders().collect();
    let draws = |b: &&PageBorderFill| -> bool {
        b.border_fill > 0
            && info
                .border_fills
                .get(b.border_fill as usize - 1)
                .is_some_and(|f| {
                    f.sides.iter().any(|s| s.kind != 0)
                        || f.fill.as_ref().is_some_and(|f| f.kind != 0)
                })
    };
    let own = borders.get(if n % 2 == 0 { 1 } else { 2 }).copied();
    let b = own
        .filter(draws)
        .or(borders.first().copied())
        .filter(draws)?;
    let [l, r, t, bt] = b.offsets.map(i32::from);
    // Bit 0 set measures from the body; clear, from the paper edges. Hancom
    // saves 쪽 기준 with the bit set (HWPX `textBorder="PAPER"`).
    let rect = if b.attr & 1 == 0 {
        Rect {
            x: l,
            y: t,
            w: page.width - l - r,
            h: page.height - t - bt,
        }
    } else {
        // Around the body, optionally taking in the header and footer
        // areas, pushed out by the offsets.
        let mut top = page.body.y;
        let mut bottom = page.body.y + page.body.h;
        if b.attr & 2 != 0 {
            top -= sat(page_def.margin_header);
        }
        if b.attr & 4 != 0 {
            bottom += sat(page_def.margin_footer);
        }
        Rect {
            x: page.body.x - l,
            y: top - t,
            w: page.body.w + l + r,
            h: bottom - top + t + bt,
        }
    };
    let fill = match (b.attr >> 3) & 3 {
        0 => Rect {
            x: 0,
            y: 0,
            w: page.width,
            h: page.height,
        },
        1 => page.body,
        _ => rect,
    };
    Some(PageBorder {
        rect,
        fill,
        border_fill: b.border_fill,
        lines: true,
        background: true,
    })
}

/// Decorates the pages of one section: numbers, master pages, headers,
/// footers, page numbers and borders.
#[allow(clippy::too_many_arguments)]
pub(crate) fn decorate(
    doc: &Document,
    section: u32,
    paras: &[ParaLayout],
    pages: &mut [Page],
    styles: &mut Styles<'_>,
    state: &mut State,
    default_tab: i32,
) {
    let sec = &doc.sections[section as usize];
    let Some(def) = sec.definition() else {
        return;
    };
    if let Some(shape) = &def.line_numbers {
        let first = !state.numbered_lines;
        state.numbered_lines = true;
        crate::linenum::number_lines(shape, paras, pages, styles, &mut state.line_number, first);
    }
    let page_def = def.page_def().cloned().unwrap_or_default();
    let hwp5 = doc.meta.format != hm_model::Format::Hwpx;
    let events = events(section, sec, paras, pages, hwp5);
    let masters = def.master_pages();
    for (index, (side, _)) in masters.iter().enumerate() {
        state.masters[side_index(*side)] = Some((section, index as u32));
        // A master page for both sides replaces the one-sided ones.
        if *side == PageSide::Both {
            state.masters[1] = None;
            state.masters[2] = None;
        }
    }
    let info = &doc.info;
    let env = ParaEnv {
        column_width: 0,
        default_tab,
    };
    let pages_len = pages.len();
    for (k, page) in pages.iter_mut().enumerate() {
        // Numbering: a section may restart, a new number control restarts
        // on its page.
        let mut n = if k == 0 {
            first_number(def, info, state.page)
        } else {
            state.page + 1
        };
        let mut hidden = if k == 0 { def.attr & 0x3F } else { 0 };
        for ev in &events[k] {
            match ev {
                &Event::Header(side, ref list, footer) => {
                    let slot = if footer {
                        &mut state.footers
                    } else {
                        &mut state.headers
                    };
                    slot[side_index(side)] = Some(ListAt { list: list.clone() });
                    // A header for both sides replaces the one-sided ones.
                    if side == PageSide::Both {
                        slot[1] = None;
                        slot[2] = None;
                    }
                }
                Event::PageNumber(p) => state.page_number = Some((*p).clone()),
                &Event::NewPage(v) => n = v,
                &Event::Hide(bits) => hidden |= bits,
                &Event::Parity(kind) => n = with_parity(n, kind),
            }
        }
        state.page = n;
        page.number = n;
        // Automatic page numbers in the page's furniture show this page.
        styles.page_number = n;
        let side = if n % 2 == 0 {
            PageSide::Even
        } else {
            PageSide::Odd
        };
        if hidden & hide::MASTER == 0 {
            // The section's last page takes its last-page master page in
            // place of the others.
            let last = sec
                .last_page_master
                .as_ref()
                .filter(|_| k + 1 == pages_len)
                .map(|l| (LAST_PAGE_MASTER as usize, l));
            let chosen = last.map(|(i, l)| (section, i as u32, l)).or_else(|| {
                let (s, i) = state.masters[side_index(side)].or(state.masters[0])?;
                let list = doc.sections[s as usize]
                    .definition()?
                    .master_pages()
                    .get(i as usize)?
                    .1;
                Some((s, i, list))
            });
            let chosen = sec
                .page_masters
                .iter()
                .enumerate()
                .find(|(_, (p, _))| *p as usize == k + 1)
                .map(|(index, (_, list))| {
                    (
                        ListRef::PageMaster {
                            section,
                            index: index as u32,
                        },
                        list,
                    )
                })
                .or_else(|| {
                    chosen.map(|(section, index, list)| {
                        (ListRef::MasterPage { section, index }, list)
                    })
                });
            if let Some((r, list)) = chosen {
                let geometry = (page.width, page.height, page.body);
                page.frames.push(frame(
                    FrameKind::MasterPage,
                    r,
                    list,
                    page.body,
                    geometry,
                    info,
                    styles,
                    env,
                ));
            }
        }
        let header_area = Rect {
            x: page.body.x,
            y: sat(page_def.margin_top),
            w: page.body.w,
            h: sat(page_def.margin_header),
        };
        let footer_area = Rect {
            x: page.body.x,
            y: page.height - sat(page_def.margin_bottom) - sat(page_def.margin_footer),
            w: page.body.w,
            h: sat(page_def.margin_footer),
        };
        for (footer, slots, area, bit) in [
            (false, &state.headers, header_area, hide::HEADER),
            (true, &state.footers, footer_area, hide::FOOTER),
        ] {
            if hidden & bit != 0 {
                continue;
            }
            let pick = slots[side_index(side)].as_ref().or(slots[0].as_ref());
            let Some(at) = pick else { continue };
            let Some(list) = list_of(doc, &at.list) else {
                continue;
            };
            let kind = if footer {
                FrameKind::Footer
            } else {
                FrameKind::Header
            };
            let geometry = (page.width, page.height, page.body);
            page.frames.push(frame(
                kind,
                at.list.clone(),
                list,
                area,
                geometry,
                info,
                styles,
                env,
            ));
        }
        if hidden & hide::PAGE_NUMBER == 0
            && let Some(p) = &state.page_number
            && let Some(m) = page_number_mark(p, n, page, &page_def, info, styles)
        {
            page.marks.push(m);
        }
        if let Some(mut b) = page_border(def, info, page, n, &page_def) {
            b.lines = hidden & hide::BORDER == 0;
            b.background = hidden & hide::FILL == 0;
            // Bits 8 and 9: border and background on the section's first
            // page only.
            if k > 0 {
                b.lines &= def.attr & (1 << 8) == 0;
                b.background &= def.attr & (1 << 9) == 0;
            }
            if b.lines || b.background {
                page.border = Some(b);
            }
        }
    }
    styles.page_number = 0;
}

/// The list of a header, footer or note a reference names.
fn list_of<'a>(doc: &'a Document, r: &ListRef) -> Option<&'a ParaList> {
    let (section, para, control, steps) = match r {
        ListRef::Control {
            section,
            para,
            control,
        } => (*section, *para, *control, &[][..]),
        ListRef::Cell {
            section,
            para,
            control,
            steps,
        } => (*section, *para, *control, &steps[..]),
        ListRef::MasterPage { .. } | ListRef::PageMaster { .. } => return None,
    };
    let mut c = doc
        .sections
        .get(section as usize)?
        .paras
        .get(para as usize)?
        .controls
        .get(control as usize)?;
    for step in steps {
        // A step leads into a table's cell or a drawing object's text box.
        let paras = match c.as_table() {
            Some(t) => &t.cells.get(step.cell as usize)?.paras,
            None => {
                &crate::notes::text_boxes(c.as_object()?)
                    .get(step.cell as usize)?
                    .paras
            }
        };
        c = paras
            .get(step.para as usize)?
            .controls
            .get(step.control as usize)?;
    }
    match &c.kind {
        ControlKind::HeaderFooter(h) => Some(&h.list),
        ControlKind::Note(n) => Some(&n.list),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hm_model::SectionChild;

    #[test]
    fn a_top_paper_drawing_in_a_header_uses_the_header_origin() {
        let mut list = ParaList {
            paras: vec![Paragraph {
                text: vec![11, 0, 0, 0, 0, 0, 0, 11, hm_model::PARA_END],
                controls: vec![hm_model::Control {
                    kind: ControlKind::Object(Box::new(hm_model::ObjectControl {
                        id: hm_model::CtrlId::SHAPE,
                        header: hm_model::ObjectHeader {
                            common: hm_model::ObjectCommon {
                                attr: (4 << 15) | (2 << 18) | (3 << 21),
                                width: 4000,
                                height: 1000,
                                y_offset: 250,
                                ..Default::default()
                            },
                            ..Default::default()
                        },
                        parts: Vec::new(),
                    })),
                    src: None,
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let info = DocInfo::default();
        let fonts = hm_fonts::FontSystem::new();
        let mut styles = Styles::new(&info, &fonts);
        for (y, kind, align, expected) in [
            (2000, FrameKind::Header, 0, 2250),
            (4000, FrameKind::Header, 0, 4250),
            (2000, FrameKind::Footer, 0, 250),
            (2000, FrameKind::MasterPage, 0, 250),
            (2000, FrameKind::Header, 1, 9750),
            (2000, FrameKind::Header, 2, 18750),
        ] {
            let ControlKind::Object(object) = &mut list.paras[0].controls[0].kind else {
                unreachable!()
            };
            object.header.common.attr = (object.header.common.attr & !(7 << 5)) | (align << 5);
            let rect = Rect {
                x: 1000,
                y,
                w: 10000,
                h: 3000,
            };
            let result = frame(
                kind,
                ListRef::Control {
                    section: 0,
                    para: 0,
                    control: 0,
                },
                &list,
                rect,
                (
                    12000,
                    20000,
                    Rect {
                        x: 1000,
                        y: 5000,
                        w: 10000,
                        h: 10000,
                    },
                ),
                &info,
                &mut styles,
                ParaEnv {
                    column_width: 10000,
                    default_tab: 4000,
                },
            );
            assert_eq!(result.objects.len(), 1);
            assert_eq!(
                result.objects[0].rect.y, expected,
                "{kind:?} alignment {align}"
            );
        }
    }

    #[test]
    fn frame_drawings_after_a_flowing_table_use_the_advanced_cursor() {
        use hm_model::{Cell, Control, ObjectCommon, ObjectControl, ObjectHeader, Table};
        let common = ObjectCommon {
            attr: (2 << 3) | (1 << 21),
            width: 4000,
            height: 3000,
            ..Default::default()
        };
        let table = Control {
            kind: ControlKind::Table(Box::new(Table {
                header: ObjectHeader {
                    common,
                    ..Default::default()
                },
                rows: 1,
                cols: 1,
                cells: vec![Cell {
                    width: 4000,
                    height: 3000,
                    ..Default::default()
                }],
                ..Default::default()
            })),
            src: None,
        };
        let drawing = Control {
            kind: ControlKind::Object(Box::new(ObjectControl {
                id: hm_model::CtrlId::SHAPE,
                parts: Vec::new(),
                header: ObjectHeader {
                    common: ObjectCommon {
                        attr: (2 << 3) | (3 << 21),
                        y_offset: -3000,
                        ..common
                    },
                    ..Default::default()
                },
            })),
            src: None,
        };
        let mut list = ParaList {
            paras: vec![Paragraph {
                text: vec![
                    11,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    11,
                    11,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    11,
                    hm_model::PARA_END,
                ],
                controls: vec![table, drawing],
                ..Default::default()
            }],
            ..Default::default()
        };
        let info = DocInfo::default();
        let fonts = hm_fonts::FontSystem::new();
        let mut styles = Styles::new(&info, &fonts);
        let rect = Rect {
            x: 0,
            y: 1000,
            w: 10000,
            h: 10000,
        };
        let mut place = |list: &ParaList| {
            frame(
                FrameKind::Header,
                ListRef::Control {
                    section: 0,
                    para: 0,
                    control: 0,
                },
                list,
                rect,
                (10000, 20000, rect),
                &info,
                &mut styles,
                ParaEnv {
                    column_width: 10000,
                    default_tab: 4000,
                },
            )
        };
        assert_eq!(place(&list).objects[0].rect.y, 1000);
        list.paras[0].controls.swap(0, 1);
        assert_eq!(place(&list).objects[0].rect.y, -2000);
    }

    #[test]
    fn individual_masters_replace_the_regular_master_on_their_section_page() {
        let master = ParaList {
            paras: vec![Paragraph {
                text: vec![hm_model::PARA_END],
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut first = master.paras[0].clone();
        first.text = vec![2, 0, 0, 0, 0, 0, 0, 2, hm_model::PARA_END];
        first.controls.push(hm_model::Control {
            kind: ControlKind::Section(Box::new(SectionDef {
                attr: 1 << 29,
                children: vec![
                    SectionChild::Page(PageDef::default()),
                    SectionChild::MasterPage(master.clone()),
                ],
                ..Default::default()
            })),
            src: None,
        });
        let mut next = master.paras[0].clone();
        next.header.break_kind = 4;
        let doc = Document {
            meta: hm_model::DocMeta {
                format: hm_model::Format::Hwp5,
                version: Default::default(),
                flags: 0,
            },
            info: DocInfo::default(),
            sections: vec![Section {
                paras: vec![first, next.clone(), next],
                page_masters: vec![(2, master.clone())],
                last_page_master: Some(master),
                ..Default::default()
            }],
            source: None,
            bins: None,
            diagnostics: Vec::new(),
        };
        let layout = crate::layout(&doc, &hm_fonts::FontSystem::new());
        let masters: Vec<_> = layout.pages.iter().map(|p| &p.frames[0].list).collect();
        assert_eq!(
            masters,
            [
                &ListRef::MasterPage {
                    section: 0,
                    index: 0
                },
                &ListRef::PageMaster {
                    section: 0,
                    index: 0
                },
                &ListRef::MasterPage {
                    section: 0,
                    index: LAST_PAGE_MASTER
                },
            ]
        );
    }

    #[test]
    fn a_section_keeps_the_master_page_of_a_side_it_does_not_set() {
        let master = ParaList {
            paras: vec![Paragraph {
                text: vec![hm_model::PARA_END],
                ..Default::default()
            }],
            ..Default::default()
        };
        // A section starting with its definition, then a page break.
        let section = |attr: u32, masters: usize| {
            let mut first = master.paras[0].clone();
            first.text = vec![2, 0, 0, 0, 0, 0, 0, 2, hm_model::PARA_END];
            let mut children = vec![SectionChild::Page(PageDef::default())];
            children.extend((0..masters).map(|_| SectionChild::MasterPage(master.clone())));
            first.controls.push(hm_model::Control {
                kind: ControlKind::Section(Box::new(SectionDef {
                    attr,
                    children,
                    ..Default::default()
                })),
                src: None,
            });
            let mut next = master.paras[0].clone();
            next.header.break_kind = 4;
            Section {
                paras: vec![first, next],
                ..Default::default()
            }
        };
        let doc = Document {
            meta: hm_model::DocMeta {
                format: hm_model::Format::Hwp5,
                version: Default::default(),
                flags: 0,
            },
            info: DocInfo::default(),
            // Even and odd master pages, then a section with an odd one only.
            sections: vec![section(3 << 30, 2), section(1 << 31, 1)],
            source: None,
            bins: None,
            diagnostics: Vec::new(),
        };
        let layout = crate::layout(&doc, &hm_fonts::FontSystem::new());
        let masters: Vec<_> = layout
            .pages
            .iter()
            .map(|p| p.frames.first().map(|f| &f.list))
            .collect();
        let at = |section, index| ListRef::MasterPage { section, index };
        let (odd, even, next) = (at(0, 1), at(0, 0), at(1, 0));
        assert_eq!(
            masters,
            [Some(&odd), Some(&even), Some(&next), Some(&even)],
            "the even pages of the second section keep the first section's even master page"
        );
    }

    #[test]
    fn page_numbers_take_their_side_decoration() {
        let p = hm_model::PageNumberPos {
            attr: 5 << 8,
            side: '-' as u16,
            ..Default::default()
        };
        assert_eq!(page_number_text(&p, 3), "- 3 -");
        let roman = hm_model::PageNumberPos {
            attr: 3 | 5 << 8,
            suffix: ')' as u16,
            ..Default::default()
        };
        assert_eq!(page_number_text(&roman, 4), "iv)");
    }
}
