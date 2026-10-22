//! A small XML reader that keeps the byte span of every node.
//!
//! The whole document becomes a flat node array (elements, text, CDATA) with
//! parent, child and sibling links. Names are resolved to a known namespace
//! and a local name; attribute values stay raw until asked for. Spans let
//! writers copy untouched elements exactly as stored.

use std::borrow::Cow;
use std::ops::Range;

use hm_model::SharedBytes;

use crate::Error;

/// Namespaces OWPML packages use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Ns {
    /// No namespace (unprefixed attributes, undeclared prefixes).
    None,
    Hp,
    Hp10,
    Hh,
    Hc,
    Hs,
    Ha,
    Hm,
    Hhs,
    Hv,
    Hpf,
    Opf,
    Ocf,
    Config,
    Dc,
    Xml,
    Other,
}

impl Ns {
    fn from_uri(uri: &[u8]) -> Ns {
        match uri {
            b"http://www.hancom.co.kr/hwpml/2011/paragraph"
            | b"http://www.hancom.co.kr/hwpml/2010/paragraph" => Ns::Hp,
            b"http://www.hancom.co.kr/hwpml/2016/paragraph" => Ns::Hp10,
            b"http://www.hancom.co.kr/hwpml/2011/head"
            | b"http://www.hancom.co.kr/hwpml/2010/head" => Ns::Hh,
            b"http://www.hancom.co.kr/hwpml/2011/core"
            | b"http://www.hancom.co.kr/hwpml/2010/core" => Ns::Hc,
            b"http://www.hancom.co.kr/hwpml/2011/section"
            | b"http://www.hancom.co.kr/hwpml/2010/section" => Ns::Hs,
            b"http://www.hancom.co.kr/hwpml/2011/app"
            | b"http://www.hancom.co.kr/hwpml/2010/app" => Ns::Ha,
            b"http://www.hancom.co.kr/hwpml/2011/master-page" => Ns::Hm,
            b"http://www.hancom.co.kr/hwpml/2011/history" => Ns::Hhs,
            b"http://www.hancom.co.kr/hwpml/2011/version" => Ns::Hv,
            b"http://www.hancom.co.kr/schema/2011/hpf" => Ns::Hpf,
            b"http://www.idpf.org/2007/opf/" | b"http://www.idpf.org/2007/opf" => Ns::Opf,
            b"urn:oasis:names:tc:opendocument:xmlns:container" => Ns::Ocf,
            b"urn:oasis:names:tc:opendocument:xmlns:config:1.0" => Ns::Config,
            b"http://purl.org/dc/elements/1.1/" => Ns::Dc,
            b"http://www.w3.org/XML/1998/namespace" => Ns::Xml,
            _ => Ns::Other,
        }
    }
}

pub type NodeId = u32;
const NONE: u32 = u32::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Element,
    /// Character data with entities still escaped.
    Text,
    /// A CDATA section; `inner` excludes the markers.
    CData,
}

#[derive(Clone, Copy, Debug)]
struct Node {
    kind: Kind,
    ns: Ns,
    /// Local name (elements).
    local: (u32, u32),
    /// The whole node, start tag to end tag.
    span: (u32, u32),
    /// Element content between the tags; text and CDATA content.
    inner: (u32, u32),
    attrs: (u32, u32),
    parent: u32,
    first_child: u32,
    next: u32,
}

#[derive(Clone, Copy, Debug)]
struct Attr {
    ns: Ns,
    local: (u32, u32),
    value: (u32, u32),
}

/// A parsed XML document.
pub struct Xml {
    pub src: SharedBytes,
    nodes: Vec<Node>,
    attrs: Vec<Attr>,
    root: NodeId,
}

impl std::fmt::Debug for Xml {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Xml({} nodes)", self.nodes.len())
    }
}

struct Scope {
    prefix: (u32, u32),
    ns: Ns,
}

fn bad(at: usize, m: &str) -> Error {
    Error::Xml(format!("{m} at byte {at}"))
}

