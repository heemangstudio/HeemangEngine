//! Drawing placement and geometry, mapped independently between XML dialects.
use crate::node::{Node, local, mapped};
const TAGS: &[(&str, &str)] = &[
    ("PICTURE", "hp:pic"),
    ("RECTANGLE", "hp:rect"),
    ("ELLIPSE", "hp:ellipse"),
    ("ARC", "hp:arc"),
    ("LINE", "hp:line"),
    ("POLYGON", "hp:polygon"),
    ("CURVE", "hp:curve"),
    ("CONTAINER", "hp:container"),
    ("OLE", "hp:ole"),
    ("EQUATION", "hp:equation"),
    ("SIZE", "hp:sz"),
    ("POSITION", "hp:pos"),
    ("OUTSIDEMARGIN", "hp:outMargin"),
    ("SHAPECOMMENT", "hp:shapeComment"),
    ("ROTATIONINFO", "hp:rotationInfo"),
    ("RENDERINGINFO", "hp:renderingInfo"),
    ("TRANSMATRIX", "hc:transMatrix"),
    ("SCAMATRIX", "hc:scaMatrix"),
    ("ROTMATRIX", "hc:rotMatrix"),
    ("LINESHAPE", "hp:lineShape"),
    ("FILLBRUSH", "hc:fillBrush"),
    ("WINDOWBRUSH", "hc:winBrush"),
    ("GRADATION", "hc:gradation"),
    ("COLOR", "hc:color"),
    ("IMAGEBRUSH", "hc:imgBrush"),
    ("SHADOW", "hp:shadow"),
    ("DRAWTEXT", "hp:drawText"),
    ("TEXTMARGIN", "hp:textMargin"),
    ("SCRIPT", "hp:script"),
    ("IMAGERECT", "hp:imgRect"),
    ("IMAGECLIP", "hp:imgClip"),
    ("IMAGE", "hc:img"),
    ("IMAGEDIM", "hp:imgDim"),
    ("EFFECTS", "hp:effects"),
    ("POINT", "hc:pt"),
    ("SEGMENT", "hp:seg"),
];
const ATTRS: &[(&str, &str)] = &[
    ("InstId", "id"),
    ("InstID", "instid"),
    ("BinItem", "binaryItemIDRef"),
    ("HeadSize", "headSz"),
    ("TailSize", "tailSz"),
    ("HasArcProperty", "hasArcPr"),
    ("DimWidth", "dimwidth"),
    ("DimHeight", "dimheight"),
];
const HEADER: &[&str] = &[
    "id",
    "zOrder",
    "numberingType",
    "textWrap",
    "textFlow",
    "lock",
];
const PLACEMENT: &[&str] = &["sz", "pos", "outMargin", "shapeComment"];
pub fn is_hml(n: &str) -> bool {
    TAGS.iter().take(10).any(|(a, _)| *a == n)
}
pub fn is_owpml(n: &str) -> bool {
    TAGS.iter().take(10).any(|(_, b)| local(b) == n)
}
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
fn simple(n: &Node, up: bool) -> Node {
    let mut out = mapped(n, &tag(n, up), ATTRS, up);
    out.children = n
        .children
        .iter()
        .map(|c| {
            if if up {
                is_hml(&c.name)
            } else {
                is_owpml(local(&c.name))
            } {
                return if up { decode(c) } else { encode(c) };
            }
            if TAGS.iter().any(|(a, b)| {
                if up {
                    c.name == *a
                } else {
                    local(&c.name) == local(b)
                }
            }) {
                simple(c, up)
            } else if up {
                crate::body::decode(c)
            } else {
                crate::body::encode(c)
            }
        })
        .collect();
    if n.name == "IMAGE" && n.get("Effect") == Some("RealPic") {
        out.set("effect", "REAL_PIC");
    }
    out
}
pub fn read_header(h: &Node, out: &mut Node) {
    let h = simple(h, true);
    for (k, v) in h.attrs {
        out.set(k, v);
    }
    out.children.extend(h.children.into_iter().map(|c| {
        let mut c = c;
        if c.name == "hp:sz" {
            for k in ["widthRelTo", "heightRelTo"] {
                if c.get(k) == Some("Absolute") {
                    c.set(k, "ABSOLUTE");
                }
            }
        }
        c
    }));
    if out.get("numberingType") == Some("Figure") {
        out.set("numberingType", "PICTURE");
    }
}
pub fn write_header(n: &Node) -> Node {
    let mut out = Node::new("SHAPEOBJECT");
    let mut source = n.clone();
    source.attrs.retain(|(k, _)| HEADER.contains(&k.as_str()));
    let h = mapped(&source, "SHAPEOBJECT", ATTRS, false);
    out.attrs = h.attrs;
    if let Some(v) = out.get("NumberingType") {
        if v == "PICTURE" {
            out.set("NumberingType", "Figure");
        }
    }
    out.children = n
        .children
        .iter()
        .filter(|c| PLACEMENT.contains(&local(&c.name)))
        .map(|c| simple(c, false))
        .collect();
    out
}
pub fn remove_header_attrs(n: &mut Node) {
    n.attrs.retain(|(k, _)| {
        ![
            "InstId",
            "InstID",
            "Id",
            "ZOrder",
            "NumberingType",
            "TextWrap",
            "TextFlow",
            "Lock",
        ]
        .contains(&k.as_str())
    });
}
fn component(c: &Node, out: &mut Node) {
    out.set("groupLevel", c.get("GroupLevel").unwrap_or("0"));
    out.set("instid", c.get("InstID").unwrap_or("0"));
    for (tag, a, b) in [
        ("hp:offset", "XPos", "YPos"),
        ("hp:orgSz", "OriWidth", "OriHeight"),
        ("hp:curSz", "CurWidth", "CurHeight"),
    ] {
        out.children.push(
            Node::new(tag)
                .a(
                    if tag == "hp:offset" { "x" } else { "width" },
                    c.get(a).unwrap_or("0"),
                )
                .a(
                    if tag == "hp:offset" { "y" } else { "height" },
                    c.get(b).unwrap_or("0"),
                ),
        );
    }
    out.children.push(
        Node::new("hp:flip")
            .a("horizontal", c.get("HorzFlip").unwrap_or("false"))
            .a("vertical", c.get("VertFlip").unwrap_or("false")),
    );
    out.children
        .extend(c.children.iter().map(|n| simple(n, true)));
}
fn point(n: &Node, out: &mut Node, prefix: &str, tag: &str) {
    if let (Some(x), Some(y)) = (n.get(&format!("{prefix}X")), n.get(&format!("{prefix}Y"))) {
        out.children.push(Node::new(tag).a("x", x).a("y", y));
    }
}
pub fn decode(n: &Node) -> Node {
    let mut out = mapped(n, &tag(n, true), ATTRS, true);
    for c in &n.children {
        match c.name.as_str() {
            "SHAPEOBJECT" => read_header(c, &mut out),
            "DRAWINGOBJECT" => {
                for c in &c.children {
                    if c.name == "SHAPECOMPONENT" {
                        component(c, &mut out);
                    } else {
                        out.children.push(simple(c, true));
                    }
                }
            }
            "SHAPECOMPONENT" => component(c, &mut out),
            "IMAGERECT" => {
                let mut rect = Node::new("hp:imgRect");
                for i in 0..4 {
                    rect.children.push(
                        Node::new(format!("hc:pt{i}"))
                            .a("x", c.get(&format!("X{i}")).unwrap_or("0"))
                            .a("y", c.get(&format!("Y{i}")).unwrap_or("0")),
                    );
                }
                out.children.push(rect);
            }
            "INSIDEMARGIN" => {
                out.children.push(mapped(c, "hp:inMargin", ATTRS, true));
            }
            _ => out.children.push(simple(c, true)),
        }
    }
    match n.name.as_str() {
        "RECTANGLE" => {
            for i in 0..4 {
                if let (Some(x), Some(y)) = (n.get(&format!("X{i}")), n.get(&format!("Y{i}"))) {
                    out.children
                        .push(Node::new(format!("hc:pt{i}")).a("x", x).a("y", y));
                }
            }
        }
        "LINE" => {
            point(n, &mut out, "Start", "hc:startPt");
            point(n, &mut out, "End", "hc:endPt");
        }
        "ELLIPSE" | "ARC" => {
            for (p, t) in [
                ("Center", "hc:center"),
                ("Axis1", "hc:ax1"),
                ("Axis2", "hc:ax2"),
                ("Start1", "hc:start1"),
                ("End1", "hc:end1"),
                ("Start2", "hc:start2"),
                ("End2", "hc:end2"),
            ] {
                point(n, &mut out, p, t);
            }
        }
        "PICTURE" if !out.children.iter().any(|c| local(&c.name) == "imgDim") => {
            if let Some(sz) = out.children.iter().find(|c| local(&c.name) == "orgSz") {
                out.children.push(
                    Node::new("hp:imgDim")
                        .a("dimwidth", sz.get("width").unwrap_or("0"))
                        .a("dimheight", sz.get("height").unwrap_or("0")),
                );
            }
        }
        _ => {}
    }
    out.attrs.retain(|(k, _)| {
        !((k.starts_with('x') || k.starts_with('y')) && k[1..].parse::<u8>().is_ok())
    });
    out
}
pub fn encode(n: &Node) -> Node {
    let key = local(&n.name);
    let mut out = mapped(n, &tag(n, false), ATTRS, false);
    remove_header_attrs(&mut out);
    out.attrs
        .retain(|(k, _)| !["GroupLevel", "Instid"].contains(&k.as_str()));
    out.children.push(write_header(n));
    let mut comp = Node::new("SHAPECOMPONENT")
        .a("GroupLevel", n.get("groupLevel").unwrap_or("0"))
        .a("InstID", n.get("instid").unwrap_or("0"));
    let mut draw = Node::new("DRAWINGOBJECT");
    for c in &n.children {
        let ck = local(&c.name);
        match ck {
            "offset" => {
                comp.set("XPos", c.get("x").unwrap_or("0"));
                comp.set("YPos", c.get("y").unwrap_or("0"));
            }
            "orgSz" | "curSz" => {
                comp.set(
                    if ck == "orgSz" {
                        "OriWidth"
                    } else {
                        "CurWidth"
                    },
                    c.get("width").unwrap_or("0"),
                );
                comp.set(
                    if ck == "orgSz" {
                        "OriHeight"
                    } else {
                        "CurHeight"
                    },
                    c.get("height").unwrap_or("0"),
                );
            }
            "flip" => {
                comp.set("HorzFlip", c.get("horizontal").unwrap_or("0"));
                comp.set("VertFlip", c.get("vertical").unwrap_or("0"));
            }
            "rotationInfo" | "renderingInfo" => comp.children.push(simple(c, false)),
            "sz" | "pos" | "outMargin" | "shapeComment" => {}
            "lineShape" | "fillBrush" | "drawText" | "shadow" if key != "pic" => {
                draw.children.push(simple(c, false))
            }
            "pt0" | "pt1" | "pt2" | "pt3" if key == "rect" => {
                let i = &ck[2..];
                out.set(format!("X{i}"), c.get("x").unwrap_or("0"));
                out.set(format!("Y{i}"), c.get("y").unwrap_or("0"));
            }
            "startPt" | "endPt" if key == "line" => {
                let p = if ck == "startPt" { "Start" } else { "End" };
                out.set(format!("{p}X"), c.get("x").unwrap_or("0"));
                out.set(format!("{p}Y"), c.get("y").unwrap_or("0"));
            }
            "center" | "ax1" | "ax2" | "start1" | "end1" | "start2" | "end2"
                if key == "ellipse" || key == "arc" =>
            {
                let p = match ck {
                    "center" => "Center",
                    "ax1" => "Axis1",
                    "ax2" => "Axis2",
                    "start1" => "Start1",
                    "end1" => "End1",
                    "start2" => "Start2",
                    _ => "End2",
                };
                out.set(format!("{p}X"), c.get("x").unwrap_or("0"));
                out.set(format!("{p}Y"), c.get("y").unwrap_or("0"));
            }
            "imgRect" => {
                let mut rect = Node::new("IMAGERECT");
                for (i, p) in c.children.iter().enumerate() {
                    rect.set(format!("X{i}"), p.get("x").unwrap_or("0"));
                    rect.set(format!("Y{i}"), p.get("y").unwrap_or("0"));
                }
                out.children.push(rect);
            }
            "inMargin" => out.children.push(mapped(c, "INSIDEMARGIN", ATTRS, false)),
            _ => out.children.push(simple(c, false)),
        }
    }
    if key != "equation" {
        if key == "pic" || key == "ole" || key == "container" {
            out.children.insert(1, comp);
        } else {
            draw.children.insert(0, comp);
            out.children.insert(1, draw);
        }
    }
    out
}
