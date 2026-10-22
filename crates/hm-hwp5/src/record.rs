//! The HWP 5 record layer and little-endian field access.

use hm_model::{HwpString, SharedBytes};

use crate::Error;

/// One framed record. `start..end` covers header and payload in the stream.
#[derive(Clone, Debug)]
pub struct Record {
    pub tag: u16,
    pub level: u16,
    pub payload: SharedBytes,
    pub start: usize,
    pub end: usize,
}

/// Frames a whole stream. A truncated record ends the list and is reported
/// with its offset; everything before it is kept.
pub fn read_records(stream: &SharedBytes) -> (Vec<Record>, Option<Error>) {
    let b = stream.as_slice();
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos < b.len() {
        if b.len() - pos < 4 {
            return (
                out,
                Some(Error::Corrupt(format!("record header truncated at {pos}"))),
            );
        }
        let h = u32::from_le_bytes(b[pos..pos + 4].try_into().unwrap());
        let tag = (h & 0x3FF) as u16;
        let level = ((h >> 10) & 0x3FF) as u16;
        let mut size = (h >> 20) as usize;
        let mut body = pos + 4;
        if size == 0xFFF {
            if b.len() - body < 4 {
                return (
                    out,
                    Some(Error::Corrupt(format!("record size truncated at {pos}"))),
                );
            }
            size = u32::from_le_bytes(b[body..body + 4].try_into().unwrap()) as usize;
            body += 4;
        }
        let Some(end) = body.checked_add(size).filter(|&e| e <= b.len()) else {
            return (
                out,
                Some(Error::Corrupt(format!("record payload truncated at {pos}"))),
            );
        };
        out.push(Record {
            tag,
            level,
            payload: stream.slice(body..end),
            start: pos,
            end,
        });
        pos = end;
    }
    (out, None)
}

/// Appends a record header and payload.
pub fn write_record(out: &mut Vec<u8>, tag: u16, level: u16, payload: &[u8]) {
    let size = payload.len();
    let small = if size >= 0xFFF { 0xFFF } else { size as u32 };
    let h = (tag as u32 & 0x3FF) | ((level as u32 & 0x3FF) << 10) | (small << 20);
    out.extend_from_slice(&h.to_le_bytes());
    if size >= 0xFFF {
        out.extend_from_slice(&(size as u32).to_le_bytes());
    }
    out.extend_from_slice(payload);
}

/// Copies already framed records, shifting every level by `delta`.
pub fn copy_shifted(out: &mut Vec<u8>, src: &[u8], delta: i32) {
    if delta == 0 {
        out.extend_from_slice(src);
        return;
    }
    let mut pos = 0;
    while pos + 4 <= src.len() {
        let h = u32::from_le_bytes(src[pos..pos + 4].try_into().unwrap());
        let level = ((h >> 10) & 0x3FF) as i32;
        let new_level = (level + delta).clamp(0, 0x3FF) as u32;
        let h2 = (h & !(0x3FF << 10)) | (new_level << 10);
        out.extend_from_slice(&h2.to_le_bytes());
        let mut size = (h >> 20) as usize;
        let mut body = pos + 4;
        if size == 0xFFF && body + 4 <= src.len() {
            size = u32::from_le_bytes(src[body..body + 4].try_into().unwrap()) as usize;
            out.extend_from_slice(&src[body..body + 4]);
            body += 4;
        }
        let end = (body + size).min(src.len());
        out.extend_from_slice(&src[body..end]);
        pos = end;
    }
}

/// Reads little-endian fields from a payload, keeping the rest as a tail.
pub struct Reader<'a> {
    bytes: &'a SharedBytes,
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(bytes: &'a SharedBytes) -> Self {
        Self { bytes, pos: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.bytes.len() - self.pos
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let b: &'a [u8] = self.bytes.as_slice();
        let s = b.get(self.pos..self.pos + n).ok_or(Error::Truncated)?;
        self.pos += n;
        Ok(s)
    }

    pub fn u8(&mut self) -> Result<u8, Error> {
        Ok(self.take(1)?[0])
    }
    pub fn i8(&mut self) -> Result<i8, Error> {
        Ok(self.u8()? as i8)
    }
    pub fn u16(&mut self) -> Result<u16, Error> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    pub fn i16(&mut self) -> Result<i16, Error> {
        Ok(self.u16()? as i16)
    }
    pub fn u32(&mut self) -> Result<u32, Error> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn i32(&mut self) -> Result<i32, Error> {
        Ok(self.u32()? as i32)
    }
    pub fn array<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        Ok(self.take(N)?.try_into().unwrap())
    }
    /// A `u16` length followed by that many UTF-16 units.
    pub fn string(&mut self) -> Result<HwpString, Error> {
        let n = self.u16()? as usize;
        let b = self.take(n * 2)?;
        Ok(HwpString(
            b.chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect(),
        ))
    }
    /// `n` bytes as a shared slice.
    pub fn bytes(&mut self, n: usize) -> Result<SharedBytes, Error> {
        if self.remaining() < n {
            return Err(Error::Truncated);
        }
        let s = self.bytes.slice(self.pos..self.pos + n);
        self.pos += n;
        Ok(s)
    }

    /// Runs `f`, rewinding to the current position when it fails.
    pub fn try_read<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let at = self.pos;
        let out = f(self);
        if out.is_err() {
            self.pos = at;
        }
        out
    }

    /// Everything not read yet.
    pub fn tail(&self) -> SharedBytes {
        self.bytes.tail_from(self.pos)
    }
}

