//! Faces read on demand from a file the host keeps
//! ([`crate::FontSystem::add_source`]).
//!
//! The catalog reads a face's name and style tables once. A face reads its
//! metric tables when it is first loaded, and an outline only its glyph
//! records, drawn from a font holding just those records. Every table a
//! face reads keeps the file's bytes, so metrics, kerning and outlines are
//! the whole file's. A file the table directory checks below do not cover
//! is registered whole.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use skrifa::GlyphId;
use skrifa::raw::{CollectionRef, FontRef, TableProvider};

use crate::FontData;
use crate::compact::{FontSource, be16, be32, component_slots, sfnt, tables};
use crate::face::Outline;

/// A font file the host keeps and reads from synchronously whenever a face
/// needs a table or a glyph.
pub type SharedSource = Arc<dyn FontSource + Send + Sync>;

/// Tables the catalog reads: names and style.
const CATALOG: [&[u8; 4]; 4] = [b"OS/2", b"head", b"name", b"post"];

/// Tables a face reads besides outlines.
const METRICS: [&[u8; 4]; 7] = [
    b"OS/2", b"cmap", b"head", b"hhea", b"hmtx", b"kern", b"maxp",
];

/// Tables whose outlines the single-glyph fonts below cannot stand in for:
/// such faces draw from all their tables.
const WHOLE_OUTLINES: [&[u8; 4]; 2] = [b"VARC", b"gvar"];

type Records = Vec<([u8; 4], usize, usize)>;

/// The table records of every face of the file, when each face reads the
/// same tables through its records here as through the whole file: a
/// collection header and table directories the font reader accepts, every
/// table inside the file at a nonzero offset, and no tag twice.
pub(crate) fn faces<S: FontSource + ?Sized>(src: &S) -> Option<Vec<Records>> {
    let head = src.read(0, 12)?;
    let count = if &head[..4] == b"ttcf" {
        let n = be32(&head, 8)? as usize;
        // The header with the version 2 signature fields, when the file is
        // that long.
        let len = n.checked_mul(4)?.checked_add(24)?.min(src.size());
        let header = src.read(0, len)?;
        if CollectionRef::new(&header).ok()?.len() as usize != n {
            return None;
        }
        n as u32
    } else {
        1
    };
    (0..count)
        .map(|index| {
            let records = tables(src, index)?;
            let at = if &head[..4] == b"ttcf" {
                be32(&src.read(12 + 4 * index as usize, 4)?, 0)? as usize
            } else {
                0
            };
            let version = src.read(at, 4)?;
            if !matches!(&version[..], [0, 1, 0, 0] | b"OTTO" | b"true") {
                return None;
            }
            let mut tags: Vec<[u8; 4]> = records.iter().map(|r| r.0).collect();
            tags.sort_unstable();
            tags.dedup();
            if tags.len() != records.len() || records.iter().any(|r| r.1 == 0) {
                return None;
            }
            Some(records)
        })
        .collect()
}

fn subset<S: FontSource + ?Sized>(
    src: &S,
    records: &Records,
    keep: &[&[u8; 4]],
) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    for r in records.iter().filter(|r| keep.contains(&&r.0)) {
        out.push((r.0, src.read(r.1, r.2)?));
    }
    out.sort_by_key(|t| t.0);
    Some(sfnt(&out))
}

/// A single-face font with the tables the catalog reads.
pub(crate) fn catalog<S: FontSource + ?Sized>(src: &S, records: &Records) -> Option<Vec<u8>> {
    subset(src, records, &CATALOG)
}

/// The `glyf` and `loca` tables of a face as (offset, length) in the file,
/// and whether `loca` holds long offsets.
#[derive(Clone, Copy)]
struct Glyf {
    glyf: (usize, usize),
    loca: (usize, usize),
    long: bool,
}

/// Where a face's outlines live in the file.
pub(crate) struct Glyphs {
    src: SharedSource,
    records: Records,
    /// `None` when the face draws from all its tables (CFF, variations).
    tables: Option<Glyf>,
    /// All the face's tables as a single-face font, read the first time an
    /// outline needs them.
    all: OnceLock<Option<FontData>>,
}

/// Face `index` of `src`: a single-face font with the tables a face reads
/// besides outlines, and where its outlines are.
pub(crate) fn face(src: &SharedSource, index: u32) -> Option<(Vec<u8>, Glyphs)> {
    let records = faces(&**src)?.into_iter().nth(index as usize)?;
    let meta = subset(&**src, &records, &METRICS)?;
    let record = |tag: &[u8; 4]| records.iter().find(|r| &r.0 == tag).map(|r| (r.1, r.2));
    let tables = (|| {
        if WHOLE_OUTLINES.iter().any(|t| record(t).is_some()) {
            return None;
        }
        let font = FontRef::new(&meta).ok()?;
        // The tables skrifa's TrueType outlines need, else the face draws
        // from all its tables.
        font.hmtx().ok()?;
        let long = font.head().ok()?.index_to_loc_format() == 1;
        Some(Glyf {
            glyf: record(b"glyf")?,
            loca: record(b"loca")?,
            long,
        })
    })();
    let glyphs = Glyphs {
        src: src.clone(),
        records,
        tables,
        all: OnceLock::new(),
    };
    Some((meta, glyphs))
}

impl Glyphs {
    /// Bytes of all the face's tables, when an outline needed them.
    pub(crate) fn all_bytes(&self) -> usize {
        self.all
            .get()
            .and_then(Option::as_ref)
            .map_or(0, |d| (**d).as_ref().len())
    }

