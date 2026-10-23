//! [`DocInfo`] into `Contents/header.xml`.

use hm_model::{
    BorderFill, BorderLine, Bullet, CharShape, CompatibleDocument, DocInfo, Document, FaceName,
    Fill, LayoutCompatibility, Numbering, ParaHead, ParaShape, Style, TabDef,
};

use super::{Root, X, color, name};
use crate::enums;
use crate::head::COMPAT_FLAGS;
use crate::origin::Parts;
use crate::xml::{El, Ns, Xml};

/// Manifest item ids of the BinData entries, for references by 1-based id.
pub fn bin_ids(doc: &Document) -> Vec<String> {
    doc.info
        .bin_data
        .iter()
        .enumerate()
        .map(|(i, e)| {
            crate::rest::pieces(&e.tail)
                .iter()
                .find_map(|p| {
                    let v = p.bytes.strip_prefix(b"id=\"")?.strip_suffix(b"\"")?;
                    Some(crate::xml::unescape(v).into_owned())
                })
                .unwrap_or_else(|| {
                    let n = e.storage_id.unwrap_or(i as u16 + 1);
                    if e.kind() == 2 {
                        format!("ole{n}")
                    } else {
                        format!("image{n}")
                    }
                })
        })
        .collect()
}

pub fn bin_ref(bins: &[String], id: u32) -> String {
    id.checked_sub(1)
        .and_then(|i| bins.get(i as usize))
        .cloned()
        .unwrap_or_default()
}

fn hstr(s: &hm_model::HwpString) -> String {
    s.to_string_lossy()
}

pub fn header(doc: &Document, src: &Xml, bins: &[String], parts: &Parts<'_>) -> Vec<u8> {
    let info = &doc.info;
    let root_el = src.root();
    let mut root = if root_el.is(Ns::Hh, "head") {
        Root::from_source(src)
    } else {
        Root::fresh("hh:head", " version=\"1.4\"")
    };
    root.set_attr("secCnt", &doc.sections.len().to_string());
    let mut out = root.prolog.clone();
    out.extend_from_slice(&root.start);
    let mut done = [false; 3];
    let kids: Vec<El<'_>> = if root_el.is(Ns::Hh, "head") {
        root_el.children().collect()
    } else {
        Vec::new()
    };
    let emit = |which: usize, done: &mut [bool; 3], out: &mut Vec<u8>, from: Option<El<'_>>| {
        done[which] = true;
        match which {
            0 => match info.properties.src.as_ref().and_then(|s| clean(parts, s)) {
                Some(m) => out.extend_from_slice(&m),
                None => begin_num(info).write(out),
            },
            1 => ref_list(info, from, bins, parts).write(out),
            _ => compatible(info, out, parts),
        }
    };
    for k in &kids {
        match (k.ns(), k.local()) {
            (Ns::Hh, b"beginNum") => emit(0, &mut done, &mut out, None),
            (Ns::Hh, b"refList") => {
                if !done[0] {
                    emit(0, &mut done, &mut out, None);
                }
                emit(1, &mut done, &mut out, Some(*k));
            }
            (Ns::Hh, b"compatibleDocument") => emit(2, &mut done, &mut out, None),
            _ => out.extend_from_slice(&k.source()),
        }
    }
    for w in 0..3 {
        if !done[w] {
            emit(w, &mut done, &mut out, None);
        }
    }
    out.extend_from_slice(&root.end_tag());
    out
}

fn begin_num(info: &DocInfo) -> X {
    let p = &info.properties;
    X::new("hh:beginNum")
        .a("page", p.page_start)
        .a("footnote", p.footnote_start)
        .a("endnote", p.endnote_start)
        .a("pic", p.figure_start)
        .a("tbl", p.table_start)
        .a("equation", p.equation_start)
}

