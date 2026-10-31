//! Sections and paragraphs into section XML.

use hm_model::{
    ColumnDef, Control, ControlKind, LineSeg, NoteShape, PageBorderFill, PageDef, Paragraph,
    Section, SectionChild, SectionDef, SharedBytes, Unit,
};

use super::{K, Root, X, color, name, object};
use crate::Error;
use crate::enums;
use crate::origin::Parts;
use crate::xml::{Ns, Xml};

/// State shared across a section's paragraphs: fields can span them.
pub struct Ctx<'a> {
    pub bins: &'a [String],
    /// Ids of open field begins, innermost last.
    pub fields: Vec<String>,
    /// The source parts clean nodes are copied from.
    pub parts: &'a Parts<'a>,
    /// Master page parts written so far, `(id, part)`, when the package is
    /// written new: section definitions then refer to them.
    pub masters: Option<Vec<(String, Vec<u8>)>>,
    pub extended_masters: Vec<(&'static str, u32, hm_model::ParaList)>,
}

pub fn section(
    s: &Section,
    src: &Xml,
    bins: &[String],
    parts: &Parts<'_>,
    masters: Option<&mut Vec<(String, Vec<u8>)>>,
) -> Result<Vec<u8>, Error> {
    let root = if src.root().is(Ns::Hs, "sec") {
        Root::from_source(src)
    } else {
        Root::fresh("hs:sec", "")
    };
    let mut out = root.prolog.clone();
    out.extend_from_slice(&root.start);
    let fresh = masters.is_some();
    let mut cx = Ctx {
        bins,
        fields: Vec::new(),
        parts,
        masters: masters.as_ref().map(|m| Vec::with_capacity(m.len())),
        extended_masters: s
            .last_page_master
            .iter()
            .map(|l| ("LAST_PAGE", 0, l.clone()))
            .chain(
                s.page_masters
                    .iter()
                    .map(|(p, l)| ("OPTIONAL_PAGE", *p, l.clone())),
            )
            .collect(),
    };
    let before = masters.as_ref().map_or(0, |m| m.len());
    if let (Some(all), Some(own)) = (masters.as_ref(), cx.masters.as_mut()) {
        // Ids continue across sections.
        own.extend(all.iter().map(|(id, _)| (id.clone(), Vec::new())));
    }
    for p in &s.paras {
        match paragraph_kid(p, &mut cx) {
            K::Raw(b) => out.extend_from_slice(&b),
            K::E(x) => x.write(&mut out),
            K::Text(_) => {}
        }
    }
    out.extend_from_slice(&root.end_tag());
    if fresh && let (Some(all), Some(own)) = (masters, cx.masters) {
        all.extend(own.into_iter().skip(before));
    }
    Ok(out)
}

/// A paragraph list (cells, text boxes, captions, notes).
pub fn paragraphs(ps: &[Paragraph], cx: &mut Ctx<'_>) -> Vec<K> {
    ps.iter().map(|p| paragraph_kid(p, cx)).collect()
}

fn paragraph_kid(p: &Paragraph, cx: &mut Ctx<'_>) -> K {
    let kept = p.src.as_ref().filter(|_| p.is_clean());
    match kept.and_then(|s| cx.parts.markup(s)) {
        Some(m) => {
            note_fields(&m, cx);
            K::Raw(m)
        }
        None => K::E(paragraph(p, cx)),
    }
}

/// Keeps the field stack right across copied paragraphs.
fn note_fields(src: &SharedBytes, cx: &mut Ctx<'_>) {
    if !src.windows(6).any(|w| w == b":field") {
        return;
    }
    let Ok(x) = Xml::parse(src.clone()) else {
        return;
    };
    fn walk(e: crate::xml::El<'_>, cx: &mut Ctx<'_>) {
        for c in e.children() {
            if c.is(Ns::Hp, "fieldBegin") {
                cx.fields
                    .push(c.attr("id").unwrap_or_default().into_owned());
            } else if c.is(Ns::Hp, "fieldEnd") {
                cx.fields.pop();
            }
            walk(c, cx);
        }
    }
    walk(x.root(), cx);
}

