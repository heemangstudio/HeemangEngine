//! Object elements (`hp:tbl`, `hp:pic`, shapes, `hp:equation`): the common
//! object attributes and the typed object controls.

use hm_model::{
    CtrlId, Equation, HwpString, ObjectCommon, ObjectControl, ObjectHeader, ObjectPart,
};

use crate::enums;
use crate::head::{attr_color, code};
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
    for (slot, c) in el.children().enumerate() {
        if c.is(Ns::Hp, "script") {
            script = c.text();
        } else if !is_header_child(c) {
            erest.child("", slot as u16, c);
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
