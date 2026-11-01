//! Object controls and tables into OWPML elements.

use hm_model::{
    Caption, Cell, CtrlId, LineStyle, Matrix, ObjectControl, ObjectHeader, Shape, ShapePart,
    ShapeStyle, Table, TextBox,
};

use super::body::{Ctx, paragraphs};
use super::head::{bin_ref, fill_brush};
use super::{X, color, name};
use crate::enums;
use crate::object as read;

/// The element attributes the common header holds.
fn common_attrs(x: X, h: &ObjectHeader) -> X {
    let a = h.common.attr;
    x.a("id", h.instance_id.unwrap_or(0))
        .a("zOrder", h.common.z_order)
        .a("numberingType", name(enums::NUMBERING_TYPE, (a >> 26) & 7))
        .a("textWrap", name(enums::TEXT_WRAP, (a >> 21) & 7))
        .a("textFlow", name(enums::TEXT_FLOW, (a >> 24) & 3))
}

/// `hp:sz`, `hp:pos` and `hp:outMargin`.
pub(crate) fn placement(h: &ObjectHeader) -> [X; 3] {
    let c = &h.common;
    let a = c.attr;
    [
        X::new("hp:sz")
            .a("width", c.width)
            .a("widthRelTo", name(enums::WIDTH_REL_TO, (a >> 15) & 7))
            .a("height", c.height)
            .a("heightRelTo", name(enums::HEIGHT_REL_TO, (a >> 18) & 3))
            .b("protect", a & (1 << 20) != 0),
        X::new("hp:pos")
            .b("treatAsChar", a & 1 != 0)
            .b("affectLSpacing", a & 4 != 0)
            .b("flowWithText", a & (1 << 13) != 0)
            .b("allowOverlap", a & (1 << 14) != 0)
            .b("holdAnchorAndSO", h.prevent_split.unwrap_or(0) != 0)
            .a("vertRelTo", name(enums::VERT_REL_TO, (a >> 3) & 3))
            .a("horzRelTo", name(enums::HORZ_REL_TO, (a >> 8) & 3))
            .a("vertAlign", name(enums::VERT_ALIGN, (a >> 5) & 7))
            .a("horzAlign", name(enums::HORZ_ALIGN, (a >> 10) & 7))
            .a("vertOffset", c.y_offset)
            .a("horzOffset", c.x_offset),
        X::new("hp:outMargin")
            .a("left", c.margins[0])
            .a("right", c.margins[1])
            .a("top", c.margins[2])
            .a("bottom", c.margins[3]),
    ]
}

fn comment(h: &ObjectHeader) -> Option<X> {
    let d = h.description.as_ref()?.to_string_lossy();
    (!d.is_empty()).then(|| X::new("hp:shapeComment").text(d))
}

/// A `hp:subList` around a paragraph list.
fn sub_list(attr: u32, paras: &[hm_model::Paragraph], cx: &mut Ctx<'_>) -> X {
    let mut x = X::new("hp:subList")
        .a("textDirection", name(enums::TEXT_DIRECTION, attr & 7))
        .a("lineWrap", name(enums::LINE_WRAP, (attr >> 3) & 3))
        .a("vertAlign", name(read::SUBLIST_V_ALIGN, (attr >> 5) & 3));
    x.kids.extend(paragraphs(paras, cx));
    x
}

pub fn caption(c: &Caption, cx: &mut Ctx<'_>) -> X {
    let a = c.attr.unwrap_or(3);
    X::new("hp:caption")
        .a("side", name(read::CAPTION_SIDE, a & 3))
        .b("fullSz", a & 4 != 0)
        .a("width", c.width.unwrap_or(0))
        .a("gap", c.gap.unwrap_or(0))
        .a("lastWidth", c.max_width.unwrap_or(0))
        .kid(sub_list(c.list_attr, &c.paras, cx))
        .with_rest(&c.tail)
}