/// Controls Hancom writes inside `hp:ctrl`.
fn wrapped(local: &[u8]) -> bool {
    matches!(
        local,
        b"colPr"
            | b"fieldBegin"
            | b"fieldEnd"
            | b"header"
            | b"footer"
            | b"footNote"
            | b"endNote"
            | b"autoNum"
            | b"newNum"
            | b"pageNum"
            | b"pageHiding"
            | b"pageNumCtrl"
            | b"bookmark"
            | b"indexmark"
            | b"hiddenComment"
    )
}

fn local_of(markup: &[u8]) -> &[u8] {
    let s = markup.strip_prefix(b"<").unwrap_or(markup);
    let end = s
        .iter()
        .position(|c| c.is_ascii_whitespace() || *c == b'>' || *c == b'/')
        .unwrap_or(s.len());
    let q = &s[..end];
    match q.iter().position(|&c| c == b':') {
        Some(i) => &q[i + 1..],
        None => q,
    }
}

struct Runs<'a, 'b> {
    cx: &'a mut Ctx<'b>,
    runs: Vec<X>,
    run: Option<X>,
    t: Option<X>,
    buf: String,
}

impl Runs<'_, '_> {
    fn flush_text(&mut self) {
        if !self.buf.is_empty() {
            let t = self.t.get_or_insert_with(|| X::new("hp:t"));
            t.kids.push(K::Text(std::mem::take(&mut self.buf)));
        }
    }

    fn close_t(&mut self) {
        self.flush_text();
        if let (Some(t), Some(r)) = (self.t.take(), self.run.as_mut()) {
            r.kids.push(K::E(t));
        }
    }

    fn start(&mut self, shape: u32) {
        self.close_run();
        self.run = Some(X::new("hp:run").a("charPrIDRef", shape));
    }

    fn close_run(&mut self) {
        self.close_t();
        if let Some(mut r) = self.run.take() {
            if r.kids.is_empty() {
                r.kids.push(K::E(X::new("hp:t")));
            }
            self.runs.push(r);
        }
    }

    fn inline_el(&mut self, x: X) {
        self.flush_text();
        self.t
            .get_or_insert_with(|| X::new("hp:t"))
            .kids
            .push(K::E(x));
    }

    fn control(&mut self, k: K, local: &[u8]) {
        self.close_t();
        let r = self
            .run
            .get_or_insert_with(|| X::new("hp:run").a("charPrIDRef", 0));
        if wrapped(local) {
            r.kids.push(K::E(X::new("hp:ctrl").kid_k(k)));
        } else {
            r.kids.push(k);
        }
    }
}

impl X {
    fn kid_k(mut self, k: K) -> X {
        self.kids.push(k);
        self
    }
}

