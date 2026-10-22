//! Object elements (`hp:tbl`, `hp:pic`, shapes, `hp:equation`): the common
//! object attributes and the typed object controls.

use hm_model::{
    ArcGeom, Caption, CtrlId, CurveGeom, EllipseGeom, Equation, HwpString, ImageRef, LineGeom,
    LineStyle, Matrix, ObjectCommon, ObjectControl, ObjectHeader, ObjectPart, Ole, Picture,
    PolyGeom, RectGeom, Shadow, Shape, ShapeComponent, ShapePart, ShapeStyle, SharedBytes, TextBox,
};

use crate::body;
use crate::enums;
use crate::head::{attr_color, code, fill_brush};
use crate::read::Ctx;
use crate::rest::Rest;
use crate::xml::{El, Ns};

/// Attributes of an object element that the common header covers.
pub const COMMON_ATTRS: &[&str] = &["id", "zOrder", "numberingType", "textWrap", "textFlow"];

/// The common object attributes from an object element's `hp:sz`, `hp:pos`
/// and `hp:outMargin` children and its own attributes (HWP 5 table 70),
/// with the instance id. Unmapped attributes of those children go to `rest`.
pub fn common(el: El<'_>, rest: &mut Rest, ctx: &mut Ctx) -> (ObjectCommon, u32) {
    let mut c = ObjectCommon {
        z_order: el.i32("zOrder").unwrap_or(0),
        ..Default::default()
    };
    let mut attr = 0u32;
    if let Some(sz) = el.child(Ns::Hp, "sz") {
        rest.attrs(
            "sz",
            sz,
            &["width", "widthRelTo", "height", "heightRelTo", "protect"],
        );
        c.width = sz.i32("width").unwrap_or(0);
        c.height = sz.i32("height").unwrap_or(0);
        attr |= code(sz, "widthRelTo", enums::WIDTH_REL_TO, ctx).unwrap_or(4) << 15;
        attr |= code(sz, "heightRelTo", enums::HEIGHT_REL_TO, ctx).unwrap_or(2) << 18;
        attr |= (sz.flag("protect").unwrap_or(false) as u32) << 20;
    } else {
        attr |= 4 << 15 | 2 << 18;
    }
    if let Some(pos) = el.child(Ns::Hp, "pos") {
        rest.attrs(
            "pos",
            pos,
            &[
                "treatAsChar",
                "affectLSpacing",
                "flowWithText",
                "allowOverlap",
                "vertRelTo",
                "horzRelTo",
                "vertAlign",
                "horzAlign",
                "vertOffset",
                "horzOffset",
            ],
        );
        let f = |k| pos.flag(k).unwrap_or(false) as u32;
        attr |= f("treatAsChar") | f("affectLSpacing") << 2;
        attr |= code(pos, "vertRelTo", enums::VERT_REL_TO, ctx).unwrap_or(2) << 3;
        attr |= code(pos, "vertAlign", enums::VERT_ALIGN, ctx).unwrap_or(0) << 5;
        attr |= code(pos, "horzRelTo", enums::HORZ_REL_TO, ctx).unwrap_or(3) << 8;
        attr |= code(pos, "horzAlign", enums::HORZ_ALIGN, ctx).unwrap_or(0) << 10;
        attr |= f("flowWithText") << 13 | f("allowOverlap") << 14;
        c.y_offset = pos.i32("vertOffset").unwrap_or(0);
        c.x_offset = pos.i32("horzOffset").unwrap_or(0);
    }
    if let Some(m) = el.child(Ns::Hp, "outMargin") {
        rest.attrs("outMargin", m, &["left", "right", "top", "bottom"]);
        let v = |k| m.int(k).unwrap_or(0) as i16;
        c.margins = [v("left"), v("right"), v("top"), v("bottom")];
    }
    attr |= code(el, "textWrap", enums::TEXT_WRAP, ctx).unwrap_or(0) << 21;
    attr |= code(el, "textFlow", enums::TEXT_FLOW, ctx).unwrap_or(0) << 24;
    attr |= code(el, "numberingType", enums::NUMBERING_TYPE, ctx).unwrap_or(0) << 26;
    c.attr = attr;
    (c, el.u32("id").unwrap_or(0))
}