fn is_name_end(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\r' | b'\n' | b'/' | b'>' | b'=')
}

fn find(b: &[u8], from: usize, pat: &[u8]) -> Option<usize> {
    b.get(from..)?
        .windows(pat.len())
        .position(|w| w == pat)
        .map(|p| p + from)
}

impl Xml {
    pub fn parse(src: SharedBytes) -> Result<Xml, Error> {
        let b: &[u8] = &src;
        let mut nodes: Vec<Node> = Vec::with_capacity(b.len() / 40);
        let mut attrs: Vec<Attr> = Vec::with_capacity(b.len() / 30);
        let mut scopes: Vec<Scope> = Vec::new();
        // Open elements: node id, last child, scope depth at open.
        let mut open: Vec<(u32, u32, usize)> = Vec::new();
        let mut root = NONE;
        let mut i = 0usize;
        if b.starts_with(&[0xEF, 0xBB, 0xBF]) {
            i = 3;
        }

        fn link(nodes: &mut [Node], open: &mut [(u32, u32, usize)], id: u32) {
            if let Some(top) = open.last_mut() {
                nodes[id as usize].parent = top.0;
                if top.1 == NONE {
                    nodes[top.0 as usize].first_child = id;
                } else {
                    nodes[top.1 as usize].next = id;
                }
                top.1 = id;
            }
        }

        let resolve = |scopes: &[Scope], b: &[u8], prefix: &[u8]| -> Ns {
            for s in scopes.iter().rev() {
                if &b[s.prefix.0 as usize..s.prefix.1 as usize] == prefix {
                    return s.ns;
                }
            }
            match prefix {
                b"xml" => Ns::Xml,
                _ => Ns::None,
            }
        };

        while i < b.len() {
            if b[i] != b'<' {
                let end = memchr(b'<', b, i).unwrap_or(b.len());
                if !open.is_empty() {
                    let id = nodes.len() as u32;
                    nodes.push(Node {
                        kind: Kind::Text,
                        ns: Ns::None,
                        local: (0, 0),
                        span: (i as u32, end as u32),
                        inner: (i as u32, end as u32),
                        attrs: (0, 0),
                        parent: NONE,
                        first_child: NONE,
                        next: NONE,
                    });
                    link(&mut nodes, &mut open, id);
                }
                i = end;
                continue;
            }
            let rest = &b[i..];
            if rest.starts_with(b"<?") {
                i = find(b, i, b"?>").ok_or_else(|| bad(i, "unterminated declaration"))? + 2;
            } else if rest.starts_with(b"<!--") {
                i = find(b, i + 4, b"-->").ok_or_else(|| bad(i, "unterminated comment"))? + 3;
            } else if rest.starts_with(b"<![CDATA[") {
                let end = find(b, i + 9, b"]]>").ok_or_else(|| bad(i, "unterminated CDATA"))?;
                if !open.is_empty() {
                    let id = nodes.len() as u32;
                    nodes.push(Node {
                        kind: Kind::CData,
                        ns: Ns::None,
                        local: (0, 0),
                        span: (i as u32, end as u32 + 3),
                        inner: (i as u32 + 9, end as u32),
                        attrs: (0, 0),
                        parent: NONE,
                        first_child: NONE,
                        next: NONE,
                    });
                    link(&mut nodes, &mut open, id);
                }
                i = end + 3;
            } else if rest.starts_with(b"<!") {
                // DOCTYPE and similar: skip to the matching '>' outside brackets.
                let mut depth = 0i32;
                let mut j = i + 2;
                while j < b.len() {
                    match b[j] {
                        b'[' => depth += 1,
                        b']' => depth -= 1,
                        b'>' if depth <= 0 => break,
                        _ => {}
                    }
                    j += 1;
                }
                i = j + 1;
            } else if rest.starts_with(b"</") {
                let name_start = i + 2;
                let gt =
                    memchr(b'>', b, name_start).ok_or_else(|| bad(i, "unterminated end tag"))?;
                let name = trim(&b[name_start..gt]);
                let Some((id, _, depth)) = open.pop() else {
                    return Err(bad(i, "unexpected end tag"));
                };
                let n = &mut nodes[id as usize];
                let qname_start = n.span.0 as usize + 1;
                let qname = &b[qname_start..n.local.1 as usize];
                if qname != name {
                    return Err(bad(i, "mismatched end tag"));
                }
                n.inner.1 = i as u32;
                n.span.1 = gt as u32 + 1;
                scopes.truncate(depth);
                i = gt + 1;
            } else {
                // Start tag.
                let name_start = i + 1;
                let mut j = name_start;
                while j < b.len() && !is_name_end(b[j]) {
                    j += 1;
                }
                let name_end = j;
                if name_end == name_start {
                    return Err(bad(i, "empty element name"));
                }
                let scope_depth = scopes.len();
                let attr_start = attrs.len();
                let mut raw_attrs: Vec<((usize, usize), (usize, usize))> = Vec::new();
                let self_closing;
                loop {
                    while j < b.len() && b[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    match b.get(j) {
                        None => return Err(bad(i, "unterminated start tag")),
                        Some(b'>') => {
                            self_closing = false;
                            j += 1;
                            break;
                        }
                        Some(b'/') if b.get(j + 1) == Some(&b'>') => {
                            self_closing = true;
                            j += 2;
                            break;
                        }
                        _ => {}
                    }
                    let an = j;
                    while j < b.len() && !is_name_end(b[j]) {
                        j += 1;
                    }
                    let an_end = j;
                    while j < b.len() && b[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    if b.get(j) != Some(&b'=') || an_end == an {
                        return Err(bad(j, "malformed attribute"));
                    }
                    j += 1;
                    while j < b.len() && b[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    let q = *b.get(j).ok_or_else(|| bad(j, "unterminated attribute"))?;
                    if q != b'"' && q != b'\'' {
                        return Err(bad(j, "unquoted attribute"));
                    }
                    let vs = j + 1;
                    let ve = memchr(q, b, vs).ok_or_else(|| bad(j, "unterminated attribute"))?;
                    j = ve + 1;
                    let aname = &b[an..an_end];
                    if aname == b"xmlns" {
                        scopes.push(Scope {
                            prefix: (0, 0),
                            ns: Ns::from_uri(&b[vs..ve]),
                        });
                    } else if let Some(p) = aname.strip_prefix(b"xmlns:") {
                        let ps = an_end - p.len();
                        scopes.push(Scope {
                            prefix: (ps as u32, an_end as u32),
                            ns: Ns::from_uri(&b[vs..ve]),
                        });
                    } else {
                        raw_attrs.push(((an, an_end), (vs, ve)));
                    }
                }
                for ((an, ae), (vs, ve)) in raw_attrs {
                    let name = &b[an..ae];
                    let (ns, ls) = match name.iter().position(|&c| c == b':') {
                        Some(p) => (resolve(&scopes, b, &name[..p]), an + p + 1),
                        None => (Ns::None, an),
                    };
                    attrs.push(Attr {
                        ns,
                        local: (ls as u32, ae as u32),
                        value: (vs as u32, ve as u32),
                    });
                }
                let qname = &b[name_start..name_end];
                let (ns, ls) = match qname.iter().position(|&c| c == b':') {
                    Some(p) => (resolve(&scopes, b, &qname[..p]), name_start + p + 1),
                    None => (resolve(&scopes, b, b""), name_start),
                };
                let id = nodes.len() as u32;
                nodes.push(Node {
                    kind: Kind::Element,
                    ns,
                    local: (ls as u32, name_end as u32),
                    span: (i as u32, j as u32),
                    inner: (j as u32, j as u32),
                    attrs: (attr_start as u32, attrs.len() as u32),
                    parent: NONE,
                    first_child: NONE,
                    next: NONE,
                });
                if open.is_empty() {
                    if root != NONE {
                        return Err(bad(i, "second root element"));
                    }
                    root = id;
                } else {
                    link(&mut nodes, &mut open, id);
                }
                if self_closing {
                    scopes.truncate(scope_depth);
                } else {
                    open.push((id, NONE, scope_depth));
                }
                i = j;
            }
        }
        if !open.is_empty() {
            return Err(Error::Xml("unclosed element".into()));
        }
        if root == NONE {
            return Err(Error::Xml("no root element".into()));
        }
        Ok(Xml {
            src,
            nodes,
            attrs,
            root,
        })
    }

    pub fn root(&self) -> El<'_> {
        El {
            x: self,
            id: self.root,
        }
    }

    pub fn el(&self, id: NodeId) -> El<'_> {
        El { x: self, id }
    }
}

fn memchr(c: u8, b: &[u8], from: usize) -> Option<usize> {
    b.get(from..)?
        .iter()
        .position(|&x| x == c)
        .map(|p| p + from)
}

fn trim(b: &[u8]) -> &[u8] {
    let s = b
        .iter()
        .position(|c| !c.is_ascii_whitespace())
        .unwrap_or(b.len());
    let e = b
        .iter()
        .rposition(|c| !c.is_ascii_whitespace())
        .map_or(s, |p| p + 1);
    &b[s..e]
}

/// A node handle.
#[derive(Clone, Copy)]
pub struct El<'a> {
    x: &'a Xml,
    pub id: NodeId,
}