pub fn object(o: &ObjectControl, cx: &mut Ctx<'_>) -> X {
    if o.id == CtrlId::EQUATION {
        return equation(o, cx);
    }
    let cap = o.caption().map(|c| caption(c, cx));
    match o.shape() {
        Some(s) => shape(s, Some(&o.header), cap, cx).with_rest(&o.header.tail),
        None => common_attrs(X::new("hp:rect"), &o.header).with_rest(&o.header.tail),
    }
}

fn equation(o: &ObjectControl, cx: &mut Ctx<'_>) -> X {
    let h = &o.header;
    let mut x = common_attrs(X::new("hp:equation"), h);
    let e = o.equation().cloned().unwrap_or_default();
    if let Some(v) = &e.version {
        x = x.a("version", v.to_string_lossy());
    }
    x = x
        .a("baseLine", e.baseline)
        .a("textColor", color(e.color))
        .a("baseUnit", e.size)
        .a("lineMode", if e.attr & 1 != 0 { "LINE" } else { "CHAR" });
    if let Some(f) = &e.font {
        x = x.a("font", f.to_string_lossy());
    }
    x = x.kids(placement(h));
    if let Some(c) = o.caption() {
        x = x.kid(caption(c, cx));
    }
    if let Some(c) = comment(h) {
        x = x.kid(c);
    }
    // Hancom writes the control characters an HWP 5 script may hold, which
    // XML cannot, as spaces (windows1 capture of an HWP file with them
    // saved as HWPX); a script reads them as spaces either way.
    let script: String = e
        .script
        .to_string_lossy()
        .chars()
        .map(|c| {
            if c.is_control() && !matches!(c, '\t' | '\n' | '\r') {
                ' '
            } else {
                c
            }
        })
        .collect();
    x.kid(X::new("hp:script").a("xml:space", "preserve").text(script))
        .with_rest(&e.tail)
        .with_rest(&h.tail)
}

fn element_name(kind: CtrlId) -> &'static str {
    match &kind.chars() {
        b"$lin" => "hp:line",
        b"$rec" => "hp:rect",
        b"$ell" => "hp:ellipse",
        b"$arc" => "hp:arc",
        b"$pol" => "hp:polygon",
        b"$cur" => "hp:curve",
        b"$pic" => "hp:pic",
        b"$ole" => "hp:ole",
        b"$con" => "hp:container",
        b"$col" => "hp:connectLine",
        _ => "hp:rect",
    }
}

fn num(v: f64) -> String {
    // The shortest form that reads back as the same value.
    format!("{v}")
}

fn matrix(tag: &'static str, m: &Matrix) -> X {
    let mut x = X::new(tag);
    for (i, k) in ["e1", "e2", "e3", "e4", "e5", "e6"].iter().enumerate() {
        x = x.a(k, num(m.0[i]));
    }
    x
}

fn pt(tag: &'static str, p: (i32, i32)) -> X {
    X::new(tag).a("x", p.0).a("y", p.1)
}

pub fn line_shape(l: &LineStyle) -> X {
    let a = l.attr;
    let mut x = X::new("hp:lineShape")
        .a("color", color(l.color))
        .a("width", l.width)
        .a("style", name(enums::LINE_KIND, a & 0x3F))
        // HWP 5 files carry cap codes OWPML has no name for (15 in
        // hwpspec); every code but 0 draws a flat cap, the one other name.
        .a("endCap", name(read::END_CAP, ((a >> 6) & 0xF).min(1)))
        .a("headStyle", name(read::ARROW, (a >> 10) & 0x3F))
        .a("tailStyle", name(read::ARROW, (a >> 16) & 0x3F))
        .b("headfill", a & (1 << 30) != 0)
        .b("tailfill", a & (1 << 31) != 0)
        .a("headSz", name(read::ARROW_SIZE, (a >> 22) & 0xF))
        .a("tailSz", name(read::ARROW_SIZE, (a >> 26) & 0xF));
    if let Some(o) = l.outline {
        x = x.a("outlineStyle", name(read::OUTLINE_STYLE, o as u32));
    }
    x
}