/// The 32 bytes of the common attributes after the control id.
pub fn encode_common(c: &ObjectCommon) -> Vec<u8> {
    let mut v = Vec::with_capacity(32);
    for x in [
        c.attr as i32,
        c.y_offset,
        c.x_offset,
        c.width,
        c.height,
        c.z_order,
    ] {
        v.extend_from_slice(&x.to_le_bytes());
    }
    for m in c.margins {
        v.extend_from_slice(&m.to_le_bytes());
    }
    v
}

/// Children of an object element that the header covers.
pub fn is_header_child(c: El<'_>) -> bool {
    c.ns() == Ns::Hp && matches!(c.local(), b"sz" | b"pos" | b"outMargin" | b"shapeComment")
}

/// The object header of an object element.
pub fn header(el: El<'_>, rest: &mut Rest, ctx: &mut Ctx) -> ObjectHeader {
    let (common, instance) = common(el, rest, ctx);
    let description = el
        .child(Ns::Hp, "shapeComment")
        .map(|c| HwpString::from_text(&c.text()));
    ObjectHeader {
        common,
        instance_id: Some(instance),
        prevent_split: Some(0),
        description: Some(description.unwrap_or_default()),
        tail: Default::default(),
    }
}

pub fn equation(el: El<'_>, ctx: &mut Ctx) -> ObjectControl {
    let mut hrest = Rest::new();
    let mut known: Vec<&str> = COMMON_ATTRS.to_vec();
    known.extend_from_slice(&[
        "version",
        "baseLine",
        "textColor",
        "baseUnit",
        "lineMode",
        "font",
    ]);
    hrest.attrs("", el, &known);
    let mut header = header(el, &mut hrest, ctx);
    let mut erest = Rest::new();
    let mut script = String::new();
    for c in el.children() {
        if c.is(Ns::Hp, "script") {
            script = c.text();
        } else if !is_header_child(c) {
            erest.child("", c);
        }
    }
    let line_mode = el
        .attr("lineMode")
        .is_some_and(|m| m.eq_ignore_ascii_case("LINE"));
    let eq = Equation {
        attr: line_mode as u32,
        script: HwpString::from_text(&script),
        size: el.u32("baseUnit").unwrap_or(1000),
        color: attr_color(el, "textColor").unwrap_or(0),
        baseline: el.int("baseLine").unwrap_or(85) as i16,
        reserved: Some(0),
        version: el.attr("version").map(|v| HwpString::from_text(&v)),
        font: el.attr("font").map(|v| HwpString::from_text(&v)),
        tail: Default::default(),
    };
    ctx.note_rest(el, &hrest);
    ctx.note_rest(el, &erest);
    header.tail = hrest.into_tail();
    ObjectControl {
        id: CtrlId::EQUATION,
        header,
        parts: vec![ObjectPart::Equation(Equation {
            tail: erest.into_tail(),
            ..eq
        })],
    }
}

const LINE_ID: CtrlId = CtrlId::from_chars(b"$lin");
const RECT_ID: CtrlId = CtrlId::from_chars(b"$rec");
const ELLIPSE_ID: CtrlId = CtrlId::from_chars(b"$ell");
const ARC_ID: CtrlId = CtrlId::from_chars(b"$arc");
const POLYGON_ID: CtrlId = CtrlId::from_chars(b"$pol");
const CURVE_ID: CtrlId = CtrlId::from_chars(b"$cur");
const PICTURE_ID: CtrlId = CtrlId::from_chars(b"$pic");
const OLE_ID: CtrlId = CtrlId::from_chars(b"$ole");
const GROUP_ID: CtrlId = CtrlId::from_chars(b"$con");
/// Connectors: the model keeps their geometry as markup.
const CONNECTOR_ID: CtrlId = CtrlId::from_chars(b"$col");

/// The shape component kind of a drawing element, when the model types it.
pub fn shape_kind(el: El<'_>) -> Option<CtrlId> {
    if !matches!(el.ns(), Ns::Hp | Ns::Hp10) {
        return None;
    }
    Some(match el.local() {
        b"line" => LINE_ID,
        b"rect" => RECT_ID,
        b"ellipse" => ELLIPSE_ID,
        b"arc" => ARC_ID,
        b"polygon" => POLYGON_ID,
        b"curve" => CURVE_ID,
        b"pic" => PICTURE_ID,
        b"ole" => OLE_ID,
        b"container" => GROUP_ID,
        b"connectLine" => CONNECTOR_ID,
        _ => return None,
    })
}

