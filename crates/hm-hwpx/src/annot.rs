//! Fields, bookmarks, form objects, overlaid characters and ruby text,
//! between OWPML and the model.
//!
//! Form properties live in the model as Hancom's parameter sets (the names
//! HWP 5 stores); OWPML spells them as attributes of the form element.

use hm_model::{
    Bookmark, Compose, ComposeFrame, CtrlId, Dutmal, Field, FieldKind, FormObject, HwpString,
    ParamValue, Params, SharedBytes,
};

use crate::enums::{self, Table};
use crate::furniture;
use crate::head::attr_color;
use crate::object;
use crate::read::Ctx;
use crate::rest::Rest;
use crate::write::{X, color, name};
use crate::xml::{El, Ns};

fn param<'a>(el: El<'a>, kind: &str, n: &str) -> Option<El<'a>> {
    el.child(Ns::Hp, "parameters")?
        .children()
        .find(|p| p.is(Ns::Hp, kind) && p.attr("name").as_deref() == Some(n))
}

pub fn field(el: El<'_>, id: CtrlId, ctx: &mut Ctx) -> Field {
    let mut rest = Rest::new();
    rest.attrs("", el, &["type", "name", "editable", "dirty", "id"]);
    let props = param(el, "integerParam", "Prop");
    let command = param(el, "stringParam", "Command");
    for c in el.children() {
        if c.is(Ns::Hp, "parameters") {
            rest.attrs("parameters", c, &["cnt"]);
            for p in c.children() {
                let known = p.is(Ns::Hp, "integerParam")
                    && p.attr("name").as_deref() == Some("Prop")
                    || p.is(Ns::Hp, "stringParam") && p.attr("name").as_deref() == Some("Command");
                if !known {
                    rest.child("parameters", p);
                }
            }
        } else if !c.is(Ns::Hp, "subList") {
            rest.child("", c);
        }
    }
    let memo = el
        .child(Ns::Hp, "subList")
        .map(|l| furniture::para_list(Some(l), false, ctx));
    ctx.note_rest(el, &rest);
    Field {
        id,
        attr: el.flag("editable").unwrap_or(false) as u32
            | (el.flag("dirty").unwrap_or(false) as u32) << 15,
        props: props
            .and_then(|p| p.text().trim().parse::<u32>().ok())
            .unwrap_or(0) as u8,
        command: HwpString::from_text(&command.map(|c| c.text()).unwrap_or_default()),
        instance: el.u32("id").unwrap_or(0),
        tail: rest.into_tail(),
        name: el.attr("name").map(|n| HwpString::from_text(&n)),
        data: Vec::new(),
        memo,
    }
}

pub fn write_field(f: &Field, cx: &mut crate::write::Cx<'_>) -> X {
    let type_name = if f.kind() == FieldKind::Memo {
        "MEMO".to_string()
    } else {
        enums::name(enums::FIELD_TYPE, f.id.0).map_or_else(|| "UNKNOWN".to_string(), str::to_string)
    };
    let mut params = X::new("hp:parameters")
        .kid(
            X::new("hp:integerParam")
                .a("name", "Prop")
                .text(f.props.to_string()),
        )
        .kid(
            X::new("hp:stringParam")
                .a("name", "Command")
                .a("xml:space", "preserve")
                .text(f.command.to_string_lossy()),
        );
    let mut x = X::new("hp:fieldBegin")
        .a("id", f.instance)
        .a("type", type_name);
    if let Some(n) = &f.name {
        x = x.a("name", n.to_string_lossy());
    }
    x = x.b("editable", f.editable()).b("dirty", f.dirty());
    // The parameter count covers the leftovers put back below.
    let extra = crate::rest::pieces(&f.tail)
        .iter()
        .filter(|p| !p.kind_attr && p.path == "parameters")
        .count();
    params.attrs.push(("cnt", (2 + extra).to_string()));
    x = x.kid(params);
    if let Some(m) = &f.memo {
        x = x.kid(crate::write::sub_list(m, cx));
    }
    x.with_rest(&f.tail)
}

