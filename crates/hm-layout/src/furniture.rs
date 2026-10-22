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
    BorderLine, ControlKind, DocInfo, Document, NumberKind, PageDef, PageSide, ParaList, Paragraph,
    Section, SectionDef, hide,
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

/// Where a frame's paragraphs live in the document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListRef {
    /// The list of control `control` in body paragraph `para` of `section`
    /// (a header, footer or note).
    Control {
        section: u32,
        para: u32,
        control: u32,
    },
    /// Master page `index` of a section's definition, in stored order.
    MasterPage { section: u32, index: u32 },
}

impl ListRef {
    /// The paragraphs a reference names.
    pub fn paras(self, doc: &Document) -> Option<&[Paragraph]> {
        match self {
            ListRef::Control {
                section,
                para,
                control,
            } => {
                let c = doc
                    .sections
                    .get(section as usize)?
                    .paras
                    .get(para as usize)?
                    .controls
                    .get(control as usize)?;
                match &c.kind {
                    ControlKind::HeaderFooter(h) => Some(&h.list.paras),
                    ControlKind::Note(n) => Some(&n.list.paras),
                    _ => None,
                }
            }
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
    page_number: Option<hm_model::PageNumberPos>,
}

#[derive(Clone, Copy)]
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
}

/// Pages (indices into the section's pages) of every furniture control.
fn events<'a>(
    section: u32,
    sec: &'a Section,
    paras: &[ParaLayout],
    pages: &[Page],
) -> Vec<Vec<Event<'a>>> {
    // Page of every line of every paragraph.
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
            let ev = match &c.kind {
                ControlKind::HeaderFooter(h) => Event::Header(h.side(), list, h.footer),
                ControlKind::PageNumber(p) => Event::PageNumber(p),
                ControlKind::NewNumber(n) if n.kind() == NumberKind::Page => {
                    Event::NewPage(n.number as u32)
                }
                ControlKind::PageHide(h) => Event::Hide(h.attr),
                _ => continue,
            };
            if let Some(v) = out.get_mut(page) {
                v.push(ev);
            }
        }
    }
    out
}

/// A frame for a list in `rect`, its content aligned by the list's
/// vertical alignment.
fn frame(
    kind: FrameKind,
    list: ListRef,
    l: &ParaList,
    rect: Rect,
    info: &DocInfo,
    styles: &mut Styles<'_>,
    env: ParaEnv,
) -> Frame {
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
    Frame {
        kind,
        list,
        rect,
        paras,
        lines: lines
            .into_iter()
            .map(|l| FrameLine {
                para: l.para,
                line: l.line,
                x: rect.x,
                y: rect.y + shift + l.y,
            })
            .collect(),
    }
}

