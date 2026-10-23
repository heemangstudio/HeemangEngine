//! Where a mapped node came from.
//!
//! A node read from a package part keeps, as its model `src`, a short token
//! naming the part and the node's byte span instead of the markup itself,
//! so an open document holds none of its decompressed XML. Writers inflate
//! the part again from the source package and copy the span. Nodes mapped
//! from XML that is not a package part keep their markup.

use std::cell::RefCell;
use std::ops::Range;

use hm_model::SharedBytes;

use crate::zip::Archive;

/// First byte of a token; markup always starts with `<`.
const TAG: u8 = 0;
const LEN: usize = 13;

/// The token for `span` of archive member `part`.
pub fn token(part: u32, span: Range<usize>) -> SharedBytes {
    let mut t = Vec::with_capacity(LEN);
    t.push(TAG);
    t.extend_from_slice(&part.to_le_bytes());
    t.extend_from_slice(&(span.start as u32).to_le_bytes());
    t.extend_from_slice(&(span.end as u32).to_le_bytes());
    SharedBytes::from(t)
}

/// The part and span a token names.
pub fn parse(src: &[u8]) -> Option<(u32, Range<usize>)> {
    if src.len() != LEN || src[0] != TAG {
        return None;
    }
    let word = |i: usize| u32::from_le_bytes([src[i], src[i + 1], src[i + 2], src[i + 3]]);
    Some((word(1), word(5) as usize..word(9) as usize))
}

/// The parts of a source package, inflated on first use while writing.
pub struct Parts<'a> {
    archive: Option<&'a Archive>,
    inflated: RefCell<Vec<(u32, SharedBytes)>>,
}

impl<'a> Parts<'a> {
    pub fn new(archive: &'a Archive) -> Self {
        Parts {
            archive: Some(archive),
            inflated: RefCell::new(Vec::new()),
        }
    }

    /// Parts of no package: every node is encoded from the model.
    pub fn none() -> Self {
        Parts {
            archive: None,
            inflated: RefCell::new(Vec::new()),
        }
    }

    /// Makes an already inflated part available without reading it again.
    pub fn seed(&self, part: u32, bytes: SharedBytes) {
        self.inflated.borrow_mut().push((part, bytes));
    }

    /// The markup a node's `src` stands for; `None` when the token names
    /// nothing the package holds, so the node is encoded from the model.
    pub fn markup(&self, src: &SharedBytes) -> Option<SharedBytes> {
        if src.first() == Some(&b'<') {
            return Some(src.clone());
        }
        let (part, span) = parse(src)?;
        let bytes = self.part(part)?;
        (span.start <= span.end && span.end <= bytes.len()).then(|| bytes.slice(span))
    }

    fn part(&self, part: u32) -> Option<SharedBytes> {
        if let Some((_, b)) = self.inflated.borrow().iter().find(|(p, _)| *p == part) {
            return Some(b.clone());
        }
        let archive = self.archive?;
        if part as usize >= archive.entries.len() {
            return None;
        }
        let b = archive.read(part as usize).ok()?;
        self.inflated.borrow_mut().push((part, b.clone()));
        Some(b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_name_part_and_span() {
        let t = token(7, 120..4000);
        assert_eq!(parse(&t), Some((7, 120..4000)));
        assert_eq!(parse(b"<hp:p/>"), None);
        let p = Parts::none();
        p.seed(7, SharedBytes::from(vec![b'x'; 4000]));
        assert_eq!(p.markup(&t).map(|m| m.len()), Some(3880));
        assert!(p.markup(&token(8, 0..1)).is_none());
        assert!(p.markup(&token(7, 0..4001)).is_none());
        let m = SharedBytes::from(&b"<a/>"[..]);
        assert_eq!(p.markup(&m), Some(m));
    }
}