pub fn bookmark(el: El<'_>, ctx: &mut Ctx) -> Bookmark {
    let mut rest = Rest::new();
    rest.attrs("", el, &["name"]);
    for c in el.children() {
        rest.child("", c);
    }
    ctx.note_rest(el, &rest);
    Bookmark {
        name: el.attr("name").map(|n| HwpString::from_text(&n)),
        data: Vec::new(),
        tail: rest.into_tail(),
    }
}

pub fn write_bookmark(b: &Bookmark) -> X {
    let mut x = X::new("hp:bookmark");
    if let Some(n) = &b.name {
        x = x.a("name", n.to_string_lossy());
    }
    x.with_rest(&b.tail)
}

pub fn compose(el: El<'_>, ctx: &mut Ctx) -> Compose {
    let mut rest = Rest::new();
    rest.attrs(
        "",
        el,
        &[
            "circleType",
            "charSz",
            "composeType",
            "charPrCnt",
            "composeText",
        ],
    );
    let mut char_shapes = Vec::new();
    for c in el.children() {
        if c.is(Ns::Hp, "charPr") {
            rest.attrs("charPr", c, &["prIDRef"]);
            char_shapes.push(c.u32("prIDRef").unwrap_or(u32::MAX));
        } else {
            rest.child("", c);
        }
    }
    ctx.note_rest(el, &rest);
    let code = |a: &str, t: Table| el.attr(a).and_then(|v| enums::code(t, &v)).unwrap_or(0);
    Compose {
        text: HwpString::from_text(&el.attr("composeText").unwrap_or_default()),
        frame: Some(ComposeFrame {
            shape: code("circleType", crate::fields::COMPOSE_CIRCLE) as u8,
            inner_size: el.int("charSz").unwrap_or(0) as i8,
            kind: code("composeType", crate::fields::COMPOSE_TYPE) as u8,
            char_shapes,
        }),
        tail: rest.into_tail(),
    }
}

pub fn write_compose(c: &Compose) -> X {
    let f = c.frame.clone().unwrap_or(ComposeFrame {
        kind: 1,
        ..Default::default()
    });
    X::new("hp:compose")
        .a(
            "circleType",
            name(crate::fields::COMPOSE_CIRCLE, f.shape as u32),
        )
        .a("charSz", f.inner_size)
        .a(
            "composeType",
            name(crate::fields::COMPOSE_TYPE, f.kind as u32),
        )
        .a("charPrCnt", f.char_shapes.len())
        .a("composeText", c.text.to_string_lossy())
        .kids(
            f.char_shapes
                .iter()
                .map(|&id| X::new("hp:charPr").a("prIDRef", id)),
        )
        .with_rest(&c.tail)
}

pub fn dutmal(el: El<'_>, ctx: &mut Ctx) -> Dutmal {
    let mut rest = Rest::new();
    rest.attrs(
        "",
        el,
        &["posType", "szRatio", "option", "styleIDRef", "align"],
    );
    let (mut main, mut sub) = (String::new(), String::new());
    for c in el.children() {
        if c.is(Ns::Hp, "mainText") {
            rest.attrs("mainText", c, &[]);
            main = c.text();
        } else if c.is(Ns::Hp, "subText") {
            rest.attrs("subText", c, &[]);
            sub = c.text();
        } else {
            rest.child("", c);
        }
    }
    ctx.note_rest(el, &rest);
    Dutmal {
        main: HwpString::from_text(&main),
        sub: HwpString::from_text(&sub),
        position: el
            .attr("posType")
            .and_then(|p| enums::code(crate::fields::DUTMAL_POS, &p))
            .unwrap_or(0),
        size_ratio: el.u32("szRatio").unwrap_or(0),
        option: el.u32("option").unwrap_or(0),
        style: el.u32("styleIDRef").unwrap_or(0),
        align: el
            .attr("align")
            .and_then(|a| enums::code(enums::H_ALIGN, &a))
            .unwrap_or(3),
        tail: rest.into_tail(),
    }
}

