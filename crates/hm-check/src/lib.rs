//! Strict structural checks of saved HWP 5 and HWPX files.
//!
//! The readers in `hm-hwp5` and `hm-hwpx` open what Hancom opens and more:
//! they skip damaged records, follow broken chains as far as they go and
//! default what is missing. These checks do not. They parse the container
//! and the record or XML tree again on their own and report every count,
//! chain, reference and nesting rule a file breaks, so a save can be tested
//! for damage without Hancom. Errors are what Hancom rejects or repairs;
//! warnings are departures from what Hancom writes that it is known to
//! accept.

mod cfb;
mod hwp5;
mod hwpx;
mod xml;
mod zip;

pub use hwp5::tag_counts;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    Warning,
    Error,
}

#[derive(Clone, Debug)]
pub struct Issue {
    pub severity: Severity,
    /// What part of the file broke the rule (`cfb`, `docinfo`, `body`, `zip`, ...).
    pub area: &'static str,
    pub message: String,
}

impl std::fmt::Display for Issue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        write!(f, "{s} [{}] {}", self.area, self.message)
    }
}

/// The findings for one file.
#[derive(Clone, Debug, Default)]
pub struct Report {
    /// `hwp5`, `hwpx` or `hml` once recognized.
    pub format: Option<&'static str>,
    pub issues: Vec<Issue>,
}

/// Issues kept per area; a damaged table or section repeats one rule
/// thousands of times.
const AREA_LIMIT: usize = 64;

impl Report {
    pub fn errors(&self) -> impl Iterator<Item = &Issue> {
        self.issues.iter().filter(|i| i.severity == Severity::Error)
    }

    /// True when no rule Hancom enforces is broken.
    pub fn is_ok(&self) -> bool {
        self.errors().next().is_none()
    }

    fn push(&mut self, severity: Severity, area: &'static str, message: String) {
        if self.issues.iter().filter(|i| i.area == area).count() >= AREA_LIMIT {
            return;
        }
        self.issues.push(Issue {
            severity,
            area,
            message,
        });
    }

    pub(crate) fn error(&mut self, area: &'static str, message: impl Into<String>) {
        self.push(Severity::Error, area, message.into());
    }

    pub(crate) fn warn(&mut self, area: &'static str, message: impl Into<String>) {
        self.push(Severity::Warning, area, message.into());
    }
}

/// Checks one saved file, recognizing its format from its first bytes.
pub fn check(bytes: &[u8]) -> Report {
    let mut r = Report::default();
    if bytes.starts_with(b"HWP Document File V") {
        r.format = Some("hwp3");
        r.error("format", "an HWP 3 file; nothing writes this format");
    } else if bytes.starts_with(&[0xD0, 0xCF, 0x11, 0xE0]) {
        r.format = Some("hwp5");
        hwp5::check(bytes, &mut r);
    } else if bytes.starts_with(b"PK\x03\x04") {
        r.format = Some("hwpx");
        hwpx::check(bytes, &mut r);
    } else if looks_like_xml(bytes) {
        r.format = Some("hml");
        if let Err(e) = xml::parse(bytes) {
            r.error("xml", format!("HML: {e}"));
        }
    } else {
        r.error("format", "not an HWP 5, HWPX or HML file");
    }
    r
}

fn looks_like_xml(bytes: &[u8]) -> bool {
    let b = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let start = b.iter().position(|c| !c.is_ascii_whitespace()).unwrap_or(0);
    b[start..].starts_with(b"<")
}

/// IEEE CRC-32, as ZIP and Hancom's deflate trailer use it.
pub(crate) fn crc32(data: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, slot) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
            *slot = c;
        }
        t
    });
    let mut c = !0u32;
    for &b in data {
        c = table[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    !c
}

/// Inflates a raw deflate stream to its end; returns the output and the
/// number of input bytes the stream used. A stream that ends before its
/// final block, or decodes past `limit`, is an error.
pub(crate) fn inflate_raw(data: &[u8], limit: usize) -> Result<(Vec<u8>, usize), String> {
    use miniz_oxide::inflate::stream::{InflateState, inflate};
    use miniz_oxide::{DataFormat, MZFlush, MZStatus};
    let mut state = InflateState::new_boxed(DataFormat::Raw);
    let mut out = Vec::new();
    let mut buf = vec![0u8; 1 << 16];
    let mut input = data;
    let mut used = 0usize;
    loop {
        let res = inflate(&mut state, input, &mut buf, MZFlush::None);
        used += res.bytes_consumed;
        input = &input[res.bytes_consumed..];
        out.extend_from_slice(&buf[..res.bytes_written]);
        if out.len() > limit {
            return Err(format!("inflates past {limit} bytes"));
        }
        match res.status {
            Ok(MZStatus::StreamEnd) => return Ok((out, used)),
            Ok(_) => {
                if res.bytes_consumed == 0 && res.bytes_written == 0 {
                    return Err(format!(
                        "deflate data ends before its final block ({} bytes in, {} out)",
                        used,
                        out.len()
                    ));
                }
            }
            Err(e) => {
                return Err(format!(
                    "deflate data is damaged at input byte {used}: {e:?}"
                ));
            }
        }
    }
}

pub(crate) fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        b.get(o..o.checked_add(2)?)?.try_into().ok()?,
    ))
}

pub(crate) fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        b.get(o..o.checked_add(4)?)?.try_into().ok()?,
    ))
}

pub(crate) fn u64_at(b: &[u8], o: usize) -> Option<u64> {
    Some(u64::from_le_bytes(
        b.get(o..o.checked_add(8)?)?.try_into().ok()?,
    ))
}
