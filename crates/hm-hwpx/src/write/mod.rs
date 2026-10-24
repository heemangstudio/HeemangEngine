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
        let start = match start.strip_suffix(b"/>") {
            Some(s) => [s, b">"].concat(),
            None => start,
        };
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
    if pkg.sections.len() != doc.sections.len() {
        return Err(Error::Unsupported("adding or removing sections".into()));
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
        let name = &pkg.sections[k];
        let i = archive
            .find(name)
            .ok_or_else(|| Error::Corrupt(format!("missing {name}")))?;
        let x = Xml::parse(archive.read(i)?)?;
        parts.seed(i as u32, x.src.clone());
        replaced.push((name.clone(), body::section(s, &x, &bins, &parts, None)?));
    }
    let members: Vec<Member<'_>> = archive
        .entries
        .iter()
        .enumerate()
        .map(|(i, e)| match replaced.iter().find(|(n, _)| *n == e.name) {
            Some((n, data)) => Member::New {
                name: n,
                data,
                stored: false,
            },
            None => Member::Copy(i),
        })
        .collect();
    Ok(crate::zip::write(Some(archive), &members))
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
    // Pictures are compressed already.
    all.extend(bin_files.into_iter().map(|(n, d)| (n, d, true)));
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
}
