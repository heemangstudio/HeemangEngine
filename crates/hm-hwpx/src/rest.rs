//! Markup the mapping does not interpret, carried in a model entry's `tail`.
//!
//! For HWPX documents a typed entry's tail holds the attributes and child
//! elements of its source element that no model field covers, so a dirty
//! entry is written back with them. Each piece records the path of the
//! element it belongs to (local names below the entry element, joined by
//! `/`) and, for child elements, how many modeled children preceded it,
//! which keeps sibling order.
//!
//! Encoding: a sequence of `kind u8, path_len u8, path, slot u16, len u32,
//! bytes`; kind 1 is an attribute (`name="value"` as stored), kind 2 a child
//! element (its full markup).

use hm_model::SharedBytes;

use crate::xml::{El, Ns};

const ATTR: u8 = 1;
const CHILD: u8 = 2;

/// Collects leftovers while an element is mapped.
#[derive(Default)]
pub struct Rest {
    buf: Vec<u8>,
    /// Count of pieces recorded, for coverage reports.
    pub pieces: u32,
}

impl Rest {
    pub fn new() -> Self {
        Self::default()
    }

    fn push(&mut self, kind: u8, path: &str, slot: u16, bytes: &[u8]) {
        self.buf.push(kind);
        let p = &path.as_bytes()[..path.len().min(255)];
        self.buf.push(p.len() as u8);
        self.buf.extend_from_slice(p);
        self.buf.extend_from_slice(&slot.to_le_bytes());
        self.buf
            .extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        self.buf.extend_from_slice(bytes);
        self.pieces += 1;
    }

    /// Records every attribute of `el` whose local name is not in `known`.
    pub fn attrs(&mut self, path: &str, el: El<'_>, known: &[&str]) {
        for (ns, local, value) in el.attrs() {
            if ns == Ns::None && known.iter().any(|k| k.as_bytes() == local) {
                continue;
            }
            let mut raw = Vec::with_capacity(local.len() + value.len() + 3);
            if ns != Ns::None {
                raw.extend_from_slice(prefix(ns).as_bytes());
                raw.push(b':');
            }
            raw.extend_from_slice(local);
            raw.extend_from_slice(b"=\"");
            raw.extend_from_slice(value);
            raw.push(b'"');
            self.push(ATTR, path, 0, &raw);
        }
    }

    /// Records a child element the mapping skipped. Its slot counts the
    /// modeled siblings before it: its element index less the leftovers
    /// already recorded under the same path.
    pub fn child(&mut self, path: &str, el: El<'_>) {
        let index = el
            .parent()
            .map_or(0, |p| p.children().take_while(|c| c.id != el.id).count());
        let earlier = pieces(&self.buf)
            .iter()
            .filter(|p| !p.kind_attr && p.path == path)
            .count();
        let slot = index.saturating_sub(earlier) as u16;
        self.push(CHILD, path, slot, &el.source());
    }

    /// The encoded pieces so far.
    pub fn peek(&self) -> &[u8] {
        &self.buf
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn into_tail(self) -> SharedBytes {
        SharedBytes::from(self.buf)
    }
}

/// The standard prefix for a namespace, as writers emit it.
pub fn prefix(ns: Ns) -> &'static str {
    match ns {
        Ns::Hp => "hp",
        Ns::Hp10 => "hp10",
        Ns::Hh => "hh",
        Ns::Hc => "hc",
        Ns::Hs => "hs",
        Ns::Ha => "ha",
        Ns::Hm => "hm",
        Ns::Hhs => "hhs",
        Ns::Hv => "hv",
        Ns::Hpf => "hpf",
        Ns::Opf => "opf",
        Ns::Ocf => "ocf",
        Ns::Config => "config",
        Ns::Dc => "dc",
        Ns::Xml => "xml",
        Ns::None | Ns::Other => "",
    }
}

/// One recorded piece.
#[derive(Clone, Copy, Debug)]
pub struct Piece<'a> {
    pub kind_attr: bool,
    pub path: &'a str,
    pub slot: u16,
    pub bytes: &'a [u8],
}

/// Reads the pieces of a tail written by [`Rest`]. Tails that do not parse
/// (HWP 5 bytes) yield nothing.
pub fn pieces(tail: &[u8]) -> Vec<Piece<'_>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < tail.len() {
        let kind = tail[i];
        if kind != ATTR && kind != CHILD {
            return Vec::new();
        }
        let Some(&plen) = tail.get(i + 1) else {
            return Vec::new();
        };
        let ps = i + 2;
        let pe = ps + plen as usize;
        let Some(head) = tail.get(pe..pe + 6) else {
            return Vec::new();
        };
        let slot = u16::from_le_bytes([head[0], head[1]]);
        let len = u32::from_le_bytes([head[2], head[3], head[4], head[5]]) as usize;
        let bs = pe + 6;
        let Some(bytes) = tail.get(bs..bs + len) else {
            return Vec::new();
        };
        let Ok(path) = std::str::from_utf8(&tail[ps..pe]) else {
            return Vec::new();
        };
        out.push(Piece {
            kind_attr: kind == ATTR,
            path,
            slot,
            bytes,
        });
        i = bs + len;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xml::Xml;

    #[test]
    fn records_unknown_attributes_and_children() {
        let x = Xml::parse(SharedBytes::from(
            &br#"<a xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph" id="1" odd="x"><b/><hp:zz q="1"/></a>"#[..],
        ))
        .unwrap();
        let root = x.root();
        let mut r = Rest::new();
        r.attrs("", root, &["id"]);
        let zz = root.children().nth(1).unwrap();
        r.child("", zz);
        let tail = r.into_tail();
        let p = pieces(&tail);
        assert_eq!(p.len(), 2);
        assert!(p[0].kind_attr);
        assert_eq!(p[0].bytes, b"odd=\"x\"");
        assert_eq!(p[1].slot, 1);
        assert_eq!(p[1].bytes, br#"<hp:zz q="1"/>"#);
        assert!(pieces(&[0x30, 1, 2]).is_empty());
    }
}
