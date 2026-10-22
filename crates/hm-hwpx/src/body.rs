//! Section XML into paragraphs in HWP 5 wire space.
//!
//! Text units follow HWP 5: characters as UTF-16, `hp:tab` and other inline
//! marks as 8-unit inline controls, every control element as an 8-unit
//! extended control holding its id, and a closing paragraph end.

use hm_model::{
    CharRun, ColumnDef, Control, ControlKind, CtrlId, LineSeg, NoteShape, PageBorderFill, PageDef,
    ParaHeader, Paragraph, RangeTag, RawControl, SectionChild, SectionDef, SharedBytes, break_kind,
};

use crate::enums;
use crate::head::{attr_color, code, tag};
use crate::object;
use crate::read::Ctx;
use crate::rest::Rest;
use crate::xml::{El, Kind, Ns, Xml};

pub fn parse_section(x: &Xml, ctx: &mut Ctx) -> Vec<Paragraph> {
    let root = x.root();
    if !root.is(Ns::Hs, "sec") {
        ctx.diag(format!("section root is {}", tag(root)));
    }
    paragraphs(root, ctx)
}

/// The `hp:p` children of a list element (section, `hp:subList`).
pub fn paragraphs(list: El<'_>, ctx: &mut Ctx) -> Vec<Paragraph> {
    list.children()
        .filter(|c| c.is(Ns::Hp, "p"))
        .map(|p| paragraph(p, ctx))
        .collect()
}

/// The control id an element stands for, and its HWP 5 control code.
pub fn control_of(el: El<'_>) -> Option<(u16, CtrlId)> {
    let id = |c: &[u8; 4]| CtrlId::from_chars(c);
    Some(match (el.ns(), el.local()) {
        (Ns::Hp, b"secPr") => (2, CtrlId::SECTION),
        (Ns::Hp, b"colPr") => (2, CtrlId::COLUMNS),
        (Ns::Hp, b"fieldBegin") => {
            let t = el.attr("type").unwrap_or_default();
            let f = enums::code(enums::FIELD_TYPE, &t).unwrap_or(0x2575_6e6b);
            (3, CtrlId(f))
        }
        (Ns::Hp, b"tbl") => (11, CtrlId::TABLE),
        (Ns::Hp, b"equation") => (11, CtrlId::EQUATION),
        (
            Ns::Hp | Ns::Hp10,
            b"pic" | b"rect" | b"line" | b"ellipse" | b"arc" | b"polygon" | b"curve"
            | b"connectLine" | b"container" | b"ole" | b"textart" | b"chart" | b"video",
        ) => (11, CtrlId::SHAPE),
        (
            Ns::Hp,
            b"btn" | b"checkBtn" | b"radioBtn" | b"comboBox" | b"edit" | b"listBox" | b"scrollBar",
        ) => (11, id(b"form")),
        (Ns::Hp, b"hiddenComment") => (15, id(b"tcmt")),
        (Ns::Hp, b"header") => (16, CtrlId::HEADER),
        (Ns::Hp, b"footer") => (16, CtrlId::FOOTER),
        (Ns::Hp, b"footNote") => (17, CtrlId::FOOTNOTE),
        (Ns::Hp, b"endNote") => (17, CtrlId::ENDNOTE),
        (Ns::Hp, b"autoNum") => (18, CtrlId::AUTO_NUMBER),
        (Ns::Hp, b"newNum") => (21, CtrlId::NEW_NUMBER),
        (Ns::Hp, b"pageHiding") => (21, CtrlId::PAGE_HIDE),
        (Ns::Hp, b"pageNumCtrl") => (21, id(b"pgct")),
        (Ns::Hp, b"pageNum") => (21, CtrlId::PAGE_NUMBER_POS),
        (Ns::Hp, b"bookmark") => (22, CtrlId::BOOKMARK),
        (Ns::Hp, b"indexmark") => (22, id(b"idxm")),
        (Ns::Hp, b"compose") => (23, id(b"tcps")),
        (Ns::Hp, b"dutmal") => (23, id(b"tdut")),
        _ => return None,
    })
}