fn style_kids(x: X, s: &ShapeStyle, bins: &[String]) -> X {
    let mut x = x.kid(line_shape(&s.line));
    if s.fill.kind != 0 {
        x = x.kid(fill_brush(&s.fill, bins));
    }
    let sh = s.shadow.unwrap_or_default();
    x.kid(
        X::new("hp:shadow")
            .a("type", name(read::SHADOW_KIND, sh.kind))
            .a("color", color(sh.color))
            .a("offsetX", sh.offset.0)
            .a("offsetY", sh.offset.1)
            .a("alpha", s.shadow_alpha.unwrap_or(0)),
    )
}

fn text_box(t: &TextBox, cx: &mut Ctx<'_>) -> X {
    let m = t.margins.unwrap_or_default();
    X::new("hp:drawText")
        .a("lastWidth", t.max_width.unwrap_or(0))
        .kid(sub_list(t.attr, &t.paras, cx))
        .kid(
            X::new("hp:textMargin")
                .a("left", m[0])
                .a("right", m[1])
                .a("top", m[2])
                .a("bottom", m[3]),
        )
}

/// A shape: the top-level element of an object (with its header) or a
/// group member.
fn shape(s: &Shape, h: Option<&ObjectHeader>, cap: Option<X>, cx: &mut Ctx<'_>) -> X {
    let c = &s.component;
    let tag = element_name(c.kind);
    let mut x = X::new(tag);
    if let Some(h) = h {
        x = common_attrs(x, h);
    }
    x = x.a("groupLevel", c.group_level);
    let instid = c
        .group_instance
        .or_else(|| c.style.as_ref().and_then(|st| st.instance_id))
        .or_else(|| {
            s.parts.iter().find_map(|p| match p {
                ShapePart::Picture(p) => Some(p.instance_id),
                ShapePart::Ole(o) => o.instance_id,
                _ => None,
            })
        });
    if let Some(i) = instid {
        x = x.a("instid", i);
    }
    for p in &s.parts {
        x = match p {
            ShapePart::Rect(r) => x.a("ratio", r.round),
            ShapePart::Ellipse(e) => x
                .b("intervalDirty", e.attr & 1 != 0)
                .b("hasArcPr", e.attr & 2 != 0)
                .a("arcType", name(read::ARC_TYPE, (e.attr >> 2) & 0xFF)),
            ShapePart::Arc(a) => x.a("type", name(read::ARC_TYPE, a.attr & 0xFF)),
            ShapePart::Line(l) if c.kind == CtrlId::from_chars(b"$col") => {
                x.a("type", name(read::CONNECT_TYPE, l.attr.unwrap_or(0) as u32))
            }
            ShapePart::Line(l) => x.a("isReverseHV", l.attr.unwrap_or(0)),
            ShapePart::Ole(o) => x
                .a("objectType", name(read::OLE_TYPE, (o.attr >> 16) & 0x3F))
                .a("binaryItemIDRef", bin_ref(cx.bins, o.bin_id as u32))
                .b("hasMoniker", o.attr & (1 << 8) != 0)
                .a("drawAspect", name(read::DRAW_ASPECT, o.attr & 0xFF))
                .a("eqBaseLine", (o.attr >> 9) & 0x7F),
            _ => x,
        };
    }
    let (w, hgt) = c.size;
    x = x
        .kid(pt("hp:offset", c.offset).with_unsigned())
        .kid(
            X::new("hp:orgSz")
                .a("width", c.initial_size.0)
                .a("height", c.initial_size.1),
        )
        .kid(X::new("hp:curSz").a("width", w).a("height", hgt))
        .kid(
            X::new("hp:flip")
                .b("horizontal", c.flip & 1 != 0)
                .b("vertical", c.flip & 2 != 0),
        )
        .kid(
            X::new("hp:rotationInfo")
                .a("angle", c.rotation)
                .a("centerX", c.rotation_center.0)
                .a("centerY", c.rotation_center.1)
                .b("rotateimage", c.flip & (1 << 19) != 0),
        );
    let mut ri = X::new("hp:renderingInfo").kid(matrix("hc:transMatrix", &c.translation));
    for (sca, rot) in &c.transforms {
        ri = ri
            .kid(matrix("hc:scaMatrix", sca))
            .kid(matrix("hc:rotMatrix", rot));
    }
    x = x.kid(ri);
    if let Some(st) = &c.style {
        x = style_kids(x, st, cx.bins);
    }
    for p in &s.parts {
        match p {
            ShapePart::Text(t) => x = x.kid(text_box(t, cx)),
            ShapePart::Child(m) => {
                let member = shape(m, None, None, cx).with_rest(&m.component.tail);
                x = x.kid(member);
            }
            ShapePart::Picture(pic) => {
                if pic.border != LineStyle::default() {
                    x = x.kid(line_shape(&pic.border));
                }
                let cr = pic.corners;
                x = x.kid(
                    X::new("hp:imgRect")
                        .kid(pt("hc:pt0", cr[0]))
                        .kid(pt("hc:pt1", cr[1]))
                        .kid(pt("hc:pt2", cr[2]))
                        .kid(pt("hc:pt3", cr[3])),
                );
                x = x
                    .kid(
                        X::new("hp:imgClip")
                            .a("left", pic.crop[0])
                            .a("right", pic.crop[2])
                            .a("top", pic.crop[1])
                            .a("bottom", pic.crop[3]),
                    )
                    .kid(
                        X::new("hp:inMargin")
                            .a("left", pic.margins[0])
                            .a("right", pic.margins[1])
                            .a("top", pic.margins[2])
                            .a("bottom", pic.margins[3]),
                    );
                let (dw, dh) = pic.original_size().unwrap_or((0, 0));
                x = x
                    .kid(X::new("hp:imgDim").a("dimwidth", dw).a("dimheight", dh))
                    .kid(
                        X::new("hc:img")
                            .a("binaryItemIDRef", bin_ref(cx.bins, pic.image.bin_id as u32))
                            .a("bright", pic.image.brightness)
                            .a("contrast", pic.image.contrast)
                            .a("effect", name(enums::IMG_EFFECT, pic.image.effect as u32))
                            .a("alpha", pic.alpha()),
                    )
                    .kid(effects(pic));
            }
            ShapePart::Ole(o) => {
                x = x.kid(pt("hc:extent", o.extent));
                if let Some(b) = &o.border {
                    x = x.kid(line_shape(b));
                }
            }
            ShapePart::Rect(r) => {
                for (i, t) in ["hc:pt0", "hc:pt1", "hc:pt2", "hc:pt3"].iter().enumerate() {
                    x = x.kid(pt(t, r.corners[i]));
                }
            }
            ShapePart::Ellipse(e) => {
                for (t, p) in [
                    ("hc:center", e.center),
                    ("hc:ax1", e.axis1),
                    ("hc:ax2", e.axis2),
                    ("hc:start1", e.start),
                    ("hc:end1", e.end),
                    ("hc:start2", e.start2),
                    ("hc:end2", e.end2),
                ] {
                    x = x.kid(pt(t, p));
                }
            }
            ShapePart::Arc(a) => {
                for (t, p) in [
                    ("hc:center", a.center),
                    ("hc:ax1", a.axis1),
                    ("hc:ax2", a.axis2),
                ] {
                    x = x.kid(pt(t, p));
                }
            }
            ShapePart::Line(l) if c.kind == CtrlId::from_chars(b"$col") => {
                let refs = l.connector.as_ref();
                let start = refs.map(|c| c.start).unwrap_or_default();
                let end = refs.map(|c| c.end).unwrap_or_default();
                x = x
                    .kid(
                        pt("hp:startPt", l.start)
                            .a("subjectIDRef", start.0)
                            .a("subjectIdx", start.1),
                    )
                    .kid(
                        pt("hp:endPt", l.end)
                            .a("subjectIDRef", end.0)
                            .a("subjectIdx", end.1),
                    );
                if let Some(c) = refs {
                    let mut points = X::new("hp:controlPoints");
                    for &(px, py, kind) in &c.points {
                        points = points.kid(pt("hp:point", (px, py)).a("type", kind));
                    }
                    x = x.kid(points);
                }
            }
            ShapePart::Line(l) => {
                x = x.kid(pt("hc:startPt", l.start)).kid(pt("hc:endPt", l.end));
            }
            ShapePart::Polygon(p) => {
                for &q in &p.points {
                    x = x.kid(pt("hc:pt", q));
                }
            }
            ShapePart::Curve(cv) => {
                for (i, w) in cv.points.windows(2).enumerate() {
                    let curve = cv.segments.get(i).copied().unwrap_or(0) != 0;
                    x = x.kid(
                        X::new("hp:seg")
                            .a("type", if curve { "CURVE" } else { "LINE" })
                            .a("x1", w[0].0)
                            .a("y1", w[0].1)
                            .a("x2", w[1].0)
                            .a("y2", w[1].1),
                    );
                }
            }
            ShapePart::Raw(_) => {}
        }
    }
    if let Some(h) = h {
        x = x.kids(placement(h));
        if let Some(cap) = cap {
            x = x.kid(cap);
        }
        if let Some(c) = comment(h) {
            x = x.kid(c);
        }
    }
    x
}