pub fn paragraph(p: &Paragraph, cx: &mut Ctx<'_>) -> X {
    let h = &p.header;
    let mut x = X::new("hp:p")
        .a("id", h.instance_id)
        .a("paraPrIDRef", h.para_shape)
        .a("styleIDRef", h.style)
        .b("pageBreak", h.break_kind & hm_model::break_kind::PAGE != 0)
        .b(
            "columnBreak",
            h.break_kind & hm_model::break_kind::COLUMN != 0,
        )
        .a("merged", h.merge_flag.unwrap_or(0));
    let marks: Vec<_> = p.range_tags.iter().filter(|t| t.tag >> 24 == 2).collect();
    let mut r = Runs {
        cx,
        runs: Vec::new(),
        run: None,
        t: None,
        buf: String::new(),
    };
    let mut next_run = 0usize;
    let first = p.runs.first().map_or(0, |c| c.shape);
    r.start(first);
    let text = &p.text;
    for (pos, unit) in p.units() {
        while next_run < p.runs.len() && p.runs[next_run].pos <= pos {
            if p.runs[next_run].pos == pos && pos > 0 {
                r.start(p.runs[next_run].shape);
            }
            next_run += 1;
        }
        for m in &marks {
            if m.end == pos && m.end > m.start {
                r.inline_el(X::new("hp:markpenEnd"));
            }
            if m.start == pos {
                r.inline_el(X::new("hp:markpenBegin").a("color", color(m.tag & 0xFF_FFFF)));
            }
        }
        for m in p.tracks.iter().filter(|m| m.pos == pos) {
            r.inline_el(crate::annot::write_track_mark(m));
        }
        let info = |k: usize| text.get(pos as usize + k).copied().unwrap_or(0);
        match unit {
            Unit::Char(c) => {
                r.t.get_or_insert_with(|| X::new("hp:t"));
                r.buf.push(c);
            }
            Unit::CharControl(13) => {}
            Unit::CharControl(10) => r.inline_el(X::new("hp:lineBreak")),
            Unit::CharControl(30) => r.inline_el(X::new("hp:nbSpace")),
            Unit::CharControl(31) => r.inline_el(X::new("hp:fwSpace")),
            Unit::CharControl(24) => r.inline_el(X::new("hp:hypen")),
            Unit::CharControl(_) => {}
            Unit::Inline(9) => {
                let w = info(1) as u32 | (info(2) as u32) << 16;
                r.inline_el(
                    X::new("hp:tab")
                        .a("width", w)
                        .a("leader", info(3) & 0xFF)
                        .a("type", info(3) >> 8),
                )
            }
            Unit::Inline(8) => r.inline_el(X::new("hp:titleMark").b("ignore", info(3) == 0)),
            Unit::Inline(4) => {
                let id = info(1) as u32 | (info(2) as u32) << 16;
                let begin = r.cx.fields.pop().unwrap_or_default();
                let k = K::E(
                    X::new("hp:fieldEnd")
                        .a("beginIDRef", begin)
                        .a("fieldid", id),
                );
                r.control(k, b"fieldEnd");
            }
            Unit::Inline(_) => {}
            Unit::Extended { index, .. } => {
                if let Some(c) = p.controls.get(index as usize) {
                    let (k, local) = control(c, r.cx);
                    r.control(k, &local);
                }
            }
        }
    }
    r.close_run();
    x.kids.extend(r.runs.into_iter().map(K::E));
    if !p.line_segs.is_empty() {
        x = x.kid(X::new("hp:linesegarray").kids(p.line_segs.iter().map(line_seg)));
    }
    x.with_rest(&h.tail)
}

fn line_seg(s: &LineSeg) -> X {
    X::new("hp:lineseg")
        .a("textpos", s.text_start)
        .a("vertpos", s.vpos)
        .a("vertsize", s.height)
        .a("textheight", s.text_height)
        .a("baseline", s.baseline)
        .a("spacing", s.spacing)
        .a("horzpos", s.col_start)
        .a("horzsize", s.width)
        .a("flags", s.flags)
}