impl std::fmt::Debug for El<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "<{:?}:{}>",
            self.ns(),
            String::from_utf8_lossy(self.local())
        )
    }
}

impl<'a> El<'a> {
    fn node(&self) -> &'a Node {
        &self.x.nodes[self.id as usize]
    }

    fn bytes(&self, r: (u32, u32)) -> &'a [u8] {
        &self.x.src[r.0 as usize..r.1 as usize]
    }

    pub fn kind(&self) -> Kind {
        self.node().kind
    }

    pub fn is_element(&self) -> bool {
        self.node().kind == Kind::Element
    }

    pub fn ns(&self) -> Ns {
        self.node().ns
    }

    pub fn local(&self) -> &'a [u8] {
        self.bytes(self.node().local)
    }

    pub fn is(&self, ns: Ns, local: &str) -> bool {
        self.is_element() && self.node().ns == ns && self.local() == local.as_bytes()
    }

    /// The node's byte range in the source.
    pub fn span(&self) -> Range<usize> {
        let s = self.node().span;
        s.0 as usize..s.1 as usize
    }

    /// The node's source bytes, shared.
    pub fn source(&self) -> SharedBytes {
        self.x.src.slice(self.span())
    }

    /// The element's start tag bytes.
    pub fn start_tag(&self) -> &'a [u8] {
        let n = self.node();
        let end = if n.inner.0 == n.span.1 {
            n.span.1
        } else {
            n.inner.0
        };
        &self.x.src[n.span.0 as usize..end as usize]
    }

    pub fn parent(&self) -> Option<El<'a>> {
        let p = self.node().parent;
        (p != NONE).then_some(El { x: self.x, id: p })
    }

    /// All child nodes, text included.
    pub fn nodes(&self) -> Children<'a> {
        Children {
            x: self.x,
            next: self.node().first_child,
        }
    }

    /// Child elements.
    pub fn children(&self) -> impl Iterator<Item = El<'a>> + 'a {
        self.nodes().filter(|c| c.is_element())
    }

    pub fn child(&self, ns: Ns, local: &str) -> Option<El<'a>> {
        self.children().find(|c| c.is(ns, local))
    }

    fn raw_attr(&self, ns: Ns, local: &str) -> Option<&'a [u8]> {
        let n = self.node();
        self.x.attrs[n.attrs.0 as usize..n.attrs.1 as usize]
            .iter()
            .find(|a| a.ns == ns && self.bytes(a.local) == local.as_bytes())
            .map(|a| self.bytes(a.value))
    }

    /// An unprefixed attribute, unescaped.
    pub fn attr(&self, local: &str) -> Option<Cow<'a, str>> {
        self.raw_attr(Ns::None, local).map(unescape)
    }

    pub fn attr_ns(&self, ns: Ns, local: &str) -> Option<Cow<'a, str>> {
        self.raw_attr(ns, local).map(unescape)
    }

    /// Attribute names and raw values, in order, namespace declarations
    /// excluded.
    pub fn attrs(&self) -> impl Iterator<Item = (Ns, &'a [u8], &'a [u8])> + 'a {
        let n = self.node();
        let x = self.x;
        x.attrs[n.attrs.0 as usize..n.attrs.1 as usize]
            .iter()
            .map(move |a| {
                (
                    a.ns,
                    &x.src[a.local.0 as usize..a.local.1 as usize],
                    &x.src[a.value.0 as usize..a.value.1 as usize],
                )
            })
    }

    /// An integer attribute; decimal, with an optional sign. Values out of
    /// range wrap like Hancom's unsigned fields (`4294967295` reads as -1).
    pub fn int(&self, local: &str) -> Option<i64> {
        let v = self.raw_attr(Ns::None, local)?;
        parse_int(v)
    }

    pub fn i32(&self, local: &str) -> Option<i32> {
        self.int(local).map(|v| v as i32)
    }

    pub fn u32(&self, local: &str) -> Option<u32> {
        self.int(local).map(|v| v as u32)
    }

    /// A boolean attribute: `1`/`true` are true.
    pub fn flag(&self, local: &str) -> Option<bool> {
        self.raw_attr(Ns::None, local)
            .map(|v| matches!(trim(v), b"1" | b"true"))
    }

    pub fn raw(&self, local: &str) -> Option<&'a [u8]> {
        self.raw_attr(Ns::None, local)
    }

    /// Raw content of a text or CDATA node.
    pub fn content(&self) -> &'a [u8] {
        self.bytes(self.node().inner)
    }

    /// The element's text content (direct text and CDATA children), unescaped.
    pub fn text(&self) -> String {
        let mut s = String::new();
        for c in self.nodes() {
            match c.kind() {
                Kind::Text => s.push_str(&unescape(c.content())),
                Kind::CData => s.push_str(&String::from_utf8_lossy(c.content())),
                Kind::Element => {}
            }
        }
        s
    }
}

