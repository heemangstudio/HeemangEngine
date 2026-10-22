//! ZIP containers: reading members (stored or deflated, zip64 included) and
//! writing archives that copy untouched members byte for byte.

use std::sync::Arc;

use hm_model::SharedBytes;
use miniz_oxide::inflate::{self, TINFLStatus};

use crate::Error;

const LOCAL_SIG: u32 = 0x0403_4b50;
const CENTRAL_SIG: u32 = 0x0201_4b50;
const EOCD_SIG: u32 = 0x0605_4b50;
const ZIP64_EOCD_SIG: u32 = 0x0606_4b50;
const ZIP64_LOCATOR_SIG: u32 = 0x0706_4b50;
const DESCRIPTOR_SIG: u32 = 0x0807_4b50;

/// Largest member we inflate.
const MEMBER_LIMIT: usize = 512 << 20;

pub const STORED: u16 = 0;
pub const DEFLATED: u16 = 8;

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}

fn u64_at(b: &[u8], o: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(o..o + 8)?.try_into().ok()?))
}

fn corrupt(m: &str) -> Error {
    Error::Container(m.into())
}

/// One member as listed in the central directory.
#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub method: u16,
    pub flags: u16,
    pub crc: u32,
    pub compressed_size: u64,
    pub size: u64,
    /// Offset of the local header.
    pub local_offset: u64,
    /// Offset of the member data, after the local header.
    pub data_offset: u64,
    /// End of the member's local record: data plus any data descriptor.
    pub record_end: u64,
    /// The central directory record as stored.
    pub central: SharedBytes,
}

impl Entry {
    pub fn is_dir(&self) -> bool {
        self.name.ends_with('/')
    }
}

/// A parsed archive over shared file bytes.
#[derive(Clone, Debug)]
pub struct Archive {
    pub bytes: SharedBytes,
    pub entries: Vec<Entry>,
    /// The archive comment from the end record.
    pub comment: SharedBytes,
}

/// True when `bytes` start like a ZIP archive.
pub fn is_zip(bytes: &[u8]) -> bool {
    u32_at(bytes, 0) == Some(LOCAL_SIG)
}

impl Archive {
    pub fn open(bytes: SharedBytes) -> Result<Self, Error> {
        let b: &[u8] = &bytes;
        let eocd = find_eocd(b).ok_or_else(|| corrupt("no end of central directory"))?;
        let mut count = u16_at(b, eocd + 10).unwrap_or(0) as u64;
        let mut cd_size = u32_at(b, eocd + 12).unwrap_or(0) as u64;
        let mut cd_offset = u32_at(b, eocd + 16).unwrap_or(0) as u64;
        let comment_len = u16_at(b, eocd + 20).unwrap_or(0) as usize;
        let comment_end = (eocd + 22 + comment_len).min(b.len());
        let comment = bytes.slice(eocd + 22..comment_end);
        if (count == 0xFFFF || cd_size == 0xFFFF_FFFF || cd_offset == 0xFFFF_FFFF) && eocd >= 20 {
            let loc = eocd - 20;
            if u32_at(b, loc) == Some(ZIP64_LOCATOR_SIG) {
                let rec = u64_at(b, loc + 8).unwrap_or(u64::MAX) as usize;
                if u32_at(b, rec) == Some(ZIP64_EOCD_SIG) {
                    count = u64_at(b, rec + 32).unwrap_or(count);
                    cd_size = u64_at(b, rec + 40).unwrap_or(cd_size);
                    cd_offset = u64_at(b, rec + 48).unwrap_or(cd_offset);
                }
            }
        }
        let cd_end = cd_offset
            .checked_add(cd_size)
            .filter(|&e| e <= b.len() as u64)
            .ok_or_else(|| corrupt("central directory out of range"))?;
        let mut entries = Vec::with_capacity(count.min(1 << 16) as usize);
        let mut at = cd_offset as usize;
        while at + 46 <= cd_end as usize && u32_at(b, at) == Some(CENTRAL_SIG) {
            let name_len = u16_at(b, at + 28).unwrap() as usize;
            let extra_len = u16_at(b, at + 30).unwrap() as usize;
            let comment_len = u16_at(b, at + 32).unwrap() as usize;
            let end = at + 46 + name_len + extra_len + comment_len;
            if end > b.len() {
                return Err(corrupt("central record out of range"));
            }
            let raw_name = &b[at + 46..at + 46 + name_len];
            let extra = &b[at + 46 + name_len..at + 46 + name_len + extra_len];
            let mut size = u32_at(b, at + 24).unwrap() as u64;
            let mut compressed_size = u32_at(b, at + 20).unwrap() as u64;
            let mut local_offset = u32_at(b, at + 42).unwrap() as u64;
            apply_zip64(extra, &mut size, &mut compressed_size, &mut local_offset);
            let flags = u16_at(b, at + 8).unwrap();
            let name = if flags & 0x800 != 0 || raw_name.is_ascii() {
                String::from_utf8_lossy(raw_name).into_owned()
            } else {
                raw_name.iter().map(|&c| c as char).collect()
            };
            let lo = local_offset as usize;
            if u32_at(b, lo) != Some(LOCAL_SIG) {
                return Err(corrupt("missing local header"));
            }
            let data_offset = (lo + 30) as u64
                + u16_at(b, lo + 26).unwrap_or(0) as u64
                + u16_at(b, lo + 28).unwrap_or(0) as u64;
            let data_end = data_offset.saturating_add(compressed_size);
            if data_end > b.len() as u64 {
                return Err(corrupt("member data out of range"));
            }
            let record_end = if flags & 8 != 0 {
                data_end + descriptor_len(b, data_end as usize, size, compressed_size)
            } else {
                data_end
            };
            entries.push(Entry {
                name,
                method: u16_at(b, at + 10).unwrap(),
                flags,
                crc: u32_at(b, at + 16).unwrap(),
                compressed_size,
                size,
                local_offset,
                data_offset,
                record_end,
                central: bytes.slice(at..end),
            });
            at = end;
        }
        Ok(Self {
            bytes,
            entries,
            comment,
        })
    }

