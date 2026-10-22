//! Hancom's HFT font files ("Han Unified Font File 1.0").
//!
//! A file holds one script range of a face: a 512-byte header with Johab
//! names, advance tables keyed by HChar, and records of bitmap strikes and
//! outline sets. Outlines are a small drawing program, obfuscated with a
//! rolling XOR in two of the four flavors.

use crate::FontData;
use crate::face::PathOp;
use crate::hchar::{self, decode_johab_bytes};

const MAGIC: &[u8] = b"Han Unified Font File 1.0";
const HEADER: usize = 0x200;

/// The loader and stream cipher a file asks for (header offset 0x134).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flavor {
    Plain,
    Human,
    Hanyang,
    Mapsi,
}

impl Flavor {
    fn from_header(raw: &[u8]) -> Flavor {
        let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
        match &raw[..end] {
            b"Hanyang outline font for HWP 2.1" => Flavor::Hanyang,
            b"Hangul Mapsi font for HWP 2.5" => Flavor::Mapsi,
            [] => Flavor::Plain,
            _ => Flavor::Human,
        }
    }

    /// Seed and increment of the rolling XOR, when the flavor has one.
    fn cipher(self) -> Option<(u32, u32)> {
        match self {
            Flavor::Hanyang => Some((0xE696, 0xC863)),
            Flavor::Mapsi => Some((0xA729, 0xE696)),
            Flavor::Plain | Flavor::Human => None,
        }
    }
}

/// How an element maps character codes to glyph slots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Codes {
    /// Slot is the code minus the element's first code.
    Range,
    /// Slot is the KS X 1001 syllable index (the block refers to Hancom's
    /// global KS table).
    Ks,
    /// Slot is the position in a list of `count` u16 codes at this offset.
    List(usize),
}

/// One glyph set of an outline record.
#[derive(Clone, Debug)]
struct Element {
    flags: u16,
    first: u16,
    last: u16,
    count: u16,
    /// Cell metrics in font units, used when bit 4 of `flags` is set.
    bbox: [i16; 4],
    /// Start of the per-slot offset table.
    table: usize,
    codes: Codes,
    /// The record's design em.
    em: i32,
}

/// A glyph of one file: element and slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GlyphRef {
    pub element: u16,
    pub slot: u16,
}

/// A decoded outline in the file's font units, y up, with the glyph's
/// declared box `(x0, y0, width, height)`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HftOutline {
    pub ops: Vec<PathOp>,
    pub bbox: [i16; 4],
    pub em: i32,
}

/// A parsed HFT file.
pub struct HftFile {
    data: FontData,
    /// Family name (header 0x2C) and face name (0x6C).
    pub family: String,
    pub face_name: String,
    pub em: i32,
    pub flavor: Flavor,
    /// Styles the face supports itself, tested as `1 << style & mask`.
    pub styles: u32,
    /// Payload ranges of the advance blocks, in file order.
    widths: Vec<(usize, usize)>,
    elements: Vec<Element>,
}

impl std::fmt::Debug for HftFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HftFile")
            .field("family", &self.family)
            .field("em", &self.em)
            .field("flavor", &self.flavor)
            .finish()
    }
}

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}

fn i16_at(b: &[u8], o: usize) -> Option<i16> {
    u16_at(b, o).map(|v| v as i16)
}

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}

/// True when the bytes start like an HFT file.
pub fn is_hft(bytes: &[u8]) -> bool {
    bytes.starts_with(MAGIC)
}

/// Why a file was rejected, as Hancom's loader distinguishes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HftError {
    Magic,
    Size,
    Checksum,
    Structure,
}