fn ref_list(info: &DocInfo, src: Option<El<'_>>, bins: &[String], parts: &Parts<'_>) -> X {
    let groups: Vec<El<'_>> = src.map(|s| s.children().collect()).unwrap_or_default();
    let ours = [
        "fontfaces",
        "borderFills",
        "charProperties",
        "tabProperties",
        "numberings",
        "bullets",
        "paraProperties",
        "styles",
    ];
    let mut x = X::new("hh:refList");
    let mut done = [false; 8];
    let put = |x: &mut X, done: &mut [bool; 8], i: usize| {
        done[i] = true;
        let g = match i {
            0 => faces(info, parts),
            1 => group(
                "hh:borderFills",
                info.border_fills
                    .iter()
                    .enumerate()
                    .map(|(i, b)| entry(parts, &b.src, || border_fill(b, i, bins))),
            ),
            2 => group(
                "hh:charProperties",
                info.char_shapes
                    .iter()
                    .enumerate()
                    .map(|(i, c)| entry(parts, &c.src, || char_shape(c, i))),
            ),
            3 => group(
                "hh:tabProperties",
                info.tab_defs
                    .iter()
                    .enumerate()
                    .map(|(i, t)| entry(parts, &t.src, || tab_def(t, i))),
            ),
            4 => group(
                "hh:numberings",
                info.numberings
                    .iter()
                    .enumerate()
                    .map(|(i, n)| entry(parts, &n.src, || numbering(n, i))),
            ),
            5 => group(
                "hh:bullets",
                info.bullets
                    .iter()
                    .enumerate()
                    .map(|(i, b)| entry(parts, &b.src, || bullet(b, i, bins))),
            ),
            6 => group(
                "hh:paraProperties",
                info.para_shapes
                    .iter()
                    .enumerate()
                    .map(|(i, p)| entry(parts, &p.src, || para_shape(p, i))),
            ),
            _ => group(
                "hh:styles",
                info.styles
                    .iter()
                    .enumerate()
                    .map(|(i, s)| entry(parts, &s.src, || style(s, i))),
            ),
        };
        // Hancom leaves out empty bullet and numbering lists.
        if !(matches!(i, 4 | 5) && g.kids.is_empty()) {
            x.kids.push(super::K::E(g));
        }
    };
    for g in &groups {
        let local = std::str::from_utf8(g.local()).unwrap_or("");
        match ours.iter().position(|o| *o == local) {
            Some(i) if g.ns() == Ns::Hh => {
                // Lists missing from the source go before the first later one.
                for j in 0..i {
                    if !done[j] && !groups.iter().any(|h| h.local() == ours[j].as_bytes()) {
                        put(&mut x, &mut done, j);
                    }
                }
                put(&mut x, &mut done, i);
            }
            _ => x.kids.push(super::K::Raw(g.source())),
        }
    }
    for i in 0..8 {
        if !done[i] {
            put(&mut x, &mut done, i);
        }
    }
    x
}

/// A list element with its item count.
fn group(name: &'static str, items: impl Iterator<Item = super::K>) -> X {
    let mut x = X::new(name);
    x.kids.extend(items);
    let n = x.kids.len();
    x.attrs.push(("itemCnt", n.to_string()));
    x
}

/// A clean entry's source markup, or its encoding.
fn entry(
    parts: &Parts<'_>,
    src: &Option<hm_model::SharedBytes>,
    f: impl FnOnce() -> X,
) -> super::K {
    match src.as_ref().and_then(|s| clean(parts, s)) {
        Some(m) => super::K::Raw(m),
        None => super::K::E(f()),
    }
}

/// The markup of a clean entry; an empty `src` marks a clean entry the
/// source has no element for.
fn clean(parts: &Parts<'_>, src: &hm_model::SharedBytes) -> Option<hm_model::SharedBytes> {
    if src.is_empty() {
        return None;
    }
    parts.markup(src)
}

fn faces(info: &DocInfo, parts: &Parts<'_>) -> X {
    let mut x = X::new("hh:fontfaces");
    for (slot, list) in info.faces.iter().enumerate() {
        let mut ff = X::new("hh:fontface")
            .a("lang", enums::LANG[slot])
            .a("fontCnt", list.len());
        for (i, f) in list.iter().enumerate() {
            ff.kids.push(entry(parts, &f.src, || font(f, i)));
        }
        x = x.kid(ff);
    }
    let n = x.kids.len();
    x.attrs.insert(0, ("itemCnt", n.to_string()));
    x
}

