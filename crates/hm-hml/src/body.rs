//! Paragraphs, tables and page furniture in HWPML and OWPML.
use crate::node::{Node, local, mapped};
use hm_hwpx::xml::El;
const TAGS: &[(&str, &str)] = &[
    ("SECTION", "hs:sec"),
    ("P", "hp:p"),
    ("TEXT", "hp:run"),
    ("CHAR", "hp:t"),
    ("PARALIST", "hp:subList"),
    ("SECDEF", "hp:secPr"),
    ("PAGEDEF", "hp:pagePr"),
    ("PAGEMARGIN", "hp:margin"),
    ("STARTNUMBER", "hp:startNum"),
    ("COLDEF", "hp:colPr"),
    ("COLUMN", "hp:colSz"),
    ("COLLINE", "hp:colLine"),
    ("HEADER", "hp:header"),
    ("FOOTER", "hp:footer"),
    ("FOOTNOTE", "hp:footNote"),
    ("ENDNOTE", "hp:endNote"),
    ("FOOTNOTESHAPE", "hp:footNotePr"),
    ("ENDNOTESHAPE", "hp:endNotePr"),
    ("AUTONUMFORMAT", "hp:autoNumFormat"),
    ("NOTELINE", "hp:noteLine"),
    ("NOTESPACING", "hp:noteSpacing"),
    ("NOTENUMBERING", "hp:numbering"),
    ("NOTEPLACEMENT", "hp:placement"),
    ("PAGEBORDERFILL", "hp:pageBorderFill"),
    ("PAGEOFFSET", "hp:offset"),
    ("TABLE", "hp:tbl"),
    ("ROW", "hp:tr"),
    ("CELL", "hp:tc"),
    ("INSIDEMARGIN", "hp:inMargin"),
    ("CELLMARGIN", "hp:cellMargin"),
    ("CAPTION", "hp:caption"),
    ("TAB", "hp:tab"),
    ("LINEBREAK", "hp:lineBreak"),
    ("NBSPACE", "hp:nbSpace"),
    ("FWSPACE", "hp:fwSpace"),
    ("HYPHEN", "hp:hyphen"),
    // The HWPX writer uses Hancom's spelling.
    ("HYPHEN", "hp:hypen"),
    ("AUTONUM", "hp:autoNum"),
    ("NEWNUM", "hp:newNum"),
    ("PAGENUM", "hp:pageNum"),
    ("PAGEHIDING", "hp:pageHiding"),
    ("PAGENUMCTRL", "hp:pageNumCtrl"),
    ("INDEXMARK", "hp:indexmark"),
    ("KEYFIRST", "hp:firstKey"),
    ("KEYSECOND", "hp:secondKey"),
    ("BOOKMARK", "hp:bookmark"),
    ("FIELDEND", "hp:fieldEnd"),
    ("FIELDBEGIN", "hp:fieldBegin"),
    ("DUTMAL", "hp:dutmal"),
    ("COMPOSE", "hp:compose"),
    ("CELLZONE", "hp:cellzone"),
    ("CELLZONELIST", "hp:cellzoneList"),
];
const ATTRS: &[(&str, &str)] = &[
    ("Id", "id"),
    ("InstId", "id"),
    ("ParaShape", "paraPrIDRef"),
    ("Style", "styleIDRef"),
    ("CharShape", "charPrIDRef"),
    ("BorderFill", "borderFillIDRef"),
    ("BorferFill", "borderFillIDRef"),
    ("OutlineShape", "outlineShapeIDRef"),
    ("RowCount", "rowCnt"),
    ("ColCount", "colCnt"),
    ("Count", "colCount"),
    ("Layout", "layout"),
    ("Type", "type"),
    ("Figure", "pic"),
    ("Table", "tbl"),
    ("Superscript", "supscript"),
    ("BetweenNotes", "betweenNotes"),
    ("NewNumber", "newNum"),
    ("ApplyPageType", "applyPageType"),
    ("NumType", "numType"),
    ("NumberType", "numType"),
    ("Number", "number"),
    ("Num", "num"),
];
pub fn read(el: El<'_>) -> Node {
    decode(&Node::from_el(el))
}
pub fn write(el: El<'_>) -> Node {
    encode(&Node::from_el(el))
}
fn name(n: &Node, up: bool) -> String {
    if let Some((a, b)) = TAGS.iter().find(|(a, b)| {
        if up {
            n.name == *a
        } else {
            local(&n.name) == local(b)
        }
    }) {
        return if up { (*b).into() } else { (*a).into() };
    }
    n.name.clone()
}
pub fn decode(n: &Node) -> Node {
    if crate::object::is_hml(&n.name) {
        return crate::object::decode(n);
    }
    let mut out = mapped(n, &name(n, true), ATTRS, true);
    out.children = n.children.iter().map(decode).collect();
    match n.name.as_str() {
        "P" => {
            out.set("id", n.get("InstId").unwrap_or("0"));
        }
        "SECTION" => {
            out.attrs.clear();
        }
        "CHAR" => {
            out.children = n.children.iter().map(decode).collect();
        }
        "TABLE" => {
            out.children = n
                .children
                .iter()
                .filter(|c| c.name != "SHAPEOBJECT")
                .map(decode)
                .collect();
            if let Some(h) = n.find("SHAPEOBJECT") {
                crate::object::read_header(h, &mut out);
            }
        }
        "CELL" => {
            out.attrs.retain(|(k, _)| {
                ![
                    "colAddr", "rowAddr", "colSpan", "rowSpan", "width", "height",
                ]
                .contains(&k.as_str())
            });
            for (tag, keys) in [
                ("hp:cellAddr", vec!["ColAddr", "RowAddr"]),
                ("hp:cellSpan", vec!["ColSpan", "RowSpan"]),
                ("hp:cellSz", vec!["Width", "Height"]),
            ] {
                let mut child = Node::new(tag);
                for k in keys {
                    if let Some(v) = n.get(k) {
                        let mut it = k.chars();
                        child.set(
                            format!("{}{}", it.next().unwrap().to_ascii_lowercase(), it.as_str()),
                            v,
                        );
                    }
                }
                out.children.push(child);
            }
        }
        "SECDEF" => {
            for k in ["charGrid", "lineGrid", "firstBorder", "firstFill"] {
                out.attrs.retain(|(a, _)| a != k);
            }
            out.children.push(
                Node::new("hp:grid")
                    .a("charGrid", n.get("CharGrid").unwrap_or("0"))
                    .a("lineGrid", n.get("LineGrid").unwrap_or("0")),
            );
            if let Some(h) = n.find("HIDE") {
                let mut v = Node::new("hp:visibility");
                for (a, b) in [
                    ("Header", "hideFirstHeader"),
                    ("Footer", "hideFirstFooter"),
                    ("MasterPage", "hideFirstMasterPage"),
                    ("PageNumPos", "hideFirstPageNum"),
                    ("EmptyLine", "hideFirstEmptyLine"),
                ] {
                    if let Some(value) = h.get(a) {
                        v.set(b, value);
                    }
                }
                for (a, b) in [("Border", "border"), ("Fill", "fill")] {
                    v.set(
                        b,
                        if h.get(a) == Some("true") {
                            "HIDE_FIRST"
                        } else {
                            "SHOW_ALL"
                        },
                    );
                }
                for (a, b) in [("FirstBorder", "border"), ("FirstFill", "fill")] {
                    if n.get(a) == Some("true") {
                        v.set(b, "SHOW_FIRST");
                    }
                }
                out.children.retain(|c| c.name != "HIDE");
                out.children.push(v);
            } else {
                out.children.push(
                    Node::new("hp:visibility")
                        .a(
                            "border",
                            if n.get("FirstBorder") == Some("true") {
                                "SHOW_FIRST"
                            } else {
                                "SHOW_ALL"
                            },
                        )
                        .a(
                            "fill",
                            if n.get("FirstFill") == Some("true") {
                                "SHOW_FIRST"
                            } else {
                                "SHOW_ALL"
                            },
                        ),
                );
            }
        }
        "AUTONUM" | "NEWNUM" => {
            out.set("num", n.get("Number").or(n.get("Num")).unwrap_or("1"));
            out.attrs.retain(|(k, _)| k != "number");
            if out.get("numType") == Some("Figure") {
                out.set("numType", "PICTURE");
            }
        }
        "NOTELINE" => {
            // Hancom names its separator length presets.
            if let Some(code) = n.get("Length").and_then(note_line_preset_up) {
                out.set("length", code);
            } else if let Some(v) = n
                .get("Length")
                .and_then(|v| v.strip_suffix("cm"))
                .and_then(|v| v.parse::<f64>().ok())
            {
                out.set("length", (v * 7200.0 / 2.54).round() as i32);
            }
            if let Some(v) = n.get("Type") {
                out.set("type", note_line_up(v));
            }
        }
        "FOOTNOTE" | "ENDNOTE" => {
            // HWPML notes carry no attributes; the reader fills number and
            // mark characters in from the note shapes.
            out.attrs.clear();
        }
        "PAGEDEF" => {
            out.set(
                "landscape",
                if n.get("Landscape") == Some("1") {
                    "WIDELY"
                } else {
                    "NARROWLY"
                },
            );
        }
        "PARALIST" => {
            if n.get("TextDirection") == Some("0") {
                out.set("textDirection", "HORIZONTAL");
            }
        }
        "HEADER" | "FOOTER" => {
            out.set(
                "applyPageType",
                n.get("Type").or(n.get("ApplyPageType")).unwrap_or("BOTH"),
            );
            out.attrs.retain(|(k, _)| k != "type");
        }
        "PAGEBORDERFILL" => {
            out.set(
                "textBorder",
                if n.get("TextBorder") == Some("true") {
                    "CONTENT"
                } else {
                    "PAPER"
                },
            );
        }
        _ => {}
    }
    if n.name == "SECDEF" && n.get("TextDirection") == Some("0") {
        out.set("textDirection", "HORIZONTAL");
    }
    out
}
pub fn encode(n: &Node) -> Node {
    if crate::object::is_owpml(local(&n.name)) {
        return crate::object::encode(n);
    }
    let key = local(&n.name);
    let mut out = mapped(n, &name(n, false), ATTRS, false);
    out.children = n
        .children
        .iter()
        .filter(|c| local(&c.name) != "linesegarray")
        .map(encode)
        .collect();
    match key {
        "p" => {
            out.attrs.retain(|(k, _)| k != "Id");
            out.set("InstId", n.get("id").unwrap_or("0"));
        }
        "run" => {
            out.children.clear();
            for c in &n.children {
                if local(&c.name) == "ctrl" {
                    out.children.extend(c.children.iter().map(encode));
                } else {
                    out.children.push(encode(c));
                }
            }
        }
        "sec" => {
            out.attrs.clear();
        }
        "tbl" => {
            out.children.insert(0, crate::object::write_header(n));
            out.children
                .retain(|c| !["sz", "pos", "outMargin", "shapeComment"].contains(&local(&c.name)));
            crate::object::remove_header_attrs(&mut out);
        }
        "tc" => {
            out.children
                .retain(|c| !["cellAddr", "cellSpan", "cellSz"].contains(&local(&c.name)));
            for c in n
                .children
                .iter()
                .filter(|c| ["cellAddr", "cellSpan", "cellSz"].contains(&local(&c.name)))
            {
                for (k, v) in &c.attrs {
                    let mut it = k.chars();
                    out.set(
                        format!("{}{}", it.next().unwrap().to_ascii_uppercase(), it.as_str()),
                        v,
                    );
                }
            }
        }
        "secPr" => {
            out.children
                .retain(|c| !["grid", "visibility"].contains(&local(&c.name)));
            if let Some(g) = n.children.iter().find(|c| local(&c.name) == "grid") {
                for (a, b) in [("charGrid", "CharGrid"), ("lineGrid", "LineGrid")] {
                    if let Some(v) = g.get(a) {
                        out.set(b, v);
                    }
                }
            }
            if let Some(v) = n.children.iter().find(|c| local(&c.name) == "visibility") {
                let mut h = Node::new("HIDE");
                for (a, b) in [
                    ("hideFirstHeader", "Header"),
                    ("hideFirstFooter", "Footer"),
                    ("hideFirstMasterPage", "MasterPage"),
                    ("hideFirstPageNum", "PageNumPos"),
                    ("hideFirstEmptyLine", "EmptyLine"),
                ] {
                    if let Some(v) = v.get(a) {
                        h.set(b, v);
                    }
                }
                for (a, b, c) in [
                    ("border", "Border", "FirstBorder"),
                    ("fill", "Fill", "FirstFill"),
                ] {
                    h.set(
                        b,
                        if v.get(a) == Some("HIDE_FIRST") {
                            "true"
                        } else {
                            "false"
                        },
                    );
                    out.set(
                        c,
                        if v.get(a) == Some("SHOW_FIRST") {
                            "true"
                        } else {
                            "false"
                        },
                    );
                }
                out.children.push(h);
            }
            if n.get("textDirection") == Some("HORIZONTAL") {
                out.set("TextDirection", "0");
            }
        }
        "footNote" | "endNote" => out.attrs.clear(),
        "autoNumFormat" => {
            out.attrs.retain(|(k, v)| {
                !(matches!(k.as_str(), "UserChar" | "PrefixChar" | "SuffixChar") && v.is_empty())
            });
            if let Some(v) = n.get("supscript") {
                out.set("Superscript", hml_bool(v));
            }
        }
        "noteLine" => {
            if let Some(name) = n.get("length").and_then(note_line_preset_down) {
                out.set("Length", name);
            }
            if let Some(v) = n.get("type") {
                out.set("Type", note_line_down(v));
            }
            if let Some(v) = n.get("width") {
                out.set("Width", v.replace(' ', ""));
            }
            out.attrs.retain(|(k, v)| !(k == "Color" && v == "0"));
        }
        "placement" => {
            if let Some(v) = n.get("beneathText") {
                out.set("BeneathText", hml_bool(v));
            }
        }
        "endNotePr" => {
            // An endnote shape restarting on every page has no OWPML type;
            // Hancom's HWPML names it.
            for c in &mut out.children {
                if c.name == "NOTENUMBERING" && c.get("Type").is_none() {
                    c.set("Type", "OnPage");
                }
            }
        }
        "autoNum" | "newNum" => {
            out.attrs.retain(|(k, _)| k != "Num");
            out.set("Number", n.get("num").unwrap_or("1"));
            if out.get("NumberType") == Some("PICTURE") || out.get("NumType") == Some("PICTURE") {
                out.set("NumberType", "Figure");
            } else if let Some(v) = n.get("numType") {
                out.set("NumberType", v);
            }
            out.attrs.retain(|(k, _)| k != "NumType");
        }
        "pagePr" => {
            out.set(
                "Landscape",
                if n.get("landscape") == Some("WIDELY") {
                    1
                } else {
                    0
                },
            );
        }
        "subList" => {
            if n.get("textDirection") == Some("HORIZONTAL") {
                out.set("TextDirection", "0");
            }
        }
        "header" | "footer" => {
            out.attrs.retain(|(k, _)| k != "ApplyPageType");
            out.set("ApplyPageType", n.get("applyPageType").unwrap_or("BOTH"));
        }
        "pageBorderFill" => {
            out.set(
                "TextBorder",
                if n.get("textBorder") == Some("CONTENT") {
                    "true"
                } else {
                    "false"
                },
            );
        }
        _ => {}
    }
    out
}

/// Separator length presets as Hancom's HWPML names them: 5 cm, 2 cm, a
/// third of the column and the column width.
const NOTE_LINE_PRESETS: [(&str, &str); 4] = [
    ("5cm", "-1"),
    ("2cm", "-2"),
    ("Column/3", "-3"),
    ("Column", "-4"),
];
fn note_line_preset_up(v: &str) -> Option<&'static str> {
    NOTE_LINE_PRESETS
        .iter()
        .find(|(name, _)| *name == v)
        .map(|(_, code)| *code)
}
fn note_line_preset_down(v: &str) -> Option<&'static str> {
    NOTE_LINE_PRESETS
        .iter()
        .find(|(_, code)| *code == v)
        .map(|(name, _)| *name)
}
/// HWPML and OWPML name line kinds 2 and 3 the other way round.
fn note_line_up(v: &str) -> &str {
    match v {
        "Dash" => "DOT",
        "Dot" => "DASH",
        v => v,
    }
}
fn note_line_down(v: &str) -> &str {
    match v {
        "DOT" => "Dash",
        "DASH" => "Dot",
        v => v,
    }
}
fn hml_bool(v: &str) -> &'static str {
    if matches!(v, "1" | "true") {
        "true"
    } else {
        "false"
    }
}