impl HftFile {
    pub fn parse(data: FontData) -> Result<HftFile, HftError> {
        let b: &[u8] = (*data).as_ref();
        if !b.starts_with(MAGIC) {
            return Err(HftError::Magic);
        }
        if b.len() < HEADER {
            return Err(HftError::Size);
        }
        let declared = u32_at(b, 0x24).ok_or(HftError::Size)? as usize;
        if declared > b.len() {
            return Err(HftError::Size);
        }
        let stored = u16_at(b, 0x2A).ok_or(HftError::Size)?;
        let sum = b[..HEADER]
            .iter()
            .enumerate()
            .filter(|&(i, _)| i != 0x2A && i != 0x2B)
            .fold(0u16, |s, (_, &v)| s.wrapping_add(v as u16));
        if sum != stored {
            return Err(HftError::Checksum);
        }
        let em = i16_at(b, 0x17A).ok_or(HftError::Structure)? as i32;
        let mut file = HftFile {
            family: decode_johab_bytes(&b[0x2C..0x4C]),
            face_name: decode_johab_bytes(&b[0x6C..0x8C]),
            em,
            flavor: Flavor::from_header(&b[0x134..0x154]),
            styles: u32_at(b, 0x1A0).ok_or(HftError::Structure)?,
            widths: Vec::new(),
            elements: Vec::new(),
            data: data.clone(),
        };
        // Advance blocks: `{u32 length including itself, payload}`.
        let blocks = u16_at(b, 0x1A4).ok_or(HftError::Structure)?;
        let mut at = u32_at(b, 0x1AA).ok_or(HftError::Structure)? as usize;
        for _ in 0..blocks {
            let Some(len) = u32_at(b, at).map(|l| l as usize) else {
                break;
            };
            if len < 10 || at + len > b.len() {
                break;
            }
            file.widths.push((at + 4, at + len));
            at += len;
        }
        // Records: `{u32 next, u16 code, u16 type, u16 count, u16 rel}`,
        // chained by relative offsets. Type 0 is a bitmap strike; the rest
        // are outline sets whose code is the design em.
        let records =
            u16_at(b, 0x1A6).unwrap_or(0) as usize + u16_at(b, 0x1A8).unwrap_or(0) as usize;
        let mut rec = u32_at(b, 0x1AE).ok_or(HftError::Structure)? as usize;
        for _ in 0..records {
            if rec == 0 || rec + 12 > b.len() {
                break;
            }
            let next = u32_at(b, rec).unwrap() as usize;
            let code = u16_at(b, rec + 4).unwrap() as i32;
            let kind = u16_at(b, rec + 6).unwrap();
            let count = u16_at(b, rec + 8).unwrap();
            let rel = u16_at(b, rec + 10).unwrap() as usize;
            if kind != 0 {
                let rec_em = if code > 0 { code } else { em };
                file.read_elements(b, rec + rel, count, rec_em);
            }
            if next == 0 {
                break;
            }
            rec += next;
        }
        Ok(file)
    }

    fn read_elements(&mut self, b: &[u8], mut at: usize, count: u16, em: i32) {
        for _ in 0..count {
            if at + 22 > b.len() {
                return;
            }
            let next = u32_at(b, at).unwrap() as usize;
            let flags = u16_at(b, at + 4).unwrap();
            let first = u16_at(b, at + 6).unwrap();
            let last = u16_at(b, at + 8).unwrap();
            let slots = u16_at(b, at + 10).unwrap();
            let bbox = [
                i16_at(b, at + 14).unwrap(),
                i16_at(b, at + 16).unwrap(),
                i16_at(b, at + 18).unwrap(),
                i16_at(b, at + 20).unwrap(),
            ];
            let data = at + 22;
            // Kinds 1, 2 and 4 put a `{u32 size, payload}` block before the
            // offset table; bit 31 marks a reference to Hancom's KS table.
            let (table, codes) = match flags & 0xF {
                1 | 2 | 4 => {
                    let Some(size) = u32_at(b, data) else { return };
                    let len = (size & 0xFFFF) as usize;
                    let codes = if size & 0x8000_0000 != 0 {
                        Codes::Ks
                    } else if flags & 0xF == 1 {
                        Codes::List(data + 4)
                    } else {
                        Codes::Range
                    };
                    (data + len.max(4), codes)
                }
                _ => (data, Codes::Range),
            };
            self.elements.push(Element {
                flags,
                first,
                last,
                count: slots,
                bbox,
                table,
                codes,
                em,
            });
            if next == 0 {
                return;
            }
            at += next;
        }
    }

