//! HWPML 2.x single-file XML. Its paragraph and definition vocabularies are
//! translated into OWPML so both XML formats use the same typed model mapping.
mod body;
mod head;
mod node;
mod object;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use hm_hwpx::{
    xml::{El, Ns, Xml},
    zip::{Archive, Member},
};
use hm_model::{Document, Format, SharedBytes};
use node::Node;

pub type Error = hm_hwpx::Error;
const LIMIT: usize = 512 << 20;
const NS: &[(&str, &str)] = &[
    ("hh", "head"),
    ("hp", "paragraph"),
    ("hc", "core"),
    ("hs", "section"),
];
fn namespaced(mut n: Node) -> Node {
    for (prefix, kind) in NS {
        n.set(
            format!("xmlns:{prefix}"),
            format!("http://www.hancom.co.kr/hwpml/2011/{kind}"),
        );
    }
    n
}
fn child<'a>(e: El<'a>, name: &str) -> Option<El<'a>> {
    e.children().find(|e| e.local() == name.as_bytes())
}
fn corrupt(s: impl Into<String>) -> Error {
    Error::Corrupt(s.into())
}

/// Detects the XML root, including UTF-16 BOMs and legacy Korean declarations.
pub fn is_hml(bytes: &[u8]) -> bool {
    let Ok(text) = decode_text(bytes.get(..bytes.len().min(16384)).unwrap_or(bytes), false) else {
        return false;
    };
    text.split("<HWPML")
        .skip(1)
        .any(|s| s.starts_with(|c: char| c.is_ascii_whitespace() || c == '>' || c == '/'))
}
fn decode_text(bytes: &[u8], strict: bool) -> Result<String, Error> {
    if strict && bytes.len() > LIMIT {
        return Err(Error::TooLarge);
    }
    let (encoding, skip) = encoding_rs::Encoding::for_bom(bytes).unwrap_or_else(|| {
        let start = String::from_utf8_lossy(&bytes[..bytes.len().min(300)]).to_ascii_lowercase();
        let declared = start
            .split("encoding")
            .nth(1)
            .and_then(|s| s.split(['\'', '"']).nth(1))
            .and_then(|s| encoding_rs::Encoding::for_label(s.as_bytes()));
        (declared.unwrap_or(encoding_rs::UTF_8), 0)
    });
    let (text, errors) = encoding.decode_without_bom_handling(&bytes[skip..]);
    if errors && strict {
        return Err(Error::Xml("invalid XML character encoding".into()));
    }
    Ok(text.into_owned())
}
/// Reads HWPML 2.x, including embedded and linked BinData resources.
pub fn read(bytes: SharedBytes) -> Result<Document, Error> {
    let text = decode_text(&bytes, true)?;
    let x = Xml::parse(text.into_bytes().into())?;
    let root = x.root();
    if root.local() != b"HWPML" {
        return Err(corrupt("not an HWPML document"));
    }
    let mut pending = vec![(root, 0usize)];
    while let Some((e, depth)) = pending.pop() {
        if depth > 256 {
            return Err(corrupt("XML nesting exceeds 256 elements"));
        }
        pending.extend(e.children().map(|c| (c, depth + 1)));
    }
    let h = child(root, "HEAD").ok_or_else(|| corrupt("HWPML has no HEAD"))?;
    let b = child(root, "BODY").ok_or_else(|| corrupt("HWPML has no BODY"))?;
    let sections: Vec<_> = b.children().filter(|s| s.local() == b"SECTION").collect();
    if sections.is_empty() {
        return Err(corrupt("HWPML has no sections"));
    }
    let mut header = head::read(h);
    header.set("secCnt", sections.len());
    let mut files = vec![
        ("Contents/header.xml".into(), namespaced(header).bytes()),
        (
            "version.xml".into(),
            b"<version major=\"5\" minor=\"1\" xmlVersion=\"1.2\"/>".to_vec(),
        ),
    ];
    let mut manifest = Node::new("opf:manifest").child(
        Node::new("opf:item")
            .a("id", "header")
            .a("href", "Contents/header.xml"),
    );
    let mut spine = Node::new("opf:spine");
    for (i, s) in sections.iter().enumerate() {
        let id = format!("section{i}");
        let path = format!("Contents/{id}.xml");
        files.push((path.clone(), namespaced(body::read(*s)).bytes()));
        manifest
            .children
            .push(Node::new("opf:item").a("id", &id).a("href", path));
        spine.children.push(Node::new("opf:itemref").a("idref", id));
    }
    let storage = child(root, "TAIL").and_then(|t| child(t, "BINDATASTORAGE"));
    let entries = child(h, "MAPPINGTABLE").and_then(|m| child(m, "BINDATALIST"));
    if let Some(entries) = entries {
        let mut seen = std::collections::HashSet::new();
        for item in entries.children() {
            let id = item.attr("Id").unwrap_or_default();
            if id.is_empty() || !seen.insert(id.to_string()) {
                return Err(corrupt("missing or duplicate BinData id"));
            }
            let kind = item.attr("Type").unwrap_or_default();
            let linked = kind.eq_ignore_ascii_case("Link");
            let ext = item.attr("Format").unwrap_or_default();
            if linked {
                manifest.children.push(
                    Node::new("opf:item")
                        .a("id", &id)
                        .a(
                            "href",
                            item.attr("APath")
                                .or_else(|| item.attr("RPath"))
                                .unwrap_or_default(),
                        )
                        .a("isEmbeded", "0"),
                );
                continue;
            }
            let data = storage
                .and_then(|s| {
                    s.children()
                        .find(|c| c.attr("Id").as_deref() == Some(id.as_ref()))
                })
                .ok_or_else(|| corrupt(format!("missing embedded BinData {id}")))?;
            let compact: String = data
                .text()
                .chars()
                .filter(|c| !c.is_ascii_whitespace())
                .collect();
            let mut payload = STANDARD
                .decode(compact)
                .map_err(|e| corrupt(format!("BinData {id}: {e}")))?;
            if data.flag("Compress").unwrap_or(true) {
                payload = miniz_oxide::inflate::decompress_to_vec_with_limit(&payload, LIMIT)
                    .or_else(|_| {
                        miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(&payload, LIMIT)
                    })
                    .map_err(|_| corrupt(format!("invalid compressed BinData {id}")))?;
            }
            if payload.len() > LIMIT {
                return Err(Error::TooLarge);
            }
            let path = format!("BinData/BIN{id}.{ext}");
            manifest.children.push(
                Node::new("opf:item")
                    .a("id", &id)
                    .a("href", &path)
                    .a("isEmbeded", "1"),
            );
            files.push((path, payload));
        }
    }
    let mut metadata = Node::new("HWPML");
    metadata.attrs = root
        .attrs()
        .map(|(_, k, v)| {
            (
                String::from_utf8_lossy(k).into_owned(),
                hm_hwpx::xml::unescape(v).into_owned(),
            )
        })
        .collect();
    if let Some(tail) = child(root, "TAIL") {
        let mut tail = Node::from_el(tail);
        tail.children.retain(|n| n.name != "BINDATASTORAGE");
        metadata.children.push(tail);
    }
    files.push(("Hml/metadata.xml".into(), metadata.bytes()));
    let package = Node::new("opf:package")
        .a("xmlns:opf", "http://www.idpf.org/2007/opf/")
        .child(manifest)
        .child(spine);
    files.push(("Contents/content.hpf".into(), package.bytes()));
    if let Some(c) = child(h, "DOCSETTING").and_then(|s| child(s, "CARETPOS")) {
        files.push((
            "settings.xml".into(),
            Node::new("ha:HWPApplicationSetting")
                .a("xmlns:ha", "http://www.hancom.co.kr/hwpml/2011/app")
                .child(
                    Node::new("ha:CaretPosition")
                        .a("listIDRef", c.attr("List").unwrap_or_default())
                        .a("paraIDRef", c.attr("Para").unwrap_or_default())
                        .a("pos", c.attr("Pos").unwrap_or_default()),
                )
                .bytes(),
        ));
    }
    let package = hm_hwpx::zip::write(
        None,
        &files
            .iter()
            .map(|(name, data)| Member::New {
                name,
                data,
                stored: false,
            })
            .collect::<Vec<_>>(),
    );
    let mut doc = hm_hwpx::read(package.into())?;
    doc.meta.format = Format::Hml;
    Ok(doc)
}
/// Writes the current model as standard HWPML, with base64 embedded resources.
pub fn write(doc: &Document) -> Result<Vec<u8>, Error> {
    let mut output = doc.clone();
    // HML imports retain OWPML spans internally, so edits use the shared writer.
    if output.meta.format == Format::Hml {
        output.meta.format = Format::Hwpx;
    } else {
        output.convert_to(Format::Hwpx);
    }
    let bytes = hm_hwpx::write_new(&output)?;
    let archive = Archive::open(bytes.into())?;
    let header = archive
        .read_name("Contents/header.xml")?
        .ok_or_else(|| corrupt("missing output header"))?;
    let hx = Xml::parse(header)?;
    let packed = archive
        .read_name("version.xml")?
        .and_then(|b| Xml::parse(b).ok())
        .and_then(|x| {
            x.root()
                .attr("xmlVersion")
                .and_then(|v| v.parse::<f64>().ok())
        })
        .is_some_and(|v| v < 1.31);
    let mut h = head::write(hx.root(), packed);
    let manifest = archive
        .read_name("Contents/content.hpf")?
        .ok_or_else(|| corrupt("missing output manifest"))?;
    let mx = Xml::parse(manifest)?;
    let mut storage = Node::new("BINDATASTORAGE");
    let mut bindata = Node::new("BINDATALIST");
    let mut ids = std::collections::HashMap::new();
    if let Some(m) = mx.root().child(Ns::Opf, "manifest") {
        for item in m.children().filter(|n| {
            n.attr("href").is_some_and(|h| h.starts_with("BinData/"))
                || n.flag("isEmbeded") == Some(false)
        }) {
            let id = (ids.len() + 1).to_string();
            let refid = item.attr("id").unwrap_or_default().into_owned();
            let path = item.attr("href").unwrap_or_default();
            ids.insert(refid, id.clone());
            let linked = item.flag("isEmbeded") == Some(false);
            let ext = path.rsplit('.').next().unwrap_or("");
            let mut entry = Node::new("BINITEM")
                .a("Id", &id)
                .a("Type", if linked { "Link" } else { "Embedding" });
            if linked {
                entry.set("APath", &path);
            } else {
                entry.set("BinData", &id);
                entry.set("Format", ext);
                let data = archive
                    .read_name(&path)?
                    .ok_or_else(|| corrupt(format!("missing output BinData {path}")))?;
                let mut payload = Node::new("BINDATA")
                    .a("Id", &id)
                    .a("Size", data.len())
                    .a("Encoding", "Base64")
                    .a("Compress", "false");
                payload.text = STANDARD.encode(&data);
                storage.children.push(payload);
            }
            bindata.children.push(entry);
        }
    }
    bindata.set("Count", bindata.children.len());
    if let Some(mapping) = h.children.iter_mut().find(|n| n.name == "MAPPINGTABLE") {
        mapping.children.insert(0, bindata);
    }
    if let Some(setting) = h.children.iter_mut().find(|n| n.name == "DOCSETTING") {
        let p = &doc.info.properties;
        setting.children.push(
            Node::new("CARETPOS")
                .a("List", p.caret_list)
                .a("Para", p.caret_para)
                .a("Pos", p.caret_pos),
        );
    }
    let mut b = Node::new("BODY");
    for i in 0..doc.sections.len() {
        let bytes = archive
            .read_name(&format!("Contents/section{i}.xml"))?
            .ok_or_else(|| corrupt("missing output section"))?;
        let x = Xml::parse(bytes)?;
        let mut sec = body::write(x.root());
        sec.set("Id", i);
        b.children.push(sec);
    }
    fn refs(n: &mut Node, ids: &std::collections::HashMap<String, String>) {
        if let Some(value) = n.get("BinItem").and_then(|s| ids.get(s)).cloned() {
            n.set("BinItem", value);
        }
        for c in &mut n.children {
            refs(c, ids);
        }
    }
    refs(&mut h, &ids);
    refs(&mut b, &ids);
    let mut root = Node::new("HWPML")
        .a("Version", "2.91")
        .a("SubVersion", "9.0.0.0")
        .a("Style", "embed")
        .child(h)
        .child(b)
        .child(Node::new("TAIL").child(storage));
    if let Some(bytes) = archive.read_name("Hml/metadata.xml")? {
        let x = Xml::parse(bytes)?;
        let source = Node::from_el(x.root());
        for (k, v) in &source.attrs {
            root.set(k, v);
        }
        if let Some(old) = source.find("TAIL") {
            if let Some(tail) = root.children.iter_mut().find(|n| n.name == "TAIL") {
                tail.children.extend(old.children.iter().cloned());
            }
        }
    }
    fn canonical(n: &mut Node) {
        const ENUMS: &[&str] = &[
            "Type",
            "Lang",
            "HeadingType",
            "Align",
            "VerAlign",
            "BreakLatinWord",
            "LineWrap",
            "LineSpacingType",
            "NumberType",
            "PageStartsOn",
            "Layout",
            "PageBreak",
            "GutterType",
            "WidthRelTo",
            "HeightRelTo",
            "VertRelTo",
            "HorzRelTo",
            "VertAlign",
            "HorzAlign",
            "TextFlow",
            "TextWrap",
            "NumberingType",
            "Effect",
            "EndCap",
            "HeadStyle",
            "TailStyle",
            "HeadSize",
            "TailSize",
            "OutlineStyle",
            "Shape",
            "FormatType",
            "FillArea",
            "Place",
            "TargetProgram",
            "ArcType",
        ];
        for (k, v) in &mut n.attrs {
            if ENUMS.contains(&k.as_str())
                && v.chars().any(|c| c.is_ascii_uppercase())
                && !v.chars().any(|c| c.is_ascii_lowercase())
            {
                *v = v
                    .split('_')
                    .map(|s| {
                        let mut chars = s.chars();
                        chars
                            .next()
                            .map(|c| format!("{}{}", c, chars.as_str().to_ascii_lowercase()))
                            .unwrap_or_default()
                    })
                    .collect();
            }
            if n.name == "FONT" && k == "Type" {
                *v = v.to_ascii_lowercase();
            }
        }
        for c in &mut n.children {
            canonical(c);
        }
    }
    canonical(&mut root);
    let mut out = b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>".to_vec();
    root.write(&mut out);
    Ok(out)
}