impl X {
    /// Hancom writes group and table offsets as unsigned 32-bit values.
    fn with_unsigned(mut self) -> X {
        for (_, v) in &mut self.attrs {
            if let Ok(n) = v.parse::<i32>() {
                *v = (n as u32).to_string();
            }
        }
        self
    }
}

/// The grid without rows that no cell starts in: merging cells across
/// whole rows leaves them, Hancom refuses an HWPX table row without cells,
/// and Hancom itself drops such rows, shortening the spans over them.
fn collapsed(t: &Table) -> std::borrow::Cow<'_, Table> {
    let gone: Vec<u16> = (0..t.rows)
        .filter(|&r| !t.cells.iter().any(|c| c.row == r))
        .collect();
    if gone.is_empty() || gone.len() == t.rows as usize {
        return std::borrow::Cow::Borrowed(t);
    }
    let before = |at: u16| gone.iter().filter(|&&g| g < at).count() as u16;
    let mut t = t.clone();
    for c in &mut t.cells {
        c.row_span -= gone
            .iter()
            .filter(|&&g| g >= c.row && g < c.row + c.row_span)
            .count() as u16;
        c.row -= before(c.row);
    }
    if let Some(zones) = &mut t.zones {
        for z in zones {
            z.end_row -= before(z.end_row + 1);
            z.start_row -= before(z.start_row);
        }
    }
    t.rows -= gone.len() as u16;
    t.row_cells = (0..t.rows)
        .map(|r| t.cells.iter().filter(|c| c.row == r).count() as u16)
        .collect();
    std::borrow::Cow::Owned(t)
}