    fn bytes(&self) -> &[u8] {
        (*self.data).as_ref()
    }

    /// The glyph for an HChar code, when an outline element has one.
    pub fn glyph(&self, code: u16) -> Option<GlyphRef> {
        let b = self.bytes();
        for (ei, e) in self.elements.iter().enumerate() {
            if code < e.first || code > e.last {
                continue;
            }
            let slot = match e.codes {
                Codes::Range => (code - e.first) as usize,
                Codes::Ks => match hchar::ks_search(code) {
                    (i, true) => i,
                    _ => continue,
                },
                Codes::List(list) => {
                    let found = (0..e.count as usize)
                        .find(|&i| u16_at(b, list + 2 * i) == Some(code));
                    match found {
                        Some(i) => i,
                        None => continue,
                    }
                }
            };
            if slot >= e.count as usize {
                continue;
            }
            match u32_at(b, e.table + 4 * slot) {
                Some(off) if off != 0 && e.table + off as usize + 2 <= b.len() => {
                    return Some(GlyphRef {
                        element: ei as u16,
                        slot: slot as u16,
                    });
                }
                _ => continue,
            }
        }
        None
    }

    /// The design em of a glyph's outline record.
    pub fn glyph_em(&self, g: GlyphRef) -> i32 {
        self.elements
            .get(g.element as usize)
            .map_or(self.em, |e| e.em)
    }

    /// The advance of an HChar code in font units, by Hancom's ordered
    /// lookup: the first advance block whose range holds the code decides.
    pub fn advance(&self, code: u16) -> Option<i32> {
        let b = self.bytes();
        for &(start, end) in &self.widths {
            let p = &b[start..end];
            let (Some(first), Some(last), Some(kind)) = (u16_at(p, 0), u16_at(p, 2), u16_at(p, 4))
            else {
                continue;
            };
            if code < first || code > last {
                continue;
            }
            let at = |o: usize| i16_at(p, o).map(i32::from);
            return match kind {
                0 => at(6),
                1 if code & 0x8000 == 0 => at(6 + 2 * (code - first) as usize),
                1 if first == last => at(6),
                1 => {
                    if code != 0xD3C5 && hchar::is_in_ks(code) {
                        at(6 + 2 * hchar::ks_search(code).0)
                    } else {
                        // Missing jamo take a face metric Hancom keeps
                        // outside the file; the em stands in for it.
                        hchar::jaso_search(code)
                            .map_or(Some(self.em), |j| at(6 + 2 * 2350 + 2 * j))
                    }
                }
                2 => match component_advance(p, code) {
                    Some(w) => Some(w),
                    None => continue,
                },
                _ => None,
            };
        }
        None
    }

    /// Decodes the outline of a glyph.
    pub fn outline(&self, g: GlyphRef) -> Option<HftOutline> {
        let b = self.bytes();
        let e = self.elements.get(g.element as usize)?;
        let off = u32_at(b, e.table + 4 * g.slot as usize)? as usize;
        let mut p = e.table + off;
        // Bit 4 set: the element carries the metrics; otherwise each glyph
        // starts with its own box.
        let bbox = if e.flags & 0x10 != 0 {
            e.bbox
        } else {
            let v = [
                i16_at(b, p)?,
                i16_at(b, p + 2)?,
                i16_at(b, p + 4)?,
                i16_at(b, p + 6)?,
            ];
            p += 8;
            v
        };
        let len = u16_at(b, p)? as usize;
        let raw = b.get(p + 2..p + 2 + len)?;
        let program = match self.flavor.cipher() {
            Some((seed, add)) => decrypt(raw, seed, add),
            None => raw.to_vec(),
        };
        Some(HftOutline {
            ops: draw(&program),
            bbox,
            em: e.em,
        })
    }
}

