//! `Contents/header.xml` into [`DocInfo`].
//!
//! Every typed entry takes the values the HWP 5 record of the same entry
//! would hold (packed distances, attribute bits, COLORREF colors). Markup
//! no field covers rides in the entry's `tail` (see [`crate::rest`]).

use hm_model::{
    BorderFill, BorderLine, Bullet, CharShape, CompatibleDocument, DocInfo, FaceName, Fill,
    Gradient, HwpString, ImageFill, InfoItem, InfoSlot, LayoutCompatibility, Numbering,
    NumberingLevel, ParaHead, ParaShape, Script, SharedBytes, SolidFill, Style, TabDef, TabStop,
};

use crate::enums::{self, Table};
use crate::read::Ctx;
use crate::rest::Rest;
use crate::xml::{El, Ns, Xml};

/// Parses an OWPML color: `#RRGGBB`, `#AARRGGBB` or `none`, as COLORREF
/// (`0xAABBGGRR`; `none` is all ones).
pub fn color(s: &str) -> u32 {
    let s = s.trim();
    let Some(hex) = s.strip_prefix('#') else {
        return 0xFFFF_FFFF;
    };
    let Ok(v) = u32::from_str_radix(hex, 16) else {
        return 0xFFFF_FFFF;
    };
    let (a, rgb) = if hex.len() > 6 {
        (v >> 24, v & 0xFF_FFFF)
    } else {
        (0, v)
    };
    let (r, g, b) = ((rgb >> 16) & 0xFF, (rgb >> 8) & 0xFF, rgb & 0xFF);
    (a << 24) | (b << 16) | (g << 8) | r
}

pub fn attr_color(el: El<'_>, name: &str) -> Option<u32> {
    el.attr(name).map(|v| color(&v))
}

pub fn code(el: El<'_>, name: &str, t: Table, ctx: &mut Ctx) -> Option<u32> {
    let v = el.attr(name)?;
    let c = enums::code(t, &v);
    if c.is_none() {
        ctx.unknown(format!("{}@{}={}", tag(el), name, v));
    }
    c
}

pub fn tag(el: El<'_>) -> String {
    format!(
        "{}:{}",
        crate::rest::prefix(el.ns()),
        String::from_utf8_lossy(el.local())
    )
}

fn hstr(s: &str) -> HwpString {
    HwpString::from_text(s)
}

