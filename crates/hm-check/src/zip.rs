//! Strict ZIP reading: end record, central directory, local headers,
//! member data, CRCs and the stored `mimetype` member first.

use crate::{Report, u16_at, u32_at, u64_at};

const A: &str = "zip";
const LIMIT: usize = 1 << 30;

pub struct Member {
    pub name: String,
    pub data: Vec<u8>,
}

pub fn read(b: &[u8], r: &mut Report) -> Option<Vec<Member>> {
    let Some(eocd) = (0..b.len().saturating_sub(21))
        .rev()
        .take(65_557)
        .find(|&i| u32_at(b, i) == Some(0x0605_4b50))
    else {
        r.error(A, "no end of central directory record");
        return None;
    };
    let comment = u16_at(b, eocd + 20).unwrap() as usize;
    if eocd + 22 + comment != b.len() {
        r.warn(A, "bytes after the end record");
    }
    let mut count = u16_at(b, eocd + 10).unwrap() as u64;
    let mut cd_size = u32_at(b, eocd + 12).unwrap() as u64;
    let mut cd_off = u32_at(b, eocd + 16).unwrap() as u64;
    if (count == 0xFFFF || cd_off == 0xFFFF_FFFF)
        && eocd >= 20
        && u32_at(b, eocd - 20) == Some(0x0706_4b50)
    {
        let z = u64_at(b, eocd - 12).unwrap() as usize;
        if u32_at(b, z) != Some(0x0606_4b50) {
            r.error(A, "zip64 locator points at no zip64 end record");
            return None;
        }
        count = u64_at(b, z + 32)?;
        cd_size = u64_at(b, z + 40)?;
        cd_off = u64_at(b, z + 48)?;
    }
    if cd_off
        .checked_add(cd_size)
        .is_none_or(|e| e as usize > eocd)
    {
        r.error(A, "central directory lies outside the file");
        return None;
    }
    let mut pos = cd_off as usize;
    let mut members = Vec::new();
    let mut spans: Vec<(usize, usize, String)> = Vec::new();
    for k in 0..count {
        if u32_at(b, pos) != Some(0x0201_4b50) {
            r.error(A, format!("central record {k} has no signature"));
            return None;
        }
        let flags = u16_at(b, pos + 8).unwrap();
        let method = u16_at(b, pos + 10).unwrap();
        let crc = u32_at(b, pos + 16).unwrap();
        let mut csize = u32_at(b, pos + 20).unwrap() as u64;
        let mut usize_ = u32_at(b, pos + 24).unwrap() as u64;
        let nlen = u16_at(b, pos + 28).unwrap() as usize;
        let xlen = u16_at(b, pos + 30).unwrap() as usize;
        let clen = u16_at(b, pos + 32).unwrap() as usize;
        let mut local = u32_at(b, pos + 42).unwrap() as u64;
        let Some(name_bytes) = b.get(pos + 46..pos + 46 + nlen) else {
            r.error(A, format!("central record {k} is truncated"));
            return None;
        };
        let name = match std::str::from_utf8(name_bytes) {
            Ok(n) => n.to_string(),
            Err(_) => {
                r.error(A, format!("member name {name_bytes:?} is not UTF-8"));
                String::from_utf8_lossy(name_bytes).into_owned()
            }
        };
        // Zip64 extra field values replace the saturated ones.
        let mut x = pos + 46 + nlen;
        let xend = x + xlen;
        while x + 4 <= xend {
            let (id, len) = (u16_at(b, x).unwrap(), u16_at(b, x + 2).unwrap() as usize);
            if id == 1 {
                let mut v = x + 4;
                if usize_ == 0xFFFF_FFFF {
                    usize_ = u64_at(b, v).unwrap_or(0);
                    v += 8;
                }
                if csize == 0xFFFF_FFFF {
                    csize = u64_at(b, v).unwrap_or(0);
                    v += 8;
                }
                if local == 0xFFFF_FFFF {
                    local = u64_at(b, v).unwrap_or(0);
                }
            }
            x += 4 + len;
        }
        pos = xend + clen;
        if name.contains('\\') || name.starts_with('/') {
            r.error(
                A,
                format!("member name {name:?} uses a backslash or root path"),
            );
        }
        let lo = local as usize;
        if u32_at(b, lo) != Some(0x0403_4b50) {
            r.error(A, format!("{name}: no local header at {lo}"));
            continue;
        }
        let lflags = u16_at(b, lo + 6).unwrap();
        let lmethod = u16_at(b, lo + 8).unwrap();
        let lnlen = u16_at(b, lo + 26).unwrap() as usize;
        let lxlen = u16_at(b, lo + 28).unwrap() as usize;
        if b.get(lo + 30..lo + 30 + lnlen) != Some(name_bytes) {
            r.error(A, format!("{name}: local header names another member"));
        }
        if lmethod != method || (lflags & 8) != (flags & 8) {
            r.error(
                A,
                format!("{name}: local header disagrees on method or flags"),
            );
        }
        if flags & 8 == 0 {
            let lcrc = u32_at(b, lo + 14).unwrap();
            let lcs = u32_at(b, lo + 18).unwrap() as u64;
            let lus = u32_at(b, lo + 22).unwrap() as u64;
            let saturated = lcs == 0xFFFF_FFFF || lus == 0xFFFF_FFFF;
            if lcrc != crc || (!saturated && (lcs != csize || lus != usize_)) {
                r.error(A, format!("{name}: local header disagrees on CRC or sizes"));
            }
        }
        if flags & 1 != 0 {
            r.error(A, format!("{name}: member is encrypted"));
            continue;
        }
        let start = lo + 30 + lnlen + lxlen;
        let end = start + csize as usize;
        if end > b.len() {
            r.error(A, format!("{name}: data runs past the end of the file"));
            continue;
        }
        spans.push((lo, end, name.clone()));
        let raw = &b[start..end];
        let data = match method {
            0 => {
                if csize != usize_ {
                    r.error(A, format!("{name}: stored member with differing sizes"));
                }
                raw.to_vec()
            }
            8 => match crate::inflate_raw(raw, LIMIT) {
                Ok((d, used)) => {
                    if used != raw.len() {
                        r.warn(
                            A,
                            format!("{name}: {} bytes after its deflate data", raw.len() - used),
                        );
                    }
                    d
                }
                Err(e) => {
                    r.error(A, format!("{name}: {e}"));
                    continue;
                }
            },
            m => {
                r.error(A, format!("{name}: compression method {m}"));
                continue;
            }
        };
        if data.len() as u64 != usize_ {
            r.error(
                A,
                format!("{name}: {} bytes, the directory says {usize_}", data.len()),
            );
        }
        if crate::crc32(&data) != crc {
            r.error(A, format!("{name}: CRC does not match its data"));
        }
        if k == 0 {
            if name != "mimetype" {
                r.error(A, format!("first member is {name:?}, not mimetype"));
            } else {
                if method != 0 {
                    r.error(A, "mimetype is compressed");
                }
                if lo != 0 {
                    r.error(A, "mimetype does not start the file");
                }
                if lxlen != 0 {
                    r.warn(A, "mimetype has an extra field");
                }
                if data != b"application/hwp+zip" {
                    r.error(
                        A,
                        format!("mimetype is {:?}", String::from_utf8_lossy(&data)),
                    );
                }
            }
        }
        if members.iter().any(|m: &Member| m.name == name) {
            r.error(A, format!("member {name:?} appears twice"));
        }
        members.push(Member { name, data });
    }
    if pos != (cd_off + cd_size) as usize {
        r.warn(A, "central directory size disagrees with its records");
    }
    spans.sort();
    for w in spans.windows(2) {
        if w[1].0 < w[0].1 {
            r.error(A, format!("members {} and {} overlap", w[0].2, w[1].2));
        }
    }
    if let Some(last) = spans.last()
        && last.1 > cd_off as usize
    {
        r.error(
            A,
            format!("member {} overlaps the central directory", last.2),
        );
    }
    Some(members)
}