/// The element a `hp:switch` stands for: the first case content we know,
/// else the default content.
pub fn switch_target<'a>(sw: El<'a>) -> Option<El<'a>> {
    fn branch(b: El<'_>) -> Option<El<'_>> {
        b.children().find(|c| control_of(*c).is_some())
    }
    sw.children()
        .filter(|c| c.is(Ns::Hp, "case"))
        .find_map(branch)
        .or_else(|| sw.child(Ns::Hp, "default").and_then(branch))
}

struct Builder {
    text: Vec<u16>,
    runs: Vec<CharRun>,
    controls: Vec<Control>,
    tags: Vec<RangeTag>,
    open_marks: Vec<(u32, u32)>,
    breaks: u8,
}

impl Builder {
    fn pos(&self) -> u32 {
        self.text.len() as u32
    }

    fn run(&mut self, shape: u32) {
        let pos = self.pos();
        match self.runs.last_mut() {
            Some(r) if r.pos == pos => r.shape = shape,
            Some(r) if r.shape == shape => {}
            _ => self.runs.push(CharRun { pos, shape }),
        }
    }

    fn inline(&mut self, code: u16, info: [u16; 6]) {
        self.text.push(code);
        self.text.extend_from_slice(&info);
        self.text.push(code);
    }

    fn extended(&mut self, code: u16, id: CtrlId, control: Control) {
        self.inline(code, [id.0 as u16, (id.0 >> 16) as u16, 0, 0, 0, 0]);
        self.controls.push(control);
    }
}

pub fn paragraph(p: El<'_>, ctx: &mut Ctx) -> Paragraph {
    let mut rest = Rest::new();
    rest.attrs(
        "",
        p,
        &[
            "id",
            "paraPrIDRef",
            "styleIDRef",
            "pageBreak",
            "columnBreak",
            "merged",
        ],
    );
    let mut b = Builder {
        text: Vec::new(),
        runs: Vec::new(),
        controls: Vec::new(),
        tags: Vec::new(),
        open_marks: Vec::new(),
        breaks: 0,
    };
    let mut segs = Vec::new();
    let mut slot = 0u16;
    for c in p.children() {
        match (c.ns(), c.local()) {
            (Ns::Hp, b"run") => {
                run(c, &mut b, ctx);
                slot += 1;
            }
            (Ns::Hp, b"linesegarray") => {
                for s in c.children().filter(|s| s.is(Ns::Hp, "lineseg")) {
                    let i = |k| s.i32(k).unwrap_or(0);
                    segs.push(LineSeg {
                        text_start: s.u32("textpos").unwrap_or(0),
                        vpos: i("vertpos"),
                        height: i("vertsize"),
                        text_height: i("textheight"),
                        baseline: i("baseline"),
                        spacing: i("spacing"),
                        col_start: i("horzpos"),
                        width: i("horzsize"),
                        flags: s.u32("flags").unwrap_or(0),
                    });
                }
                slot += 1;
            }
            _ => {
                ctx.unknown(format!("hp:p/{}", tag(c)));
                rest.child("", slot, c);
            }
        }
    }
    let end = b.pos();
    for (start, color) in b.open_marks.drain(..) {
        b.tags.push(RangeTag {
            start,
            end,
            tag: 2 << 24 | (color & 0xFF_FFFF),
        });
    }
    if b.runs.is_empty() {
        b.runs.push(CharRun { pos: 0, shape: 0 });
    }
    b.text.push(hm_model::PARA_END);
    let mut brk = b.breaks;
    if p.flag("pageBreak") == Some(true) {
        brk |= break_kind::PAGE;
    }
    if p.flag("columnBreak") == Some(true) {
        brk |= break_kind::COLUMN;
    }
    ctx.note_rest(p, &rest);
    Paragraph {
        header: ParaHeader {
            para_shape: p.int("paraPrIDRef").unwrap_or(0) as u16,
            style: p.int("styleIDRef").unwrap_or(0) as u8,
            break_kind: brk,
            instance_id: p.u32("id").unwrap_or(0),
            merge_flag: Some(p.int("merged").unwrap_or(0) as u16),
            tail: rest.into_tail(),
            control_mask: 0,
        },
        text: b.text,
        runs: b.runs,
        line_segs: segs,
        range_tags: b.tags,
        controls: b.controls,
        extra: Vec::new(),
        src: Some(p.source()),
    }
    .with_mask()
}

