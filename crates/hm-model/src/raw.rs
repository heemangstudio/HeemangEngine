//! Records and strings carried without interpretation.

use std::fmt;

use crate::SharedBytes;

/// A record the model does not interpret, kept for lossless writing.
///
/// `level` is relative to the record's owner: a direct child of a paragraph
/// or control has level 1, its children level 2, and so on. Top-level
/// DocInfo records keep their absolute level.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawRecord {
    pub tag: u16,
    pub level: u16,
    pub payload: SharedBytes,
}

/// A four-character control id, stored as Hancom's big-endian FourCC
/// (`'t','b','l',' '` is `0x74626C20`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CtrlId(pub u32);

impl CtrlId {
    pub const fn from_chars(c: &[u8; 4]) -> Self {
        Self(u32::from_be_bytes(*c))
    }

    pub fn chars(self) -> [u8; 4] {
        self.0.to_be_bytes()
    }

    pub const SECTION: CtrlId = CtrlId::from_chars(b"secd");
    pub const COLUMNS: CtrlId = CtrlId::from_chars(b"cold");
    pub const TABLE: CtrlId = CtrlId::from_chars(b"tbl ");
    pub const SHAPE: CtrlId = CtrlId::from_chars(b"gso ");
    pub const EQUATION: CtrlId = CtrlId::from_chars(b"eqed");
    pub const HEADER: CtrlId = CtrlId::from_chars(b"head");
    pub const FOOTER: CtrlId = CtrlId::from_chars(b"foot");
    pub const FOOTNOTE: CtrlId = CtrlId::from_chars(b"fn  ");
    pub const ENDNOTE: CtrlId = CtrlId::from_chars(b"en  ");
    pub const AUTO_NUMBER: CtrlId = CtrlId::from_chars(b"atno");
    pub const NEW_NUMBER: CtrlId = CtrlId::from_chars(b"nwno");
    pub const PAGE_HIDE: CtrlId = CtrlId::from_chars(b"pghd");
    pub const PAGE_NUMBER_POS: CtrlId = CtrlId::from_chars(b"pgnp");
    pub const BOOKMARK: CtrlId = CtrlId::from_chars(b"bokm");
}

impl fmt::Debug for CtrlId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let c = self.chars();
        if c.iter().all(|b| b.is_ascii_graphic() || *b == b' ') {
            write!(f, "'{}'", String::from_utf8_lossy(&c))
        } else {
            write!(f, "{:#010x}", self.0)
        }
    }
}

impl fmt::Display for CtrlId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

/// A UTF-16 string kept as stored, so unpaired surrogates survive a round trip.
#[derive(Clone, Default, PartialEq, Eq, Hash)]
pub struct HwpString(pub Vec<u16>);

impl HwpString {
    pub fn from_text(s: &str) -> Self {
        Self(s.encode_utf16().collect())
    }

    pub fn to_string_lossy(&self) -> String {
        String::from_utf16_lossy(&self.0)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for HwpString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.to_string_lossy())
    }
}

impl fmt::Display for HwpString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_string_lossy())
    }
}