fn font(f: &FaceName, i: usize) -> X {
    let mut x = X::new("hh:font")
        .a("id", i)
        .a("face", hstr(&f.name))
        .a("type", name(enums::FONT_TYPE, (f.attr & 0x1F) as u32));
    if let Some((kind, n)) = &f.substitute {
        x = x.kid(
            X::new("hh:substFont")
                .a("face", hstr(n))
                .a("type", name(enums::FONT_TYPE, *kind as u32)),
        );
    }
    if let Some(p) = f.panose {
        let mut t = X::new("hh:typeInfo").a("familyType", name(enums::FAMILY_TYPE, p[0] as u32));
        if p[1] != 0 {
            t = t.a("serifStyle", p[1]);
        }
        t = t
            .a("weight", p[2])
            .a("proportion", p[3])
            .a("contrast", p[4])
            .a("strokeVariation", p[5])
            .a("armStyle", p[6])
            .a("letterform", p[7])
            .a("midline", p[8])
            .a("xHeight", p[9]);
        x = x.kid(t);
    }
    x.with_rest(&f.tail)
}

fn line(tag: &'static str, l: &BorderLine) -> X {
    X::new(tag)
        .a("type", name(enums::LINE_KIND, l.kind as u32))
        .a("width", enums::width_name(l.width))
        .a("color", color(l.color))
}

fn border_fill(b: &BorderFill, i: usize, bins: &[String]) -> X {
    let a = b.attr as u32;
    let diag = |tag: &'static str, shape: u32, crooked: u32, counter: u32| {
        X::new(tag)
            .a("type", name(enums::SLASH, shape))
            .a("Crooked", crooked)
            .a("isCounter", counter)
    };
    let mut x = X::new("hh:borderFill")
        .a("id", i + 1)
        .b("threeD", a & 1 != 0)
        .b("shadow", a & 2 != 0)
        .b("breakCellSeparateLine", b.break_cell_separate_line())
        .a(
            "centerLine",
            if a & (1 << 13) != 0 {
                "VERTICAL"
            } else {
                "NONE"
            },
        )
        .kid(diag("hh:slash", (a >> 2) & 7, (a >> 8) & 3, (a >> 11) & 1))
        .kid(diag("hh:backSlash", (a >> 5) & 7, 0, (a >> 12) & 1))
        .kid(line("hh:leftBorder", &b.sides[0]))
        .kid(line("hh:rightBorder", &b.sides[1]))
        .kid(line("hh:topBorder", &b.sides[2]))
        .kid(line("hh:bottomBorder", &b.sides[3]))
        .kid(line("hh:diagonal", &b.diagonal));
    if let Some(f) = b.fill.as_ref().filter(|f| f.kind != 0) {
        x = x.kid(fill_brush(f, bins));
    }
    x.with_rest(&b.tail)
}

/// A fill as `hc:fillBrush`.
pub fn fill_brush(f: &Fill, bins: &[String]) -> X {
    let mut x = X::new("hc:fillBrush");
    let mut alpha = f.alpha.iter().copied();
    if let Some(s) = &f.solid {
        let mut w = X::new("hc:winBrush")
            .a("faceColor", color(s.background))
            .a("hatchColor", color(s.pattern_color));
        if s.pattern > 0 {
            w = w.a("hatchStyle", name(enums::HATCH, s.pattern as u32));
        }
        x = x.kid(w.a("alpha", alpha.next().unwrap_or(0)));
    }
    if let Some(g) = &f.gradient {
        let mut gx = X::new("hc:gradation")
            .a("type", name(enums::GRADATION, g.kind as u32))
            .a("angle", g.angle)
            .a("centerX", g.center_x)
            .a("centerY", g.center_y)
            .a("step", g.step)
            .a("colorNum", g.colors.len())
            .a("stepCenter", g.step_center.unwrap_or(50))
            .a("alpha", alpha.next().unwrap_or(0));
        for c in &g.colors {
            gx = gx.kid(X::new("hc:color").a("value", color(*c)));
        }
        x = x.kid(gx);
    }
    if let Some(i) = &f.image {
        x = x.kid(
            X::new("hc:imgBrush")
                .a("mode", name(enums::IMG_MODE, i.mode as u32))
                .kid(
                    X::new("hc:img")
                        .a("binaryItemIDRef", bin_ref(bins, i.bin_id as u32))
                        .a("bright", i.brightness)
                        .a("contrast", i.contrast)
                        .a("effect", name(enums::IMG_EFFECT, i.effect as u32))
                        .a("alpha", alpha.next().unwrap_or(0)),
                ),
        );
    }
    x
}

fn slots<T: std::fmt::Display + Copy>(tag: &'static str, v: [T; 7]) -> X {
    let mut x = X::new(tag);
    for (i, k) in enums::FONT_ATTR_SLOT.iter().enumerate() {
        x = x.a(k, v[i]);
    }
    x
}