trait WithMask {
    fn with_mask(self) -> Self;
}

impl WithMask for Paragraph {
    /// Sets the control mask Hancom stores: one bit per control code present,
    /// the paragraph end left out.
    fn with_mask(mut self) -> Self {
        let mut mask = 0u32;
        for (_, u) in self.units() {
            match u {
                hm_model::Unit::CharControl(13) => {}
                hm_model::Unit::CharControl(c) | hm_model::Unit::Inline(c) => mask |= 1 << c,
                hm_model::Unit::Extended { code, .. } => mask |= 1 << code,
                hm_model::Unit::Char(_) => {}
            }
        }
        self.header.control_mask = mask;
        self
    }
}

fn run(r: El<'_>, b: &mut Builder, ctx: &mut Ctx) {
    b.run(r.u32("charPrIDRef").unwrap_or(0));
    for c in r.children() {
        match (c.ns(), c.local()) {
            (Ns::Hp, b"t") => text(c, b, ctx),
            (Ns::Hp, b"ctrl") => {
                for k in c.children() {
                    control(k, b, ctx);
                }
            }
            (Ns::Hp, b"markpenBegin") => {
                let color = attr_color(c, "color").unwrap_or(0);
                b.open_marks.push((b.pos(), color));
            }
            (Ns::Hp, b"markpenEnd") => close_mark(b),
            _ => control(c, b, ctx),
        }
    }
}

fn close_mark(b: &mut Builder) {
    if let Some((start, color)) = b.open_marks.pop() {
        let end = b.pos();
        b.tags.push(RangeTag {
            start,
            end,
            tag: 2 << 24 | (color & 0xFF_FFFF),
        });
    }
}

fn text(t: El<'_>, b: &mut Builder, ctx: &mut Ctx) {
    for n in t.nodes() {
        match n.kind() {
            Kind::Text | Kind::CData => {
                let s = if n.kind() == Kind::Text {
                    crate::xml::unescape(n.content())
                } else {
                    String::from_utf8_lossy(n.content())
                };
                for ch in s.chars() {
                    match ch {
                        '\t' => b.inline(9, [0; 6]),
                        '\r' | '\n' => {}
                        c if (c as u32) < 32 => {}
                        c => {
                            let mut buf = [0u16; 2];
                            b.text.extend_from_slice(c.encode_utf16(&mut buf));
                        }
                    }
                }
            }
            Kind::Element => match (n.ns(), n.local()) {
                (Ns::Hp, b"tab") => {
                    let w = n.u32("width").unwrap_or(0);
                    let leader = code(n, "leader", enums::LINE_KIND, ctx).unwrap_or(0) as u16;
                    let kind = code(n, "type", enums::TAB_TYPE, ctx).unwrap_or(0) as u16;
                    b.inline(9, [w as u16, (w >> 16) as u16, leader | kind << 8, 0, 0, 0]);
                }
                (Ns::Hp, b"lineBreak") => b.text.push(10),
                (Ns::Hp, b"nbSpace") => b.text.push(30),
                (Ns::Hp, b"fwSpace") => b.text.push(31),
                (Ns::Hp, b"hypen" | b"hyphen") => b.text.push(24),
                (Ns::Hp, b"titleMark") => {
                    let ignore = n.flag("ignore").unwrap_or(false) as u16;
                    b.inline(8, [ignore, 0, 0, 0, 0, 0]);
                }
                (Ns::Hp, b"markpenBegin") => {
                    let color = attr_color(n, "color").unwrap_or(0);
                    b.open_marks.push((b.pos(), color));
                }
                (Ns::Hp, b"markpenEnd") => close_mark(b),
                _ => ctx.unknown(format!("hp:t/{}", tag(n))),
            },
        }
    }
}

