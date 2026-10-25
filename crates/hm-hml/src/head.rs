//! HWPML mapping tables share the typed OWPML definitions.
use crate::node::{Node, local, mapped};
use hm_hwpx::xml::El;
const TAGS: &[(&str, &str)] = &[
    ("FACENAMELIST", "hh:fontfaces"),
    ("FONTFACE", "hh:fontface"),
    ("FONT", "hh:font"),
    ("TYPEINFO", "hh:typeInfo"),
    ("SUBSTFONT", "hh:substFont"),
    ("BORDERFILLLIST", "hh:borderFills"),
    ("BORDERFILL", "hh:borderFill"),
    ("LEFTBORDER", "hh:leftBorder"),
    ("RIGHTBORDER", "hh:rightBorder"),
    ("TOPBORDER", "hh:topBorder"),
    ("BOTTOMBORDER", "hh:bottomBorder"),
    ("DIAGONAL", "hh:diagonal"),
    ("FILLBRUSH", "hc:fillBrush"),
    ("WINDOWBRUSH", "hc:winBrush"),
    ("GRADATION", "hc:gradation"),
    ("COLOR", "hc:color"),
    ("IMAGEBRUSH", "hc:imgBrush"),
    ("IMAGE", "hc:img"),
    ("CHARSHAPELIST", "hh:charProperties"),
    ("CHARSHAPE", "hh:charPr"),
    ("FONTID", "hh:fontRef"),
    ("RATIO", "hh:ratio"),
    ("CHARSPACING", "hh:spacing"),
    ("RELSIZE", "hh:relSz"),
    ("CHAROFFSET", "hh:offset"),
    ("ITALIC", "hh:italic"),
    ("BOLD", "hh:bold"),
    ("UNDERLINE", "hh:underline"),
    ("STRIKEOUT", "hh:strikeout"),
    ("OUTLINE", "hh:outline"),
    ("SHADOW", "hh:shadow"),
    ("EMBOSS", "hh:emboss"),
    ("ENGRAVE", "hh:engrave"),
    ("SUPERSCRIPT", "hh:supscript"),
    ("SUBSCRIPT", "hh:subscript"),
    ("TABDEFLIST", "hh:tabProperties"),
    ("TABDEF", "hh:tabPr"),
    ("TABITEM", "hh:tabItem"),
    ("NUMBERINGLIST", "hh:numberings"),
    ("NUMBERING", "hh:numbering"),
    ("PARAHEAD", "hh:paraHead"),
    ("BULLETLIST", "hh:bullets"),
    ("BULLET", "hh:bullet"),
    ("PARASHAPELIST", "hh:paraProperties"),
    ("PARASHAPE", "hh:paraPr"),
    ("PARABORDER", "hh:border"),
    ("STYLELIST", "hh:styles"),
    ("STYLE", "hh:style"),
    ("COMPATIBLEDOCUMENT", "hh:compatibleDocument"),
    ("LAYOUTCOMPATIBILITY", "hh:layoutCompatibility"),
];
const ATTRS: &[(&str, &str)] = &[
    ("Count", "itemCnt"),
    ("Name", "face"),
    ("CharShape", "charPrIDRef"),
    ("ParaShape", "paraPrIDRef"),
    ("NextStyle", "nextStyleIDRef"),
    ("LangId", "langID"),
    ("TabDef", "tabPrIDRef"),
    ("BorderFillId", "borderFillIDRef"),
    ("BorderFill", "borderFillIDRef"),
    ("Alignment", "align"),
    ("BinItem", "binaryItemIDRef"),
    ("Image", "useImage"),
    ("Bright", "bright"),
    ("Pos", "pos"),
];
fn tag(n: &Node, up: bool) -> String {
    TAGS.iter()
        .find(|(a, b)| {
            if up {
                n.name == *a
            } else {
                local(&n.name) == local(b)
            }
        })
        .map(|(a, b)| if up { (*b).into() } else { (*a).into() })
        .unwrap_or_else(|| n.name.clone())
}
pub fn read(head: El<'_>) -> Node {
    let n = Node::from_el(head);
    let mut out = Node::new("hh:head")
        .a("version", "1.2")
        .a("secCnt", n.get("SecCnt").unwrap_or("1"));
    if let Some(s) = n.find("DOCSETTING") {
        if let Some(_begin) = s.find("BEGINNUMBER") {
            let mut begin = Node::new("hh:beginNum");
            for (a, b) in [
                ("Page", "page"),
                ("Footnote", "footnote"),
                ("Endnote", "endnote"),
                ("Picture", "pic"),
                ("Figure", "pic"),
                ("Table", "tbl"),
                ("Equation", "equation"),
            ] {
                if let Some(v) = s.find("BEGINNUMBER").and_then(|n| n.get(a)) {
                    begin.set(b, v);
                }
            }
            out.children.push(begin);
        }
    }
    if let Some(m) = n.find("MAPPINGTABLE") {
        let mut refs = Node::new("hh:refList");
        refs.children = m
            .children
            .iter()
            .filter(|c| c.name != "BINDATALIST")
            .map(decode)
            .collect();
        out.children.push(refs);
    }
    for c in n
        .children
        .iter()
        .filter(|c| !["DOCSETTING", "MAPPINGTABLE"].contains(&c.name.as_str()))
    {
        out.children.push(decode(c));
    }
    out
}
pub fn write(head: El<'_>, packed: bool) -> Node {
    let n = Node::from_el(head);
    let mut out = Node::new("HEAD").a("SecCnt", n.get("secCnt").unwrap_or("1"));
    let mut setting = Node::new("DOCSETTING");
    if let Some(b) = n.children.iter().find(|c| local(&c.name) == "beginNum") {
        let mut begin = Node::new("BEGINNUMBER");
        for (a, dest) in [
            ("page", "Page"),
            ("footnote", "Footnote"),
            ("endnote", "Endnote"),
            ("pic", "Picture"),
            ("tbl", "Table"),
            ("equation", "Equation"),
        ] {
            if let Some(v) = b.get(a) {
                begin.set(dest, v);
            }
        }
        setting.children.push(begin);
    }
    out.children.push(setting);
    if let Some(r) = n.children.iter().find(|c| local(&c.name) == "refList") {
        let mut mapping = Node::new("MAPPINGTABLE");
        mapping.children = r.children.iter().map(|n| encode_inner(n, packed)).collect();
        out.children.push(mapping);
    }
    out.children.extend(
        n.children
            .iter()
            .filter(|c| !["beginNum", "refList"].contains(&local(&c.name)))
            .map(|n| encode_inner(n, packed)),
    );
    out
}
pub fn decode(n: &Node) -> Node {
    let mut out = mapped(n, &tag(n, true), ATTRS, true);
    out.children = n.children.iter().map(decode).collect();
    match n.name.as_str() {
        "FONT" | "SUBSTFONT" => {}
        "STYLE" => {
            out.attrs.retain(|(k, _)| k != "face");
            if let Some(v) = n.get("Name") {
                out.set("name", v);
            }
        }
        "TYPEINFO" => {
            if let Some(v) = n
                .get("FamilyType")
                .and_then(|v| v.parse::<u32>().ok())
                .and_then(|v| hm_hwpx::enums::name(hm_hwpx::enums::FAMILY_TYPE, v))
            {
                out.set("familyType", v);
            }
        }
        "CHARSHAPE" => {
            if let Some(v) = n
                .get("SymMark")
                .and_then(|v| v.parse::<u32>().ok())
                .and_then(|v| hm_hwpx::enums::name(hm_hwpx::enums::SYM_MARK, v))
            {
                out.set("symMark", v);
            }
        }
        "BORDERFILL" => {
            for (a, b) in [("Slash", "hh:slash"), ("BackSlash", "hh:backSlash")] {
                if let Some(v) = n.get(a) {
                    let v = v
                        .parse::<u32>()
                        .ok()
                        .and_then(|i| hm_hwpx::enums::name(hm_hwpx::enums::SLASH, i))
                        .unwrap_or("NONE");
                    out.children.push(
                        Node::new(b)
                            .a("type", v)
                            .a("Crooked", n.get("CrookedSlash").unwrap_or("0"))
                            .a(
                                "isCounter",
                                n.get(if a == "Slash" {
                                    "CounterSlash"
                                } else {
                                    "CounterBackSlash"
                                })
                                .unwrap_or("0"),
                            ),
                    );
                }
            }
        }
        "PARASHAPE" => {
            out.attrs.retain(|(k, _)| {
                [
                    "id",
                    "tabPrIDRef",
                    "condense",
                    "fontLineHeight",
                    "snapToGrid",
                ]
                .contains(&k.as_str())
            });
            out.children.retain(|c| c.name != "PARAMARGIN");
            if n.get("SnapToGrid").is_none() {
                out.set("snapToGrid", "true");
            }
            out.children.push(
                Node::new("hh:align")
                    .a("horizontal", n.get("Align").unwrap_or("Justify"))
                    .a("vertical", n.get("VerAlign").unwrap_or("Baseline")),
            );
            out.children.push(
                Node::new("hh:heading")
                    .a("type", n.get("HeadingType").unwrap_or("None"))
                    .a("level", n.get("Level").unwrap_or("0"))
                    .a(
                        "idRef",
                        n.get("Heading")
                            .or(n.get("HeadingId"))
                            .or(n.get("HeadingIDRef"))
                            .unwrap_or("0"),
                    ),
            );
            let mut br = Node::new("hh:breakSetting");
            for k in [
                "BreakLatinWord",
                "WidowOrphan",
                "KeepWithNext",
                "KeepLines",
                "PageBreakBefore",
                "LineWrap",
            ] {
                if let Some(v) = n.get(k) {
                    let mut it = k.chars();
                    br.set(
                        format!("{}{}", it.next().unwrap().to_ascii_lowercase(), it.as_str()),
                        v,
                    );
                }
            }
            br.set(
                "breakNonLatinWord",
                if n.get("BreakNonLatinWord").unwrap_or("true") == "true" {
                    "BREAK_WORD"
                } else {
                    "KEEP_WORD"
                },
            );
            out.children.push(br);
            out.children.push(
                Node::new("hh:autoSpacing")
                    .a("eAsianEng", n.get("AutoSpaceEAsianEng").unwrap_or("true"))
                    .a("eAsianNum", n.get("AutoSpaceEAsianNum").unwrap_or("true")),
            );
            if n.find("PARAMARGIN").is_none() {
                out.children.push(
                    Node::new("hh:lineSpacing")
                        .a("type", "PERCENT")
                        .a("value", 160),
                );
            }
            if let Some(m) = n.find("PARAMARGIN") {
                let mut margin = Node::new("hh:margin");
                for (a, b) in [
                    ("Indent", "hc:intent"),
                    ("Left", "hc:left"),
                    ("Right", "hc:right"),
                    ("Prev", "hc:prev"),
                    ("Next", "hc:next"),
                ] {
                    margin
                        .children
                        .push(Node::new(b).a("value", m.get(a).unwrap_or("0")));
                }
                out.children.push(margin);
                out.children.push(
                    Node::new("hh:lineSpacing")
                        .a("type", m.get("LineSpacingType").unwrap_or("Percent"))
                        .a("value", m.get("LineSpacing").unwrap_or("160")),
                );
            }
        }
        _ => {}
    }
    out
}
fn encode_inner(n: &Node, packed: bool) -> Node {
    let key = local(&n.name);
    let mut out = mapped(n, &tag(n, false), ATTRS, false);
    out.children = n.children.iter().map(|n| encode_inner(n, packed)).collect();
    match key {
        "style" => {
            out.attrs.retain(|(k, _)| k != "Name");
            if let Some(v) = n.get("name") {
                out.set("Name", v);
            }
        }
        "borderFill" => {
            out.children
                .retain(|c| !["slash", "backSlash"].contains(&local(&c.name)));
            for (a, b) in [("slash", "Slash"), ("backSlash", "BackSlash")] {
                if let Some(c) = n.children.iter().find(|c| local(&c.name) == a) {
                    out.set(
                        b,
                        c.get("type")
                            .and_then(|v| hm_hwpx::enums::code(hm_hwpx::enums::SLASH, v))
                            .unwrap_or(0),
                    );
                }
            }
        }
        "border" => {
            if let Some(v) = n.get("borderFillIDRef") {
                out.attrs.retain(|(k, _)| k != "BorderFillId");
                out.set("BorderFill", v);
            }
        }
        "typeInfo" => {
            if let Some(v) = n
                .get("familyType")
                .and_then(|v| hm_hwpx::enums::code(hm_hwpx::enums::FAMILY_TYPE, v))
            {
                out.set("FamilyType", v);
            }
        }
        "charPr" => {
            if let Some(v) = n
                .get("symMark")
                .and_then(|v| hm_hwpx::enums::code(hm_hwpx::enums::SYM_MARK, v))
            {
                out.set("SymMark", v);
            }
        }
        "tabPr" => {
            out.children.clear();
            for c in &n.children {
                if local(&c.name) == "switch" {
                    if let Some(branch) = c
                        .children
                        .iter()
                        .find(|c| local(&c.name) == "default")
                        .or_else(|| c.children.first())
                    {
                        out.children
                            .extend(branch.children.iter().map(|c| encode_inner(c, true)));
                    }
                } else {
                    out.children.push(encode_inner(c, packed));
                }
            }
        }
        "paraPr" => {
            let mut margin = Node::new("PARAMARGIN");
            let mut children = Vec::new();
            let mut selected = Vec::new();
            for c in &n.children {
                if local(&c.name) == "switch" {
                    if let Some(b) = c
                        .children
                        .iter()
                        .find(|x| local(&x.name) == "default")
                        .or_else(|| c.children.first())
                    {
                        selected
                            .extend(b.children.iter().map(|c| (c, local(&b.name) == "default")));
                    }
                } else {
                    selected.push((c, packed));
                }
            }
            for (c, packed) in selected {
                match local(&c.name) {
                    "align" => {
                        if let Some(v) = c.get("horizontal") {
                            out.set("Align", v);
                        }
                        if let Some(v) = c.get("vertical") {
                            out.set("VerAlign", v);
                        }
                    }
                    "heading" => {
                        for (a, b) in [
                            ("type", "HeadingType"),
                            ("level", "Level"),
                            ("idRef", "Heading"),
                        ] {
                            if let Some(v) = c.get(a) {
                                out.set(b, v);
                            }
                        }
                    }
                    "breakSetting" => {
                        for (k, v) in &c.attrs {
                            let mut it = k.chars();
                            out.set(
                                format!(
                                    "{}{}",
                                    it.next().unwrap().to_ascii_uppercase(),
                                    it.as_str()
                                ),
                                v,
                            );
                        }
                        out.set(
                            "BreakNonLatinWord",
                            if c.get("breakNonLatinWord").and_then(|v| {
                                hm_hwpx::enums::code(hm_hwpx::enums::BREAK_NON_LATIN, v)
                            }) == Some(0)
                            {
                                "true"
                            } else {
                                "false"
                            },
                        );
                    }
                    "autoSpacing" => {
                        out.set("AutoSpaceEAsianEng", c.get("eAsianEng").unwrap_or("0"));
                        out.set("AutoSpaceEAsianNum", c.get("eAsianNum").unwrap_or("0"));
                    }
                    "margin" => {
                        for m in &c.children {
                            let k = match local(&m.name) {
                                "intent" | "indent" => "Indent",
                                "left" => "Left",
                                "right" => "Right",
                                "prev" => "Prev",
                                "next" => "Next",
                                _ => continue,
                            };
                            let raw = m.get("value").unwrap_or("0");
                            let value = if !packed && m.get("unit").is_some() {
                                raw.parse::<i32>().unwrap_or(0).wrapping_shl(1)
                                    | (m.get("unit") == Some("CHAR")) as i32
                            } else {
                                raw.parse().unwrap_or(0)
                            };
                            margin.set(k, value);
                        }
                    }
                    "lineSpacing" => {
                        margin.set("LineSpacingType", c.get("type").unwrap_or("PERCENT"));
                        let raw = c.get("value").unwrap_or("160");
                        let value = if !packed
                            && c.get("unit").is_some()
                            && c.get("type") != Some("PERCENT")
                        {
                            raw.parse::<i32>().unwrap_or(0).wrapping_shl(1)
                                | (c.get("unit") == Some("CHAR")) as i32
                        } else {
                            raw.parse().unwrap_or(160)
                        };
                        margin.set("LineSpacing", value);
                    }
                    _ => children.push(encode_inner(c, packed)),
                }
            }
            if !margin.attrs.is_empty() {
                children.push(margin);
            }
            out.children = children;
        }
        _ => {}
    }
    out
}