    pub fn find(&self, name: &str) -> Option<usize> {
        self.entries
            .iter()
            .position(|e| e.name == name)
            .or_else(|| {
                self.entries
                    .iter()
                    .position(|e| e.name.eq_ignore_ascii_case(name))
            })
    }

    /// The member's stored (possibly compressed) bytes.
    pub fn raw(&self, i: usize) -> SharedBytes {
        let e = &self.entries[i];
        self.bytes
            .slice(e.data_offset as usize..(e.data_offset + e.compressed_size) as usize)
    }

    /// The member's content. Stored members are sliced without copying.
    pub fn read(&self, i: usize) -> Result<SharedBytes, Error> {
        let e = &self.entries[i];
        let raw = self.raw(i);
        match e.method {
            STORED => Ok(raw),
            DEFLATED => {
                let limit = (e.size as usize).clamp(1, MEMBER_LIMIT);
                let v = match inflate::decompress_to_vec_with_limit(&raw, limit) {
                    Ok(v) => v,
                    Err(err) if err.status == TINFLStatus::HasMoreOutput => {
                        return Err(Error::TooLarge);
                    }
                    Err(err) => {
                        return Err(corrupt(&format!("{}: deflate {:?}", e.name, err.status)));
                    }
                };
                Ok(SharedBytes::new(Arc::from(v)))
            }
            m => Err(Error::Unsupported(format!(
                "{}: compression method {m}",
                e.name
            ))),
        }
    }

    pub fn read_name(&self, name: &str) -> Result<Option<SharedBytes>, Error> {
        self.find(name).map(|i| self.read(i)).transpose()
    }
}

fn find_eocd(b: &[u8]) -> Option<usize> {
    if b.len() < 22 {
        return None;
    }
    let lowest = b.len().saturating_sub(22 + 0xFFFF);
    (lowest..=b.len() - 22)
        .rev()
        .find(|&i| u32_at(b, i) == Some(EOCD_SIG))
}

/// Reads the zip64 extended information field: each value stored as all ones
/// in the fixed record follows here in order.
fn apply_zip64(extra: &[u8], size: &mut u64, csize: &mut u64, offset: &mut u64) {
    let mut at = 0;
    while at + 4 <= extra.len() {
        let id = u16_at(extra, at).unwrap();
        let len = u16_at(extra, at + 2).unwrap() as usize;
        let body = extra.get(at + 4..at + 4 + len).unwrap_or(&[]);
        if id == 1 {
            let mut k = 0;
            for v in [size, csize, offset] {
                if *v == 0xFFFF_FFFF {
                    if let Some(x) = u64_at(body, k) {
                        *v = x;
                    }
                    k += 8;
                }
            }
            return;
        }
        at += 4 + len;
    }
}

fn descriptor_len(b: &[u8], at: usize, size: u64, csize: u64) -> u64 {
    let signed = u32_at(b, at) == Some(DESCRIPTOR_SIG);
    let base = if signed { 4 } else { 0 };
    let wide = size > 0xFFFF_FFFF || csize > 0xFFFF_FFFF;
    base + if wide { 20 } else { 12 }
}