fn control(el: El<'_>, b: &mut Builder, ctx: &mut Ctx) {
    if el.is(Ns::Hp, "fieldEnd") {
        let id = el.u32("fieldid").unwrap_or(0);
        b.inline(4, [id as u16, (id >> 16) as u16, 0, 0, 0, 0]);
        return;
    }
    let target = if el.is(Ns::Hp, "switch") {
        switch_target(el)
    } else {
        Some(el)
    };
    let Some((code, id)) = target.and_then(control_of) else {
        ctx.unknown(format!("control {}", tag(el)));
        return;
    };
    let target = target.unwrap();
    let kind = match (code, id) {
        (2, CtrlId::SECTION) => {
            b.breaks |= break_kind::SECTION;
            ControlKind::Section(Box::new(section_def(target, ctx)))
        }
        (2, CtrlId::COLUMNS) => {
            b.breaks |= break_kind::MULTI_COLUMN;
            ControlKind::Columns(column_def(target, ctx))
        }
        (11, CtrlId::EQUATION) => ControlKind::Object(Box::new(object::equation(target, ctx))),
        _ => ControlKind::Raw(raw_control(el, target, id, ctx)),
    };
    b.extended(
        code,
        id,
        Control {
            kind,
            src: Some(el.source()),
        },
    );
}

/// A control the model keeps as markup, with the HWP 5 header fields it
/// maps to.
fn raw_control(el: El<'_>, target: El<'_>, id: CtrlId, ctx: &mut Ctx) -> RawControl {
    let mut header = id.0.to_le_bytes().to_vec();
    if target.child(Ns::Hp, "pos").is_some() || target.child(Ns::Hp, "sz").is_some() {
        let (common, instance) = object::common(target, &mut Rest::new(), ctx);
        header.extend_from_slice(&object::encode_common(&common));
        header.extend_from_slice(&instance.to_le_bytes());
        header.extend_from_slice(&0i32.to_le_bytes());
        header.extend_from_slice(&0u16.to_le_bytes());
    }
    RawControl {
        id,
        header: SharedBytes::from(header),
        children: Vec::new(),
        xml: Some(el.source()),
    }
}