fn wrap(v: Option<i64>) -> i32 {
    v.unwrap_or(0) as i32
}

fn pt(el: Option<El<'_>>) -> (i32, i32) {
    el.map_or((0, 0), |p| (wrap(p.int("x")), wrap(p.int("y"))))
}

fn matrix(el: El<'_>) -> Matrix {
    let mut m = Matrix::IDENTITY.0;
    for (i, k) in ["e1", "e2", "e3", "e4", "e5", "e6"].iter().enumerate() {
        if let Some(v) = el.attr(k).and_then(|v| v.trim().parse::<f64>().ok()) {
            m[i] = v;
        }
    }
    Matrix(m)
}

pub const ARROW: crate::enums::Table = &[
    ("NORMAL", 0),
    ("ARROW", 1),
    ("SPEAR", 2),
    ("CONCAVE_ARROW", 3),
    ("EMPTY_DIAMOND", 4),
    ("EMPTY_CIRCLE", 5),
    ("EMPTY_BOX", 6),
    ("FILLED_DIAMOND", 7),
    ("FILLED_CIRCLE", 8),
    ("FILLED_BOX", 9),
    ("NONE", 0),
];

pub const ARROW_SIZE: crate::enums::Table = &[
    ("SMALL_SMALL", 0),
    ("SMALL_MEDIUM", 1),
    ("SMALL_LARGE", 2),
    ("MEDIUM_SMALL", 3),
    ("MEDIUM_MEDIUM", 4),
    ("MEDIUM_LARGE", 5),
    ("LARGE_SMALL", 6),
    ("LARGE_MEDIUM", 7),
    ("LARGE_LARGE", 8),
];

pub const END_CAP: crate::enums::Table = &[("ROUND", 0), ("FLAT", 1)];

pub const OUTLINE_STYLE: crate::enums::Table = &[("NORMAL", 0), ("OUTER", 1), ("INNER", 2)];

/// A `hp:lineShape` as the outline of table 87.
pub fn line_style(
    el: El<'_>,
    outline: bool,
    rest: &mut Rest,
    path: &str,
    ctx: &mut Ctx,
) -> LineStyle {
    rest.attrs(
        path,
        el,
        &[
            "color",
            "width",
            "style",
            "endCap",
            "headStyle",
            "tailStyle",
            "headfill",
            "tailfill",
            "headSz",
            "tailSz",
            "outlineStyle",
            "alpha",
        ],
    );
    let mut attr = code(el, "style", enums::LINE_KIND, ctx).unwrap_or(0) & 0x3F;
    attr |= (code(el, "endCap", END_CAP, ctx).unwrap_or(0) & 0xF) << 6;
    attr |= (code(el, "headStyle", ARROW, ctx).unwrap_or(0) & 0x3F) << 10;
    attr |= (code(el, "tailStyle", ARROW, ctx).unwrap_or(0) & 0x3F) << 16;
    attr |= (code(el, "headSz", ARROW_SIZE, ctx).unwrap_or(0) & 0xF) << 22;
    attr |= (code(el, "tailSz", ARROW_SIZE, ctx).unwrap_or(0) & 0xF) << 26;
    attr |= (el.flag("headfill").unwrap_or(false) as u32) << 30;
    attr |= (el.flag("tailfill").unwrap_or(false) as u32) << 31;
    LineStyle {
        color: attr_color(el, "color").unwrap_or(0),
        width: el.i32("width").unwrap_or(0),
        attr,
        outline: outline.then(|| code(el, "outlineStyle", OUTLINE_STYLE, ctx).unwrap_or(0) as u8),
    }
}