pub fn write_dutmal(d: &Dutmal) -> X {
    X::new("hp:dutmal")
        .a("posType", name(crate::fields::DUTMAL_POS, d.position))
        .a("szRatio", d.size_ratio)
        .a("option", d.option)
        .a("styleIDRef", d.style)
        .a("align", name(enums::H_ALIGN, d.align))
        .kid(X::new("hp:mainText").text(d.main.to_string_lossy()))
        .kid(X::new("hp:subText").text(d.sub.to_string_lossy()))
        .with_rest(&d.tail)
}

/// How one form attribute maps to a parameter item.
#[derive(Clone, Copy)]
enum Ty {
    Str,
    Int,
    Bool,
    Color,
    Code(Table),
}

/// `(attribute, item, type)` of each parameter set.
const COMMON: &[(&str, &str, Ty)] = &[
    ("name", "Name", Ty::Str),
    ("foreColor", "ForeColor", Ty::Color),
    ("backColor", "BackColor", Ty::Color),
    ("groupName", "GroupName", Ty::Str),
    ("tabStop", "TabStop", Ty::Bool),
    ("tabOrder", "TabOrder", Ty::Int),
    ("enabled", "Enabled", Ty::Bool),
    ("borderTypeIDRef", "BorderType", Ty::Int),
    ("drawFrame", "DrawFrame", Ty::Bool),
    ("command", "Command", Ty::Str),
    ("editable", "Editable", Ty::Bool),
    ("printable", "Printable", Ty::Bool),
];

const CHAR_SHAPE: &[(&str, &str, Ty)] = &[
    ("charPrIDRef", "CharShapeID", Ty::Int),
    ("followContext", "FollowContext", Ty::Bool),
    ("autoSz", "AutoSize", Ty::Bool),
    ("wordWrap", "WordWrap", Ty::Bool),
];

pub const BUTTON_VALUE: Table = &[("UNCHECKED", 0), ("CHECKED", 1), ("INDETERMINATE", 2)];
pub const BACK_STYLE: Table = &[("TRANSPARENT", 0), ("OPAQUE", 1)];
pub const SCROLL_BARS: Table = &[("NONE", 0), ("HORIZONTAL", 1), ("VERTICAL", 2), ("BOTH", 3)];
pub const TAB_KEY: Table = &[("NEXT_OBJECT", 0), ("INSERT_TAB", 1)];
pub const ALIGN_TEXT: Table = &[("LEFT", 0), ("CENTER", 1), ("RIGHT", 2)];

const PUSH: &[(&str, &str, Ty)] = &[("caption", "Caption", Ty::Str)];
const CHECK: &[(&str, &str, Ty)] = &[
    ("caption", "Caption", Ty::Str),
    ("value", "Value", Ty::Code(BUTTON_VALUE)),
    ("triState", "TriState", Ty::Bool),
    ("backStyle", "BackStyle", Ty::Code(BACK_STYLE)),
];
const RADIO: &[(&str, &str, Ty)] = &[
    ("caption", "Caption", Ty::Str),
    ("radioGroupName", "RadioGroupName", Ty::Str),
    ("value", "Value", Ty::Code(BUTTON_VALUE)),
    ("triState", "TriState", Ty::Bool),
    ("backStyle", "BackStyle", Ty::Code(BACK_STYLE)),
];
const COMBO: &[(&str, &str, Ty)] = &[
    ("listBoxRows", "ListBoxRows", Ty::Int),
    ("selectedValue", "Text", Ty::Str),
    ("listBoxWidth", "ListBoxWidth", Ty::Int),
    ("editEnable", "EditEnable", Ty::Bool),
];
const EDIT: &[(&str, &str, Ty)] = &[
    ("@text", "Text", Ty::Str),
    ("multiLine", "MultiLine", Ty::Bool),
    ("passwordChar", "PasswordChar", Ty::Str),
    ("maxLength", "MaxLength", Ty::Int),
    ("scrollBars", "ScrollBars", Ty::Code(SCROLL_BARS)),
    ("tabKeyBehavior", "TabKeyBehavior", Ty::Code(TAB_KEY)),
    ("numOnly", "Number", Ty::Bool),
    ("readOnly", "ReadOnly", Ty::Bool),
    ("alignText", "AlignText", Ty::Code(ALIGN_TEXT)),
];

