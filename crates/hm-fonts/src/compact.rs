//! Compact copies of TrueType faces for hosts that pay for every resident
//! byte (wasm). A copy holds the tables layout reads (character map,
//! advances, kerning, vertical metrics) for every glyph, and outlines for only the
//! glyphs it was built with. Asking for any other outline records the glyph,
//! so the host can hand the file over again and the copy grows.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

/// Tables a face reads besides outlines, and its names for the catalog.
const KEPT: [&[u8; 4]; 8] = [
    b"OS/2", b"cmap", b"head", b"hhea", b"hmtx", b"kern", b"maxp", b"name",
];

/// The glyphs a compact copy has outlines for, and those asked for since.
#[derive(Debug, Default)]
pub(crate) struct Residency {
    /// One bit per glyph id.
    bits: Vec<u64>,
    wanted: Mutex<Vec<u16>>,
    /// An outline was missing while a layout ran, so the layout read an
    /// empty outline (equations measure ink).
    missed_in_layout: AtomicBool,
}

impl Residency {
    pub fn has(&self, glyph: u16) -> bool {
        self.bits
            .get(glyph as usize / 64)
            .is_some_and(|w| w & (1 << (glyph % 64)) != 0)
    }

    pub fn want(&self, glyph: u16, in_layout: bool) {
        let mut w = self.wanted.lock().unwrap();
        if !w.contains(&glyph) {
            w.push(glyph);
        }
        if in_layout {
            self.missed_in_layout.store(true, Ordering::Relaxed);
        }
    }

    pub fn wanted(&self) -> Vec<u16> {
        self.wanted.lock().unwrap().clone()
    }

    pub fn missed_in_layout(&self) -> bool {
        self.missed_in_layout.load(Ordering::Relaxed)
    }

    /// Every resident glyph id.
    pub fn glyphs(&self) -> impl Iterator<Item = u16> + '_ {
        self.bits.iter().enumerate().flat_map(|(k, &w)| {
            let mut rest = w;
            std::iter::from_fn(move || {
                let b = (rest != 0).then(|| rest.trailing_zeros())?;
                rest &= rest - 1;
                Some((k * 64) as u16 + b as u16)
            })
        })
    }

    /// Bytes the residency itself takes.
    pub fn size(&self) -> usize {
        self.bits.len() * 8
    }
}

fn be16(d: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(d.get(at..at + 2)?.try_into().ok()?))
}

