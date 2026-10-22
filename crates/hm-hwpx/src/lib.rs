//! HWPX reader and writer: ZIP container, XML and the OWPML mapping.

mod body;
pub mod enums;
mod head;
mod object;
mod read;
pub mod rest;
pub mod xml;
pub mod zip;

use std::collections::BTreeMap;

use hm_model::{Document, SharedBytes};

/// True when `bytes` start like a ZIP package; [`read`] checks the parts.
pub fn is_hwpx(bytes: &[u8]) -> bool {
    zip::is_zip(bytes)
}

/// Reads an HWPX document.
pub fn read(bytes: SharedBytes) -> Result<Document, Error> {
    read::document(bytes).map(|(d, _)| d)
}

/// Writes a document as HWPX. A clean document returns its source bytes.
pub fn write(doc: &Document) -> Result<Vec<u8>, Error> {
    let source = doc
        .source
        .as_ref()
        .filter(|s| zip::is_zip(s))
        .ok_or_else(|| Error::Unsupported("saving needs an HWPX source package".into()))?;
    if doc.is_clean() {
        return Ok(source.to_vec());
    }
    Err(Error::Unsupported("saving edited HWPX documents".into()))
}

/// Reads an HWPX document and reports the markup the mapping keeps
/// uninterpreted (element or attribute paths with counts).
pub fn read_with_report(bytes: SharedBytes) -> Result<(Document, BTreeMap<String, u32>), Error> {
    read::document(bytes)
}

#[derive(Debug)]
pub enum Error {
    /// The bytes are not an HWPX package.
    NotHwpx,
    Container(String),
    Xml(String),
    Corrupt(String),
    TooLarge,
    Unsupported(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NotHwpx => f.write_str("not an HWPX document"),
            Error::Container(m) => write!(f, "container: {m}"),
            Error::Xml(m) => write!(f, "xml: {m}"),
            Error::Corrupt(m) => write!(f, "corrupt: {m}"),
            Error::TooLarge => f.write_str("member exceeds the size limit"),
            Error::Unsupported(m) => write!(f, "unsupported: {m}"),
        }
    }
}

impl std::error::Error for Error {}
