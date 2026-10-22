//! HWP 5.x reader and writer: compound file, records, DocInfo and BodyText.
//!
//! [`read`] turns file bytes into a [`Document`]; [`write`] turns a document
//! back into file bytes, copying every part that did not change.

pub mod body;
pub mod cfb;
pub mod deflate;
pub mod docinfo;
pub mod record;

use std::sync::Arc;

use hm_model::{Diagnostic, DocMeta, Document, Format, Section, SharedBytes, Version};

/// Largest structural stream we inflate (DocInfo, a section).
const STREAM_LIMIT: usize = 256 << 20;

#[derive(Debug)]
pub enum Error {
    /// The bytes are not an HWP 5 compound file.
    NotHwp5,
    Container(String),
    Corrupt(String),
    Truncated,
    TooLarge,
    /// Password-protected documents are not readable yet.
    Encrypted,
    /// A save that needs container changes the writer does not make yet.
    Unsupported(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NotHwp5 => f.write_str("not an HWP 5 document"),
            Error::Container(m) => write!(f, "container: {m}"),
            Error::Corrupt(m) => write!(f, "corrupt: {m}"),
            Error::Truncated => f.write_str("record truncated"),
            Error::TooLarge => f.write_str("stream exceeds the size limit"),
            Error::Encrypted => f.write_str("password-protected document"),
            Error::Unsupported(m) => write!(f, "unsupported: {m}"),
        }
    }
}

impl std::error::Error for Error {}

/// True when `bytes` start like a compound file.
pub fn is_hwp5(bytes: &[u8]) -> bool {
    cfb::Cfb::is_cfb(bytes)
}

/// The FileHeader fields we use.
fn file_header(raw: &[u8]) -> Result<(Version, u32), Error> {
    if raw.len() < 40 || !raw.starts_with(b"HWP Document File") {
        return Err(Error::NotHwp5);
    }
    let version = u32::from_le_bytes(raw[32..36].try_into().unwrap());
    let flags = u32::from_le_bytes(raw[36..40].try_into().unwrap());
    Ok((Version(version), flags))
}

fn stream_bytes(
    cfb: &cfb::Cfb<'_>,
    path: &str,
    compressed: bool,
) -> Result<Option<SharedBytes>, Error> {
    let Some(raw) = cfb.read(path) else {
        return Ok(None);
    };
    let data = if compressed {
        deflate::inflate(&raw, STREAM_LIMIT)?
    } else {
        raw
    };
    Ok(Some(SharedBytes::new(Arc::from(data))))
}

/// Reads an HWP 5 document.
pub fn read(bytes: SharedBytes) -> Result<Document, Error> {
    let cfb = cfb::Cfb::open(&bytes)?;
    let header = cfb.read("FileHeader").ok_or(Error::NotHwp5)?;
    let (version, flags) = file_header(&header)?;
    let meta = DocMeta {
        format: Format::Hwp5,
        version,
        flags,
    };
    if meta.password() {
        return Err(Error::Encrypted);
    }
    let mut diagnostics = Vec::new();
    if meta.distribution() {
        diagnostics.push(Diagnostic {
            message: "distribution document: the protected body is not decrypted yet".into(),
        });
    }
    let compressed = meta.compressed();
    let info_stream = stream_bytes(&cfb, "DocInfo", compressed)?
        .ok_or_else(|| Error::Corrupt("missing DocInfo".into()))?;
    let info = docinfo::parse(&info_stream, &mut diagnostics);
    let mut sections = Vec::new();
    for i in 0u32.. {
        let Some(stream) = stream_bytes(&cfb, &format!("BodyText/Section{i}"), compressed)? else {
            break;
        };
        let paras = body::parse_section(&stream, &mut diagnostics);
        sections.push(Section {
            paras,
            src: Some(stream),
            source_index: Some(i),
        });
    }
    Ok(Document {
        meta,
        info,
        sections,
        source: Some(bytes),
        diagnostics,
    })
}

/// Writes a document as HWP 5.
///
/// A clean document returns its source bytes. Otherwise the source container
/// is rebuilt with DocInfo and changed sections re-encoded and every other
/// stream copied exactly as stored.
pub fn write(doc: &Document) -> Result<Vec<u8>, Error> {
    let source = doc
        .source
        .as_ref()
        .filter(|s| is_hwp5(s))
        .ok_or_else(|| Error::Unsupported("saving needs an HWP 5 source container".into()))?;
    if doc.is_clean() {
        return Ok(source.to_vec());
    }
    let cfb = cfb::Cfb::open(source)?;
    let compressed = doc.meta.compressed();
    let encode = |plain: Vec<u8>| {
        if compressed {
            deflate::deflate(&plain)
        } else {
            plain
        }
    };

    let mut replaced: Vec<(usize, Vec<u8>)> = Vec::new();
    if !doc.info.is_clean() {
        let i = cfb
            .find("DocInfo")
            .ok_or_else(|| Error::Corrupt("missing DocInfo".into()))?;
        replaced.push((i, encode(docinfo::encode(&doc.info))));
    }
    let existing = (0u32..)
        .take_while(|i| cfb.exists(&format!("BodyText/Section{i}")))
        .count();
    if existing != doc.sections.len() {
        return Err(Error::Unsupported("adding or removing sections".into()));
    }
    for (k, s) in doc.sections.iter().enumerate() {
        let clean_here = s.is_clean() && s.source_index == Some(k as u32);
        if clean_here {
            continue;
        }
        let i = cfb
            .find(&format!("BodyText/Section{k}"))
            .expect("section stream exists");
        replaced.push((i, encode(body::encode_section(&s.paras))));
    }

    let originals: Vec<Option<Vec<u8>>> = (0..cfb.entries.len())
        .map(|i| (cfb.entries[i].kind == cfb::STREAM).then(|| cfb.read_entry(i)))
        .collect();
    let mut streams: Vec<Option<&[u8]>> = originals.iter().map(|o| o.as_deref()).collect();
    for (i, data) in &replaced {
        streams[*i] = Some(data);
    }
    Ok(cfb::write(&cfb.entries, &streams))
}
