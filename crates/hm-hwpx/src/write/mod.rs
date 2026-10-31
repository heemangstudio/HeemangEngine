//! Writing HWPX: the package, and an element builder the part writers share.
//!
//! A clean document is its source file. Otherwise every package member is
//! copied as stored except `header.xml` and the sections that changed,
//! which are written from the model: clean nodes copy their source markup,
//! changed ones are encoded with their leftover markup (see
//! [`crate::rest`]) put back in place.

mod body;
mod head;
mod object;

pub(crate) use body::{Ctx as Cx, list as sub_list};

/// `hp:sz`, `hp:pos` and `hp:outMargin` appended to an object element.
pub(crate) fn object_placement(x: X, h: &hm_model::ObjectHeader) -> X {
    x.kids(object::placement(h))
}

use hm_model::{Document, SharedBytes};

use crate::Error;
use crate::origin::Parts;
use crate::rest;
use crate::xml::{Xml, escape_attr, escape_text};
use crate::zip::Member;

/// An element under construction.
#[derive(Clone, Debug)]
pub struct X {
    pub name: &'static str,
    pub attrs: Vec<(&'static str, String)>,
    /// Attributes as stored (`name="value"`), from leftovers.
    pub raw_attrs: Vec<Vec<u8>>,
    pub kids: Vec<K>,
}

#[derive(Clone, Debug)]
pub enum K {
    E(X),
    /// Markup copied as is.
    Raw(SharedBytes),
    Text(String),
}

impl X {
    pub fn new(name: &'static str) -> X {
        X {
            name,
            attrs: Vec::new(),
            raw_attrs: Vec::new(),
            kids: Vec::new(),
        }
    }

    /// Adds an attribute.
    pub fn a(mut self, k: &'static str, v: impl std::fmt::Display) -> X {
        self.attrs.push((k, v.to_string()));
        self
    }

    /// Adds a boolean attribute as `0` or `1`.
    pub fn b(self, k: &'static str, v: bool) -> X {
        self.a(k, v as u8)
    }

    pub fn kid(mut self, x: X) -> X {
        self.kids.push(K::E(x));
        self
    }

    pub fn kids(mut self, xs: impl IntoIterator<Item = X>) -> X {
        self.kids.extend(xs.into_iter().map(K::E));
        self
    }

    pub fn text(mut self, t: impl Into<String>) -> X {
        self.kids.push(K::Text(t.into()));
        self
    }

    fn local(&self) -> &str {
        self.name.rsplit(':').next().unwrap_or(self.name)
    }

    /// The child element at a leftover path segment (`name` or `name:i`).
    fn find_mut(&mut self, seg: &str) -> Option<&mut X> {
        let (name, index) = match seg.split_once(':') {
            Some((n, i)) => (n, i.parse().unwrap_or(0)),
            None => (seg, 0usize),
        };
        self.kids
            .iter_mut()
            .filter_map(|k| match k {
                K::E(x) if x.local() == name => Some(x),
                _ => None,
            })
            .nth(index)
    }

    /// Puts leftover markup back: attributes on the element at their path,
    /// child elements at their slot among the modeled children.
    pub fn with_rest(mut self, tail: &[u8]) -> X {
        // Pieces go in recorded order: each lands just before the modeled
        // child it preceded, after earlier pieces with the same slot.
        for p in rest::pieces(tail) {
            let Some(x) = self.at_path(p.path) else {
                continue;
            };
            if p.kind_attr {
                x.raw_attrs.push(p.bytes.to_vec());
                continue;
            }
            let at = x
                .kids
                .iter()
                .enumerate()
                .filter(|(_, k)| matches!(k, K::E(_)))
                .map(|(i, _)| i)
                .nth(p.slot as usize)
                .unwrap_or(x.kids.len());
            x.kids
                .insert(at, K::Raw(SharedBytes::from(p.bytes.to_vec())));
        }
        self
    }