pub fn table(t: &Table, cx: &mut Ctx<'_>) -> X {
    let t = &*collapsed(t);
    let h = &t.header;
    let pb = read_page_break(t.attr & 3);
    let [size, pos, margins] = placement(h);
    let mut x = common_attrs(X::new("hp:tbl"), h)
        .b("lock", h.common.attr & (1 << 30) != 0)
        .a("pageBreak", pb)
        .b("repeatHeader", t.attr & 4 != 0)
        .a("rowCnt", t.rows)
        .a("colCnt", t.cols)
        .a("cellSpacing", t.spacing)
        .a("borderFillIDRef", t.border_fill)
        .b("noAdjust", t.attr & 8 != 0)
        .kids([size, pos.with_unsigned(), margins]);
    if let Some(c) = &t.caption {
        x = x.kid(caption(c, cx));
    }
    x = x.kid(
        X::new("hp:inMargin")
            .a("left", t.padding[0])
            .a("right", t.padding[1])
            .a("top", t.padding[2])
            .a("bottom", t.padding[3]),
    );
    if let Some(z) = t.zones.as_ref().filter(|z| !z.is_empty()) {
        x = x.kid(X::new("hp:cellzoneList").kids(z.iter().map(|z| {
            X::new("hp:cellzone")
                .a("startRowAddr", z.start_row)
                .a("startColAddr", z.start_col)
                .a("endRowAddr", z.end_row)
                .a("endColAddr", z.end_col)
                .a("borderFillIDRef", z.border_fill)
        })));
    }
    let mut cells = t.cells.iter();
    for &n in &t.row_cells {
        let mut tr = X::new("hp:tr");
        for c in cells.by_ref().take(n as usize) {
            tr = tr.kid(cell(c, cx));
        }
        x = x.kid(tr);
    }
    if let Some(c) = comment(h) {
        x = x.kid(c);
    }
    x.with_rest(&h.tail)
}