fn section_def(el: El<'_>, ctx: &mut Ctx) -> SectionDef {
    let mut rest = Rest::new();
    rest.attrs(
        "",
        el,
        &[
            "id",
            "textDirection",
            "spaceColumns",
            "tabStop",
            "tabStopVal",
            "tabStopUnit",
            "outlineShapeIDRef",
        ],
    );
    let mut s = SectionDef {
        column_gap: el.int("spaceColumns").unwrap_or(0) as i16,
        default_tab: el.i32("tabStop").unwrap_or(8000),
        outline_numbering: el.int("outlineShapeIDRef").unwrap_or(0) as u16,
        language: Some(0),
        ..Default::default()
    };
    let mut attr = code(el, "textDirection", enums::TEXT_DIRECTION, ctx).unwrap_or(0) << 16;
    let mut slot = 0u16;
    let mut notes = 0;
    for c in el.children() {
        let known = match (c.ns(), c.local()) {
            (Ns::Hp, b"grid") => {
                rest.attrs("grid", c, &["lineGrid", "charGrid", "wonggojiFormat"]);
                s.vertical_grid = c.int("lineGrid").unwrap_or(0) as i16;
                s.horizontal_grid = c.int("charGrid").unwrap_or(0) as i16;
                attr |= (c.flag("wonggojiFormat").unwrap_or(false) as u32) << 22;
                true
            }
            (Ns::Hp, b"startNum") => {
                rest.attrs(
                    "startNum",
                    c,
                    &["pageStartsOn", "page", "pic", "tbl", "equation"],
                );
                attr |= code(c, "pageStartsOn", enums::PAGE_STARTS_ON, ctx).unwrap_or(0) << 20;
                s.page_number = c.int("page").unwrap_or(0) as u16;
                s.figure_number = c.int("pic").unwrap_or(0) as u16;
                s.table_number = c.int("tbl").unwrap_or(0) as u16;
                s.equation_number = c.int("equation").unwrap_or(0) as u16;
                true
            }
            (Ns::Hp, b"visibility") => {
                rest.attrs(
                    "visibility",
                    c,
                    &[
                        "hideFirstHeader",
                        "hideFirstFooter",
                        "hideFirstMasterPage",
                        "border",
                        "fill",
                        "hideFirstPageNum",
                        "hideFirstEmptyLine",
                    ],
                );
                let f = |k| c.flag(k).unwrap_or(false) as u32;
                attr |= f("hideFirstHeader")
                    | f("hideFirstFooter") << 1
                    | f("hideFirstMasterPage") << 2
                    | f("hideFirstPageNum") << 5
                    | f("hideFirstEmptyLine") << 19;
                match code(c, "border", enums::VISIBILITY, ctx).unwrap_or(0) {
                    1 => attr |= 1 << 3,
                    2 => attr |= 1 << 8,
                    _ => {}
                }
                match code(c, "fill", enums::VISIBILITY, ctx).unwrap_or(0) {
                    1 => attr |= 1 << 4,
                    2 => attr |= 1 << 9,
                    _ => {}
                }
                true
            }
            (Ns::Hp, b"pagePr") => {
                s.children
                    .push(SectionChild::Page(page_def(c, &mut rest, ctx)));
                true
            }
            (Ns::Hp, b"footNotePr" | b"endNotePr") => {
                let path = String::from_utf8_lossy(c.local()).into_owned();
                s.children.push(SectionChild::NoteShape(note_shape(
                    c, &path, &mut rest, ctx,
                )));
                notes += 1;
                true
            }
            (Ns::Hp, b"pageBorderFill") => {
                rest.attrs(
                    "pageBorderFill",
                    c,
                    &[
                        "type",
                        "borderFillIDRef",
                        "textBorder",
                        "headerInside",
                        "footerInside",
                        "fillArea",
                    ],
                );
                let mut a = code(c, "textBorder", enums::TEXT_BORDER, ctx).unwrap_or(0);
                a |= (c.flag("headerInside").unwrap_or(false) as u32) << 1;
                a |= (c.flag("footerInside").unwrap_or(false) as u32) << 2;
                a |= code(c, "fillArea", enums::FILL_AREA, ctx).unwrap_or(0) << 3;
                let o = c.child(Ns::Hp, "offset");
                let off = |k| o.and_then(|o| o.int(k)).unwrap_or(0) as i16;
                s.children.push(SectionChild::PageBorder(PageBorderFill {
                    attr: a,
                    offsets: [off("left"), off("right"), off("top"), off("bottom")],
                    border_fill: c.int("borderFillIDRef").unwrap_or(0) as u16,
                    tail: SharedBytes::empty(),
                }));
                true
            }
            _ => false,
        };
        if known {
            slot += 1;
        } else {
            rest.child("", slot, c);
        }
    }
    let _ = notes;
    s.attr = attr;
    ctx.note_rest(el, &rest);
    s.tail = rest.into_tail();
    s
}

fn page_def(el: El<'_>, rest: &mut Rest, ctx: &mut Ctx) -> PageDef {
    rest.attrs(
        "pagePr",
        el,
        &["landscape", "width", "height", "gutterType"],
    );
    let mut p = PageDef {
        width: el.u32("width").unwrap_or(59528),
        height: el.u32("height").unwrap_or(84186),
        attr: code(el, "landscape", enums::LANDSCAPE, ctx).unwrap_or(0)
            | code(el, "gutterType", enums::GUTTER, ctx).unwrap_or(0) << 1,
        ..Default::default()
    };
    if let Some(m) = el.child(Ns::Hp, "margin") {
        rest.attrs(
            "pagePr/margin",
            m,
            &[
                "header", "footer", "gutter", "left", "right", "top", "bottom",
            ],
        );
        let v = |k| m.u32(k).unwrap_or(0);
        p.margin_left = v("left");
        p.margin_right = v("right");
        p.margin_top = v("top");
        p.margin_bottom = v("bottom");
        p.margin_header = v("header");
        p.margin_footer = v("footer");
        p.gutter = v("gutter");
    }
    p
}