/// A control's markup and the local name that decides its wrapping.
fn control(c: &Control, cx: &mut Ctx<'_>) -> (K, Vec<u8>) {
    let kept = c.src.as_ref().filter(|_| c.is_clean());
    let k = match (kept.and_then(|s| cx.parts.markup(s)), &c.kind) {
        (Some(m), _) => {
            if local_of(&m) == b"fieldBegin" {
                note_fields(&m, cx);
            }
            K::Raw(m)
        }
        (_, ControlKind::Section(s)) => K::E(sec_pr(s, cx)),
        (_, ControlKind::Columns(col)) => K::E(col_pr(col)),
        (_, ControlKind::Object(o)) => match crate::rest::child_at(&o.header.tail, "@switch") {
            Some(sw) => K::Raw(SharedBytes::from(sw)),
            None => K::E(object::object(o, cx)),
        },
        (_, ControlKind::Table(t)) => K::E(object::table(t, cx)),
        (_, ControlKind::Field(f)) => {
            cx.fields.push(f.instance.to_string());
            K::E(crate::annot::write_field(f, cx))
        }
        (_, ControlKind::Bookmark(b)) => K::E(crate::annot::write_bookmark(b)),
        (_, ControlKind::Compose(c)) => K::E(crate::annot::write_compose(c)),
        (_, ControlKind::Dutmal(d)) => K::E(crate::annot::write_dutmal(d)),
        (_, ControlKind::Form(f)) => match crate::annot::write_form(f) {
            Some(x) => K::E(x),
            None => K::Raw(SharedBytes::empty()),
        },
        (_, ControlKind::IndexMark(_)) => K::Raw(SharedBytes::empty()),
        (_, ControlKind::HeaderFooter(h)) => K::E(header_footer(h, cx)),
        (_, ControlKind::Note(n)) => K::E(note(n, cx)),
        (_, ControlKind::AutoNumber(a)) => K::E(auto_number(a)),
        (_, ControlKind::NewNumber(n)) => K::E(
            X::new("hp:newNum")
                .a("num", n.number)
                .a("numType", name(enums::AUTO_NUM_TYPE, n.attr & 0xF))
                .with_rest(&n.tail),
        ),
        (_, ControlKind::PageNumber(p)) => K::E(
            X::new("hp:pageNum")
                .a("pos", name(enums::PAGE_NUM_POS, (p.attr >> 8) & 0xF))
                .a("formatType", name(enums::NUMBER_FORMAT, p.attr & 0xFF))
                .a("sideChar", ch(p.side))
                .with_rest(&p.tail),
        ),
        (_, ControlKind::PageHide(h)) => {
            let mut x = X::new("hp:pageHiding");
            for (i, k) in crate::furniture::HIDE_ATTRS.iter().enumerate() {
                x = x.b(k, h.attr & (1 << i) != 0);
            }
            K::E(x.with_rest(&h.tail))
        }
        (_, ControlKind::PageParity(p)) => K::E(
            X::new("hp:pageNumCtrl")
                .a("pageStartsOn", name(enums::PAGE_TYPE, p.attr & 3))
                .with_rest(&p.tail),
        ),
        (_, ControlKind::Raw(r)) => match &r.xml {
            Some(x) => {
                if local_of(x) == b"fieldBegin" {
                    note_fields(x, cx);
                }
                K::Raw(x.clone())
            }
            None => K::Raw(SharedBytes::empty()),
        },
    };
    let local = match &k {
        K::Raw(b) => local_of(b).to_vec(),
        K::E(x) => x.name.rsplit(':').next().unwrap_or("").as_bytes().to_vec(),
        K::Text(_) => Vec::new(),
    };
    (k, local)
}

fn sec_pr(s: &SectionDef, cx: &mut Ctx<'_>) -> X {
    let a = s.attr;
    let (tab_val, tab_unit) = (
        s.default_tab >> 1,
        if s.default_tab & 1 != 0 {
            "CHAR"
        } else {
            "HWPUNIT"
        },
    );
    let vis = |hide: u32, first: u32| {
        if a & (1 << hide) != 0 {
            "HIDE_FIRST"
        } else if a & (1 << first) != 0 {
            "SHOW_FIRST"
        } else {
            "SHOW_ALL"
        }
    };
    let mut x = X::new("hp:secPr")
        .a("id", "")
        .a("textDirection", name(enums::TEXT_DIRECTION, (a >> 16) & 7))
        .a("spaceColumns", s.column_gap)
        .a("tabStop", s.default_tab)
        .a("tabStopVal", tab_val)
        .a("tabStopUnit", tab_unit)
        .a("outlineShapeIDRef", s.outline_numbering)
        .kid(
            X::new("hp:grid")
                .a("lineGrid", s.vertical_grid)
                .a("charGrid", s.horizontal_grid)
                .b("wonggojiFormat", a & (1 << 22) != 0),
        )
        .kid(
            X::new("hp:startNum")
                .a("pageStartsOn", name(enums::PAGE_STARTS_ON, (a >> 20) & 3))
                .a("page", s.page_number)
                .a("pic", s.figure_number)
                .a("tbl", s.table_number)
                .a("equation", s.equation_number),
        )
        .kid(
            X::new("hp:visibility")
                .b("hideFirstHeader", a & 1 != 0)
                .b("hideFirstFooter", a & 2 != 0)
                .b("hideFirstMasterPage", a & 4 != 0)
                .a("border", vis(3, 8))
                .a("fill", vis(4, 9))
                .b("hideFirstPageNum", a & (1 << 5) != 0)
                .b("hideFirstEmptyLine", a & (1 << 19) != 0),
        );
    let mut notes = 0;
    let mut borders = 0;
    for c in &s.children {
        match c {
            SectionChild::Page(p) => x = x.kid(page_pr(p)),
            SectionChild::NoteShape(n) => {
                x = x.kid(note_pr(n, notes == 0));
                notes += 1;
            }
            SectionChild::PageBorder(b) => {
                x = x.kid(page_border(b, borders));
                borders += 1;
            }
            _ => {}
        }
    }
    // A package written new keeps master pages in parts of their own.
    if cx.masters.is_some() {
        let mut pages: Vec<_> = s
            .master_pages()
            .into_iter()
            .map(|(side, l)| {
                let kind = match side {
                    hm_model::PageSide::Both => "BOTH",
                    hm_model::PageSide::Even => "EVEN",
                    hm_model::PageSide::Odd => "ODD",
                };
                (kind, 0, l.clone())
            })
            .collect();
        pages.append(&mut cx.extended_masters);
        if !pages.is_empty() {
            x = x.a("masterPageCnt", pages.len());
        }
        for (kind, page, l) in pages {
            let n = cx.masters.as_ref().map_or(0, Vec::len);
            let id = format!("masterpage{n}");
            let root = crate::write::Root::fresh(
                "masterPage",
                &format!(
                    " id=\"{id}\" type=\"{kind}\" pageNumber=\"{page}\" pageDuplicate=\"0\" pageFront=\"0\""
                ),
            );
            let mut part = root.prolog.clone();
            part.extend_from_slice(&root.start);
            list(&l, cx).write(&mut part);
            part.extend_from_slice(&root.end_tag());
            x = x.kid(X::new("hp:masterPage").a("idRef", &id));
            if let Some(m) = cx.masters.as_mut() {
                m.push((id, part));
            }
        }
    }
    x.with_rest(&s.tail)
}