pub const SHADOW_KIND: crate::enums::Table = &[
    ("NONE", 0),
    ("PARELLEL_LEFTTOP", 1),
    ("PARELLEL_RIGHTTOP", 2),
    ("PARELLEL_LEFTBOTTOM", 3),
    ("PARELLEL_RIGHTBOTTOM", 4),
    ("SHEAR_LEFTTOP", 5),
    ("SHEAR_RIGHTTOP", 6),
    ("SHEAR_LEFTBOTTOM", 7),
    ("SHEAR_RIGHTBOTTOM", 8),
    ("SHEAR_LEFTTOP_TRANS", 9),
    ("SHEAR_RIGHTTOP_TRANS", 10),
    ("SHEAR_LEFTBOTTOM_TRANS", 11),
    ("SHEAR_RIGHTBOTTOM_TRANS", 12),
    ("SHEAR_LEFTTOP_SCALE", 13),
    ("SHEAR_RIGHTTOP_SCALE", 14),
    ("SHEAR_LEFTBOTTOM_SCALE", 15),
    ("SHEAR_RIGHTBOTTOM_SCALE", 16),
    ("ROUND", 17),
    ("DROP", 18),
];

fn component(el: El<'_>, kind: CtrlId, top: bool, rest: &mut Rest) -> ShapeComponent {
    let size = |n: &str| {
        el.child(Ns::Hp, n).map_or((0, 0), |s| {
            (s.u32("width").unwrap_or(0), s.u32("height").unwrap_or(0))
        })
    };
    let initial = size("orgSz");
    let cur = size("curSz");
    let mut c = ShapeComponent {
        kind,
        doubled: top,
        offset: pt(el.child(Ns::Hp, "offset")),
        group_level: el.int("groupLevel").unwrap_or(0) as u16,
        version: 1,
        initial_size: initial,
        // A zero current size dimension means the original one.
        size: (
            if cur.0 == 0 { initial.0 } else { cur.0 },
            if cur.1 == 0 { initial.1 } else { cur.1 },
        ),
        flip: 0,
        rotation: 0,
        rotation_center: (0, 0),
        translation: Matrix::IDENTITY,
        transforms: Vec::new(),
        style: None,
        members: None,
        group_instance: None,
        tail: SharedBytes::empty(),
    };
    if let Some(f) = el.child(Ns::Hp, "flip") {
        rest.attrs("flip", f, &["horizontal", "vertical"]);
        c.flip = f.flag("horizontal").unwrap_or(false) as u32
            | (f.flag("vertical").unwrap_or(false) as u32) << 1;
    }
    if let Some(r) = el.child(Ns::Hp, "rotationInfo") {
        rest.attrs(
            "rotationInfo",
            r,
            &["angle", "centerX", "centerY", "rotateimage"],
        );
        c.rotation = r.int("angle").unwrap_or(0) as i16;
        c.rotation_center = (wrap(r.int("centerX")), wrap(r.int("centerY")));
        c.flip |= (r.flag("rotateimage").unwrap_or(false) as u32) << 19;
    }
    if let Some(r) = el.child(Ns::Hp, "renderingInfo") {
        let mut scale = None;
        for m in r.children() {
            match m.local() {
                b"transMatrix" => c.translation = matrix(m),
                b"scaMatrix" => scale = Some(matrix(m)),
                b"rotMatrix" => {
                    c.transforms
                        .push((scale.take().unwrap_or(Matrix::IDENTITY), matrix(m)));
                }
                _ => {}
            }
        }
    }
    c
}

fn style(el: El<'_>, rest: &mut Rest, ctx: &mut Ctx) -> ShapeStyle {
    let mut s = ShapeStyle {
        line: LineStyle {
            outline: Some(0),
            ..Default::default()
        },
        fill: hm_model::Fill {
            extra: Some(SharedBytes::empty()),
            ..Default::default()
        },
        shadow: None,
        instance_id: Some(el.u32("instid").unwrap_or(0)),
        reserved: Some(0),
        shadow_alpha: Some(0),
    };
    if let Some(l) = el.child(Ns::Hp, "lineShape") {
        s.line = line_style(l, true, rest, "lineShape", ctx);
    }
    if let Some(f) = el.child(Ns::Hc, "fillBrush") {
        s.fill = fill_brush(f, rest, "fillBrush", ctx);
    }
    let sh = el.child(Ns::Hp, "shadow");
    if let Some(sh) = sh {
        rest.attrs(
            "shadow",
            sh,
            &["type", "color", "offsetX", "offsetY", "alpha"],
        );
        s.shadow_alpha = Some(sh.int("alpha").unwrap_or(0).clamp(0, 255) as u8);
    }
    s.shadow = Some(Shadow {
        kind: sh
            .and_then(|e| code(e, "type", SHADOW_KIND, ctx))
            .unwrap_or(0),
        color: sh
            .and_then(|e| attr_color(e, "color"))
            .unwrap_or(0x00B2_B2B2),
        offset: sh.map_or((0, 0), |e| (wrap(e.int("offsetX")), wrap(e.int("offsetY")))),
    });
    s
}