    /// The outline of `glyph`, as the whole file draws it.
    pub(crate) fn outline(&self, meta: &[u8], glyph: u16, units_per_em: u16) -> Outline {
        match self.tables.and_then(|t| self.single(meta, t, glyph)) {
            Some(Some(font)) => return crate::face::draw(&font, 0, 0, units_per_em),
            // A record the whole file's reader fails on fails the glyph.
            Some(None) => {
                return Outline {
                    ops: Vec::new(),
                    units_per_em,
                };
            }
            None => {}
        }
        let all = self.all.get_or_init(|| {
            let tags: Vec<&[u8; 4]> = self.records.iter().map(|r| &r.0).collect();
            subset(&*self.src, &self.records, &tags).map(|b| Arc::new(b) as FontData)
        });
        match all {
            Some(data) => crate::face::draw((**data).as_ref(), 0, glyph, units_per_em),
            None => Outline {
                ops: Vec::new(),
                units_per_em,
            },
        }
    }

    /// A font whose glyph 0 is `glyph` and whose other glyphs are the ones
    /// its composites place, renumbered, with the advances and side
    /// bearings of the glyphs they stand for. `Some(None)` when `loca`
    /// cannot locate a record or the reader cannot read one, which fails
    /// the glyph in the whole file too; `None` when the file cannot be read
    /// or the records are out of the ordinary.
    fn single(
        &self,
        meta: &[u8],
        Glyf { glyf, loca, long }: Glyf,
        glyph: u16,
    ) -> Option<Option<Vec<u8>>> {
        use skrifa::raw::tables::glyf::Glyph;
        use skrifa::raw::{FontData as Data, FontRead};
        let font = FontRef::new(meta).ok()?;
        let width = if long { 4 } else { 2 };
        // As the reader's `loca`: the record's span in `glyf`, `None` when
        // it fails.
        let span = |g: u16| -> Option<Option<(usize, usize)>> {
            let at = g as usize * width;
            if at + 2 * width > loca.1 {
                return Some(None);
            }
            let raw = self.src.read(loca.0 + at, 2 * width)?;
            let (a, b) = if long {
                (be32(&raw, 0)? as usize, be32(&raw, 4)? as usize)
            } else {
                (be16(&raw, 0)? as usize * 2, be16(&raw, 2)? as usize * 2)
            };
            Some((a == b || (a < b && b <= glyf.1)).then_some((a, b)))
        };
        let mut order = vec![glyph];
        let mut ids: HashMap<u16, u16> = HashMap::from([(glyph, 0)]);
        let mut records: Vec<Vec<u8>> = Vec::new();
        while records.len() < order.len() {
            let Some((a, b)) = span(order[records.len()])? else {
                return Some(None);
            };
            let mut rec = if b > a {
                self.src.read(glyf.0 + a, b - a)?
            } else {
                Vec::new()
            };
            let slots = component_slots(&rec);
            if b > a {
                match Glyph::read(Data::new(&rec)) {
                    Err(_) => return Some(None),
                    Ok(Glyph::Simple(_)) if !slots.is_empty() => return None,
                    Ok(Glyph::Simple(_)) => {}
                    Ok(Glyph::Composite(c)) => {
                        let theirs = c.component_glyphs_and_flags().map(|(g, _)| g.to_u16());
                        if !theirs.eq(slots.iter().filter_map(|&at| be16(&rec, at))) {
                            return None;
                        }
                    }
                }
            }
            for at in slots {
                let child = be16(&rec, at)?;
                let next = u16::try_from(order.len()).ok()?;
                let id = *ids.entry(child).or_insert_with(|| {
                    order.push(child);
                    next
                });
                rec[at..at + 2].copy_from_slice(&id.to_be_bytes());
            }
            records.push(rec);
        }
        let n = u16::try_from(order.len()).ok()?;
        let hmtx = font.hmtx().ok()?;
        let raw = |tag: &[u8; 4]| {
            font.table_data(skrifa::raw::types::Tag::new(tag))
                .map(|d| d.as_bytes().to_vec())
        };
        let mut head = raw(b"head")?;
        let mut hhea = raw(b"hhea")?;
        let mut maxp = raw(b"maxp")?;
        if head.len() < 54 || hhea.len() < 36 || maxp.len() < 6 {
            return None;
        }
        head[50..52].copy_from_slice(&1u16.to_be_bytes());
        hhea[34..36].copy_from_slice(&n.to_be_bytes());
        maxp[4..6].copy_from_slice(&n.to_be_bytes());
        let mut metrics = Vec::with_capacity(4 * order.len());
        let mut new_glyf = Vec::new();
        let mut new_loca = Vec::with_capacity(4 * (order.len() + 1));
        for (&g, rec) in order.iter().zip(&records) {
            let id = GlyphId::new(g as u32);
            metrics.extend_from_slice(&hmtx.advance(id).unwrap_or_default().to_be_bytes());
            metrics.extend_from_slice(&hmtx.side_bearing(id).unwrap_or_default().to_be_bytes());
            new_loca.extend_from_slice(&(new_glyf.len() as u32).to_be_bytes());
            new_glyf.extend_from_slice(rec);
            new_glyf.resize(new_glyf.len().next_multiple_of(4), 0);
        }
        new_loca.extend_from_slice(&(new_glyf.len() as u32).to_be_bytes());
        let mut out = vec![
            (*b"glyf", new_glyf),
            (*b"head", head),
            (*b"hhea", hhea),
            (*b"hmtx", metrics),
            (*b"loca", new_loca),
            (*b"maxp", maxp),
        ];
        if let Some(os2) = raw(b"OS/2") {
            out.push((*b"OS/2", os2));
        }
        out.sort_by_key(|t| t.0);
        Some(Some(sfnt(&out)))
    }
}