/// The char shape Hancom draws page numbers with: the style named "Page
/// Number" ("쪽 번호"), else the first char shape.
fn page_number_shape(info: &DocInfo) -> u32 {
    info.styles
        .iter()
        .find(|s| {
            s.english_name.to_string_lossy() == "Page Number"
                || s.local_name.to_string_lossy() == "쪽 번호"
        })
        .map(|s| s.char_shape as u32)
        .filter(|&id| info.char_shape(id).is_some())
        .unwrap_or(0)
}

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
    let text = page_number_text(p, n);
    let mut glyphs = Vec::new();
    let mut x = 0;
    let (mut ascent, mut descent) = (0, 0);
    for c in text.chars() {
        let script = script_of(c).unwrap_or(hm_model::Script::Latin);
        let si = styles.style_for(cs, script, Some(c));
        let st = styles.get(si).clone();
        let face = styles.face(si).cloned();
        if let Some(f) = &face {
            let em = f.units_per_em.max(1) as i32;
            ascent = ascent.max(hm_model::units::mul_div(st.size, f.ascent, em));
            descent = descent.max(hm_model::units::mul_div(st.size, f.descent, em));
        }
        glyphs.push((c, si, x));
        x += advance(&st, face.as_deref(), c);
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
/// even and odd pages in stored order).
fn page_border(def: &SectionDef, page: &Page, n: u32, page_def: &PageDef) -> Option<PageBorder> {
    let borders: Vec<_> = def.page_borders().collect();
    let b = match borders.len() {
        0 => return None,
        1 | 2 => borders[0],
        _ if n % 2 == 0 => borders[1],
        _ => borders[2],
    };
    if b.border_fill == 0 {
        return None;
    }
    let [l, r, t, bt] = b.offsets.map(i32::from);
    let rect = if b.attr & 1 != 0 {
        // Paper-based: inset from the paper edges.
        Rect {
            x: l,
            y: t,
            w: page.width - l - r,
            h: page.height - t - bt,
        }
    } else {
        // Body-based: around the body, optionally taking in the header and
        // footer areas, pushed out by the offsets.
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
    let page_def = def.page_def().cloned().unwrap_or_default();
    let events = events(section, sec, paras, pages);
    let masters = def.master_pages();
    let info = &doc.info;
    let env = ParaEnv {
        column_width: 0,
        default_tab,
    };
    for (k, page) in pages.iter_mut().enumerate() {
        // Numbering: a section may restart, a new number control restarts
        // on its page.
        let mut n = if k == 0 && def.page_number > 0 {
            def.page_number as u32
        } else if k == 0 && state.page == 0 {
            (info.properties.page_start as u32).max(1)
        } else {
            state.page + 1
        };
        let mut hidden = if k == 0 { def.attr & 0x3F } else { 0 };
        for ev in &events[k] {
            match *ev {
                Event::Header(side, list, footer) => {
                    let slot = if footer {
                        &mut state.footers
                    } else {
                        &mut state.headers
                    };
                    slot[side_index(side)] = Some(ListAt { list });
                    // A header for both sides replaces the one-sided ones.
                    if side == PageSide::Both {
                        slot[1] = None;
                        slot[2] = None;
                    }
                }
                Event::PageNumber(p) => state.page_number = Some(p.clone()),
                Event::NewPage(v) => n = v,
                Event::Hide(bits) => hidden |= bits,
            }
        }
        state.page = n;
        page.number = n;
        let side = if n % 2 == 0 {
            PageSide::Even
        } else {
            PageSide::Odd
        };
        if hidden & hide::MASTER == 0 {
            let chosen = masters
                .iter()
                .enumerate()
                .filter(|(_, (s, _))| *s == side)
                .chain(
                    masters
                        .iter()
                        .enumerate()
                        .filter(|(_, (s, _))| *s == PageSide::Both),
                )
                .next();
            if let Some((index, (_, list))) = chosen {
                let r = ListRef::MasterPage {
                    section,
                    index: index as u32,
                };
                page.frames.push(frame(
                    FrameKind::MasterPage,
                    r,
                    list,
                    page.body,
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
            let pick = slots[side_index(side)].or(slots[0]);
            let Some(at) = pick else { continue };
            let Some(list) = list_of(doc, at.list) else {
                continue;
            };
            let kind = if footer {
                FrameKind::Footer
            } else {
                FrameKind::Header
            };
            page.frames
                .push(frame(kind, at.list, list, area, info, styles, env));
        }
        if hidden & hide::PAGE_NUMBER == 0
            && let Some(p) = &state.page_number
            && let Some(m) = page_number_mark(p, n, page, &page_def, info, styles)
        {
            page.marks.push(m);
        }
        if let Some(mut b) = page_border(def, page, n, &page_def) {
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
}

fn list_of(doc: &Document, r: ListRef) -> Option<&ParaList> {
    let ListRef::Control {
        section,
        para,
        control,
    } = r
    else {
        return None;
    };
    let c = doc
        .sections
        .get(section as usize)?
        .paras
        .get(para as usize)?
        .controls
        .get(control as usize)?;
    match &c.kind {
        ControlKind::HeaderFooter(h) => Some(&h.list),
        ControlKind::Note(n) => Some(&n.list),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