/// Builds a payload.
#[derive(Default)]
pub struct Writer(pub Vec<u8>);

impl Writer {
    pub fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    pub fn i8(&mut self, v: i8) {
        self.0.push(v as u8);
    }
    pub fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn i16(&mut self, v: i16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn i32(&mut self, v: i32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn bytes(&mut self, b: &[u8]) {
        self.0.extend_from_slice(b);
    }
    pub fn string(&mut self, s: &HwpString) {
        self.u16(s.0.len() as u16);
        for u in &s.0 {
            self.u16(*u);
        }
    }
}

/// Record tags (HWPTAG_BEGIN = 0x10).
pub mod tag {
    pub const DOCUMENT_PROPERTIES: u16 = 0x10;
    pub const ID_MAPPINGS: u16 = 0x11;
    pub const BIN_DATA: u16 = 0x12;
    pub const FACE_NAME: u16 = 0x13;
    pub const BORDER_FILL: u16 = 0x14;
    pub const CHAR_SHAPE: u16 = 0x15;
    pub const TAB_DEF: u16 = 0x16;
    pub const NUMBERING: u16 = 0x17;
    pub const BULLET: u16 = 0x18;
    pub const PARA_SHAPE: u16 = 0x19;
    pub const STYLE: u16 = 0x1A;
    pub const COMPATIBLE_DOCUMENT: u16 = 0x1E;
    pub const LAYOUT_COMPATIBILITY: u16 = 0x1F;

    pub const PARA_HEADER: u16 = 0x42;
    pub const PARA_TEXT: u16 = 0x43;
    pub const PARA_CHAR_SHAPE: u16 = 0x44;
    pub const PARA_LINE_SEG: u16 = 0x45;
    pub const PARA_RANGE_TAG: u16 = 0x46;
    pub const CTRL_HEADER: u16 = 0x47;
    pub const LIST_HEADER: u16 = 0x48;
    pub const PAGE_DEF: u16 = 0x49;
    pub const FOOTNOTE_SHAPE: u16 = 0x4A;
    pub const PAGE_BORDER_FILL: u16 = 0x4B;
    pub const TABLE: u16 = 0x4D;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framing_round_trips_including_long_records() {
        let mut out = Vec::new();
        write_record(&mut out, 0x42, 0, &[1, 2, 3]);
        write_record(&mut out, 0x43, 1, &vec![9u8; 5000]);
        write_record(&mut out, 0x44, 1, &[]);
        let s = SharedBytes::from(out.clone());
        let (recs, err) = read_records(&s);
        assert!(err.is_none());
        assert_eq!(recs.len(), 3);
        assert_eq!(
            (recs[1].tag, recs[1].level, recs[1].payload.len()),
            (0x43, 1, 5000)
        );
        assert_eq!(recs[2].end, out.len());

        let mut shifted = Vec::new();
        copy_shifted(&mut shifted, &out, 2);
        let (recs2, _) = read_records(&SharedBytes::from(shifted));
        assert_eq!(
            recs2.iter().map(|r| r.level).collect::<Vec<_>>(),
            vec![2, 3, 3]
        );
    }

    #[test]
    fn truncation_keeps_earlier_records() {
        let mut out = Vec::new();
        write_record(&mut out, 0x42, 0, &[1, 2, 3]);
        out.extend_from_slice(&[0x43, 0x04, 0x50, 0x00, 1]);
        let (recs, err) = read_records(&SharedBytes::from(out));
        assert_eq!(recs.len(), 1);
        assert!(err.is_some());
    }
}