/// A `hp:drawText` as a text box.
fn text_box(el: El<'_>, rest: &mut Rest, ctx: &mut Ctx) -> TextBox {
    rest.attrs("drawText", el, &["lastWidth"]);
    let mut t = TextBox {
        max_width: Some(el.i32("lastWidth").unwrap_or(0)),
        margins: Some([0; 4]),
        ..Default::default()
    };
    if let Some(m) = el.child(Ns::Hp, "textMargin") {
        let v = |k| m.int(k).unwrap_or(0) as i16;
        t.margins = Some([v("left"), v("right"), v("top"), v("bottom")]);
    }
    if let Some(l) = el.child(Ns::Hp, "subList") {
        rest.attrs(
            "drawText/subList",
            l,
            &["textDirection", "lineWrap", "vertAlign"],
        );
        t.attr = list_attr(l, ctx);
        t.paras = body::paragraphs(l, ctx);
        t.count = t.paras.len() as u32;
    }
    t
}

pub const SUBLIST_V_ALIGN: crate::enums::Table = &[("TOP", 0), ("CENTER", 1), ("BOTTOM", 2)];

/// The paragraph list attribute bits of a `hp:subList`: text direction,
/// line wrap and vertical alignment.
pub fn list_attr(l: El<'_>, ctx: &mut Ctx) -> u32 {
    code(l, "textDirection", enums::TEXT_DIRECTION, ctx).unwrap_or(0) & 7
        | (code(l, "lineWrap", enums::LINE_WRAP, ctx).unwrap_or(0) & 3) << 3
        | (code(l, "vertAlign", SUBLIST_V_ALIGN, ctx).unwrap_or(0) & 3) << 5
}

pub const CAPTION_SIDE: crate::enums::Table =
    &[("LEFT", 0), ("RIGHT", 1), ("TOP", 2), ("BOTTOM", 3)];

pub fn caption(el: El<'_>, ctx: &mut Ctx) -> Caption {
    let mut rest = Rest::new();
    rest.attrs("", el, &["side", "fullSz", "width", "gap", "lastWidth"]);
    let mut c = Caption {
        attr: Some(
            code(el, "side", CAPTION_SIDE, ctx).unwrap_or(3)
                | (el.flag("fullSz").unwrap_or(false) as u32) << 2,
        ),
        width: Some(el.i32("width").unwrap_or(0)),
        gap: Some(el.int("gap").unwrap_or(0) as i16),
        max_width: Some(el.i32("lastWidth").unwrap_or(0)),
        ..Default::default()
    };
    for k in el.children() {
        if k.is(Ns::Hp, "subList") {
            rest.attrs("subList", k, &["textDirection", "lineWrap", "vertAlign"]);
            c.list_attr = list_attr(k, ctx);
            c.paras = body::paragraphs(k, ctx);
            c.count = c.paras.len() as u32;
        } else {
            rest.child("", k);
        }
    }
    ctx.note_rest(el, &rest);
    c.tail = rest.into_tail();
    c
}

/// Attributes of a drawing element the model covers, by kind.
fn element_attrs(kind: CtrlId) -> &'static [&'static str] {
    match kind {
        RECT_ID => &["ratio"],
        ELLIPSE_ID => &["intervalDirty", "hasArcPr", "arcType"],
        ARC_ID => &["type"],
        LINE_ID => &["isReverseHV"],
        CONNECTOR_ID => &["type"],
        OLE_ID => &[
            "objectType",
            "binaryItemIDRef",
            "hasMoniker",
            "drawAspect",
            "eqBaseLine",
        ],
        _ => &[],
    }
}