fn page_pr(p: &PageDef) -> X {
    X::new("hp:pagePr")
        .a("landscape", name(enums::LANDSCAPE, p.attr & 1))
        .a("width", p.width)
        .a("height", p.height)
        .a("gutterType", name(enums::GUTTER, (p.attr >> 1) & 3))
        .kid(
            X::new("hp:margin")
                .a("header", p.margin_header)
                .a("footer", p.margin_footer)
                .a("gutter", p.gutter)
                .a("left", p.margin_left)
                .a("right", p.margin_right)
                .a("top", p.margin_top)
                .a("bottom", p.margin_bottom),
        )
}

fn ch(c: u16) -> String {
    if c == 0 {
        String::new()
    } else {
        char::from_u32(c as u32)
            .map(String::from)
            .unwrap_or_default()
    }
}

fn note_pr(n: &NoteShape, footnote: bool) -> X {
    let a = n.attr;
    let place = if footnote {
        name(enums::FOOTNOTE_PLACE, (a >> 8) & 3)
    } else {
        name(enums::ENDNOTE_PLACE, (a >> 8) & 3)
    };
    X::new(if footnote {
        "hp:footNotePr"
    } else {
        "hp:endNotePr"
    })
    .kid(
        X::new("hp:autoNumFormat")
            .a("type", name(enums::NUMBER_FORMAT, a & 0xFF))
            .a("userChar", ch(n.user_char))
            .a("prefixChar", ch(n.prefix))
            .a("suffixChar", ch(n.suffix))
            .b("supscript", a & (1 << 12) != 0),
    )
    .kid(
        X::new("hp:noteLine")
            .a("length", n.separator_length)
            .a("type", name(enums::LINE_KIND, n.line_kind as u32))
            .a("width", enums::width_name(n.line_width))
            .a("color", color(n.line_color)),
    )
    .kid(
        X::new("hp:noteSpacing")
            .a("betweenNotes", n.space_between)
            .a("belowLine", n.space_below)
            .a("aboveLine", n.space_above),
    )
    .kid({
        let numbering = X::new("hp:numbering");
        // An endnote shape restarting on every page has no type in
        // Hancom's files.
        let numbering = if footnote || (a >> 10) & 3 != 2 {
            numbering.a("type", name(enums::NOTE_NUMBERING, (a >> 10) & 3))
        } else {
            numbering
        };
        numbering.a("newNum", n.start)
    })
    .kid(
        X::new("hp:placement")
            .a("place", place)
            .b("beneathText", a & (1 << 13) != 0),
    )
}