/// A member to write.
pub enum Member<'a> {
    /// An entry of the source archive, copied with its local record.
    Copy(usize),
    /// New content under a name. Deflated unless `stored`.
    New {
        name: &'a str,
        data: &'a [u8],
        stored: bool,
    },
}

/// Writes an archive. Copied members keep their local header, data and
/// descriptor exactly; their central records are copied with the local
/// offset patched.
pub fn write(src: Option<&Archive>, members: &[Member<'_>]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    let mut count = 0u64;
    for m in members {
        let offset = out.len() as u64;
        match m {
            Member::Copy(i) => {
                let a = src.expect("copying needs a source archive");
                let e = &a.entries[*i];
                out.extend_from_slice(&a.bytes[e.local_offset as usize..e.record_end as usize]);
                let mut rec = e.central.to_vec();
                patch_offset(&mut rec, offset);
                central.extend_from_slice(&rec);
            }
            Member::New { name, data, stored } => {
                let crc = crc32(data);
                let (method, body) = if *stored || data.is_empty() {
                    (STORED, std::borrow::Cow::Borrowed(*data))
                } else {
                    let z = miniz_oxide::deflate::compress_to_vec(data, 6);
                    (DEFLATED, std::borrow::Cow::Owned(z))
                };
                let wide = offset > 0xFFFF_FFFE
                    || data.len() as u64 > 0xFFFF_FFFE
                    || body.len() as u64 > 0xFFFF_FFFE;
                let flags: u16 = if name.is_ascii() { 0 } else { 0x800 };
                let version: u16 = if wide { 45 } else { 20 };
                let (csz, usz) = if wide {
                    (0xFFFF_FFFFu32, 0xFFFF_FFFFu32)
                } else {
                    (body.len() as u32, data.len() as u32)
                };
                let mut local_extra = Vec::new();
                if wide {
                    local_extra.extend_from_slice(&1u16.to_le_bytes());
                    local_extra.extend_from_slice(&16u16.to_le_bytes());
                    local_extra.extend_from_slice(&(data.len() as u64).to_le_bytes());
                    local_extra.extend_from_slice(&(body.len() as u64).to_le_bytes());
                }
                // Hancom writes 1980-01-01 00:00 for every member.
                let (time, date) = (0u16, 0x21u16);
                out.extend_from_slice(&LOCAL_SIG.to_le_bytes());
                for v in [version, flags, method, time, date] {
                    out.extend_from_slice(&v.to_le_bytes());
                }
                for v in [crc, csz, usz] {
                    out.extend_from_slice(&v.to_le_bytes());
                }
                out.extend_from_slice(&(name.len() as u16).to_le_bytes());
                out.extend_from_slice(&(local_extra.len() as u16).to_le_bytes());
                out.extend_from_slice(name.as_bytes());
                out.extend_from_slice(&local_extra);
                out.extend_from_slice(&body);

                let mut central_extra = Vec::new();
                let off32 = if wide { 0xFFFF_FFFFu32 } else { offset as u32 };
                if wide {
                    central_extra.extend_from_slice(&1u16.to_le_bytes());
                    central_extra.extend_from_slice(&24u16.to_le_bytes());
                    central_extra.extend_from_slice(&(data.len() as u64).to_le_bytes());
                    central_extra.extend_from_slice(&(body.len() as u64).to_le_bytes());
                    central_extra.extend_from_slice(&offset.to_le_bytes());
                }
                central.extend_from_slice(&CENTRAL_SIG.to_le_bytes());
                for v in [version, version, flags, method, time, date] {
                    central.extend_from_slice(&v.to_le_bytes());
                }
                for v in [crc, csz, usz] {
                    central.extend_from_slice(&v.to_le_bytes());
                }
                for v in [name.len() as u16, central_extra.len() as u16, 0, 0, 0] {
                    central.extend_from_slice(&v.to_le_bytes());
                }
                central.extend_from_slice(&0u32.to_le_bytes());
                central.extend_from_slice(&off32.to_le_bytes());
                central.extend_from_slice(name.as_bytes());
                central.extend_from_slice(&central_extra);
            }
        }
        count += 1;
    }
    let cd_offset = out.len() as u64;
    let cd_size = central.len() as u64;
    out.extend_from_slice(&central);
    let comment: &[u8] = src.map_or(&[], |a| &a.comment);
    let wide = count >= 0xFFFF || cd_offset >= 0xFFFF_FFFF || cd_size >= 0xFFFF_FFFF;
    if wide {
        let rec = out.len() as u64;
        out.extend_from_slice(&ZIP64_EOCD_SIG.to_le_bytes());
        out.extend_from_slice(&44u64.to_le_bytes());
        out.extend_from_slice(&45u16.to_le_bytes());
        out.extend_from_slice(&45u16.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        for v in [count, count, cd_size, cd_offset] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&ZIP64_LOCATOR_SIG.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&rec.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
    }
    let c16 = count.min(0xFFFF) as u16;
    out.extend_from_slice(&EOCD_SIG.to_le_bytes());
    for v in [0u16, 0, c16, c16] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&(cd_size.min(0xFFFF_FFFF) as u32).to_le_bytes());
    out.extend_from_slice(&(cd_offset.min(0xFFFF_FFFF) as u32).to_le_bytes());
    out.extend_from_slice(&(comment.len().min(0xFFFF) as u16).to_le_bytes());
    out.extend_from_slice(&comment[..comment.len().min(0xFFFF)]);
    out
}

/// Points a copied central record at its new local header.
fn patch_offset(rec: &mut [u8], offset: u64) {
    if u32_at(rec, 42) == Some(0xFFFF_FFFF) {
        // The offset lives in the zip64 field, after the sizes that overflow.
        let name_len = u16_at(rec, 28).unwrap_or(0) as usize;
        let extra_len = u16_at(rec, 30).unwrap_or(0) as usize;
        let start = 46 + name_len;
        let mut at = start;
        while at + 4 <= start + extra_len {
            let id = u16_at(rec, at).unwrap();
            let len = u16_at(rec, at + 2).unwrap() as usize;
            if id == 1 {
                let mut k = at + 4;
                for field in [24, 20] {
                    if u32_at(rec, field) == Some(0xFFFF_FFFF) {
                        k += 8;
                    }
                }
                if k + 8 <= at + 4 + len {
                    rec[k..k + 8].copy_from_slice(&offset.to_le_bytes());
                }
                return;
            }
            at += 4 + len;
        }
    } else if offset < 0xFFFF_FFFF {
        rec[42..46].copy_from_slice(&(offset as u32).to_le_bytes());
    }
}

/// CRC-32 (IEEE 802.3, reflected), as ZIP stores it.
pub fn crc32(data: &[u8]) -> u32 {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn archive(members: &[Member<'_>]) -> Archive {
        Archive::open(SharedBytes::from(write(None, members))).unwrap()
    }

    #[test]
    fn writes_and_reads_members() {
        let big: Vec<u8> = (0..50_000u32).map(|i| (i % 7) as u8).collect();
        let a = archive(&[
            Member::New {
                name: "mimetype",
                data: b"application/hwp+zip",
                stored: true,
            },
            Member::New {
                name: "Contents/section0.xml",
                data: &big,
                stored: false,
            },
        ]);
        assert_eq!(a.entries.len(), 2);
        assert_eq!(a.entries[0].method, STORED);
        assert_eq!(a.entries[1].method, DEFLATED);
        assert_eq!(&*a.read(0).unwrap(), b"application/hwp+zip");
        assert_eq!(&*a.read(1).unwrap(), &big[..]);
        assert_eq!(a.entries[1].crc, crc32(&big));
    }

    #[test]
    fn copies_members_exactly() {
        let a = archive(&[
            Member::New {
                name: "a",
                data: b"first",
                stored: true,
            },
            Member::New {
                name: "b",
                data: b"second second second",
                stored: false,
            },
        ]);
        let same = write(Some(&a), &[Member::Copy(0), Member::Copy(1)]);
        assert_eq!(&same[..], &*a.bytes);
        let swapped = Archive::open(SharedBytes::from(write(
            Some(&a),
            &[Member::Copy(1), Member::Copy(0)],
        )))
        .unwrap();
        assert_eq!(swapped.entries[0].name, "b");
        assert_eq!(&*swapped.read(0).unwrap(), b"second second second");
        assert_eq!(&*swapped.read(1).unwrap(), b"first");
    }

    #[test]
    fn reads_zip64_fields() {
        let mut size = 0xFFFF_FFFF;
        let mut csize = 10;
        let mut offset = 0xFFFF_FFFF;
        let mut extra = vec![1, 0, 16, 0];
        extra.extend_from_slice(&(5u64 << 32).to_le_bytes());
        extra.extend_from_slice(&(7u64 << 32).to_le_bytes());
        apply_zip64(&extra, &mut size, &mut csize, &mut offset);
        assert_eq!((size, csize, offset), (5 << 32, 10, 7 << 32));
    }
}