/// The values of a `hp:switch`: the default branch carries the HWP 5 form.
fn switch_branch(sw: El<'_>) -> Option<El<'_>> {
    sw.child(Ns::Hp, "default")
        .or_else(|| sw.child(Ns::Hp, "case"))
}

/// Packs a distance from a value and unit (`HWPUNIT` or `CHAR`).
fn pack(value: i32, unit: Option<&str>) -> i32 {
    match unit {
        Some(u) if u.eq_ignore_ascii_case("CHAR") => value.wrapping_shl(1) | 1,
        _ => value.wrapping_shl(1),
    }
}

pub fn parse(x: &Xml, info: &mut DocInfo, ctx: &mut Ctx) {
    let head = x.root();
    if !head.is(Ns::Hh, "head") {
        ctx.diag("header.xml has no hh:head root".into());
        return;
    }
    info.order.push(InfoSlot {
        item: InfoItem::Properties,
        level: 0,
    });
    info.order.push(InfoSlot {
        item: InfoItem::IdMappings,
        level: 0,
    });
    if let Some(n) = head.int("secCnt") {
        info.properties.section_count = n as u16;
    }
    for child in head.children() {
        match (child.ns(), child.local()) {
            (Ns::Hh, b"beginNum") => begin_num(child, info),
            (Ns::Hh, b"refList") => ref_list(child, info, ctx),
            (Ns::Hh, b"compatibleDocument") => compatible(child, info, ctx),
            _ => {}
        }
    }
    let counts = &mut info.id_mappings.counts;
    counts.clear();
    counts.push(info.bin_data.len() as i32);
    for list in &info.faces {
        counts.push(list.len() as i32);
    }
    for n in [
        info.border_fills.len(),
        info.char_shapes.len(),
        info.tab_defs.len(),
        info.numberings.len(),
        info.bullets.len(),
        info.para_shapes.len(),
        info.styles.len(),
    ] {
        counts.push(n as i32);
    }
    counts.push(ctx.memo_shapes as i32);
    counts.push(0);
    counts.push(0);
    info.id_mappings.src = Some(SharedBytes::empty());
}

fn begin_num(el: El<'_>, info: &mut DocInfo) {
    let p = &mut info.properties;
    let n = |k| el.int(k).unwrap_or(1) as u16;
    p.page_start = n("page");
    p.footnote_start = n("footnote");
    p.endnote_start = n("endnote");
    p.figure_start = n("pic");
    p.table_start = n("tbl");
    p.equation_start = n("equation");
    p.src = Some(el.source());
}

fn ref_list(list: El<'_>, info: &mut DocInfo, ctx: &mut Ctx) {
    for group in list.children() {
        match (group.ns(), group.local()) {
            (Ns::Hh, b"fontfaces") => {
                for ff in group.children().filter(|c| c.is(Ns::Hh, "fontface")) {
                    let lang = ff.attr("lang").unwrap_or_default();
                    let slot = enums::LANG
                        .iter()
                        .position(|l| l.eq_ignore_ascii_case(&lang))
                        .unwrap_or(6);
                    for f in ff.children().filter(|c| c.is(Ns::Hh, "font")) {
                        let face = font(f, ctx);
                        let list = &mut info.faces[slot];
                        list.push(face);
                        info.order.push(InfoSlot {
                            item: InfoItem::Face(Script::from_index(slot), list.len() as u32 - 1),
                            level: 1,
                        });
                    }
                }
            }
            (Ns::Hh, b"borderFills") => {
                for b in group.children().filter(|c| c.is(Ns::Hh, "borderFill")) {
                    info.border_fills.push(border_fill(b, ctx));
                    info.order.push(InfoSlot {
                        item: InfoItem::BorderFill(info.border_fills.len() as u32 - 1),
                        level: 1,
                    });
                }
            }
            (Ns::Hh, b"charProperties") => {
                for c in group.children().filter(|c| c.is(Ns::Hh, "charPr")) {
                    info.char_shapes.push(char_shape(c, ctx));
                    info.order.push(InfoSlot {
                        item: InfoItem::CharShape(info.char_shapes.len() as u32 - 1),
                        level: 1,
                    });
                }
            }
            (Ns::Hh, b"tabProperties") => {
                for c in group.children().filter(|c| c.is(Ns::Hh, "tabPr")) {
                    info.tab_defs.push(tab_def(c, ctx));
                    info.order.push(InfoSlot {
                        item: InfoItem::TabDef(info.tab_defs.len() as u32 - 1),
                        level: 1,
                    });
                }
            }
            (Ns::Hh, b"numberings") => {
                for c in group.children().filter(|c| c.is(Ns::Hh, "numbering")) {
                    info.numberings.push(numbering(c, ctx));
                    info.order.push(InfoSlot {
                        item: InfoItem::Numbering(info.numberings.len() as u32 - 1),
                        level: 1,
                    });
                }
            }
            (Ns::Hh, b"bullets") => {
                for c in group.children().filter(|c| c.is(Ns::Hh, "bullet")) {
                    info.bullets.push(bullet(c, ctx));
                    info.order.push(InfoSlot {
                        item: InfoItem::Bullet(info.bullets.len() as u32 - 1),
                        level: 1,
                    });
                }
            }
            (Ns::Hh, b"paraProperties") => {
                for c in group.children().filter(|c| c.is(Ns::Hh, "paraPr")) {
                    info.para_shapes.push(para_shape(c, ctx));
                    info.order.push(InfoSlot {
                        item: InfoItem::ParaShape(info.para_shapes.len() as u32 - 1),
                        level: 1,
                    });
                }
            }
            (Ns::Hh, b"styles") => {
                for c in group.children().filter(|c| c.is(Ns::Hh, "style")) {
                    info.styles.push(style(c, ctx));
                    info.order.push(InfoSlot {
                        item: InfoItem::Style(info.styles.len() as u32 - 1),
                        level: 1,
                    });
                }
            }
            (Ns::Hh, b"memoProperties") => {
                ctx.memo_shapes = group.children().count() as u32;
            }
            _ => {}
        }
    }
}

fn font(f: El<'_>, ctx: &mut Ctx) -> FaceName {
    let mut rest = Rest::new();
    rest.attrs(
        "",
        f,
        &["id", "face", "type", "isEmbedded", "binaryItemIDRef"],
    );
    let kind = code(f, "type", enums::FONT_TYPE, ctx).unwrap_or(1) as u8;
    let mut face = FaceName {
        attr: kind & 0x1F,
        name: hstr(&f.attr("face").unwrap_or_default()),
        ..Default::default()
    };
    for (slot, c) in f.children().enumerate() {
        if c.is(Ns::Hh, "substFont") {
            rest.attrs(
                "substFont",
                c,
                &["face", "type", "isEmbedded", "binaryItemIDRef"],
            );
            let k = code(c, "type", enums::FONT_TYPE, ctx).unwrap_or(0) as u8;
            face.substitute = Some((k, hstr(&c.attr("face").unwrap_or_default())));
            face.attr |= 0x80;
        } else if c.is(Ns::Hh, "typeInfo") {
            rest.attrs(
                "typeInfo",
                c,
                &[
                    "familyType",
                    "serifStyle",
                    "weight",
                    "proportion",
                    "contrast",
                    "strokeVariation",
                    "armStyle",
                    "letterform",
                    "midline",
                    "xHeight",
                ],
            );
            let fam = code(c, "familyType", enums::FAMILY_TYPE, ctx).unwrap_or(0) as u8;
            let b = |k| c.int(k).unwrap_or(0) as u8;
            face.panose = Some([
                fam,
                b("serifStyle"),
                b("weight"),
                b("proportion"),
                b("contrast"),
                b("strokeVariation"),
                b("armStyle"),
                b("letterform"),
                b("midline"),
                b("xHeight"),
            ]);
            face.attr |= 0x40;
        } else {
            rest.child("", slot as u16, c);
        }
    }
    ctx.note_rest(f, &rest);
    face.tail = rest.into_tail();
    face.src = Some(f.source());
    face
}

fn border_line(el: Option<El<'_>>, ctx: &mut Ctx) -> BorderLine {
    let Some(el) = el else {
        return BorderLine::default();
    };
    BorderLine {
        kind: code(el, "type", enums::LINE_KIND, ctx).unwrap_or(0) as u8,
        width: el
            .attr("width")
            .and_then(|w| enums::width_index(&w))
            .unwrap_or(0),
        color: attr_color(el, "color").unwrap_or(0),
    }
}

/// A diagonal: shape bits, crooked bits and the 180-degree flag.
fn diagonal_bits(el: El<'_>, ctx: &mut Ctx) -> (u16, u16, u16) {
    let shape = code(el, "type", enums::SLASH, ctx).unwrap_or(0) as u16;
    let crooked = el
        .int("Crooked")
        .or_else(|| el.int("isCrooked"))
        .unwrap_or(0) as u16;
    let counter = el.flag("isCounter").unwrap_or(false) as u16;
    (shape, crooked, counter)
}

fn border_fill(el: El<'_>, ctx: &mut Ctx) -> BorderFill {
    let mut rest = Rest::new();
    rest.attrs("", el, &["id", "threeD", "shadow", "centerLine"]);
    let mut attr = el.flag("threeD").unwrap_or(false) as u16;
    attr |= (el.flag("shadow").unwrap_or(false) as u16) << 1;
    let mut b = BorderFill::default();
    let mut slot = 0u16;
    for c in el.children() {
        let known = match (c.ns(), c.local()) {
            (Ns::Hh, b"slash") => {
                let (s, cr, co) = diagonal_bits(c, ctx);
                attr |= s << 2 | (cr & 3) << 8 | co << 11;
                true
            }
            (Ns::Hh, b"backSlash") => {
                let (s, cr, co) = diagonal_bits(c, ctx);
                attr |= s << 5 | (cr & 1) << 10 | co << 12;
                true
            }
            (Ns::Hh, b"leftBorder") => {
                b.sides[0] = border_line(Some(c), ctx);
                true
            }
            (Ns::Hh, b"rightBorder") => {
                b.sides[1] = border_line(Some(c), ctx);
                true
            }
            (Ns::Hh, b"topBorder") => {
                b.sides[2] = border_line(Some(c), ctx);
                true
            }
            (Ns::Hh, b"bottomBorder") => {
                b.sides[3] = border_line(Some(c), ctx);
                true
            }
            (Ns::Hh, b"diagonal") => {
                b.diagonal = border_line(Some(c), ctx);
                true
            }
            (Ns::Hc, b"fillBrush") => {
                b.fill = Some(fill_brush(c, &mut rest, "fillBrush", ctx));
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
    let center = code(el, "centerLine", enums::CENTER_LINE, ctx).unwrap_or(0) as u16;
    attr |= (center & 1) << 13;
    b.attr = attr;
    if b.fill.is_none() {
        b.fill = Some(Fill {
            extra: Some(SharedBytes::empty()),
            ..Fill::default()
        });
    }
    ctx.note_rest(el, &rest);
    b.tail = rest.into_tail();
    b.src = Some(el.source());
    b
}

/// A `hc:fillBrush` as the HWP 5 fill description.
pub fn fill_brush(el: El<'_>, rest: &mut Rest, path: &str, ctx: &mut Ctx) -> Fill {
    let mut f = Fill::default();
    let alpha = |c: El<'_>| c.int("alpha").unwrap_or(0).clamp(0, 255) as u8;
    let (mut a_solid, mut a_grad, mut a_img) = (0u8, 0u8, 0u8);
    let mut step_center = None;
    for (slot, c) in el.children().enumerate() {
        match (c.ns(), c.local()) {
            (Ns::Hc, b"winBrush") => {
                f.kind |= Fill::SOLID;
                rest.attrs(
                    &format!("{path}/winBrush"),
                    c,
                    &["faceColor", "hatchColor", "hatchStyle", "alpha"],
                );
                let pattern = match c.attr("hatchStyle") {
                    Some(h) => enums::code(enums::HATCH, &h).map_or(-1, |v| v as i32),
                    None => -1,
                };
                f.solid = Some(SolidFill {
                    background: attr_color(c, "faceColor").unwrap_or(SolidFill::NO_COLOR),
                    pattern_color: attr_color(c, "hatchColor").unwrap_or(0),
                    pattern,
                });
                a_solid = alpha(c);
            }
            (Ns::Hc, b"gradation") => {
                f.kind |= Fill::GRADIENT;
                let gp = format!("{path}/gradation");
                rest.attrs(
                    &gp,
                    c,
                    &[
                        "type",
                        "angle",
                        "centerX",
                        "centerY",
                        "step",
                        "colorNum",
                        "stepCenter",
                        "alpha",
                    ],
                );
                let mut g = Gradient {
                    kind: code(c, "type", enums::GRADATION, ctx).unwrap_or(1) as u8,
                    angle: c.i32("angle").unwrap_or(0),
                    center_x: c.i32("centerX").unwrap_or(0),
                    center_y: c.i32("centerY").unwrap_or(0),
                    step: c.i32("step").unwrap_or(0),
                    ..Gradient::default()
                };
                for (k, col) in c.children().enumerate() {
                    if col.is(Ns::Hc, "color") {
                        rest.attrs(&format!("{gp}/color"), col, &["value"]);
                        g.colors.push(attr_color(col, "value").unwrap_or(0));
                    } else {
                        rest.child(&gp, k as u16, col);
                    }
                }
                if g.colors.len() > 2 {
                    let n = g.colors.len() as i32 - 1;
                    g.positions = (0..=n).map(|k| k * 100 / n).collect();
                }
                let sc = c.int("stepCenter").unwrap_or(50).clamp(0, 255) as u8;
                g.step_center = Some(sc);
                step_center = Some(sc);
                f.gradient = Some(g);
                a_grad = alpha(c);
            }
            (Ns::Hc, b"imgBrush") => {
                f.kind |= Fill::IMAGE;
                let ip = format!("{path}/imgBrush");
                rest.attrs(&ip, c, &["mode"]);
                let mut img = ImageFill {
                    mode: code(c, "mode", enums::IMG_MODE, ctx).unwrap_or(5) as u8,
                    ..ImageFill::default()
                };
                for (k, i) in c.children().enumerate() {
                    if i.is(Ns::Hc, "img") {
                        rest.attrs(
                            &format!("{ip}/img"),
                            i,
                            &["binaryItemIDRef", "bright", "contrast", "effect", "alpha"],
                        );
                        img.brightness = i.int("bright").unwrap_or(0) as i8;
                        img.contrast = i.int("contrast").unwrap_or(0) as i8;
                        img.effect = code(i, "effect", enums::IMG_EFFECT, ctx).unwrap_or(0) as u8;
                        img.bin_id = i
                            .attr("binaryItemIDRef")
                            .and_then(|r| ctx.bin_index(&r))
                            .unwrap_or(0) as u16;
                        a_img = alpha(i);
                    } else {
                        rest.child(&ip, k as u16, i);
                    }
                }
                f.image = Some(img);
            }
            _ => {
                ctx.unknown(format!("hc:fillBrush/{}", tag(c)));
                rest.child(path, slot as u16, c);
            }
        }
    }
    f.extra = Some(SharedBytes::from(
        step_center.map(|c| vec![c]).unwrap_or_default(),
    ));
    if f.solid.is_some() {
        f.alpha.push(a_solid);
    }
    if f.gradient.is_some() {
        f.alpha.push(a_grad);
    }
    if f.image.is_some() {
        f.alpha.push(a_img);
    }
    f
}

fn per_slot<T: Copy>(el: Option<El<'_>>, default: T, f: impl Fn(i64) -> T) -> [T; 7] {
    let mut out = [default; 7];
    if let Some(el) = el {
        for (i, k) in enums::FONT_ATTR_SLOT.iter().enumerate() {
            if let Some(v) = el.int(k) {
                out[i] = f(v);
            }
        }
    }
    out
}

fn char_shape(el: El<'_>, ctx: &mut Ctx) -> CharShape {
    let mut rest = Rest::new();
    rest.attrs(
        "",
        el,
        &[
            "id",
            "height",
            "textColor",
            "shadeColor",
            "useFontSpace",
            "useKerning",
            "symMark",
            "borderFillIDRef",
        ],
    );
    let mut c = CharShape {
        base_size: el.i32("height").unwrap_or(1000),
        color: attr_color(el, "textColor").unwrap_or(0),
        shade_color: attr_color(el, "shadeColor").unwrap_or(0xFFFF_FFFF),
        border_fill: el.int("borderFillIDRef").map(|v| v as u16),
        ..Default::default()
    };
    let mut attr = 0u32;
    if el.flag("useFontSpace") == Some(true) {
        attr |= 1 << 25;
    }
    if el.flag("useKerning") == Some(true) {
        attr |= 1 << 30;
    }
    attr |= code(el, "symMark", enums::SYM_MARK, ctx).unwrap_or(0) << 21;
    // Elements Hancom leaves out stand for these stored values.
    c.shadow_color = 0x00C0_C0C0;
    c.shadow_offset = [10, 10];
    c.underline_color = 0;
    let mut underline = false;
    for (slot, k) in el.children().enumerate() {
        let path = String::from_utf8_lossy(k.local()).into_owned();
        let known = match (k.ns(), k.local()) {
            (Ns::Hh, b"fontRef") => {
                c.faces = per_slot(Some(k), 0, |v| v as u16);
                true
            }
            (Ns::Hh, b"ratio") => {
                c.ratios = per_slot(Some(k), 100, |v| v as u8);
                true
            }
            (Ns::Hh, b"spacing") => {
                c.spacings = per_slot(Some(k), 0, |v| v as i8);
                true
            }
            (Ns::Hh, b"relSz") => {
                c.rel_sizes = per_slot(Some(k), 100, |v| v as u8);
                true
            }
            (Ns::Hh, b"offset") => {
                c.offsets = per_slot(Some(k), 0, |v| v as i8);
                true
            }
            (Ns::Hh, b"italic") => {
                attr |= 1;
                true
            }
            (Ns::Hh, b"bold") => {
                attr |= 2;
                true
            }
            (Ns::Hh, b"underline") => {
                rest.attrs(&path, k, &["type", "shape", "color"]);
                underline = true;
                let kind = code(k, "type", enums::UNDERLINE_TYPE, ctx).unwrap_or(0);
                let shape = code(k, "shape", enums::LINE_SHAPE, ctx).unwrap_or(0);
                // Character shapes whose strikeout carries Hancom's unset
                // value (shape 3D) carry the matching underline value (kind
                // 2, shape 15), written as NONE with a solid shape.
                let unset_strike = el
                    .child(Ns::Hh, "strikeout")
                    .and_then(|s| s.attr("shape"))
                    .is_some_and(|v| v == "3D");
                let (kind, shape) = if kind == 0 && shape == 0 && unset_strike {
                    (2, 15)
                } else {
                    (kind, shape)
                };
                attr |= kind << 2 | shape << 4;
                c.underline_color = attr_color(k, "color").unwrap_or(0);
                true
            }
            (Ns::Hh, b"strikeout") => {
                rest.attrs(&path, k, &["shape", "color"]);
                let shape = k.attr("shape").unwrap_or_default();
                if !shape.eq_ignore_ascii_case("NONE") {
                    attr |= 1 << 18;
                    attr |= enums::code(enums::LINE_SHAPE, &shape).unwrap_or(0) << 26;
                }
                c.strike_color = Some(attr_color(k, "color").unwrap_or(0));
                true
            }
            (Ns::Hh, b"outline") => {
                rest.attrs(&path, k, &["type"]);
                attr |= code(k, "type", enums::OUTLINE_TYPE, ctx).unwrap_or(0) << 8;
                true
            }
            (Ns::Hh, b"shadow") => {
                rest.attrs(&path, k, &["type", "color", "offsetX", "offsetY"]);
                attr |= code(k, "type", enums::SHADOW_TYPE, ctx).unwrap_or(0) << 11;
                c.shadow_color = attr_color(k, "color").unwrap_or(0);
                c.shadow_offset = [
                    k.int("offsetX").unwrap_or(0) as i8,
                    k.int("offsetY").unwrap_or(0) as i8,
                ];
                true
            }
            (Ns::Hh, b"emboss") => {
                attr |= 1 << 13;
                true
            }
            (Ns::Hh, b"engrave") => {
                attr |= 1 << 14;
                true
            }
            (Ns::Hh, b"supscript") => {
                attr |= 1 << 15;
                true
            }
            (Ns::Hh, b"subscript") => {
                attr |= 1 << 16;
                true
            }
            _ => false,
        };
        if !known {
            rest.child("", slot as u16, k);
        }
    }
    if !underline && attr & (7 << 18) != 0 && (attr >> 26) & 15 == 15 {
        attr |= 2 << 2 | 15 << 4;
    }
    c.attr = attr;
    ctx.note_rest(el, &rest);
    c.tail = rest.into_tail();
    c.src = Some(el.source());
    c
}

fn tab_def(el: El<'_>, ctx: &mut Ctx) -> TabDef {
    let mut rest = Rest::new();
    rest.attrs("", el, &["id", "autoTabLeft", "autoTabRight"]);
    let mut t = TabDef {
        attr: el.flag("autoTabLeft").unwrap_or(false) as u32
            | (el.flag("autoTabRight").unwrap_or(false) as u32) << 1,
        ..Default::default()
    };
    let item = |it: El<'_>, packed: bool, ctx: &mut Ctx| {
        let pos = it.i32("pos").unwrap_or(0);
        let unit = it.attr("unit");
        TabStop {
            position: if packed {
                pos
            } else {
                pack(pos, unit.as_deref())
            },
            kind: code(it, "type", enums::TAB_TYPE, ctx).unwrap_or(0) as u8,
            fill: code(it, "leader", enums::LINE_KIND, ctx).unwrap_or(0) as u8,
            reserved: 0,
        }
    };
    for (slot, c) in el.children().enumerate() {
        if c.is(Ns::Hh, "tabItem") {
            let s = item(c, false, ctx);
            t.tabs.push(s);
        } else if c.is(Ns::Hp, "switch") {
            if let Some(b) = switch_branch(c) {
                let packed = b.is(Ns::Hp, "default");
                for it in b.children().filter(|x| x.is(Ns::Hh, "tabItem")) {
                    let s = item(it, packed, ctx);
                    t.tabs.push(s);
                }
            }
        } else {
            rest.child("", slot as u16, c);
        }
    }
    ctx.note_rest(el, &rest);
    t.tail = rest.into_tail();
    t.src = Some(el.source());
    t
}

fn para_head(el: El<'_>, ctx: &mut Ctx) -> ParaHead {
    let mut attr = code(el, "align", enums::HEAD_ALIGN, ctx).unwrap_or(0);
    attr |= (el.flag("useInstWidth").unwrap_or(false) as u32) << 2;
    attr |= (el.flag("autoIndent").unwrap_or(false) as u32) << 3;
    if el
        .attr("textOffsetType")
        .is_some_and(|v| v.eq_ignore_ascii_case("HWPUNIT"))
    {
        attr |= 1 << 4;
    }
    attr |= code(el, "numFormat", enums::NUMBER_FORMAT, ctx).unwrap_or(0) << 5;
    ParaHead {
        attr,
        width_adjust: el.int("widthAdjust").unwrap_or(0) as i16,
        text_distance: el.int("textOffset").unwrap_or(0) as i16,
        char_shape: el.u32("charPrIDRef").unwrap_or(u32::MAX),
    }
}

const PARA_HEAD_KNOWN: &[&str] = &[
    "start",
    "level",
    "align",
    "useInstWidth",
    "autoIndent",
    "widthAdjust",
    "textOffsetType",
    "textOffset",
    "numFormat",
    "charPrIDRef",
    "checkable",
];

fn numbering(el: El<'_>, ctx: &mut Ctx) -> Numbering {
    let mut rest = Rest::new();
    rest.attrs("", el, &["id", "start"]);
    let mut n = Numbering {
        start: Some(el.int("start").unwrap_or(1) as u16),
        ..Default::default()
    };
    let mut starts = [1u32; 7];
    let mut slot = 0u16;
    for c in el.children() {
        if c.is(Ns::Hh, "paraHead") && n.levels.len() < 7 {
            rest.attrs("paraHead", c, PARA_HEAD_KNOWN);
            starts[n.levels.len()] = c.u32("start").unwrap_or(1);
            n.levels.push(NumberingLevel {
                head: para_head(c, ctx),
                format: hstr(&c.text()),
            });
            slot += 1;
        } else {
            rest.child("", slot, c);
        }
    }
    while n.levels.len() < 7 {
        n.levels.push(NumberingLevel::default());
    }
    n.level_starts = Some(starts);
    ctx.note_rest(el, &rest);
    n.tail = rest.into_tail();
    n.src = Some(el.source());
    n
}

fn bullet(el: El<'_>, ctx: &mut Ctx) -> Bullet {
    let mut rest = Rest::new();
    rest.attrs("", el, &["id", "char", "checkedChar", "useImage"]);
    let first =
        |s: Option<std::borrow::Cow<'_, str>>| s.and_then(|v| v.encode_utf16().next()).unwrap_or(0);
    let mut b = Bullet {
        bullet_char: first(el.attr("char")),
        image: Some(0),
        image_info: Some([0; 4]),
        check_char: Some(first(el.attr("checkedChar"))),
        ..Default::default()
    };
    let mut slot = 0u16;
    for c in el.children() {
        if c.is(Ns::Hh, "paraHead") {
            rest.attrs("paraHead", c, PARA_HEAD_KNOWN);
            b.head = para_head(c, ctx);
            slot += 1;
        } else if c.is(Ns::Hc, "img") {
            rest.attrs(
                "img",
                c,
                &["binaryItemIDRef", "bright", "contrast", "effect"],
            );
            let id = c
                .attr("binaryItemIDRef")
                .and_then(|r| ctx.bin_index(&r))
                .unwrap_or(0);
            b.image = Some(id as i32);
            b.image_info = Some([
                c.int("contrast").unwrap_or(0) as u8,
                c.int("bright").unwrap_or(0) as u8,
                code(c, "effect", enums::IMG_EFFECT, ctx).unwrap_or(0) as u8,
                id as u8,
            ]);
            slot += 1;
        } else {
            rest.child("", slot, c);
        }
    }
    if el.flag("useImage") != Some(true) {
        b.image = Some(0);
    }
    ctx.note_rest(el, &rest);
    b.tail = rest.into_tail();
    b.src = Some(el.source());
    b
}

fn para_shape(el: El<'_>, ctx: &mut Ctx) -> ParaShape {
    let mut rest = Rest::new();
    rest.attrs(
        "",
        el,
        &[
            "id",
            "tabPrIDRef",
            "condense",
            "fontLineHeight",
            "snapToGrid",
        ],
    );
    let mut p = ParaShape {
        tab_def: el.int("tabPrIDRef").unwrap_or(0) as u16,
        ..Default::default()
    };
    let mut a1 = 0u32;
    let mut a2 = 0u32;
    let mut a3 = 0u32;
    a1 |= (el.int("condense").unwrap_or(0) as u32 & 0x7F) << 9;
    a1 |= (el.flag("fontLineHeight").unwrap_or(false) as u32) << 22;
    a1 |= (el.flag("snapToGrid").unwrap_or(false) as u32) << 8;
    let mut spacing: Option<(u32, i32)> = None;
    let mut slot = 0u16;
    for c in el.children() {
        let known = match (c.ns(), c.local()) {
            (Ns::Hh, b"align") => {
                rest.attrs("align", c, &["horizontal", "vertical"]);
                a1 |= code(c, "horizontal", enums::H_ALIGN, ctx).unwrap_or(0) << 2;
                a1 |= code(c, "vertical", enums::V_ALIGN, ctx).unwrap_or(0) << 20;
                true
            }
            (Ns::Hh, b"heading") => {
                rest.attrs("heading", c, &["type", "idRef", "level"]);
                a1 |= code(c, "type", enums::HEADING, ctx).unwrap_or(0) << 23;
                p.numbering = c.int("idRef").unwrap_or(0) as u16;
                a1 |= (c.int("level").unwrap_or(0) as u32 & 7) << 25;
                true
            }
            (Ns::Hh, b"breakSetting") => {
                rest.attrs(
                    "breakSetting",
                    c,
                    &[
                        "breakLatinWord",
                        "breakNonLatinWord",
                        "widowOrphan",
                        "keepWithNext",
                        "keepLines",
                        "pageBreakBefore",
                        "lineWrap",
                    ],
                );
                a1 |= code(c, "breakLatinWord", enums::BREAK_LATIN, ctx).unwrap_or(0) << 5;
                a1 |= code(c, "breakNonLatinWord", enums::BREAK_NON_LATIN, ctx).unwrap_or(0) << 7;
                a1 |= (c.flag("widowOrphan").unwrap_or(false) as u32) << 16;
                a1 |= (c.flag("keepWithNext").unwrap_or(false) as u32) << 17;
                a1 |= (c.flag("keepLines").unwrap_or(false) as u32) << 18;
                a1 |= (c.flag("pageBreakBefore").unwrap_or(false) as u32) << 19;
                a2 |= code(c, "lineWrap", enums::LINE_WRAP, ctx).unwrap_or(0) & 3;
                true
            }
            (Ns::Hh, b"autoSpacing") => {
                rest.attrs("autoSpacing", c, &["eAsianEng", "eAsianNum"]);
                a2 |= (c.flag("eAsianEng").unwrap_or(false) as u32) << 4;
                a2 |= (c.flag("eAsianNum").unwrap_or(false) as u32) << 5;
                true
            }
            (Ns::Hp, b"switch") => {
                if let Some(h) = c
                    .children()
                    .filter(|b| b.is(Ns::Hp, "case"))
                    .find_map(|b| b.child(Ns::Hh, "heading"))
                {
                    // Outline levels 8 to 10 live in a 2016 case branch.
                    a1 &= !(3 << 23 | 7 << 25);
                    a1 |= code(h, "type", enums::HEADING, ctx).unwrap_or(0) << 23;
                    p.numbering = h.int("idRef").unwrap_or(0) as u16;
                    a1 |= (h.int("level").unwrap_or(0).clamp(0, 6) as u32) << 25;
                }
                if let Some(b) = switch_branch(c) {
                    let packed = b.is(Ns::Hp, "default");
                    for m in b.children() {
                        if m.is(Ns::Hh, "margin") {
                            margins(m, packed, &mut p);
                        } else if m.is(Ns::Hh, "lineSpacing") {
                            spacing = Some(line_spacing(m, packed, ctx));
                        }
                    }
                }
                true
            }
            (Ns::Hh, b"margin") => {
                margins(c, false, &mut p);
                true
            }
            (Ns::Hh, b"lineSpacing") => {
                spacing = Some(line_spacing(c, false, ctx));
                true
            }
            (Ns::Hh, b"border") => {
                rest.attrs(
                    "border",
                    c,
                    &[
                        "borderFillIDRef",
                        "offsetLeft",
                        "offsetRight",
                        "offsetTop",
                        "offsetBottom",
                        "connect",
                        "ignoreMargin",
                    ],
                );
                p.border_fill = c.int("borderFillIDRef").unwrap_or(0) as u16;
                p.border_offsets = [
                    c.int("offsetLeft").unwrap_or(0) as i16,
                    c.int("offsetRight").unwrap_or(0) as i16,
                    c.int("offsetTop").unwrap_or(0) as i16,
                    c.int("offsetBottom").unwrap_or(0) as i16,
                ];
                a1 |= (c.flag("connect").unwrap_or(false) as u32) << 28;
                a1 |= (c.flag("ignoreMargin").unwrap_or(false) as u32) << 29;
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
    let (kind, value) = spacing.unwrap_or((0, 160));
    a3 |= kind & 0x1F;
    // The legacy pair mirrors the current one: kinds 0 to 2 keep their
    // code; at-least has none and reads as percent.
    a1 |= if kind <= 2 { kind } else { 0 };
    p.line_spacing_legacy = if kind <= 2 { value } else { 100 };
    p.line_spacing = Some(value as u32);
    p.attr1 = a1;
    p.attr2 = Some(a2);
    p.attr3 = Some(a3);
    ctx.note_rest(el, &rest);
    p.tail = rest.into_tail();
    p.src = Some(el.source());
    p
}

fn margins(m: El<'_>, packed: bool, p: &mut ParaShape) {
    for v in m.children() {
        let raw = v.i32("value").unwrap_or(0);
        let value = if packed {
            raw
        } else {
            pack(raw, v.attr("unit").as_deref())
        };
        match v.local() {
            b"intent" | b"indent" => p.indent = value,
            b"left" => p.margin_left = value,
            b"right" => p.margin_right = value,
            b"prev" => p.space_before = value,
            b"next" => p.space_after = value,
            _ => {}
        }
    }
}

fn line_spacing(m: El<'_>, packed: bool, ctx: &mut Ctx) -> (u32, i32) {
    let kind = code(m, "type", enums::LINE_SPACING, ctx).unwrap_or(0);
    let raw = m.i32("value").unwrap_or(160);
    let value = if kind == 0 || packed {
        raw
    } else {
        pack(raw, m.attr("unit").as_deref())
    };
    (kind, value)
}

fn style(el: El<'_>, ctx: &mut Ctx) -> Style {
    let mut rest = Rest::new();
    rest.attrs(
        "",
        el,
        &[
            "id",
            "type",
            "name",
            "engName",
            "paraPrIDRef",
            "charPrIDRef",
            "nextStyleIDRef",
            "langID",
            "lockForm",
        ],
    );
    for (slot, c) in el.children().enumerate() {
        rest.child("", slot as u16, c);
    }
    let kind = code(el, "type", enums::STYLE_TYPE, ctx).unwrap_or(0) as u8;
    let s = Style {
        local_name: hstr(&el.attr("name").unwrap_or_default()),
        english_name: hstr(&el.attr("engName").unwrap_or_default()),
        attr: kind | (el.flag("lockForm").unwrap_or(false) as u8) << 2,
        next: el.int("nextStyleIDRef").unwrap_or(0) as u8,
        language: el.int("langID").unwrap_or(0) as i16,
        para_shape: el.int("paraPrIDRef").unwrap_or(0) as u16,
        char_shape: el.int("charPrIDRef").unwrap_or(0) as u16,
        tail: SharedBytes::empty(),
        src: Some(el.source()),
    };
    ctx.note_rest(el, &rest);
    Style {
        tail: rest.into_tail(),
        ..s
    }
}

/// Layout compatibility flag elements by word and bit.
pub const COMPAT_FLAGS: &[(&str, u8, u8)] = &[
    ("applyFontWeightToBold", 0, 0),
    ("useInnerUnderline", 0, 1),
    ("fixedUnderlineWidth", 0, 2),
    ("doNotApplyStrikeoutWithUnderline", 0, 3),
    ("useLowercaseStrikeout", 0, 5),
    ("extendLineheightToOffset", 0, 6),
    ("applyFontspaceToLatin", 0, 7),
    ("treatQuotationAsLatin", 0, 8),
    ("doNotAlignWhitespaceOnRight", 1, 0),
    ("doNotAdjustWordInJustify", 1, 1),
    ("baseCharUnitOnEAsian", 1, 2),
    ("baseCharUnitOfIndentOnFirstChar", 1, 3),
    ("adjustLineheightToFont", 1, 4),
    ("adjustBaselineInFixedLinespacing", 1, 5),
    ("applyPrevspacingBeneathObject", 1, 6),
    ("applyNextspacingOfLastPara", 1, 7),
    ("applyAtLeastToPercent100Pct", 1, 8),
    ("doNotApplyAutoSpaceEAsianEng", 1, 9),
    ("doNotApplyAutoSpaceEAsianNum", 1, 10),
    ("adjustParaBorderfillToSpacing", 1, 11),
    ("connectParaBorderfillOfEqualBorder", 1, 12),
    ("adjustParaBorderOffsetWithBorder", 1, 13),
    ("extendLineheightToParaBorderOffset", 1, 14),
    ("applyParaBorderToOutside", 1, 15),
    ("applyMinColumnWidthTo1mm", 1, 16),
    ("applyTabPosBasedOnSegment", 1, 17),
    ("breakTabOverLine", 1, 18),
    ("adjustVertPosOfLine", 1, 19),
    ("doNotApplyWhiteSpaceHeight", 1, 20),
    ("doNotAlignLastPeriod", 1, 21),
    ("doNotAlignLastForbidden", 1, 22),
    ("adjustMarginFromAdjustLineheight", 1, 23),
    ("baseLineSpacingOnLineGrid", 2, 0),
    ("applyCharSpacingToCharGrid", 2, 1),
    ("doNotApplyGridInHeaderFooter", 2, 2),
    ("applyExtendHeaderFooterEachSection", 2, 3),
    ("doNotApplyHeaderFooterAtNoSpace", 2, 4),
    ("doNotApplyColSeparatorAtNoGap", 2, 5),
    ("doNotApplyLinegridAtNoLinespacing", 2, 6),
    ("doNotApplyImageEffect", 3, 0),
    ("doNotApplyShapeComment", 3, 1),
    ("doNotAdjustEmptyAnchorLine", 3, 2),
    ("overlapBothAllowOverlap", 3, 3),
    ("doNotApplyVertOffsetOfForward", 3, 4),
    ("extendVertLimitToPageMargins", 3, 5),
    ("doNotHoldAnchorOfTable", 3, 6),
    ("doNotFormattingAtBeneathAnchor", 3, 7),
    ("adjustBaselineOfObjectToBottom", 3, 8),
    ("doNotApplyExtensionCharCompose", 4, 0),
];

fn compatible(el: El<'_>, info: &mut DocInfo, ctx: &mut Ctx) {
    let mut rest = Rest::new();
    rest.attrs("", el, &["targetProgram"]);
    let target = code(el, "targetProgram", enums::TARGET_PROGRAM, ctx).unwrap_or(0);
    let mut lc: Option<LayoutCompatibility> = None;
    for (slot, c) in el.children().enumerate() {
        if c.is(Ns::Hh, "layoutCompatibility") {
            let mut flags = [0u32; 5];
            let mut lrest = Rest::new();
            compat_flags(c, &mut flags, &mut lrest, ctx);
            ctx.note_rest(c, &lrest);
            lc = Some(LayoutCompatibility {
                flags,
                tail: lrest.into_tail(),
                src: Some(c.source()),
            });
        } else {
            rest.child("", slot as u16, c);
        }
    }
    ctx.note_rest(el, &rest);
    info.compatible = Some(CompatibleDocument {
        target,
        tail: rest.into_tail(),
        src: Some(el.source()),
    });
    info.order.push(InfoSlot {
        item: InfoItem::Compatible,
        level: 0,
    });
    if let Some(lc) = lc {
        info.layout_compat = Some(lc);
        info.order.push(InfoSlot {
            item: InfoItem::LayoutCompat,
            level: 0,
        });
    }
}

fn compat_flags(el: El<'_>, flags: &mut [u32; 5], rest: &mut Rest, ctx: &mut Ctx) {
    for c in el.children() {
        let name = String::from_utf8_lossy(c.local());
        if let Some(&(_, w, b)) = COMPAT_FLAGS
            .iter()
            .find(|(n, _, _)| n.eq_ignore_ascii_case(&name))
        {
            flags[w as usize] |= 1 << b;
        } else if matches!(
            c.local(),
            b"char" | b"paragraph" | b"section" | b"object" | b"field"
        ) {
            compat_flags(c, flags, rest, ctx);
        } else {
            ctx.unknown(format!("layoutCompatibility/{name}"));
            rest.child("", 0, c);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_become_colorref() {
        assert_eq!(color("#FF0000"), 0x0000_00FF);
        assert_eq!(color("#123456"), 0x0056_3412);
        assert_eq!(color("none"), 0xFFFF_FFFF);
        assert_eq!(color("#80FF0000"), 0x8000_00FF);
    }

    #[test]
    fn packs_units() {
        assert_eq!(pack(-790, Some("CHAR")), -1579);
        assert_eq!(pack(26478, Some("HWPUNIT")), 52956);
    }
}
