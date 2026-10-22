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
fn placement(h: &ObjectHeader) -> [X; 3] {
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
        return equation(o);
    }
    let cap = o.caption().map(|c| caption(c, cx));
    match o.shape() {
        Some(s) => shape(s, Some(&o.header), cap, cx).with_rest(&o.header.tail),
        None => common_attrs(X::new("hp:rect"), &o.header).with_rest(&o.header.tail),
    }
}

fn equation(o: &ObjectControl) -> X {
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
    if let Some(c) = comment(h) {
        x = x.kid(c);
    }
    x.kid(
        X::new("hp:script")
            .a("xml:space", "preserve")
            .text(e.script.to_string_lossy()),
    )
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
        .a("endCap", name(read::END_CAP, (a >> 6) & 0xF))
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
                    .kid(X::new("hp:effects"));
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
    /// Hancom writes group offsets as unsigned 32-bit values.
    fn with_unsigned(mut self) -> X {
        for (_, v) in &mut self.attrs {
            if let Ok(n) = v.parse::<i32>() {
                *v = (n as u32).to_string();
            }
        }
        self
    }
}

pub fn table(t: &Table, cx: &mut Ctx<'_>) -> X {
    let h = &t.header;
    let pb = read_page_break(t.attr & 3);
    let mut x = common_attrs(X::new("hp:tbl"), h)
        .a("pageBreak", pb)
        .b("repeatHeader", t.attr & 4 != 0)
        .a("rowCnt", t.rows)
        .a("colCnt", t.cols)
        .a("cellSpacing", t.spacing)
        .a("borderFillIDRef", t.border_fill)
        .b("noAdjust", t.attr & 8 != 0)
        .kids(placement(h));
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