/// The attribute-to-item map of one parameter set.
type ItemMap = &'static [(&'static str, &'static str, Ty)];

/// Form element, record kind, kind set and its items.
fn kind_of(local: &[u8]) -> Option<(CtrlId, &'static str, ItemMap)> {
    Some(match local {
        b"btn" => (FormObject::PUSH_BUTTON, "ButtonSet", PUSH),
        b"checkBtn" => (FormObject::CHECK_BOX, "ButtonSet", CHECK),
        b"radioBtn" => (FormObject::RADIO_BUTTON, "ButtonSet", RADIO),
        b"comboBox" => (FormObject::COMBO_BOX, "ComboBoxSet", COMBO),
        b"listBox" => (FormObject::LIST_BOX, "ListBoxSet", COMBO),
        b"edit" => (FormObject::EDIT, "EditSet", EDIT),
        _ => return None,
    })
}

fn element_of(kind: CtrlId) -> Option<(&'static str, &'static str, ItemMap)> {
    Some(match kind {
        FormObject::PUSH_BUTTON => ("hp:btn", "ButtonSet", PUSH),
        FormObject::CHECK_BOX => ("hp:checkBtn", "ButtonSet", CHECK),
        FormObject::RADIO_BUTTON => ("hp:radioBtn", "ButtonSet", RADIO),
        FormObject::COMBO_BOX => ("hp:comboBox", "ComboBoxSet", COMBO),
        FormObject::LIST_BOX => ("hp:listBox", "ListBoxSet", COMBO),
        FormObject::EDIT => ("hp:edit", "EditSet", EDIT),
        _ => return None,
    })
}

fn read_set(el: El<'_>, map: &[(&str, &str, Ty)], text: Option<String>) -> Params {
    let mut p = Params::default();
    for &(attr, item, ty) in map {
        let v = if attr == "@text" {
            ParamValue::Str(text.clone().unwrap_or_default())
        } else {
            let raw = el.attr(attr);
            match ty {
                Ty::Str => ParamValue::Str(raw.map(|v| v.into_owned()).unwrap_or_default()),
                Ty::Int => ParamValue::Int(el.int(attr).unwrap_or(0)),
                Ty::Bool => ParamValue::Bool(el.flag(attr).unwrap_or(false)),
                Ty::Color => ParamValue::Int(attr_color(el, attr).unwrap_or(0) as i64),
                Ty::Code(t) => {
                    ParamValue::Int(raw.and_then(|v| enums::code(t, &v)).unwrap_or(0) as i64)
                }
            }
        };
        p.items.push((item.to_string(), v));
    }
    p
}

fn write_set(x: X, set: Option<&Params>, map: &[(&'static str, &'static str, Ty)]) -> X {
    let mut x = x;
    for &(attr, item, ty) in map {
        if attr.starts_with('@') {
            continue;
        }
        let v = set.and_then(|s| s.find(item));
        let s = match (ty, v) {
            (Ty::Str, Some(ParamValue::Str(s))) => s.clone(),
            (Ty::Bool, v) => (v.and_then(|v| v.as_bool()).unwrap_or(false) as u8).to_string(),
            (Ty::Color, v) => color(v.and_then(|v| v.as_int()).unwrap_or(0) as u32),
            (Ty::Code(t), v) => name(t, v.and_then(|v| v.as_int()).unwrap_or(0) as u32),
            (_, Some(v)) => v.as_int().map(|n| n.to_string()).unwrap_or_default(),
            (_, None) => String::new(),
        };
        x.attrs.push((attr, s));
    }
    x
}