/// Child elements of a drawing element the model covers, by kind, in the
/// order Hancom writes them.
pub fn element_children(kind: CtrlId) -> &'static [&'static str] {
    match kind {
        PICTURE_ID => &[
            "offset",
            "orgSz",
            "curSz",
            "flip",
            "rotationInfo",
            "renderingInfo",
            "lineShape",
            "imgRect",
            "imgClip",
            "inMargin",
            "imgDim",
            "img",
            "effects",
            "sz",
            "pos",
            "outMargin",
            "caption",
            "shapeComment",
        ],
        OLE_ID => &[
            "offset",
            "orgSz",
            "curSz",
            "flip",
            "rotationInfo",
            "renderingInfo",
            "extent",
            "lineShape",
            "sz",
            "pos",
            "outMargin",
            "caption",
            "shapeComment",
        ],
        GROUP_ID => &[
            "offset",
            "orgSz",
            "curSz",
            "flip",
            "rotationInfo",
            "renderingInfo",
            "sz",
            "pos",
            "outMargin",
            "caption",
            "shapeComment",
        ],
        _ => &[
            "offset",
            "orgSz",
            "curSz",
            "flip",
            "rotationInfo",
            "renderingInfo",
            "lineShape",
            "fillBrush",
            "shadow",
            "drawText",
            "pt0",
            "pt1",
            "pt2",
            "pt3",
            "center",
            "ax1",
            "ax2",
            "start1",
            "end1",
            "start2",
            "end2",
            "startPt",
            "endPt",
            "pt",
            "seg",
            "sz",
            "pos",
            "outMargin",
            "caption",
            "shapeComment",
        ],
    }
}

fn is_member(c: El<'_>) -> bool {
    shape_kind(c).is_some()
}

/// Records the element's own attributes and the children the model does
/// not cover.
fn element_rest(el: El<'_>, kind: CtrlId, top: bool, rest: &mut Rest) {
    // Members of a group have no object header: their placement
    // attributes stay markup.
    let mut known: Vec<&str> = if top {
        COMMON_ATTRS.to_vec()
    } else {
        Vec::new()
    };
    known.extend_from_slice(&["groupLevel", "instid"]);
    known.extend_from_slice(element_attrs(kind));
    rest.attrs("", el, &known);
    let covered = element_children(kind);
    for c in el.children() {
        let name = std::str::from_utf8(c.local()).unwrap_or("");
        if covered.contains(&name) || (kind == GROUP_ID && is_member(c)) {
        } else {
            rest.child("", c);
        }
    }
    if let Some(e) = el.child(Ns::Hp, "effects") {
        for c in e.children() {
            rest.child("effects", c);
        }
    }
}

/// A drawing element and its members as a shape.
fn shape(el: El<'_>, kind: CtrlId, top: bool, rest: &mut Rest, ctx: &mut Ctx) -> Shape {
    element_rest(el, kind, top, rest);
    let mut component = component(el, kind, top, rest);
    let mut parts = Vec::new();
    match kind {
        GROUP_ID => {
            let mut members = Vec::new();
            for c in el.children() {
                if let Some(k) = shape_kind(c) {
                    members.push(k);
                    let mut crest = Rest::new();
                    let mut child = shape(c, k, false, &mut crest, ctx);
                    ctx.note_rest(c, &crest);
                    child.component.tail = crest.into_tail();
                    parts.push(ShapePart::Child(Box::new(child)));
                }
            }
            component.members = Some(members);
            component.group_instance = Some(el.u32("instid").unwrap_or(0));
        }
        PICTURE_ID => parts.push(ShapePart::Picture(picture(el, rest, ctx))),
        OLE_ID => parts.push(ShapePart::Ole(ole(el, rest, ctx))),
        CONNECTOR_ID => {
            for n in ["startPt", "endPt"] {
                if let Some(p) = el.child(Ns::Hp, n) {
                    rest.attrs(n, p, &["x", "y"]);
                }
            }
            parts.push(ShapePart::Line(LineGeom {
                start: pt(el.child(Ns::Hp, "startPt")),
                end: pt(el.child(Ns::Hp, "endPt")),
                attr: Some(code(el, "type", CONNECT_TYPE, ctx).unwrap_or(0) as u16),
                tail: SharedBytes::empty(),
            }));
        }
        _ => {
            component.style = Some(style(el, rest, ctx));
            if let Some(t) = el.child(Ns::Hp, "drawText") {
                parts.push(ShapePart::Text(text_box(t, rest, ctx)));
            }
            if let Some(g) = geometry(el, kind, ctx) {
                parts.push(g);
            }
        }
    }
    Shape { component, parts }
}

