//! Cheaply cloned byte ranges into shared buffers.

use std::fmt;
use std::ops::{Deref, Range};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

/// A range of an `Arc<[u8]>`.
///
/// Parsers slice decompressed streams into `SharedBytes` without copying, so
/// source spans, unread tails and raw record payloads cost a pointer and two
/// integers each.
#[derive(Clone)]
pub struct SharedBytes {
    buf: Arc<[u8]>,
    id: usize,
    start: u32,
    end: u32,
}

fn next_id() -> usize {
    static NEXT: AtomicUsize = AtomicUsize::new(1);
    NEXT.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
        .expect("byte buffer identities exhausted")
}

fn empty_buf() -> Arc<[u8]> {
    static EMPTY: OnceLock<Arc<[u8]>> = OnceLock::new();
    EMPTY.get_or_init(|| Arc::from(Vec::new())).clone()
}

impl SharedBytes {
    /// Wraps a whole buffer. Buffers are limited to `u32::MAX` bytes.
    pub fn new(buf: Arc<[u8]>) -> Self {
        let end = u32::try_from(buf.len()).expect("buffer larger than 4 GiB");
        Self {
            buf,
            id: next_id(),
            start: 0,
            end,
        }
    }

    pub fn empty() -> Self {
        Self {
            buf: empty_buf(),
            id: 0,
            start: 0,
            end: 0,
        }
    }

    /// A sub-range relative to this range. Panics when out of bounds.
    pub fn slice(&self, range: Range<usize>) -> Self {
        assert!(
            range.start <= range.end && range.end <= self.len(),
            "slice out of range"
        );
        Self {
            buf: self.buf.clone(),
            id: self.id,
            start: self.start + range.start as u32,
            end: self.start + range.end as u32,
        }
    }

    /// The bytes from `offset` to the end.
    pub fn tail_from(&self, offset: usize) -> Self {
        self.slice(offset.min(self.len())..self.len())
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.buf[self.start as usize..self.end as usize]
    }

    /// Process-local identity of this immutable byte range. Clones and
    /// equivalent slices share it; later allocations never reuse it.
    /// Caches can keep this key after the original bytes are dropped.
    pub fn identity(&self) -> (usize, usize, usize) {
        (self.id, self.start as usize, self.len())
    }

    /// Offset of this range inside its backing buffer.
    pub fn offset(&self) -> usize {
        self.start as usize
    }
}

impl Deref for SharedBytes {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl AsRef<[u8]> for SharedBytes {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl From<Vec<u8>> for SharedBytes {
    fn from(v: Vec<u8>) -> Self {
        if v.is_empty() {
            Self::empty()
        } else {
            Self::new(Arc::from(v))
        }
    }
}

impl From<&[u8]> for SharedBytes {
    fn from(v: &[u8]) -> Self {
        Self::from(v.to_vec())
    }
}

impl Default for SharedBytes {
    fn default() -> Self {
        Self::empty()
    }
}

impl PartialEq for SharedBytes {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl Eq for SharedBytes {}

impl fmt::Debug for SharedBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.len() <= 16 {
            write!(f, "Bytes({:02x?})", self.as_slice())
        } else {
            write!(f, "Bytes({} bytes)", self.len())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slices_share_the_buffer() {
        let b = SharedBytes::from(vec![1, 2, 3, 4, 5]);
        let s = b.slice(1..4);
        assert_eq!(&*s, &[2, 3, 4]);
        assert_eq!(&*s.slice(1..2), &[3]);
        assert_eq!(s.offset(), 1);
        assert_eq!(&*b.tail_from(9), &[] as &[u8]);
    }
}

// Merge property views keep only this slice, never its backing source buffer.
#[cfg(feature = "serde")]
impl serde::Serialize for SharedBytes {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use base64::Engine as _;
        serializer.serialize_str(&base64::engine::general_purpose::STANDARD.encode(self.as_slice()))
    }
}
#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for SharedBytes {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use base64::Engine as _;
        let value = <String as serde::Deserialize>::deserialize(deserializer)?;
        base64::engine::general_purpose::STANDARD
            .decode(value)
            .map(SharedBytes::from)
            .map_err(serde::de::Error::custom)
    }
}