pub struct Children<'a> {
    x: &'a Xml,
    next: u32,
}

impl<'a> Iterator for Children<'a> {
    type Item = El<'a>;
    fn next(&mut self) -> Option<El<'a>> {
        if self.next == NONE {
            return None;
        }
        let id = self.next;
        self.next = self.x.nodes[id as usize].next;
        Some(El { x: self.x, id })
    }
}

pub fn parse_int(v: &[u8]) -> Option<i64> {
    let v = trim(v);
    let (neg, digits) = match v.first()? {
        b'-' => (true, &v[1..]),
        b'+' => (false, &v[1..]),
        _ => (false, v),
    };
    if digits.is_empty() {
        return None;
    }
    let mut n: i64 = 0;
    for &c in digits {
        if !c.is_ascii_digit() {
            // Some writers emit decimals; keep the integer part.
            if c == b'.' {
                break;
            }
            return None;
        }
        n = n.checked_mul(10)?.checked_add((c - b'0') as i64)?;
    }
    Some(if neg { -n } else { n })
}

/// Decodes the five predefined entities and character references.
pub fn unescape(raw: &[u8]) -> Cow<'_, str> {
    if !raw.contains(&b'&') {
        return String::from_utf8_lossy(raw);
    }
    let mut out = String::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'&'
            && let Some(semi) = raw[i..].iter().take(12).position(|&c| c == b';')
        {
            let ent = &raw[i + 1..i + semi];
            let ch = match ent {
                b"lt" => Some('<'),
                b"gt" => Some('>'),
                b"amp" => Some('&'),
                b"quot" => Some('"'),
                b"apos" => Some('\''),
                _ if ent.first() == Some(&b'#') => {
                    let (digits, radix) = match ent.get(1) {
                        Some(b'x' | b'X') => (&ent[2..], 16),
                        _ => (&ent[1..], 10),
                    };
                    std::str::from_utf8(digits)
                        .ok()
                        .and_then(|d| u32::from_str_radix(d, radix).ok())
                        .and_then(char::from_u32)
                }
                _ => None,
            };
            if let Some(c) = ch {
                out.push(c);
                i += semi + 1;
                continue;
            }
        }
        let start = i;
        i += 1;
        while i < raw.len() && raw[i] != b'&' {
            i += 1;
        }
        out.push_str(&String::from_utf8_lossy(&raw[start..i]));
    }
    Cow::Owned(out)
}