fn geometry(el: El<'_>, kind: CtrlId, ctx: &mut Ctx) -> Option<ShapePart> {
    let hc = |n: &str| el.child(Ns::Hc, n).or_else(|| el.child(Ns::Hp, n));
    Some(match kind {
        LINE_ID => ShapePart::Line(LineGeom {
            start: pt(hc("startPt")),
            end: pt(hc("endPt")),
            attr: Some(el.flag("isReverseHV").unwrap_or(false) as u16),
            tail: SharedBytes::empty(),
        }),
        RECT_ID => ShapePart::Rect(RectGeom {
            round: el.int("ratio").unwrap_or(0) as u8,
            corners: [pt(hc("pt0")), pt(hc("pt1")), pt(hc("pt2")), pt(hc("pt3"))],
            tail: SharedBytes::empty(),
        }),
        ELLIPSE_ID => ShapePart::Ellipse(EllipseGeom {
            attr: el.flag("intervalDirty").unwrap_or(false) as u32
                | (el.flag("hasArcPr").unwrap_or(false) as u32) << 1
                | code(el, "arcType", ARC_TYPE, ctx).unwrap_or(0) << 2,
            center: pt(hc("center")),
            axis1: pt(hc("ax1")),
            axis2: pt(hc("ax2")),
            start: pt(hc("start1")),
            end: pt(hc("end1")),
            start2: pt(hc("start2")),
            end2: pt(hc("end2")),
            tail: SharedBytes::empty(),
        }),
        ARC_ID => ShapePart::Arc(ArcGeom {
            attr: code(el, "type", ARC_TYPE, ctx).unwrap_or(0),
            center: pt(hc("center")),
            axis1: pt(hc("ax1")),
            axis2: pt(hc("ax2")),
            tail: SharedBytes::empty(),
        }),
        POLYGON_ID => ShapePart::Polygon(PolyGeom {
            points: el
                .children()
                .filter(|c| c.is(Ns::Hc, "pt"))
                .map(|c| pt(Some(c)))
                .collect(),
            tail: SharedBytes::empty(),
        }),
        CURVE_ID => {
            let mut points = Vec::new();
            let mut segments = Vec::new();
            for s in el.children().filter(|c| c.is(Ns::Hp, "seg")) {
                let p1 = (wrap(s.int("x1")), wrap(s.int("y1")));
                let p2 = (wrap(s.int("x2")), wrap(s.int("y2")));
                if points.is_empty() {
                    points.push(p1);
                }
                points.push(p2);
                let curve = s
                    .attr("type")
                    .is_some_and(|t| t.eq_ignore_ascii_case("CURVE"));
                segments.push(curve as u8);
            }
            ShapePart::Curve(CurveGeom {
                points,
                segments,
                tail: SharedBytes::empty(),
            })
        }
        _ => return None,
    })
}

pub const CONNECT_TYPE: crate::enums::Table = &[
    ("STRAIGHT_NOARROW", 0),
    ("STRAIGHT_ONEWAY", 1),
    ("STRAIGHT_BOTH", 2),
    ("STROKE_NOARROW", 3),
    ("STROKE_ONEWAY", 4),
    ("STROKE_BOTH", 5),
    ("ARC_NOARROW", 6),
    ("ARC_ONEWAY", 7),
    ("ARC_BOTH", 8),
];

pub const ARC_TYPE: crate::enums::Table = &[("NORMAL", 0), ("PIE", 1), ("CHORD", 2)];

fn image_ref(el: Option<El<'_>>, ctx: &mut Ctx) -> (ImageRef, u8) {
    let Some(i) = el else {
        return (ImageRef::default(), 0);
    };
    let r = ImageRef {
        contrast: i.int("contrast").unwrap_or(0) as i8,
        brightness: i.int("bright").unwrap_or(0) as i8,
        effect: code(i, "effect", enums::IMG_EFFECT, ctx).unwrap_or(0) as u8,
        bin_id: i
            .attr("binaryItemIDRef")
            .and_then(|r| ctx.bin_index(&r))
            .unwrap_or(0) as u16,
    };
    (r, i.int("alpha").unwrap_or(0).clamp(0, 255) as u8)
}