/// Kind-2 advance blocks: Hangul widths by component groups.
fn component_advance(p: &[u8], code: u16) -> Option<i32> {
    // Index maps from Johab component values (bits 10-14, 5-9, 0-4) to
    // group table offsets; a negative vowel entry skips the block.
    const MAPS: [u8; 96] = [
        0x14, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
        0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d,
        0x1e, 0x1f, 0xff, 0xff, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0xff, 0xff, 0x06, 0x07, 0x08,
        0x09, 0x0a, 0x0b, 0xff, 0x16, 0x0c, 0x0d, 0x0e, 0x0f, 0x10, 0x11, 0x17, 0x18, 0x12, 0x13,
        0x14, 0x15, 0x19, 0x1a, 0x1c, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09,
        0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10, 0x1d, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17,
        0x18, 0x19, 0x1a, 0x1b, 0x1e, 0x1f,
    ];
    let vowel = MAPS[32 + ((code >> 5) & 31) as usize] as i8;
    if vowel < 0 {
        return None;
    }
    let initial = MAPS[((code >> 10) & 31) as usize] as usize;
    let fin = MAPS[64 + (code & 31) as usize] as usize;
    let s8 = |o: usize| p.get(o).map(|&v| v as i8 as i32);
    let a = s8(10 + initial)?;
    let v = s8(42 + vowel as usize)?;
    let c = s8(74 + fin)?;
    let index = (s8(7)?.wrapping_mul(a).wrapping_add(v)).wrapping_mul(s8(8)?) + c;
    i16_at(p, (106 + 2 * index as isize).try_into().ok()?).map(i32::from)
}

/// The rolling XOR of the Hanyang and Mapsi flavors.
fn decrypt(raw: &[u8], seed: u32, add: u32) -> Vec<u8> {
    let mut s = seed;
    raw.iter()
        .map(|&c| {
            let plain = c ^ (s >> 8) as u8;
            s = (c as u32)
                .wrapping_add(s & 0xFFFF)
                .wrapping_mul(0xC73E)
                .wrapping_add(add);
            plain
        })
        .collect()
}

struct Reader<'a> {
    b: &'a [u8],
    p: usize,
}

impl Reader<'_> {
    fn byte(&mut self) -> Option<u8> {
        let v = *self.b.get(self.p)?;
        self.p += 1;
        Some(v)
    }

    /// A packed signed coordinate delta.
    fn delta(&mut self) -> Option<i32> {
        let b = self.byte()? as i32;
        Some(match b {
            0x80 => {
                let lo = self.byte()? as i32;
                let hi = self.byte()? as i32;
                (hi << 8 | lo) as i16 as i32
            }
            0x7C..=0x7F => (b << 8 | self.byte()? as i32) - 0x7B84,
            0x81..=0x84 => (b << 8) - self.byte()? as i32 + 0x7B84 - 0x10000,
            _ => b as i8 as i32,
        })
    }

    fn skip(&mut self, n: usize) -> Option<()> {
        self.p = self.p.checked_add(n).filter(|&p| p <= self.b.len())?;
        Some(())
    }
}