fn be32(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

/// Random access to a font file the host holds, so a compact copy reads
/// only the parts it keeps.
pub trait FontSource {
    /// Bytes in the file.
    fn size(&self) -> usize;
    /// `len` bytes from `offset`; `None` past the end.
    fn read(&self, offset: usize, len: usize) -> Option<Vec<u8>>;
}

impl FontSource for [u8] {
    fn size(&self) -> usize {
        self.len()
    }
    fn read(&self, offset: usize, len: usize) -> Option<Vec<u8>> {
        self.get(offset..offset.checked_add(len)?)
            .map(<[u8]>::to_vec)
    }
}

/// Faces in the file: the collection's count, else 1.
pub(crate) fn face_count<S: FontSource + ?Sized>(src: &S) -> Option<u32> {
    let head = src.read(0, 12)?;
    if &head[..4] == b"ttcf" {
        be32(&head, 8)
    } else {
        Some(1)
    }
}

/// True when the file's first face has TrueType outlines (not CFF, not
/// HFT).
pub(crate) fn is_truetype<S: FontSource + ?Sized>(src: &S) -> bool {
    let Some(head) = src.read(0, 4) else {
        return false;
    };
    matches!(&head[..], [0, 1, 0, 0] | b"true" | b"ttcf")
}

/// Table records `(tag, offset, length)` of face `index` of an SFNT file or
/// collection, each checked to lie inside the file.
fn tables<S: FontSource + ?Sized>(src: &S, index: u32) -> Option<Vec<([u8; 4], usize, usize)>> {
    let head = src.read(0, 12)?;
    let at = if &head[..4] == b"ttcf" {
        if index >= be32(&head, 8)? {
            return None;
        }
        be32(&src.read(12 + 4 * index as usize, 4)?, 0)? as usize
    } else if index == 0 {
        0
    } else {
        return None;
    };
    let count = be16(&src.read(at + 4, 2)?, 0)? as usize;
    let dir = src.read(at + 12, 16 * count)?;
    let mut out = Vec::with_capacity(count);
    for k in 0..count {
        let r = 16 * k;
        let tag: [u8; 4] = dir.get(r..r + 4)?.try_into().ok()?;
        let offset = be32(&dir, r + 8)? as usize;
        let length = be32(&dir, r + 12)? as usize;
        if offset.checked_add(length)? > src.size() {
            return None;
        }
        out.push((tag, offset, length));
    }
    Some(out)
}

/// Whether a host supplied an explicit sparse glyph manifest. Damaged sparse
/// files must not fall back to whole-font loading with false empty glyphs.
pub(crate) fn sparse<S: FontSource + ?Sized>(src: &S) -> bool {
    tables(src, 0).is_some_and(|records| records.iter().any(|r| &r.0 == b"HMGC"))
}

/// The glyph ids a composite glyph record places.
fn components(glyph: &[u8], out: &mut Vec<u16>) {
    const WORDS: u16 = 0x0001;
    const SCALE: u16 = 0x0008;
    const MORE: u16 = 0x0020;
    const XY_SCALE: u16 = 0x0040;
    const TWO_BY_TWO: u16 = 0x0080;
    if be16(glyph, 0).is_none_or(|n| (n as i16) >= 0) {
        return;
    }
    let mut at = 10;
    loop {
        let (Some(flags), Some(id)) = (be16(glyph, at), be16(glyph, at + 2)) else {
            return;
        };
        out.push(id);
        at += 4 + if flags & WORDS != 0 { 4 } else { 2 };
        at += if flags & SCALE != 0 {
            2
        } else if flags & XY_SCALE != 0 {
            4
        } else if flags & TWO_BY_TWO != 0 {
            8
        } else {
            0
        };
        if flags & MORE == 0 {
            return;
        }
    }
}

/// Glyph records closer than this are read together.
const READ_GAP: usize = 4096;

/// A single-face SFNT holding face `index` of a file with outlines for the
/// glyphs of `chars`, for `glyphs`, and for the glyphs their composites use,
/// and the residency of the copy. `None` when the face has no TrueType
/// outlines (CFF faces are kept whole) or the file is damaged.
pub(crate) fn build<S: FontSource + ?Sized>(
    src: &S,
    index: u32,
    chars: &[char],
    glyphs: &[u16],
) -> Option<(Vec<u8>, Residency)> {
    use skrifa::MetadataProvider;
    let records = tables(src, index)?;
    let record = |tag: &[u8; 4]| records.iter().find(|r| &r.0 == tag).copied();
    let read = |tag: &[u8; 4]| record(tag).and_then(|r| src.read(r.1, r.2));
    let glyf = record(b"glyf")?;
    let (loca, head, maxp) = (read(b"loca")?, read(b"head")?, read(b"maxp")?);
    if head.len() < 54 {
        return None;
    }
    let long = be16(&head, 50)? != 0;
    let count = be16(&maxp, 4)? as usize;
    let included = if record(b"HMGC").is_some() {
        let data = read(b"HMGC")?;
        if data.get(..4)? != b"HM01"
            || be32(&data, 4)? as usize != count
            || data.len() != 8 + count.div_ceil(8)
        {
            return None;
        }
        Some(data[8..].to_vec())
    } else {
        None
    };
    let present = |glyph: u16| {
        included.as_ref().is_none_or(|bits| {
            bits.get(glyph as usize / 8)
                .is_some_and(|b| b & (1 << (glyph % 8)) != 0)
        })
    };
    if glyphs.iter().any(|&glyph| !present(glyph)) {
        return None;
    }
    let span = |g: usize| -> Option<(usize, usize)> {
        let (a, b) = if long {
            (
                be32(&loca, g * 4)? as usize,
                be32(&loca, g * 4 + 4)? as usize,
            )
        } else {
            (
                be16(&loca, g * 2)? as usize * 2,
                be16(&loca, g * 2 + 2)? as usize * 2,
            )
        };
        (a <= b && b <= glyf.2).then_some((a, b))
    };
    let mut out_tables: Vec<([u8; 4], Vec<u8>)> = KEPT
        .iter()
        .filter(|t| **t != b"head")
        .filter_map(|t| read(t).map(|d| (**t, d)))
        .collect();
    out_tables.push((*b"head", head.clone()));
    out_tables.sort_by_key(|t| t.0);
    let mut work: Vec<u16> = glyphs.to_vec();
    {
        let probe = sfnt(&out_tables);
        let font = skrifa::FontRef::new(&probe).ok()?;
        let map = font.charmap();
        work.extend(
            chars
                .iter()
                .filter_map(|&c| map.map(c))
                .map(|g| g.to_u32() as u16),
        );
    }
    // The missing-glyph box draws for unmapped characters.
    work.push(0);
    let mut records: std::collections::BTreeMap<u16, Vec<u8>> = Default::default();
    while !work.is_empty() {
        let mut spans: Vec<(usize, usize, u16)> = work
            .drain(..)
            .filter(|&g| (g as usize) < count && present(g) && !records.contains_key(&g))
            .filter_map(|g| span(g as usize).map(|(a, b)| (a, b, g)))
            .collect();
        spans.sort_unstable();
        spans.dedup();
        let mut k = 0;
        while k < spans.len() {
            let (from, mut to) = (spans[k].0, spans[k].1);
            let mut j = k + 1;
            while j < spans.len() && spans[j].0 <= to + READ_GAP {
                to = to.max(spans[j].1);
                j += 1;
            }
            let data = if to > from {
                src.read(glyf.1 + from, to - from)?
            } else {
                Vec::new()
            };
            for &(a, b, g) in &spans[k..j] {
                let rec = data.get(a - from..b - from)?;
                let mut children = Vec::new();
                components(rec, &mut children);
                if children.iter().any(|&child| !present(child)) {
                    return None;
                }
                work.extend(children);
                records.insert(g, rec.to_vec());
            }
            k = j;
        }
    }
    let mut bits = vec![0u64; count.div_ceil(64)];
    let mut new_glyf = Vec::new();
    let mut new_loca = Vec::with_capacity((count + 1) * 4);
    for g in 0..count {
        new_loca.extend_from_slice(&(new_glyf.len() as u32).to_be_bytes());
        if let Some(rec) = records.get(&(g as u16)) {
            bits[g / 64] |= 1 << (g % 64);
            new_glyf.extend_from_slice(rec);
            new_glyf.resize(new_glyf.len().next_multiple_of(4), 0);
        }
    }
    new_loca.extend_from_slice(&(new_glyf.len() as u32).to_be_bytes());
    // Long offsets; the whole-font checksum no longer applies.
    let head = &mut out_tables.iter_mut().find(|t| &t.0 == b"head")?.1;
    head[50..52].copy_from_slice(&1u16.to_be_bytes());
    head[8..12].fill(0);
    out_tables.push((*b"glyf", new_glyf));
    out_tables.push((*b"loca", new_loca));
    out_tables.sort_by_key(|t| t.0);
    Some((
        sfnt(&out_tables),
        Residency {
            bits,
            ..Default::default()
        },
    ))
}

/// An SFNT file from tables sorted by tag.
fn sfnt(tables: &[([u8; 4], Vec<u8>)]) -> Vec<u8> {
    let n = tables.len() as u16;
    let pow = 1u16 << (15 - n.max(1).leading_zeros());
    let mut out = Vec::new();
    out.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    for v in [n, pow * 16, pow.trailing_zeros() as u16, n * 16 - pow * 16] {
        out.extend_from_slice(&v.to_be_bytes());
    }
    let mut offset = 12 + 16 * tables.len();
    for (tag, data) in tables {
        let sum = data.chunks(4).fold(0u32, |s, c| {
            let mut w = [0u8; 4];
            w[..c.len()].copy_from_slice(c);
            s.wrapping_add(u32::from_be_bytes(w))
        });
        out.extend_from_slice(tag);
        out.extend_from_slice(&sum.to_be_bytes());
        out.extend_from_slice(&(offset as u32).to_be_bytes());
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        offset += data.len().next_multiple_of(4);
    }
    for (_, data) in tables {
        out.extend_from_slice(data);
        out.resize(out.len().next_multiple_of(4), 0);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composite_records_list_their_components() {
        // Two components: word arguments with a scale, then byte arguments.
        let mut g = vec![0xFF, 0xFF];
        g.extend_from_slice(&[0; 8]);
        g.extend_from_slice(&(0x0001u16 | 0x0008 | 0x0020).to_be_bytes());
        g.extend_from_slice(&7u16.to_be_bytes());
        g.extend_from_slice(&[0; 6]);
        g.extend_from_slice(&0u16.to_be_bytes());
        g.extend_from_slice(&9u16.to_be_bytes());
        g.extend_from_slice(&[0; 2]);
        let mut out = Vec::new();
        components(&g, &mut out);
        assert_eq!(out, [7, 9]);
    }

    #[test]
    fn table_directories_follow_the_sfnt_layout() {
        let t = vec![(*b"abcd", vec![1, 2, 3]), (*b"efgh", vec![4; 8])];
        let file = sfnt(&t);
        let recs = tables(file.as_slice(), 0).unwrap();
        assert_eq!(recs.len(), 2);
        assert_eq!(&file[recs[0].1..recs[0].1 + recs[0].2], &[1, 2, 3]);
        assert_eq!(recs[1].1 % 4, 0);
        assert_eq!(be16(&file, 6), Some(32));
    }
}