fn picture(el: El<'_>, rest: &mut Rest, ctx: &mut Ctx) -> Picture {
    let mut p = Picture {
        instance_id: el.u32("instid").unwrap_or(0),
        ..Default::default()
    };
    if let Some(l) = el.child(Ns::Hp, "lineShape") {
        p.border = line_style(l, false, rest, "lineShape", ctx);
        p.border_alpha = l.int("alpha").unwrap_or(0).clamp(0, 255) as u8;
    }
    if let Some(r) = el.child(Ns::Hp, "imgRect") {
        let c = |n: &str| pt(r.child(Ns::Hc, n));
        p.corners = [c("pt0"), c("pt1"), c("pt2"), c("pt3")];
    }
    if let Some(c) = el.child(Ns::Hp, "imgClip") {
        let v = |k| wrap(c.int(k));
        p.crop = [v("left"), v("top"), v("right"), v("bottom")];
    }
    if let Some(m) = el.child(Ns::Hp, "inMargin") {
        let v = |k| m.int(k).unwrap_or(0) as i16;
        p.margins = [v("left"), v("right"), v("top"), v("bottom")];
    }
    let (image, alpha) = image_ref(el.child(Ns::Hc, "img"), ctx);
    p.image = image;
    // The additional properties after the effects: original size and alpha.
    let mut tail = 0u32.to_le_bytes().to_vec();
    if let Some(d) = el.child(Ns::Hp, "imgDim") {
        tail.extend_from_slice(&wrap(d.int("dimwidth")).to_le_bytes());
        tail.extend_from_slice(&wrap(d.int("dimheight")).to_le_bytes());
    } else {
        tail.extend_from_slice(&[0; 8]);
    }
    tail.push(alpha);
    p.tail = SharedBytes::from(tail);
    p
}

pub const OLE_TYPE: crate::enums::Table = &[
    ("UNKNOWN", 0),
    ("EMBEDDED", 1),
    ("LINK", 2),
    ("STATIC", 3),
    ("EQUATION", 4),
];

pub const DRAW_ASPECT: crate::enums::Table = &[
    ("CONTENT", 1),
    ("THUMB_NAIL", 2),
    ("ICON", 4),
    ("DOC_PRINT", 8),
];

fn ole(el: El<'_>, rest: &mut Rest, ctx: &mut Ctx) -> Ole {
    let attr = code(el, "drawAspect", DRAW_ASPECT, ctx).unwrap_or(1) & 0xFF
        | (el.flag("hasMoniker").unwrap_or(false) as u32) << 8
        | (el.int("eqBaseLine").unwrap_or(0) as u32 & 0x7F) << 9
        | (code(el, "objectType", OLE_TYPE, ctx).unwrap_or(0) & 0x3F) << 16;
    Ole {
        attr,
        extent: pt(el.child(Ns::Hc, "extent")),
        bin_id: el
            .attr("binaryItemIDRef")
            .and_then(|r| ctx.bin_index(&r))
            .unwrap_or(0) as u16,
        border: el
            .child(Ns::Hp, "lineShape")
            .map(|l| line_style(l, false, rest, "lineShape", ctx)),
        tail: SharedBytes::empty(),
    }
}

/// A top-level drawing element (`gso `) as an object control.
pub fn drawing(el: El<'_>, kind: CtrlId, ctx: &mut Ctx) -> ObjectControl {
    let mut rest = Rest::new();
    let mut header = header(el, &mut rest, ctx);
    let mut parts = Vec::new();
    if let Some(c) = el.child(Ns::Hp, "caption") {
        parts.push(ObjectPart::Caption(caption(c, ctx)));
    }
    let shape = shape(el, kind, true, &mut rest, ctx);
    parts.push(ObjectPart::Shape(Box::new(shape)));
    ctx.note_rest(el, &rest);
    header.tail = rest.into_tail();
    ObjectControl {
        id: CtrlId::SHAPE,
        header,
        parts,
    }
}