    fn at_path(&mut self, path: &str) -> Option<&mut X> {
        let mut x = self;
        if path.is_empty() {
            return Some(x);
        }
        for seg in path.split('/') {
            x = x.find_mut(seg)?;
        }
        Some(x)
    }

    pub fn write(&self, out: &mut Vec<u8>) {
        out.push(b'<');
        out.extend_from_slice(self.name.as_bytes());
        for (k, v) in &self.attrs {
            out.push(b' ');
            out.extend_from_slice(k.as_bytes());
            out.extend_from_slice(b"=\"");
            escape_attr(out, v);
            out.push(b'"');
        }
        for r in &self.raw_attrs {
            out.push(b' ');
            out.extend_from_slice(r);
        }
        if self.kids.is_empty() {
            out.extend_from_slice(b"/>");
            return;
        }
        out.push(b'>');
        for k in &self.kids {
            match k {
                K::E(x) => x.write(out),
                K::Raw(b) => out.extend_from_slice(b),
                K::Text(t) => escape_text(out, t),
            }
        }
        out.extend_from_slice(b"</");
        out.extend_from_slice(self.name.as_bytes());
        out.push(b'>');
    }
}

/// `#RRGGBB` from a COLORREF; all ones is `none`, and a nonzero top byte
/// gives `#AARRGGBB`.
pub fn color(c: u32) -> String {
    if c == 0xFFFF_FFFF {
        return "none".into();
    }
    let (r, g, b, a) = (c & 0xFF, (c >> 8) & 0xFF, (c >> 16) & 0xFF, c >> 24);
    if a != 0 {
        format!("#{a:02X}{r:02X}{g:02X}{b:02X}")
    } else {
        format!("#{r:02X}{g:02X}{b:02X}")
    }
}

/// The name of a code, or the number when the table has none.
pub fn name(t: crate::enums::Table, code: u32) -> String {
    crate::enums::name(t, code).map_or_else(|| code.to_string(), str::to_string)
}

pub const PROLOG: &[u8] = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\" ?>";

/// Namespace declarations Hancom puts on every part root.
pub const NAMESPACES: &str = concat!(
    " xmlns:ha=\"http://www.hancom.co.kr/hwpml/2011/app\"",
    " xmlns:hp=\"http://www.hancom.co.kr/hwpml/2011/paragraph\"",
    " xmlns:hp10=\"http://www.hancom.co.kr/hwpml/2016/paragraph\"",
    " xmlns:hs=\"http://www.hancom.co.kr/hwpml/2011/section\"",
    " xmlns:hc=\"http://www.hancom.co.kr/hwpml/2011/core\"",
    " xmlns:hh=\"http://www.hancom.co.kr/hwpml/2011/head\"",
    " xmlns:hhs=\"http://www.hancom.co.kr/hwpml/2011/history\"",
    " xmlns:hm=\"http://www.hancom.co.kr/hwpml/2011/master-page\"",
    " xmlns:hpf=\"http://www.hancom.co.kr/schema/2011/hpf\"",
    " xmlns:dc=\"http://purl.org/dc/elements/1.1/\"",
    " xmlns:opf=\"http://www.idpf.org/2007/opf/\"",
    " xmlns:ooxmlchart=\"http://www.hancom.co.kr/hwpml/2016/ooxmlchart\"",
    " xmlns:hwpunitchar=\"http://www.hancom.co.kr/hwpml/2016/HwpUnitChar\"",
    " xmlns:epub=\"http://www.idpf.org/2007/ops\"",
    " xmlns:config=\"urn:oasis:names:tc:opendocument:xmlns:config:1.0\"",
);

/// Bytes before and the start tag of a source part's root, so a rewritten
/// part keeps its declarations and prefixes (copied markup depends on them).
pub struct Root {
    pub prolog: Vec<u8>,
    pub start: Vec<u8>,
}

impl Root {
    pub fn from_source(x: &Xml) -> Root {
        let r = x.root();
        let start = r.start_tag().to_vec();
        let prolog = x.src[..r.span().start].to_vec();
        // A self-closed root gets its end tag from the writer.
        let mut start = match start.strip_suffix(b"/>") {
            Some(s) => [s, b">"].concat(),
            None => start,
        };
        // Encoded elements use Hancom's prefixes; a root from another
        // writer may not declare them all.
        for decl in NAMESPACES.split(' ').filter(|d| !d.is_empty()) {
            let name = decl.split('=').next().unwrap_or(decl);
            let key = format!(" {name}=");
            if !start.windows(key.len()).any(|w| w == key.as_bytes()) {
                let at = start.len() - 1;
                start.splice(at..at, format!(" {decl}").into_bytes());
            }
        }
        Root { prolog, start }
    }