/// Runs an outline program into path commands. Contours close implicitly
/// at the next move and at the end; hint and property blocks are skipped.
fn draw(program: &[u8]) -> Vec<PathOp> {
    let mut ops = Vec::new();
    let mut r = Reader { b: program, p: 0 };
    let (mut x, mut y) = (0i32, 0i32);
    let (mut sx, mut sy) = (0i32, 0i32);
    let mut open = false;
    let close = |ops: &mut Vec<PathOp>, open: &mut bool| {
        if *open {
            ops.push(PathOp::Close);
            *open = false;
        }
    };
    let pt = |v: i32| v as f32;
    loop {
        let Some(op) = r.byte() else { break };
        let ok = (|| -> Option<bool> {
            match op {
                0 => return Some(false),
                1..=3 | 5..=7 => {
                    if op & 1 != 0 {
                        x += r.delta()?;
                    }
                    if op & 2 != 0 {
                        y += r.delta()?;
                    }
                    if op < 4 {
                        close(&mut ops, &mut open);
                        ops.push(PathOp::Move(pt(x), pt(y)));
                        (sx, sy) = (x, y);
                        open = true;
                    } else {
                        ops.push(PathOp::Line(pt(x), pt(y)));
                    }
                }
                4 => {
                    (x, y) = (sx, sy);
                    ops.push(PathOp::Line(pt(x), pt(y)));
                }
                9..=11 => {
                    let (c1x, c1y, c2x, c2y);
                    match op {
                        9 => {
                            c1x = x + r.delta()?;
                            c1y = y;
                            c2x = c1x + r.delta()?;
                            c2y = y + r.delta()?;
                            x = c2x;
                            y = c2y + r.delta()?;
                        }
                        10 => {
                            c1x = x;
                            c1y = y + r.delta()?;
                            c2x = x + r.delta()?;
                            c2y = c1y + r.delta()?;
                            x = c2x + r.delta()?;
                            y = c2y;
                        }
                        _ => {
                            c1x = x + r.delta()?;
                            c1y = y + r.delta()?;
                            c2x = c1x + r.delta()?;
                            c2y = c1y + r.delta()?;
                            x = c2x + r.delta()?;
                            y = c2y + r.delta()?;
                        }
                    }
                    ops.push(PathOp::Cubic(
                        pt(c1x),
                        pt(c1y),
                        pt(c2x),
                        pt(c2y),
                        pt(x),
                        pt(y),
                    ));
                }
                0x20 => {
                    let n1 = r.byte()? as usize;
                    r.skip(2 * n1 + 1)?;
                    let n2 = r.byte()? as usize;
                    r.skip(2 * n2)?;
                }
                0x21 => {
                    let n = r.byte()?;
                    for _ in 0..n {
                        let len = r.byte()? as usize;
                        r.skip(len)?;
                    }
                }
                0x22 | 0x23 => {
                    r.byte()?;
                }
                0x40 | 0x42 => {
                    r.delta()?;
                    r.delta()?;
                }
                0x41 | 0x43 => {
                    for _ in 0..6 {
                        r.delta()?;
                    }
                }
                0x44 => {}
                _ => return Some(false),
            }
            Some(true)
        })();
        if ok != Some(true) {
            break;
        }
    }
    close(&mut ops, &mut open);
    ops
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deltas_unpack() {
        let mut r = Reader {
            b: &[0x05, 0xFB, 0x7C, 0x00, 0x84, 0x00, 0x81, 0xFF, 0x80, 0x10, 0x27],
            p: 0,
        };
        assert_eq!(r.delta(), Some(5));
        assert_eq!(r.delta(), Some(-5));
        assert_eq!(r.delta(), Some(124));
        assert_eq!(r.delta(), Some(-124));
        assert_eq!(r.delta(), Some(-1147));
        assert_eq!(r.delta(), Some(10000));
    }

    #[test]
    fn draws_a_closed_square() {
        // Move to (10, 10), lines to (110, 10), (110, 110), back to start.
        let ops = draw(&[3, 10, 10, 5, 100, 6, 100, 4, 0]);
        assert_eq!(
            ops,
            vec![
                PathOp::Move(10.0, 10.0),
                PathOp::Line(110.0, 10.0),
                PathOp::Line(110.0, 110.0),
                PathOp::Line(10.0, 10.0),
                PathOp::Close,
            ]
        );
    }

    #[test]
    fn rolling_xor_round_trips_its_definition() {
        // Encrypting with the inverse of the step gives back the input.
        let plain = [3u8, 10, 10, 5, 100, 0];
        let (seed, add) = Flavor::Hanyang.cipher().unwrap();
        let mut s = seed;
        let cipher: Vec<u8> = plain
            .iter()
            .map(|&p| {
                let c = p ^ (s >> 8) as u8;
                s = (c as u32)
                    .wrapping_add(s & 0xFFFF)
                    .wrapping_mul(0xC73E)
                    .wrapping_add(add);
                c
            })
            .collect();
        assert_eq!(decrypt(&cipher, seed, add), plain);
    }
}