fn char_shape(c: &CharShape, i: usize) -> X {
    let a = c.attr;
    let mut x = X::new("hh:charPr")
        .a("id", i)
        .a("height", c.base_size)
        .a("textColor", color(c.color))
        .a("shadeColor", color(c.shade_color))
        .b("useFontSpace", a & (1 << 25) != 0)
        .b("useKerning", a & (1 << 30) != 0)
        .a("symMark", name(enums::SYM_MARK, (a >> 21) & 15));
    if let Some(b) = c.border_fill {
        x = x.a("borderFillIDRef", b);
    }
    x = x
        .kid(slots("hh:fontRef", c.faces))
        .kid(slots("hh:ratio", c.ratios))
        .kid(slots("hh:spacing", c.spacings))
        .kid(slots("hh:relSz", c.rel_sizes))
        .kid(slots("hh:offset", c.offsets));
    if a & 1 != 0 {
        x = x.kid(X::new("hh:italic"));
    }
    if a & 2 != 0 {
        x = x.kid(X::new("hh:bold"));
    }
    let (kind, shape) = ((a >> 2) & 3, (a >> 4) & 15);
    let (kind, shape) = if (kind, shape) == (2, 15) {
        (0, 0)
    } else {
        (kind, shape)
    };
    x = x.kid(
        X::new("hh:underline")
            .a("type", name(enums::UNDERLINE_TYPE, kind))
            .a("shape", name(enums::LINE_SHAPE, shape))
            .a("color", color(c.underline_color)),
    );
    let strike = if (a >> 18) & 7 == 0 {
        "NONE".to_string()
    } else {
        name(enums::LINE_SHAPE, (a >> 26) & 15)
    };
    if let Some(sc) = c.strike_color {
        x = x.kid(
            X::new("hh:strikeout")
                .a("shape", strike)
                .a("color", color(sc)),
        );
    }
    x = x
        .kid(X::new("hh:outline").a("type", name(enums::OUTLINE_TYPE, (a >> 8) & 7)))
        .kid(
            X::new("hh:shadow")
                .a("type", name(enums::SHADOW_TYPE, (a >> 11) & 3))
                .a("color", color(c.shadow_color))
                .a("offsetX", c.shadow_offset[0])
                .a("offsetY", c.shadow_offset[1]),
        );
    for (bit, tag) in [
        (13, "hh:emboss"),
        (14, "hh:engrave"),
        (15, "hh:supscript"),
        (16, "hh:subscript"),
    ] {
        if a & (1 << bit) != 0 {
            x = x.kid(X::new(tag));
        }
    }
    x.with_rest(&c.tail)
}

/// A packed distance as a value and unit.
fn unpack(v: i32) -> (i32, &'static str) {
    (v >> 1, if v & 1 != 0 { "CHAR" } else { "HWPUNIT" })
}

/// The 2016 unit-aware branch and the HWP 5 default branch of a value set.
fn switch(case: X, default: X) -> X {
    X::new("hp:switch")
        .kid(
            X::new("hp:case")
                .a(
                    "hp:required-namespace",
                    "http://www.hancom.co.kr/hwpml/2016/HwpUnitChar",
                )
                .kids(case.kids.into_iter().filter_map(|k| match k {
                    super::K::E(x) => Some(x),
                    _ => None,
                })),
        )
        .kid(
            X::new("hp:default").kids(default.kids.into_iter().filter_map(|k| match k {
                super::K::E(x) => Some(x),
                _ => None,
            })),
        )
}

fn tab_def(t: &TabDef, i: usize) -> X {
    let mut x = X::new("hh:tabPr")
        .a("id", i)
        .b("autoTabLeft", t.attr & 1 != 0)
        .b("autoTabRight", t.attr & 2 != 0);
    if !t.tabs.is_empty() {
        let mut case = X::new("c");
        let mut default = X::new("d");
        for s in &t.tabs {
            let (v, unit) = unpack(s.position);
            let kind = name(enums::TAB_TYPE, s.kind as u32);
            let leader = name(enums::LINE_KIND, s.fill as u32);
            case = case.kid(
                X::new("hh:tabItem")
                    .a("pos", v)
                    .a("type", &kind)
                    .a("leader", &leader)
                    .a("unit", unit),
            );
            default = default.kid(
                X::new("hh:tabItem")
                    .a("pos", s.position)
                    .a("type", kind)
                    .a("leader", leader),
            );
        }
        x = x.kid(switch(case, default));
    }
    x.with_rest(&t.tail)
}