fn note_shape(el: El<'_>, path: &str, rest: &mut Rest, ctx: &mut Ctx) -> NoteShape {
    let mut n = NoteShape::default();
    let ch =
        |v: Option<std::borrow::Cow<'_, str>>| v.and_then(|s| s.encode_utf16().next()).unwrap_or(0);
    let endnote = path == "endNotePr";
    for c in el.children() {
        let p = format!("{path}/{}", String::from_utf8_lossy(c.local()));
        match c.local() {
            b"autoNumFormat" => {
                rest.attrs(
                    &p,
                    c,
                    &["type", "userChar", "prefixChar", "suffixChar", "supscript"],
                );
                n.attr |= code(c, "type", enums::NUMBER_FORMAT, ctx).unwrap_or(0) & 0xFF;
                n.attr |= (c.flag("supscript").unwrap_or(false) as u32) << 12;
                n.user_char = ch(c.attr("userChar"));
                n.prefix = ch(c.attr("prefixChar"));
                n.suffix = ch(c.attr("suffixChar"));
            }
            b"noteLine" => {
                rest.attrs(&p, c, &["length", "type", "width", "color"]);
                n.separator_length = c.i32("length").unwrap_or(0);
                n.line_kind = code(c, "type", enums::LINE_KIND, ctx).unwrap_or(0) as u8;
                n.line_width = c
                    .attr("width")
                    .and_then(|w| enums::width_index(&w))
                    .unwrap_or(0);
                n.line_color = attr_color(c, "color").unwrap_or(0);
            }
            b"noteSpacing" => {
                rest.attrs(&p, c, &["betweenNotes", "belowLine", "aboveLine"]);
                n.space_between = c.int("betweenNotes").unwrap_or(0) as i16;
                n.space_below = c.int("belowLine").unwrap_or(0) as i16;
                n.space_above = c.int("aboveLine").unwrap_or(0) as i16;
            }
            b"numbering" => {
                rest.attrs(&p, c, &["type", "newNum"]);
                n.attr |= code(c, "type", enums::NOTE_NUMBERING, ctx).unwrap_or(0) << 10;
                n.start = c.int("newNum").unwrap_or(1) as u16;
            }
            b"placement" => {
                rest.attrs(&p, c, &["place", "beneathText"]);
                let t = if endnote {
                    enums::ENDNOTE_PLACE
                } else {
                    enums::FOOTNOTE_PLACE
                };
                n.attr |= code(c, "place", t, ctx).unwrap_or(0) << 8;
                n.attr |= (c.flag("beneathText").unwrap_or(false) as u32) << 13;
            }
            _ => rest.child(path, 0, c),
        }
    }
    n
}

fn column_def(el: El<'_>, ctx: &mut Ctx) -> ColumnDef {
    let mut rest = Rest::new();
    rest.attrs(
        "",
        el,
        &["id", "type", "layout", "colCount", "sameSz", "sameGap"],
    );
    let count = el.int("colCount").unwrap_or(1).clamp(1, 255) as u16;
    let same = el.flag("sameSz").unwrap_or(true);
    let mut c = ColumnDef {
        attr: code(el, "type", enums::COLUMN_TYPE, ctx).unwrap_or(0) as u16
            | count << 2
            | (code(el, "layout", enums::COLUMN_LAYOUT, ctx).unwrap_or(0) as u16) << 10
            | (same as u16) << 12,
        gap: el.int("sameGap").unwrap_or(0) as i16,
        ..Default::default()
    };
    for (slot, k) in el.children().enumerate() {
        match k.local() {
            b"colSz" => {
                rest.attrs("colSz", k, &["width", "gap"]);
                c.columns.push((
                    k.int("width").unwrap_or(0) as i16,
                    k.int("gap").unwrap_or(0) as i16,
                ));
            }
            b"colLine" => {
                rest.attrs("colLine", k, &["type", "width", "color"]);
                c.line_kind = code(k, "type", enums::LINE_KIND, ctx).unwrap_or(0) as u8;
                c.line_width = k
                    .attr("width")
                    .and_then(|w| enums::width_index(&w))
                    .unwrap_or(0);
                c.line_color = attr_color(k, "color").unwrap_or(0);
            }
            _ => rest.child("", slot as u16, k),
        }
    }
    if same {
        c.columns.clear();
    }
    ctx.note_rest(el, &rest);
    c.tail = rest.into_tail();
    c
}
