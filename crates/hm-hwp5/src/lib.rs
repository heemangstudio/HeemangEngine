//! HWP 5.x reader and writer: compound file, records, DocInfo and BodyText.
//!
//! [`read`] turns file bytes into a [`Document`]; [`write`] turns a document
//! back into file bytes, copying every part that did not change.

mod aes;
pub mod annot;
pub mod body;
pub mod cfb;
pub mod chart;
pub mod deflate;
pub mod distribute;
pub mod docinfo;
pub mod fill;
pub mod furniture;
pub mod object;
pub mod ole;
pub mod record;
pub mod table;

use std::sync::Arc;

use hm_model::{DocMeta, Document, Format, Section, SharedBytes, Version};

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

/// A distribution document's section `i`: its ViewText stream decrypted
/// and, when the document is compressed, inflated.
fn view_section(
    cfb: &cfb::Cfb<'_>,
    i: u32,
    compressed: bool,
) -> Result<Option<SharedBytes>, Error> {
    let Some(raw) = cfb.read(&format!("ViewText/Section{i}")) else {
        return Ok(None);
    };
    let (_, payload) = distribute::open(&raw).ok_or_else(|| {
        Error::Corrupt(format!("ViewText/Section{i} lacks its distribution data"))
    })?;
    let data = if compressed {
        deflate::inflate(&payload, STREAM_LIMIT)?
    } else {
        payload
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
    let compressed = meta.compressed();
    let info_stream = stream_bytes(&cfb, "DocInfo", compressed)?
        .ok_or_else(|| Error::Corrupt("missing DocInfo".into()))?;
    let info = docinfo::parse(&info_stream, &mut diagnostics);
    let mut sections = Vec::new();
    // A distribution document's body is the protected ViewText; its
    // BodyText holds only a notice for older readers.
    let view = meta.distribution() && cfb.exists("ViewText/Section0");
    for i in 0u32.. {
        let stream = if view {
            view_section(&cfb, i, compressed)?
        } else {
            stream_bytes(&cfb, &format!("BodyText/Section{i}"), compressed)?
        };
        let Some(stream) = stream else {
            break;
        };
        let mut paras = body::parse_section(&stream, &mut diagnostics);
        annot::take_memos(&mut paras, &mut diagnostics);
        let last_page_master = furniture::last_page_master(&stream, &mut diagnostics);
        let page_masters = furniture::page_masters(&stream, &mut diagnostics);
        sections.push(Section {
            paras,
            src: Some(stream),
            source_index: Some(i),
            last_page_master,
            page_masters,
        });
    }
    let bins = Some(with_ole_previews(bin_store(&bytes, &info, compressed)));
    Ok(Document {
        meta,
        info,
        sections,
        source: Some(bytes),
        bins,
        diagnostics,
    })
}

/// BinData streams read on first use and kept. Ids are 1-based positions
/// in the BinData list; each entry names its stream `BinData/BIN%04X.ext`.
fn bin_store(
    bytes: &SharedBytes,
    info: &hm_model::DocInfo,
    compressed: bool,
) -> hm_model::BinStore {
    use std::collections::HashMap;
    use std::sync::Mutex;
    let names: Vec<Option<(String, bool)>> = info
        .bin_data
        .iter()
        .map(|b| {
            // Bits 4-5: 0 follows the document, 1 compressed, 2 stored.
            let own = match (b.attr >> 4) & 3 {
                1 => true,
                2 => false,
                _ => compressed,
            };
            b.stream_name().map(|n| (format!("BinData/{n}"), own))
        })
        .collect();
    let bytes = bytes.clone();
    let cache: Mutex<HashMap<u16, Option<SharedBytes>>> = Mutex::new(HashMap::new());
    hm_model::BinStore::new(move |id| {
        if let Some(hit) = cache.lock().unwrap().get(&id) {
            return hit.clone();
        }
        let got = (|| {
            let (name, packed) = names.get((id as usize).checked_sub(1)?)?.clone()?;
            let cfb = cfb::Cfb::open(&bytes).ok()?;
            let raw = cfb.read(&name)?;
            let data = if packed {
                deflate::inflate(&raw, STREAM_LIMIT).unwrap_or(raw)
            } else {
                raw
            };
            Some(SharedBytes::from(data))
        })();
        cache.lock().unwrap().insert(id, got.clone());
        got
    })
}

/// What the OLE storages of a store's BinData show, read on first use.
pub fn with_ole_previews(store: hm_model::BinStore) -> hm_model::BinStore {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    type Cache = Mutex<HashMap<u16, Option<Arc<hm_model::chart::OleView>>>>;
    let inner = store.clone();
    let cache: Cache = Mutex::new(HashMap::new());
    store.with_ole(move |id| {
        if let Some(hit) = cache.lock().unwrap().get(&id) {
            return hit.clone();
        }
        let got = inner.get(id).and_then(|b| ole::view(&b)).map(Arc::new);
        cache.lock().unwrap().insert(id, got.clone());
        got
    })
}

/// Writes a document read from another format (or built from nothing) as
/// a new HWP 5 file: FileHeader, DocInfo, one BodyText stream per section
/// and the embedded BinData, all compressed. Call
/// [`Document::convert_to`] first, so nothing of the other format's bytes
/// is taken for HWP 5 records.
pub fn write_new(doc: &Document) -> Result<Vec<u8>, Error> {
    if doc.source.is_some() && doc.meta.format == hm_model::Format::Hwp5 {
        return write(doc);
    }
    let version = if doc.meta.version.parts()[0] == 5 {
        doc.meta.version.0
    } else {
        0x0501_0001
    };
    let mut header = vec![0u8; 256];
    header[..17].copy_from_slice(b"HWP Document File");
    header[32..36].copy_from_slice(&version.to_le_bytes());
    // Compressed; no password, distribution or other property.
    header[36..40].copy_from_slice(&1u32.to_le_bytes());
    let mut files: Vec<(String, Vec<u8>)> = vec![
        ("FileHeader".into(), header),
        (
            "DocInfo".into(),
            deflate::deflate(&docinfo::encode(&doc.info)),
        ),
    ];
    for (k, s) in doc.sections.iter().enumerate() {
        files.push((
            format!("BodyText/Section{k}"),
            deflate::deflate(&body::encode_section(&s.paras)),
        ));
    }
    for (k, b) in doc.info.bin_data.iter().enumerate() {
        let (Some(name), Some(store)) = (b.stream_name(), doc.bins.as_ref()) else {
            continue;
        };
        if b.kind() == 0 {
            continue;
        }
        let Some(data) = store.get(k as u16 + 1) else {
            continue;
        };
        // Bits 4-5: 0 follows the document (compressed), 2 stored.
        let data = if (b.attr >> 4) & 3 == 2 {
            data.to_vec()
        } else {
            deflate::deflate(&data)
        };
        files.push((format!("BinData/{name}"), data));
    }
    Ok(cfb::build(&files))
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
    // Distribution documents keep their sections protected in ViewText; an
    // edited one goes back protected with the same code and a fresh seed.
    let body_dir = if doc.meta.distribution() && cfb.exists("ViewText/Section0") {
        "ViewText"
    } else {
        "BodyText"
    };
    let existing = (0u32..)
        .take_while(|i| cfb.exists(&format!("{body_dir}/Section{i}")))
        .count();
    if existing != doc.sections.len() {
        return Err(Error::Unsupported("adding or removing sections".into()));
    }
    for (k, s) in doc.sections.iter().enumerate() {
        let clean_here = s.is_clean() && s.source_index == Some(k as u32);
        if clean_here {
            continue;
        }
        let path = format!("{body_dir}/Section{k}");
        let i = cfb.find(&path).expect("section stream exists");
        let plain = body::encode_section(&s.paras);
        let stream = if body_dir == "ViewText" {
            let raw = cfb.read_entry(i);
            let (protection, _) = distribute::open(&raw)
                .ok_or_else(|| Error::Corrupt(format!("{path} lacks its distribution data")))?;
            let seed = deflate::crc32(&plain) ^ (k as u32).wrapping_mul(0x9E37_79B9);
            distribute::seal(&protection, seed, &encode(plain))
        } else {
            encode(plain)
        };
        replaced.push((i, stream));
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