fn read_page_break(code: u32) -> String {
    name(crate::table::PAGE_BREAK, code)
}

fn cell(c: &Cell, cx: &mut Ctx<'_>) -> X {
    let a = c.list_attr;
    X::new("hp:tc")
        .a(
            "name",
            c.field_name
                .as_ref()
                .map(|n| n.to_string_lossy())
                .unwrap_or_default(),
        )
        .b("header", a & (1 << 18) != 0)
        .b("hasMargin", a & (1 << 16) != 0)
        .b("protect", a & (1 << 17) != 0)
        .b("editable", a & (1 << 19) != 0)
        .a("borderFillIDRef", c.border_fill)
        .kid(sub_list(a, &c.paras, cx))
        .kid(
            X::new("hp:cellAddr")
                .a("colAddr", c.col)
                .a("rowAddr", c.row),
        )
        .kid(
            X::new("hp:cellSpan")
                .a("colSpan", c.col_span)
                .a("rowSpan", c.row_span),
        )
        .kid(
            X::new("hp:cellSz")
                .a("width", c.width)
                .a("height", c.height),
        )
        .kid(
            X::new("hp:cellMargin")
                .a("left", c.padding[0])
                .a("right", c.padding[1])
                .a("top", c.padding[2])
                .a("bottom", c.padding[3]),
        )
        .with_rest(&c.tail)
}

#[cfg(test)]
mod equation_caption_tests {
    use super::*;
    use crate::{
        origin::Parts,
        read,
        xml::{Ns, Xml},
    };
    use hm_model::SharedBytes;