fn page_border(b: &PageBorderFill, i: usize) -> X {
    X::new("hp:pageBorderFill")
        .a("type", name(enums::PAGE_BORDER_TYPE, i as u32))
        .a("borderFillIDRef", b.border_fill)
        .a("textBorder", name(enums::TEXT_BORDER, b.attr & 1))
        .b("headerInside", b.attr & 2 != 0)
        .b("footerInside", b.attr & 4 != 0)
        .a("fillArea", name(enums::FILL_AREA, (b.attr >> 3) & 3))
        .kid(
            X::new("hp:offset")
                .a("left", b.offsets[0])
                .a("right", b.offsets[1])
                .a("top", b.offsets[2])
                .a("bottom", b.offsets[3]),
        )
}

fn col_pr(c: &ColumnDef) -> X {
    let a = c.attr as u32;
    let mut x = X::new("hp:colPr")
        .a("id", "")
        .a("type", name(enums::COLUMN_TYPE, a & 3))
        .a("layout", name(enums::COLUMN_LAYOUT, (a >> 10) & 3))
        .a("colCount", c.count())
        .b("sameSz", c.equal_widths())
        .a("sameGap", c.gap);
    if c.line_kind != 0 {
        x = x.kid(
            X::new("hp:colLine")
                .a("type", name(enums::LINE_KIND, c.line_kind as u32))
                .a("width", enums::width_name(c.line_width))
                .a("color", color(c.line_color)),
        );
    }
    for &(w, g) in &c.columns {
        x = x.kid(X::new("hp:colSz").a("width", w).a("gap", g));
    }
    x.with_rest(&c.tail)
}

/// A paragraph list as `hp:subList`; sized lists carry their text area.
pub(crate) fn list(l: &hm_model::ParaList, cx: &mut Ctx<'_>) -> X {
    let a = l.attr;
    let mut x = X::new("hp:subList")
        .a("textDirection", name(enums::TEXT_DIRECTION, a & 7))
        .a("lineWrap", name(enums::LINE_WRAP, (a >> 3) & 3))
        .a(
            "vertAlign",
            name(crate::object::SUBLIST_V_ALIGN, (a >> 5) & 3),
        );
    if let Some((w, h)) = l.text_size {
        x = x.a("textWidth", w).a("textHeight", h);
    }
    if let Some((t, n)) = l.refs {
        x = x.a("hasTextRef", t).a("hasNumRef", n);
    }
    x.kids.extend(paragraphs(&l.paras, cx));
    x.with_rest(&l.tail)
}

fn header_footer(h: &hm_model::HeaderFooter, cx: &mut Ctx<'_>) -> X {
    X::new(if h.footer { "hp:footer" } else { "hp:header" })
        .a("applyPageType", name(enums::PAGE_TYPE, h.attr & 3))
        .kid(list(&h.list, cx))
        .with_rest(&h.tail)
}

fn note(n: &hm_model::Note, cx: &mut Ctx<'_>) -> X {
    // Hancom's attribute order; empty characters and a zero flag are left out.
    let mut x = X::new(if n.endnote {
        "hp:endNote"
    } else {
        "hp:footNote"
    });
    if n.flag != 0 {
        x = x.a("flag", n.flag);
    }
    x = x.a("number", n.number);
    for (k, v) in [
        ("userChar", n.user_char),
        ("prefixChar", n.prefix),
        ("suffixChar", n.suffix),
    ] {
        if v != 0 {
            x = x.a(k, v);
        }
    }
    if let Some(id) = n.inst_id {
        x = x.a("instId", id);
    }
    x.kid(list(&n.list, cx)).with_rest(&n.tail)
}

fn auto_number(a: &hm_model::AutoNumber) -> X {
    X::new("hp:autoNum")
        .a("num", a.number)
        .a("numType", name(enums::AUTO_NUM_TYPE, a.attr & 0xF))
        .kid(
            X::new("hp:autoNumFormat")
                .a("type", name(enums::NUMBER_FORMAT, (a.attr >> 4) & 0xFF))
                .a("userChar", ch(a.user_char))
                .a("prefixChar", ch(a.prefix))
                .a("suffixChar", ch(a.suffix))
                .b("supscript", a.attr & (1 << 12) != 0),
        )
        .with_rest(&a.tail)
}