fn para_head(h: &ParaHead, level: usize, start: Option<u32>) -> X {
    let mut x = X::new("hh:paraHead");
    if let Some(s) = start {
        x = x.a("start", s);
    }
    x.a("level", level)
        .a("align", name(enums::HEAD_ALIGN, h.attr & 3))
        .b("useInstWidth", h.attr & 4 != 0)
        .b("autoIndent", h.attr & 8 != 0)
        .a("widthAdjust", h.width_adjust)
        .a(
            "textOffsetType",
            if h.attr & 16 != 0 {
                "HWPUNIT"
            } else {
                "PERCENT"
            },
        )
        .a("textOffset", h.text_distance)
        .a(
            "numFormat",
            name(enums::NUMBER_FORMAT, (h.attr >> 5) & 0xFF),
        )
        .a("charPrIDRef", h.char_shape)
}

fn numbering(n: &Numbering, i: usize) -> X {
    let mut x = X::new("hh:numbering")
        .a("id", i + 1)
        .a("start", n.start.unwrap_or(1));
    for (k, l) in n.levels.iter().enumerate() {
        let start = n.level_starts.map_or(1, |s| s[k.min(6)]);
        let mut h = para_head(&l.head, k + 1, Some(start));
        let f = l.format.to_string_lossy();
        if !f.is_empty() {
            h = h.text(f);
        }
        x = x.kid(h);
    }
    x.with_rest(&n.tail)
}

fn bullet(b: &Bullet, i: usize, bins: &[String]) -> X {
    let ch = |c: u16| {
        char::from_u32(c as u32)
            .map(String::from)
            .unwrap_or_default()
    };
    let image = b.image.unwrap_or(0);
    let mut x = X::new("hh:bullet")
        .a("id", i + 1)
        .a("char", ch(b.bullet_char));
    if let Some(c) = b.check_char.filter(|&c| c != 0) {
        x = x.a("checkedChar", ch(c));
    }
    x = x.b("useImage", image != 0);
    if image != 0 {
        let info = b.image_info.unwrap_or_default();
        x = x.kid(
            X::new("hc:img")
                .a("binaryItemIDRef", bin_ref(bins, image as u32))
                .a("bright", info[1] as i8)
                .a("contrast", info[0] as i8)
                .a("effect", name(enums::IMG_EFFECT, info[2] as u32)),
        );
    }
    x.kid(para_head(&b.head, 0, None)).with_rest(&b.tail)
}