const FORM_ATTRS: &[&str] = &[
    "name",
    "foreColor",
    "backColor",
    "groupName",
    "tabStop",
    "tabOrder",
    "enabled",
    "borderTypeIDRef",
    "drawFrame",
    "command",
    "editable",
    "printable",
];

pub fn form(el: El<'_>, ctx: &mut Ctx) -> Option<FormObject> {
    let (kind, set_name, map) = kind_of(el.local())?;
    let mut rest = Rest::new();
    let mut known: Vec<&str> = object::COMMON_ATTRS.to_vec();
    known.extend_from_slice(FORM_ATTRS);
    known.extend(map.iter().map(|(a, _, _)| *a));
    rest.attrs("", el, &known);
    let mut header = object::header(el, &mut rest, ctx);
    // Form elements carry no wrap attribute; Hancom stores top and bottom.
    if el.attr("textWrap").is_none() {
        header.common.attr |= 1 << 21;
    }
    let mut items = Vec::new();
    let mut text = None;
    let char_pr = el.child(Ns::Hp, "formCharPr");
    for c in el.children() {
        match c.local() {
            b"formCharPr" => rest.attrs(
                "formCharPr",
                c,
                &["charPrIDRef", "followContext", "autoSz", "wordWrap"],
            ),
            b"listItem" => items.push((
                HwpString::from_text(&c.attr("displayText").unwrap_or_default()),
                HwpString::from_text(&c.attr("value").unwrap_or_default()),
            )),
            b"text" => text = Some(c.text()),
            _ if object::is_header_child(c) => {}
            _ => rest.child("", c),
        }
    }
    let mut kind_set = read_set(el, map, text);
    // HWP 5 keeps a combo box's text where OWPML lists it as an item.
    if kind == FormObject::COMBO_BOX
        && kind_set.find("Text").and_then(|v| v.as_str()) == Some("")
        && let Some((_, v)) = items.first()
    {
        kind_set.set("Text", ParamValue::Str(v.to_string_lossy()));
    }
    let mut props = Params::default();
    props.items.push((
        "CommonSet".into(),
        ParamValue::Set(read_set(el, COMMON, None)),
    ));
    props.items.push((
        "CharShapeSet".into(),
        ParamValue::Set(
            char_pr
                .map(|c| read_set(c, CHAR_SHAPE, None))
                .unwrap_or_default(),
        ),
    ));
    props
        .items
        .push((set_name.into(), ParamValue::Set(kind_set)));
    ctx.note_rest(el, &rest);
    header.tail = rest.into_tail();
    Some(FormObject {
        header,
        kind,
        props,
        items,
        tail: SharedBytes::empty(),
    })
}

pub fn write_form(f: &FormObject) -> Option<X> {
    let (tag, set_name, map) = element_of(f.kind)?;
    let mut kind_set = f.props.find_set(set_name).cloned().unwrap_or_default();
    if f.kind == FormObject::COMBO_BOX
        && let Some((_, v)) = f.items.first()
        && kind_set.find("Text").and_then(|t| t.as_str()) == Some(&v.to_string_lossy())
    {
        kind_set.set("Text", ParamValue::Str(String::new()));
    }
    let mut x = write_set(X::new(tag), Some(&kind_set), map);
    x = write_set(x, f.props.find_set("CommonSet"), COMMON);
    x = x.kid(write_set(
        X::new("hp:formCharPr"),
        f.props.find_set("CharShapeSet"),
        CHAR_SHAPE,
    ));
    for (d, v) in &f.items {
        x = x.kid(
            X::new("hp:listItem")
                .a("displayText", d.to_string_lossy())
                .a("value", v.to_string_lossy()),
        );
    }
    if f.kind == FormObject::EDIT {
        let t = kind_set
            .find("Text")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        x = x.kid(if t.is_empty() {
            X::new("hp:text")
        } else {
            X::new("hp:text").text(t)
        });
    }
    // Placement and leftovers come from the shared object writer.
    Some(crate::write::object_placement(x, &f.header).with_rest(&f.header.tail))
}