    #[test]
    fn edited_equation_keeps_typed_caption_and_unknown_markup() {
        let xml = Xml::parse(SharedBytes::from(&br#"<root xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph"><hp:equation id="7" baseUnit="950" futureFlag="equation"><hp:sz width="2000" height="1000"/><hp:caption side="BOTTOM" width="8504" gap="850" lastWidth="2000" futureFlag="caption"><hp:subList textDirection="HORIZONTAL" lineWrap="BREAK" vertAlign="TOP"><hp:p id="0" paraPrIDRef="0" styleIDRef="0"><hp:run charPrIDRef="0"><hp:t>Original caption</hp:t></hp:run></hp:p></hp:subList><hp:futureCaption marker="keep"/></hp:caption><hp:script>x over y</hp:script><hp:futureEquation marker="keep"/></hp:equation></root>"#[..])).unwrap();
        let mut control = read::Ctx::default();
        let mut equation =
            crate::object::equation(xml.root().child(Ns::Hp, "equation").unwrap(), &mut control);
        assert_eq!(
            equation.caption().unwrap().paras[0].plain_text(),
            "Original caption"
        );
        for part in &mut equation.parts {
            if let hm_model::ObjectPart::Equation(e) = part {
                e.script = hm_model::HwpString::from_text("x over {z}");
            }
        }
        let parts = Parts::none();
        let mut cx = Ctx {
            bins: &[],
            fields: Vec::new(),
            parts: &parts,
            masters: None,
            extended_masters: Vec::new(),
        };
        let mut output = Vec::new();
        object(&equation, &mut cx).write(&mut output);
        let output = String::from_utf8(output).unwrap();
        for preserved in [
            "futureFlag=\"equation\"",
            "futureFlag=\"caption\"",
            "hp:futureEquation",
            "hp:futureCaption",
        ] {
            assert!(output.contains(preserved), "{preserved}");
        }
        assert_eq!(output.matches("<hp:caption").count(), 1);
        let wrapped = format!(
            "<root xmlns:hp=\"http://www.hancom.co.kr/hwpml/2011/paragraph\">{output}</root>"
        );
        let reopened = Xml::parse(wrapped.into_bytes().into()).unwrap();
        let parsed = crate::object::equation(
            reopened.root().child(Ns::Hp, "equation").unwrap(),
            &mut read::Ctx::default(),
        );
        assert_eq!(
            parsed.caption().unwrap().paras[0].plain_text(),
            "Original caption"
        );
        assert_eq!(
            parsed.equation().unwrap().script.to_string_lossy(),
            "x over {z}"
        );
    }
}

/// `hp:effects` of a picture: the outside shadow and soft edge Hancom writes
/// in both formats, from their HWP 5 layout (tables 108 to 113). Glows and
/// reflections are left out.
fn effects(pic: &hm_model::Picture) -> X {
    let mut x = X::new("hp:effects");
    let t = pic.effect_bytes();
    let flags = pic.effects();
    let f32at = |o: usize| -> f32 {
        t.get(o..o + 4)
            .map_or(0.0, |b| f32::from_le_bytes(b.try_into().unwrap()))
    };
    let u32at = |o: usize| -> u32 {
        t.get(o..o + 4)
            .map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap()))
    };
    let mut at = 4;
    if flags & 1 != 0 {
        // An outside shadow with an RGB color.
        if u32at(at) == 0 && u32at(at + 44) == 0 {
            let rgb = u32at(at + 48);
            x = x.kid(
                X::new("hp:shadow")
                    .a("style", "OUTSIDE")
                    .a("alpha", f32at(at + 4))
                    .a("radius", f32at(at + 8))
                    .a("direction", f32at(at + 12))
                    .a("distance", f32at(at + 16))
                    .a("alignStyle", name(read::SHADOW_ALIGN, u32at(at + 20)))
                    .a("rotationStyle", u32at(at + 40).min(1))
                    .kid(
                        X::new("hp:skew")
                            .a("x", f32at(at + 24))
                            .a("y", f32at(at + 28)),
                    )
                    .kid(
                        X::new("hp:scale")
                            .a("x", f32at(at + 32))
                            .a("y", f32at(at + 36)),
                    )
                    .kid(
                        X::new("hp:effectsColor")
                            .a("type", "RGB")
                            .a("schemeIdx", -1)
                            .a("systemIdx", -1)
                            .a("presetIdx", -1)
                            .kid(
                                X::new("hp:rgb")
                                    .a("r", rgb >> 16 & 0xFF)
                                    .a("g", rgb >> 8 & 0xFF)
                                    .a("b", rgb & 0xFF),
                            ),
                    ),
            );
        }
        let kind = u32at(at + 44);
        let len = if matches!(kind, 5 | 6) { 12 } else { 4 };
        let n = (u32at(at + 48 + len) as usize).min(64);
        at += 44 + 4 + len + 4 + 8 * n;
    }
    if flags & 2 != 0 {
        let kind = u32at(at + 8);
        let len = if matches!(kind, 5 | 6) { 12 } else { 4 };
        let n = (u32at(at + 12 + len) as usize).min(64);
        at += 8 + 4 + len + 4 + 8 * n;
    }
    if flags & 4 != 0 {
        x = x.kid(X::new("hp:softEdge").a("radius", f32at(at)));
    }
    x
}