fn para_shape(p: &ParaShape, i: usize) -> X {
    let a1 = p.attr1;
    let a2 = p.attr2.unwrap_or(0);
    let (kind, value) = match (p.attr3, p.line_spacing) {
        (Some(a3), Some(v)) => (a3 & 0x1F, v as i32),
        _ => (a1 & 3, p.line_spacing_legacy),
    };
    let margins = |packed: bool| {
        let v = |raw: i32, tag: &'static str| {
            let (value, unit) = if packed {
                (raw, "HWPUNIT")
            } else {
                unpack(raw)
            };
            X::new(tag).a("value", value).a("unit", unit)
        };
        X::new("hh:margin")
            .kid(v(p.indent, "hc:intent"))
            .kid(v(p.margin_left, "hc:left"))
            .kid(v(p.margin_right, "hc:right"))
            .kid(v(p.space_before, "hc:prev"))
            .kid(v(p.space_after, "hc:next"))
    };
    let spacing = |packed: bool| {
        let (v, unit) = if kind == 0 || packed {
            (value, "HWPUNIT")
        } else {
            unpack(value)
        };
        X::new("hh:lineSpacing")
            .a("type", name(enums::LINE_SPACING, kind))
            .a("value", v)
            .a("unit", unit)
    };
    X::new("hh:paraPr")
        .a("id", i)
        .a("tabPrIDRef", p.tab_def)
        .a("condense", (a1 >> 9) & 0x7F)
        .b("fontLineHeight", a1 & (1 << 22) != 0)
        .b("snapToGrid", a1 & (1 << 8) != 0)
        .kid(
            X::new("hh:align")
                .a("horizontal", name(enums::H_ALIGN, (a1 >> 2) & 7))
                .a("vertical", name(enums::V_ALIGN, (a1 >> 20) & 3)),
        )
        .kid(
            X::new("hh:heading")
                .a("type", name(enums::HEADING, (a1 >> 23) & 3))
                .a("idRef", p.numbering)
                .a("level", (a1 >> 25) & 7),
        )
        .kid(
            X::new("hh:breakSetting")
                .a("breakLatinWord", name(enums::BREAK_LATIN, (a1 >> 5) & 3))
                .a(
                    "breakNonLatinWord",
                    name(enums::BREAK_NON_LATIN, (a1 >> 7) & 1),
                )
                .b("widowOrphan", a1 & (1 << 16) != 0)
                .b("keepWithNext", a1 & (1 << 17) != 0)
                .b("keepLines", a1 & (1 << 18) != 0)
                .b("pageBreakBefore", a1 & (1 << 19) != 0)
                .a("lineWrap", name(enums::LINE_WRAP, a2 & 3)),
        )
        .kid(
            X::new("hh:autoSpacing")
                .b("eAsianEng", a2 & (1 << 4) != 0)
                .b("eAsianNum", a2 & (1 << 5) != 0),
        )
        .kid(switch(
            X::new("c").kid(margins(false)).kid(spacing(false)),
            X::new("d").kid(margins(true)).kid(spacing(true)),
        ))
        .kid(
            X::new("hh:border")
                .a("borderFillIDRef", p.border_fill)
                .a("offsetLeft", p.border_offsets[0])
                .a("offsetRight", p.border_offsets[1])
                .a("offsetTop", p.border_offsets[2])
                .a("offsetBottom", p.border_offsets[3])
                .b("connect", a1 & (1 << 28) != 0)
                .b("ignoreMargin", a1 & (1 << 29) != 0),
        )
        .with_rest(&p.tail)
}

fn style(s: &Style, i: usize) -> X {
    X::new("hh:style")
        .a("id", i)
        .a("type", name(enums::STYLE_TYPE, (s.attr & 7) as u32))
        .a("name", hstr(&s.local_name))
        .a("engName", hstr(&s.english_name))
        .a("paraPrIDRef", s.para_shape)
        .a("charPrIDRef", s.char_shape)
        .a("nextStyleIDRef", s.next)
        .a("langID", s.language)
        .b("lockForm", s.attr & 4 != 0)
        .with_rest(&s.tail)
}

fn compatible(info: &DocInfo, out: &mut Vec<u8>, parts: &Parts<'_>) {
    let (Some(c), lc) = (&info.compatible, &info.layout_compat) else {
        return;
    };
    let kept = lc.as_ref().is_none_or(|l| l.src.is_some());
    if let Some(m) = c
        .src
        .as_ref()
        .filter(|_| kept)
        .and_then(|s| parts.markup(s))
    {
        out.extend_from_slice(&m);
        return;
    }
    compat_element(c, lc.as_ref()).write(out);
}

fn compat_element(c: &CompatibleDocument, lc: Option<&LayoutCompatibility>) -> X {
    let mut x =
        X::new("hh:compatibleDocument").a("targetProgram", name(enums::TARGET_PROGRAM, c.target));
    if let Some(l) = lc {
        let mut y = X::new("hh:layoutCompatibility");
        for &(n, w, b) in COMPAT_FLAGS {
            if l.flags[w as usize] & (1 << b) != 0 {
                y.kids.push(super::K::E(X {
                    name: leak(format!("hh:{n}")),
                    attrs: Vec::new(),
                    raw_attrs: Vec::new(),
                    kids: Vec::new(),
                }));
            }
        }
        x = x.kid(y.with_rest(&l.tail));
    }
    x.with_rest(&c.tail)
}

/// Element names are static; the compatibility flags are a fixed set, so
/// their qualified names are interned once.
fn leak(s: String) -> &'static str {
    static NAMES: std::sync::OnceLock<std::sync::Mutex<Vec<&'static str>>> =
        std::sync::OnceLock::new();
    let m = NAMES.get_or_init(Default::default);
    let mut v = m.lock().unwrap();
    if let Some(n) = v.iter().find(|n| **n == s) {
        return n;
    }
    let n: &'static str = Box::leak(s.into_boxed_str());
    v.push(n);
    n
}