/// Escapes text for element content.
pub fn escape_text(out: &mut Vec<u8>, s: &str) {
    for &c in s.as_bytes() {
        match c {
            b'<' => out.extend_from_slice(b"&lt;"),
            b'>' => out.extend_from_slice(b"&gt;"),
            b'&' => out.extend_from_slice(b"&amp;"),
            _ => out.push(c),
        }
    }
}

/// Escapes text for a double-quoted attribute value.
pub fn escape_attr(out: &mut Vec<u8>, s: &str) {
    for &c in s.as_bytes() {
        match c {
            b'<' => out.extend_from_slice(b"&lt;"),
            b'>' => out.extend_from_slice(b"&gt;"),
            b'&' => out.extend_from_slice(b"&amp;"),
            b'"' => out.extend_from_slice(b"&quot;"),
            b'\n' => out.extend_from_slice(b"&#10;"),
            b'\r' => out.extend_from_slice(b"&#13;"),
            b'\t' => out.extend_from_slice(b"&#9;"),
            _ => out.push(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Xml {
        Xml::parse(SharedBytes::from(s.as_bytes())).unwrap()
    }

    #[test]
    fn resolves_namespaces_and_spans() {
        let src = r#"<?xml version="1.0"?><hs:sec xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section" xmlns:x="http://www.hancom.co.kr/hwpml/2011/paragraph"><x:p id="3"><x:run><x:t>a&amp;b<x:tab width="10"/> c</x:t></x:run></x:p><!-- c --><x:p/></hs:sec>"#;
        let x = parse(src);
        let root = x.root();
        assert!(root.is(Ns::Hs, "sec"));
        let ps: Vec<_> = root.children().collect();
        assert_eq!(ps.len(), 2);
        assert!(ps[0].is(Ns::Hp, "p"));
        assert_eq!(ps[0].int("id"), Some(3));
        assert_eq!(&src[ps[1].span()], "<x:p/>");
        let t = ps[0]
            .child(Ns::Hp, "run")
            .unwrap()
            .child(Ns::Hp, "t")
            .unwrap();
        let parts: Vec<_> = t.nodes().map(|n| n.kind()).collect();
        assert_eq!(parts, [Kind::Text, Kind::Element, Kind::Text]);
        assert_eq!(t.text(), "a&b c");
        assert!(src[ps[0].span()].starts_with("<x:p id=\"3\">"));
        assert!(src[ps[0].span()].ends_with("</x:p>"));
    }

    #[test]
    fn rejects_mismatched_tags() {
        assert!(Xml::parse(SharedBytes::from(&b"<a><b></a></b>"[..])).is_err());
        assert!(Xml::parse(SharedBytes::from(&b"<a>"[..])).is_err());
    }

    #[test]
    fn unescapes_references() {
        assert_eq!(unescape(b"&#xAC00;&#65;&lt;&bogus;"), "가A<&bogus;");
        assert_eq!(parse_int(b"4294967295"), Some(4294967295));
        assert_eq!(parse_int(b"-12"), Some(-12));
        assert_eq!(parse_int(b"1.5"), Some(1));
    }
}