    pub fn fresh(name: &str, extra: &str) -> Root {
        Root {
            prolog: PROLOG.to_vec(),
            start: format!("<{name}{NAMESPACES}{extra}>").into_bytes(),
        }
    }

    /// Replaces the value of attribute `k` in the start tag, or adds it.
    pub fn set_attr(&mut self, k: &str, v: &str) {
        let s = String::from_utf8_lossy(&self.start).into_owned();
        let key = format!(" {k}=\"");
        let out = match s.find(&key) {
            Some(i) => {
                let vs = i + key.len();
                let ve = s[vs..].find('"').map_or(s.len(), |e| vs + e);
                format!("{}{}{}", &s[..vs], v, &s[ve..])
            }
            None => {
                let at = s.len() - 1;
                format!("{} {k}=\"{v}\"{}", &s[..at], &s[at..])
            }
        };
        self.start = out.into_bytes();
    }

    pub fn end_tag(&self) -> Vec<u8> {
        let s = &self.start[1..];
        let end = s
            .iter()
            .position(|c| c.is_ascii_whitespace() || *c == b'>' || *c == b'/')
            .unwrap_or(s.len());
        [b"</", &s[..end], b">"].concat()
    }
}

/// Writes a document as HWPX.
pub fn write(doc: &Document) -> Result<Vec<u8>, Error> {
    let source = doc
        .source
        .as_ref()
        .filter(|s| crate::zip::is_zip(s))
        .ok_or_else(|| Error::Unsupported("saving needs an HWPX source package".into()))?;
    if doc.is_clean() {
        return Ok(source.to_vec());
    }
    let mut pkg = crate::read::package(source.clone())?;
    if pkg.sections.len() > doc.sections.len()
        && let Some(i) = pkg.archive.find(&pkg.header)
    {
        // Reading skipped the spine sections past the header's count.
        let x = Xml::parse(pkg.archive.read(i)?)?;
        pkg.limit_sections(x.root());
    }
    let archive = &pkg.archive;
    let reshaped = pkg.sections.len() != doc.sections.len();
    let mut section_names = pkg
        .sections
        .iter()
        .take(doc.sections.len())
        .cloned()
        .collect::<Vec<_>>();
    for k in section_names.len()..doc.sections.len() {
        let mut n = 0;
        let name = loop {
            let name = format!("Contents/section-added{k}-{n}.xml");
            if pkg.archive.find(&name).is_none() {
                break name;
            }
            n += 1;
        };
        section_names.push(name);
    }
    let bins = head::bin_ids(doc);
    let parts = Parts::new(archive);
    let mut replaced: Vec<(String, Vec<u8>)> = Vec::new();
    if !doc.info.is_clean() {
        let i = archive
            .find(&pkg.header)
            .ok_or_else(|| Error::Corrupt("missing header.xml".into()))?;
        let x = Xml::parse(archive.read(i)?)?;
        parts.seed(i as u32, x.src.clone());
        let packed_paragraphs = crate::read::version(archive).1.packed_paragraph_values();
        replaced.push((
            pkg.header.clone(),
            head::header(doc, &x, &bins, &parts, packed_paragraphs),
        ));
    }
    for (k, s) in doc.sections.iter().enumerate() {
        if s.is_clean() && s.source_index == Some(k as u32) {
            continue;
        }
        let name = &section_names[k];
        let x = if let Some(i) = archive.find(name) {
            let x = Xml::parse(archive.read(i)?)?;
            parts.seed(i as u32, x.src.clone());
            x
        } else {
            Xml::parse(SharedBytes::from(&b"<empty/>"[..]))?
        };
        replaced.push((name.clone(), body::section(s, &x, &bins, &parts, None)?));
    }
    for s in &doc.sections {
        let Some(name) = s.source_index.and_then(|i| pkg.sections.get(i as usize)) else {
            continue;
        };
        replaced.extend(master_parts(s, name, &pkg, &bins, &parts)?);
    }
    // New images need both bytes and manifest entries. Existing members remain
    // copied exactly, including unknown package parts.
    let mut image_items = Vec::new();
    for (k, e) in doc.info.bin_data.iter().enumerate() {
        if pkg.items.iter().any(|i| i.id == bins[k]) {
            continue;
        }
        if e.kind() == 0 {
            image_items
                .push(linked_item(&bins[k], e).a("xmlns:opf", "http://www.idpf.org/2007/opf/"));
            continue;
        }
        let ext = e
            .extension
            .as_ref()
            .map(|s| s.to_string_lossy())
            .unwrap_or_default();
        let href = format!("BinData/{}.{ext}", bins[k]);
        let data = doc
            .bins
            .as_ref()
            .and_then(|b| b.get(k as u16 + 1))
            .ok_or_else(|| Error::Corrupt(format!("missing embedded bytes for {href}")))?;
        let mut tail = e.tail.clone();
        for name in ["id", "href", "media-type", "isEmbeded"] {
            tail = crate::rest::without_attr(&tail, "", name);
        }
        image_items.push(
            X::new("opf:item")
                .a("xmlns:opf", "http://www.idpf.org/2007/opf/")
                .a("id", &bins[k])
                .a("href", &href)
                .a("media-type", media_type(&ext))
                .a("isEmbeded", 1)
                .with_rest(&tail),
        );
        replaced.push((href, data.to_vec()));
    }
    if reshaped || !image_items.is_empty() {
        let path = crate::read::rootfile(archive)?;
        let src = archive
            .read_name(&path)?
            .ok_or_else(|| Error::Corrupt("missing package manifest".into()))?;
        let xml = Xml::parse(src.clone())?;
        let old_ids = pkg
            .items
            .iter()
            .filter(|i| pkg.sections.contains(&i.href))
            .map(|i| i.id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        let mut section_ids = Vec::new();
        for (k, name) in section_names.iter().enumerate() {
            let id = if let Some(item) = pkg.items.iter().find(|i| &i.href == name) {
                item.id.clone()
            } else {
                let mut n = 0;
                loop {
                    let id = format!("section-added{k}-{n}");
                    if !pkg.items.iter().any(|i| i.id == id) {
                        break id;
                    }
                    n += 1;
                }
            };
            section_ids.push(id);
        }
        let mut changes = Vec::new();
        let manifest = xml
            .root()
            .child(crate::xml::Ns::Opf, "manifest")
            .ok_or_else(|| Error::Corrupt("missing manifest element".into()))?;
        let mut content = Vec::new();
        for child in manifest.children() {
            let id = child.attr("id").unwrap_or_default();
            if !reshaped || !old_ids.contains(id.as_ref()) {
                content.extend_from_slice(&child.source());
            }
        }
        if reshaped {
            for (id, name) in section_ids.iter().zip(&section_names) {
                X::new("opf:item")
                    .a("xmlns:opf", "http://www.idpf.org/2007/opf/")
                    .a("id", id)
                    .a("href", name)
                    .a("media-type", "application/xml")
                    .write(&mut content);
            }
        }
        for item in image_items {
            item.write(&mut content);
        }
        changes.push((manifest.span(), element_content(manifest, &content)));
        if reshaped {
            let spine = xml
                .root()
                .child(crate::xml::Ns::Opf, "spine")
                .ok_or_else(|| Error::Corrupt("missing spine element".into()))?;
            let mut content = Vec::new();
            for child in spine.children() {
                let id = child.attr("idref").unwrap_or_default();
                if !old_ids.contains(id.as_ref()) {
                    content.extend_from_slice(&child.source());
                }
            }
            for id in &section_ids {
                X::new("opf:itemref")
                    .a("xmlns:opf", "http://www.idpf.org/2007/opf/")
                    .a("idref", id)
                    .write(&mut content);
            }
            changes.push((spine.span(), element_content(spine, &content)));
        }
        changes.sort_by_key(|(span, _)| std::cmp::Reverse(span.start));
        let mut out = src.to_vec();
        for (span, bytes) in changes {
            out.splice(span, bytes);
        }
        replaced.push((path, out));
    }
    let mut members: Vec<Member<'_>> = archive
        .entries
        .iter()
        .enumerate()
        .filter(|(_, e)| {
            !reshaped || !pkg.sections.contains(&e.name) || section_names.contains(&e.name)
        })
        .map(|(i, e)| match replaced.iter().find(|(n, _)| *n == e.name) {
            Some((n, data)) => Member::New {
                name: n,
                data,
                stored: false,
            },
            None => Member::Copy(i),
        })
        .collect();
    members.extend(
        replaced
            .iter()
            .filter(|(name, _)| archive.find(name).is_none())
            .map(|(name, data)| Member::New {
                name,
                data,
                stored: false,
            }),
    );
    Ok(crate::zip::write(Some(archive), &members))
}

/// Master pages live in parts of their own, named by the section's
/// `hp:masterPage` references. A part whose list an edit changed is written
/// again from that list, its root and the rest of its markup kept.
fn master_parts(
    s: &hm_model::Section,
    section_part: &str,
    pkg: &crate::read::Package,
    bins: &[String],
    parts: &Parts<'_>,
) -> Result<Vec<(String, Vec<u8>)>, Error> {
    use crate::xml::Ns;
    let lists = s.definition().map(|d| d.master_pages()).unwrap_or_default();
    if lists.is_empty() {
        return Ok(Vec::new());
    }
    let archive = &pkg.archive;
    let Some(i) = archive.find(section_part) else {
        return Ok(Vec::new());
    };
    let x = Xml::parse(archive.read(i)?)?;
    // The section definition sits in a run of the first paragraph.
    let Some(first) = x.root().child(Ns::Hp, "p") else {
        return Ok(Vec::new());
    };
    let Some(sec_pr) = first
        .children()
        .filter(|r| r.is(Ns::Hp, "run"))
        .find_map(|r| r.child(Ns::Hp, "secPr"))
    else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for r in sec_pr.children().filter(|c| c.is(Ns::Hp, "masterPage")) {
        let id = r.attr("idRef").unwrap_or_default();
        let Some(item) = pkg.items.iter().find(|it| it.id == id) else {
            continue;
        };
        let Some(pi) = archive.find(&item.href) else {
            continue;
        };
        let px = Xml::parse(archive.read(pi)?)?;
        parts.seed(pi as u32, px.src.clone());
        let root = px.root();
        let side = match root.attr("type").as_deref() {
            Some("BOTH") => hm_model::PageSide::Both,
            Some("EVEN") => hm_model::PageSide::Even,
            Some("ODD") => hm_model::PageSide::Odd,
            // Last-page and optional master pages are not edited.
            _ => continue,
        };
        let Some(&(_, list)) = lists.iter().find(|(sd, _)| *sd == side) else {
            continue;
        };
        let Some(sub) = root.child(Ns::Hp, "subList") else {
            continue;
        };
        let stored = sub.children().filter(|c| c.is(Ns::Hp, "p")).count();
        if stored == list.paras.len() && list.paras.iter().all(|p| p.is_clean()) {
            continue;
        }
        let mut cx = Cx {
            bins,
            fields: Vec::new(),
            parts,
            masters: None,
            extended_masters: Vec::new(),
        };
        let start = Root::from_source(&px);
        let mut part = start.prolog.clone();
        part.extend_from_slice(&start.start);
        for c in root.children() {
            if c.span() == sub.span() {
                sub_list(list, &mut cx).write(&mut part);
            } else {
                part.extend_from_slice(&c.source());
            }
        }
        part.extend_from_slice(&start.end_tag());
        out.push((item.href.clone(), part));
    }
    Ok(out)
}

fn element_content(el: crate::xml::El<'_>, content: &[u8]) -> Vec<u8> {
    let tag = el.start_tag();
    let mut out = tag
        .strip_suffix(b"/>")
        .map_or_else(|| tag.to_vec(), |t| [t, b">"].concat());
    out.extend_from_slice(content);
    let name = tag[1..]
        .split(|b| b.is_ascii_whitespace() || *b == b'>' || *b == b'/')
        .next()
        .unwrap_or(b"element");
    out.extend_from_slice(b"</");
    out.extend_from_slice(name);
    out.push(b'>');
    out
}

/// A linked picture's manifest item. Sections name it like an embedded
/// one; Hancom gives its file path and marks it not embedded.
fn linked_item(id: &str, e: &hm_model::BinDataEntry) -> X {
    let path = e
        .link_absolute
        .as_ref()
        .or(e.link_relative.as_ref())
        .map(|s| s.to_string_lossy())
        .unwrap_or_default();
    let ext = path
        .rsplit_once('.')
        .map(|(_, x)| x.to_ascii_lowercase())
        .filter(|x| !x.contains(['\\', '/']))
        .unwrap_or_default();
    X::new("opf:item")
        .a("id", id)
        .a("href", path)
        .a("media-type", format!("image/{ext}"))
        .a("isEmbeded", 0)
}

/// Media type of a BinData item by its extension.
fn media_type(ext: &str) -> String {
    match ext {
        "jpg" | "jpeg" => "image/jpg".into(),
        "png" | "gif" | "bmp" | "tiff" | "tif" => format!("image/{ext}"),
        "wmf" => "image/x-wmf".into(),
        "emf" => "image/x-emf".into(),
        "ole" => "application/ole".into(),
        _ => "application/octet-stream".into(),
    }
}

/// Writes a document read from another format (or built from nothing) as
/// a new HWPX package: header, sections, master pages, the embedded
/// BinData and the container parts Hancom expects. Call
/// [`hm_model::Document::convert_to`] first.
pub fn write_new(doc: &Document) -> Result<Vec<u8>, Error> {
    if doc.source.is_some() && doc.meta.format == hm_model::Format::Hwpx {
        return write(doc);
    }
    let bins = head::bin_ids(doc);
    let parts = Parts::none();
    let empty = Xml::parse(SharedBytes::from(&b"<empty/>"[..]))?;
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    files.push((
        "Contents/header.xml".into(),
        head::header(doc, &empty, &bins, &parts, false),
    ));
    let mut masters = Vec::new();
    for (k, s) in doc.sections.iter().enumerate() {
        let sec = body::section(s, &empty, &bins, &parts, Some(&mut masters))?;
        files.push((format!("Contents/section{k}.xml"), sec));
    }
    let mut items = String::from(
        r#"<opf:item id="header" href="Contents/header.xml" media-type="application/xml"/>"#,
    );
    let mut bin_files: Vec<(String, Vec<u8>)> = Vec::new();
    for (k, e) in doc.info.bin_data.iter().enumerate() {
        if e.kind() == 0 {
            let mut b = Vec::new();
            linked_item(&bins[k], e).write(&mut b);
            items.push_str(&String::from_utf8_lossy(&b));
            continue;
        }
        let ext = e
            .extension
            .as_ref()
            .map(|x| x.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        let Some(data) = doc.bins.as_ref().and_then(|b| b.get(k as u16 + 1)) else {
            continue;
        };
        let id = &bins[k];
        let href = format!("BinData/{id}.{ext}");
        items.push_str(&format!(
            r#"<opf:item id="{id}" href="{href}" media-type="{}" isEmbeded="1"/>"#,
            media_type(&ext)
        ));
        bin_files.push((href, data.to_vec()));
    }
    for k in 0..doc.sections.len() {
        items.push_str(&format!(
            r#"<opf:item id="section{k}" href="Contents/section{k}.xml" media-type="application/xml"/>"#
        ));
    }
    for (id, part) in &masters {
        items.push_str(&format!(
            r#"<opf:item id="{id}" href="Contents/{id}.xml" media-type="application/xml"/>"#
        ));
        files.push((format!("Contents/{id}.xml"), part.clone()));
    }
    items.push_str(r#"<opf:item id="settings" href="settings.xml" media-type="application/xml"/>"#);
    let spine: String = std::iter::once("header".to_string())
        .chain((0..doc.sections.len()).map(|k| format!("section{k}")))
        .map(|id| format!(r#"<opf:itemref idref="{id}" linear="yes"/>"#))
        .collect();
    let prolog = std::str::from_utf8(PROLOG).unwrap_or("");
    let hpf = format!(
        concat!(
            r#"{}<opf:package{} version="" unique-identifier="" id="">"#,
            r#"<opf:metadata><opf:title/><opf:language>ko</opf:language></opf:metadata>"#,
            r#"<opf:manifest>{}</opf:manifest><opf:spine>{}</opf:spine></opf:package>"#
        ),
        prolog, NAMESPACES, items, spine
    );
    let [major, minor, micro, build] = if doc.meta.version.parts()[0] == 5 {
        doc.meta.version.parts()
    } else {
        [5, 1, 0, 1]
    };
    let version = format!(
        concat!(
            r#"{}<hv:HCFVersion xmlns:hv="http://www.hancom.co.kr/hwpml/2011/version" "#,
            r#"tagetApplication="WORDPROCESSOR" major="{}" minor="{}" micro="{}" buildNumber="{}" "#,
            r#"os="1" xmlVersion="1.4" application="Hancom Office Hangul" appVersion="11, 0, 0, 0"/>"#
        ),
        prolog, major, minor, micro, build
    );
    let p = &doc.info.properties;
    let settings = format!(
        concat!(
            r#"{}<ha:HWPApplicationSetting xmlns:ha="http://www.hancom.co.kr/hwpml/2011/app" "#,
            r#"xmlns:config="urn:oasis:names:tc:opendocument:xmlns:config:1.0">"#,
            r#"<ha:CaretPosition listIDRef="{}" paraIDRef="{}" pos="{}"/></ha:HWPApplicationSetting>"#
        ),
        prolog, p.caret_list, p.caret_para, p.caret_pos
    );
    let container = format!(
        concat!(
            r#"{}<ocf:container xmlns:ocf="urn:oasis:names:tc:opendocument:xmlns:container" "#,
            r#"xmlns:hpf="http://www.hancom.co.kr/schema/2011/hpf"><ocf:rootfiles>"#,
            r#"<ocf:rootfile full-path="Contents/content.hpf" media-type="application/hwpml-package+xml"/>"#,
            r#"</ocf:rootfiles></ocf:container>"#
        ),
        prolog
    );
    let manifest = format!(
        r#"{prolog}<odf:manifest xmlns:odf="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0"/>"#
    );
    let mut all: Vec<(String, Vec<u8>, bool)> = vec![
        ("mimetype".into(), b"application/hwp+zip".to_vec(), true),
        ("version.xml".into(), version.into_bytes(), false),
    ];
    all.extend(files.into_iter().map(|(n, d)| (n, d, false)));
    all.push(("Contents/content.hpf".into(), hpf.into_bytes(), false));
    all.extend(bin_files.into_iter().map(|(n, d)| (n, d, false)));
    all.push(("settings.xml".into(), settings.into_bytes(), false));
    all.push((
        "META-INF/container.xml".into(),
        container.into_bytes(),
        false,
    ));
    all.push(("META-INF/manifest.xml".into(), manifest.into_bytes(), false));
    let members: Vec<Member<'_>> = all
        .iter()
        .map(|(name, data, stored)| Member::New {
            name,
            data,
            stored: *stored,
        })
        .collect();
    Ok(crate::zip::write(None, &members))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leftovers_return_to_their_place() {
        let mut r = crate::rest::Rest::new();
        let x = Xml::parse(SharedBytes::from(&br#"<a><b q="1"/><z/><c/></a>"#[..])).unwrap();
        let root = x.root();
        let b = root.children().next().unwrap();
        r.attrs("b", b, &[]);
        r.child("", root.children().nth(1).unwrap());
        let tail = r.into_tail();
        let built = X::new("a")
            .kid(X::new("b"))
            .kid(X::new("c"))
            .with_rest(&tail);
        let mut out = Vec::new();
        built.write(&mut out);
        assert_eq!(out, br#"<a><b q="1"/><z/><c/></a>"#);
    }

    #[test]
    fn colors_round_trip() {
        assert_eq!(color(0x0000_00FF), "#FF0000");
        assert_eq!(color(0xFFFF_FFFF), "none");
        assert_eq!(crate::head::color(&color(0x8012_3456)), 0x8012_3456);
    }

    #[test]
    fn new_package_compresses_bitmap_resources_and_reopens_exact_bytes() {
        use hm_model::{BinDataEntry, BinStore, DocMeta, Format, HwpString, Paragraph, Section};

        let pixels = 256 * 256 * 3;
        let mut bitmap = vec![0; 54 + pixels];
        bitmap[..2].copy_from_slice(b"BM");
        bitmap[2..6].copy_from_slice(&((54 + pixels) as u32).to_le_bytes());
        bitmap[10..14].copy_from_slice(&54u32.to_le_bytes());
        bitmap[14..18].copy_from_slice(&40u32.to_le_bytes());
        bitmap[18..22].copy_from_slice(&256u32.to_le_bytes());
        bitmap[22..26].copy_from_slice(&256u32.to_le_bytes());
        bitmap[26..28].copy_from_slice(&1u16.to_le_bytes());
        bitmap[28..30].copy_from_slice(&24u16.to_le_bytes());
        bitmap[34..38].copy_from_slice(&(pixels as u32).to_le_bytes());
        let expected = bitmap.clone();
        let data = SharedBytes::from(bitmap);
        let mut doc = Document {
            meta: DocMeta {
                format: Format::Hwpx,
                version: Default::default(),
                flags: 0,
            },
            info: Default::default(),
            sections: vec![Section {
                paras: vec![Paragraph::default()],
                ..Default::default()
            }],
            source: None,
            bins: Some(BinStore::new(move |id| (id == 1).then(|| data.clone()))),
            diagnostics: Vec::new(),
        };
        doc.info.bin_data.push(BinDataEntry {
            attr: 1,
            storage_id: Some(1),
            extension: Some(HwpString::from_text("bmp")),
            ..Default::default()
        });
        let bytes = write_new(&doc).unwrap();
        let reopened = crate::read(bytes.clone().into()).unwrap();
        assert_eq!(&*reopened.bins.unwrap().get(1).unwrap(), &expected);
        assert!(
            bytes.len() < expected.len() / 2,
            "raw bitmap should compress inside the package"
        );
    }
}
