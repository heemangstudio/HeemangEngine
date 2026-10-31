//! HWP 5 checks: FileHeader, stream compression, record framing and levels,
//! DocInfo counts and references, and the BodyText paragraph and control
//! trees of every section.

use std::collections::BTreeMap;

use crate::cfb::{self, Stream};
use crate::{Report, u16_at, u32_at};

const DOC: &str = "docinfo";
const BODY: &str = "body";
const BIN: &str = "bindata";
const STREAM_LIMIT: usize = 512 << 20;

mod tag {
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
    pub const MEMO_SHAPE: u16 = 0x5C;
    pub const TRACK_CHANGE: u16 = 0x60;
    pub const TRACK_CHANGE_AUTHOR: u16 = 0x61;
    pub const PARA_HEADER: u16 = 0x42;
    pub const PARA_TEXT: u16 = 0x43;
    pub const PARA_CHAR_SHAPE: u16 = 0x44;
    pub const PARA_LINE_SEG: u16 = 0x45;
    pub const PARA_RANGE_TAG: u16 = 0x46;
    pub const CTRL_HEADER: u16 = 0x47;
    pub const LIST_HEADER: u16 = 0x48;
    pub const PAGE_BORDER_FILL: u16 = 0x4B;
    pub const TABLE: u16 = 0x4D;
    pub const SHAPE_COMPONENT: u16 = 0x4C;
    pub const SHAPE_OLE: u16 = 0x54;
    pub const SHAPE_PICTURE: u16 = 0x55;
}

struct Rec<'a> {
    tag: u16,
    level: u16,
    data: &'a [u8],
}

/// Frames a stream strictly: a truncated header or payload is an error.
fn records<'a>(s: &'a [u8], what: &str, area: &'static str, r: &mut Report) -> Vec<Rec<'a>> {
    let mut out = Vec::new();
    let mut pos = 0;
    let mut prev: Option<u16> = None;
    while pos < s.len() {
        let Some(h) = u32_at(s, pos) else {
            r.error(
                area,
                format!("{what}: record header truncated at byte {pos}"),
            );
            break;
        };
        let (tag, level) = ((h & 0x3FF) as u16, ((h >> 10) & 0x3FF) as u16);
        let mut size = (h >> 20) as usize;
        let mut body = pos + 4;
        if size == 0xFFF {
            let Some(n) = u32_at(s, body) else {
                r.error(area, format!("{what}: record size truncated at byte {pos}"));
                break;
            };
            size = n as usize;
            body += 4;
        }
        let Some(end) = body.checked_add(size).filter(|&e| e <= s.len()) else {
            r.error(
                area,
                format!("{what}: record {tag:#x} at byte {pos} runs past the stream end"),
            );
            break;
        };
        match prev {
            None if level != 0 => r.error(area, format!("{what}: first record has level {level}")),
            Some(p) if level > p + 1 => r.error(
                area,
                format!("{what}: record {tag:#x} at byte {pos} jumps from level {p} to {level}"),
            ),
            _ => {}
        }
        prev = Some(level);
        out.push(Rec {
            tag,
            level,
            data: &s[body..end],
        });
        pos = end;
    }
    out
}

/// A compressed stream inflated strictly, with Hancom's 8-byte trailer
/// (CRC-32 and length of the inflated bytes) checked when present.
fn inflate(raw: &[u8], what: &str, area: &'static str, r: &mut Report) -> Option<Vec<u8>> {
    match crate::inflate_raw(raw, STREAM_LIMIT) {
        Ok((out, used)) => {
            let rest = &raw[used..];
            if rest.len() == 8 {
                let (crc, len) = (u32_at(rest, 0).unwrap(), u32_at(rest, 4).unwrap());
                if crc != crate::crc32(&out) || len as usize != out.len() {
                    r.error(
                        area,
                        format!("{what}: deflate trailer does not match the inflated data"),
                    );
                }
            } else if !rest.is_empty() {
                r.warn(
                    area,
                    format!("{what}: {} bytes after the deflate data", rest.len()),
                );
            }
            Some(out)
        }
        Err(e) => {
            r.error(area, format!("{what}: {e}"));
            None
        }
    }
}

/// Item counts of the DocInfo tables, as stored and as found.
#[derive(Default)]
struct Tables {
    bins: usize,
    /// Binary data storage ids of embedded and storage entries, by 1-based id.
    bin_kinds: Vec<u16>,
    faces: [usize; 7],
    border_fills: usize,
    char_shapes: usize,
    tab_defs: usize,
    numberings: usize,
    bullets: usize,
    para_shapes: usize,
    styles: usize,
    sections: Option<usize>,
}

const MAPPING_NAMES: [&str; 18] = [
    "BinData",
    "Hangul faces",
    "Latin faces",
    "Hanja faces",
    "Japanese faces",
    "other faces",
    "symbol faces",
    "user faces",
    "border fills",
    "char shapes",
    "tab defs",
    "numberings",
    "bullets",
    "para shapes",
    "styles",
    "memo shapes",
    "track changes",
    "track change authors",
];

fn hwp_string(b: &[u8], at: usize) -> Option<(Vec<u16>, usize)> {
    let n = u16_at(b, at)? as usize;
    let units = (0..n)
        .map(|k| u16_at(b, at + 2 + 2 * k))
        .collect::<Option<Vec<_>>>()?;
    Some((units, at + 2 + 2 * n))
}

fn docinfo(recs: &[Rec<'_>], cfb_paths: &[String], r: &mut Report) -> Tables {
    let mut t = Tables::default();
    if recs.first().map(|x| x.tag) != Some(tag::DOCUMENT_PROPERTIES) {
        r.error(DOC, "first record is not DOCUMENT_PROPERTIES");
    }
    if recs.get(1).map(|x| x.tag) != Some(tag::ID_MAPPINGS) {
        r.error(DOC, "second record is not ID_MAPPINGS");
    }
    // Hancom reads the tables in this order and rejects a DocInfo with a
    // table record after the compatibility and other trailing records.
    let rank = |t: u16| -> Option<u8> {
        Some(match t {
            tag::DOCUMENT_PROPERTIES => 0,
            tag::ID_MAPPINGS => 1,
            tag::BIN_DATA => 2,
            tag::FACE_NAME => 3,
            tag::BORDER_FILL => 4,
            tag::CHAR_SHAPE => 5,
            tag::TAB_DEF => 6,
            tag::NUMBERING => 7,
            tag::BULLET => 8,
            tag::PARA_SHAPE => 9,
            tag::STYLE => 10,
            0x1B | 0x1E | 0x1F | 0x20 | 0x5E => 11,
            _ => return None,
        })
    };
    let mut top = 0;
    for (k, x) in recs.iter().enumerate() {
        let Some(n) = rank(x.tag) else { continue };
        if n < top {
            r.error(
                DOC,
                format!(
                    "record {k} ({:#x}) comes after records Hancom reads later",
                    x.tag
                ),
            );
            break;
        }
        top = n;
    }
    if let Some(p) = recs.iter().find(|x| x.tag == tag::DOCUMENT_PROPERTIES) {
        t.sections = u16_at(p.data, 0).map(|n| n as usize);
    }
    let count = |tg: u16| recs.iter().filter(|x| x.tag == tg).count();
    t.bins = count(tag::BIN_DATA);
    t.border_fills = count(tag::BORDER_FILL);
    t.char_shapes = count(tag::CHAR_SHAPE);
    t.tab_defs = count(tag::TAB_DEF);
    t.numberings = count(tag::NUMBERING);
    t.bullets = count(tag::BULLET);
    t.para_shapes = count(tag::PARA_SHAPE);
    t.styles = count(tag::STYLE);
    let faces = count(tag::FACE_NAME);
    let mapped: Vec<u32> = recs
        .iter()
        .find(|x| x.tag == tag::ID_MAPPINGS)
        .map(|x| {
            x.data
                .chunks_exact(4)
                .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
                .collect()
        })
        .unwrap_or_default();
    if mapped.len() < 15 {
        r.error(DOC, format!("ID_MAPPINGS holds {} counts", mapped.len()));
    }
    let face_total: u64 = mapped.iter().skip(1).take(7).map(|&v| v as u64).sum();
    for k in 0..7 {
        t.faces[k] = mapped.get(1 + k).copied().unwrap_or(0) as usize;
    }
    if face_total != faces as u64 {
        r.error(
            DOC,
            format!("ID_MAPPINGS lists {face_total} faces, DocInfo has {faces}"),
        );
    }
    let found = [
        (0, t.bins),
        (8, t.border_fills),
        (9, t.char_shapes),
        (10, t.tab_defs),
        (11, t.numberings),
        (12, t.bullets),
        (13, t.para_shapes),
        (14, t.styles),
        (15, count(tag::MEMO_SHAPE)),
        (16, count(tag::TRACK_CHANGE)),
        (17, count(tag::TRACK_CHANGE_AUTHOR)),
    ];
    for (k, n) in found {
        match mapped.get(k) {
            Some(&m) if m as usize != n => r.error(
                DOC,
                format!(
                    "ID_MAPPINGS lists {m} {}, DocInfo has {n}",
                    MAPPING_NAMES[k]
                ),
            ),
            None if n > 0 => r.error(
                DOC,
                format!("ID_MAPPINGS has no count for {n} {}", MAPPING_NAMES[k]),
            ),
            _ => {}
        }
    }

    // BinData entries and their streams.
    for (k, x) in recs.iter().filter(|x| x.tag == tag::BIN_DATA).enumerate() {
        let id = k + 1;
        let Some(attr) = u16_at(x.data, 0) else {
            r.error(DOC, format!("BinData {id} is empty"));
            t.bin_kinds.push(u16::MAX);
            continue;
        };
        let kind = attr & 0xF;
        t.bin_kinds.push(kind);
        if kind == 0 {
            continue;
        }
        if kind > 2 {
            r.error(DOC, format!("BinData {id} has type {kind}"));
            continue;
        }
        let Some(storage) = u16_at(x.data, 2) else {
            r.error(DOC, format!("BinData {id} lacks its storage id"));
            continue;
        };
        let ext = hwp_string(x.data, 4)
            .map(|(u, _)| String::from_utf16_lossy(&u))
            .unwrap_or_default();
        let name = format!("BinData/BIN{storage:04X}.{ext}");
        if !cfb_paths.iter().any(|p| p.eq_ignore_ascii_case(&name)) {
            r.error(BIN, format!("BinData {id} names missing stream {name}"));
        }
    }

    // References between the tables.
    for (k, x) in recs.iter().filter(|x| x.tag == tag::CHAR_SHAPE).enumerate() {
        if x.data.len() < 66 {
            r.error(DOC, format!("char shape {k} is {} bytes", x.data.len()));
            continue;
        }
        for slot in 0..7 {
            let f = u16_at(x.data, 2 * slot).unwrap() as usize;
            if f >= t.faces[slot] {
                r.error(
                    DOC,
                    format!(
                        "char shape {k} uses {} {f} of {}",
                        MAPPING_NAMES[1 + slot],
                        t.faces[slot]
                    ),
                );
            }
        }
        if let Some(bf) = u16_at(x.data, 68)
            && bf as usize > t.border_fills
        {
            r.error(
                DOC,
                format!("char shape {k} uses border fill {bf} of {}", t.border_fills),
            );
        }
    }
    for (k, x) in recs.iter().filter(|x| x.tag == tag::PARA_SHAPE).enumerate() {
        if x.data.len() < 42 {
            r.error(DOC, format!("para shape {k} is {} bytes", x.data.len()));
            continue;
        }
        let attr = u32_at(x.data, 0).unwrap();
        let tab = u16_at(x.data, 28).unwrap() as usize;
        let head = u16_at(x.data, 30).unwrap() as usize;
        let bf = u16_at(x.data, 32).unwrap() as usize;
        if tab >= t.tab_defs {
            r.error(
                DOC,
                format!("para shape {k} uses tab def {tab} of {}", t.tab_defs),
            );
        }
        match (attr >> 23) & 3 {
            // Hancom opens a numbered or bulleted shape with no definition
            // (id 0), not one naming a definition that does not exist.
            2 | 3 if head == 0 => r.warn(
                DOC,
                format!("para shape {k} has a head without a definition"),
            ),
            2 if head > t.numberings => r.error(
                DOC,
                format!("para shape {k} uses numbering {head} of {}", t.numberings),
            ),
            3 if head > t.bullets => r.error(
                DOC,
                format!("para shape {k} uses bullet {head} of {}", t.bullets),
            ),
            1 if head > t.numberings => r.warn(
                DOC,
                format!(
                    "outline para shape {k} names numbering {head} of {}",
                    t.numberings
                ),
            ),
            _ => {}
        }
        if bf > t.border_fills {
            r.error(
                DOC,
                format!("para shape {k} uses border fill {bf} of {}", t.border_fills),
            );
        }
    }
    for (k, x) in recs.iter().filter(|x| x.tag == tag::STYLE).enumerate() {
        let fields = hwp_string(x.data, 0)
            .and_then(|(_, at)| hwp_string(x.data, at))
            .and_then(|(_, at)| {
                Some((
                    *x.data.get(at + 1)? as usize,
                    u16_at(x.data, at + 4)? as usize,
                    u16_at(x.data, at + 6)? as usize,
                ))
            });
        let Some((next, para, chars)) = fields else {
            r.error(DOC, format!("style {k} is truncated"));
            continue;
        };
        if next >= t.styles {
            r.warn(
                DOC,
                format!("style {k} has next style {next} of {}", t.styles),
            );
        }
        if para >= t.para_shapes {
            r.error(
                DOC,
                format!("style {k} uses para shape {para} of {}", t.para_shapes),
            );
        }
        if chars >= t.char_shapes {
            r.error(
                DOC,
                format!("style {k} uses char shape {chars} of {}", t.char_shapes),
            );
        }
    }
    t
}

/// Control codes taking eight units with a CTRL_HEADER child.
fn extended(c: u16) -> bool {
    matches!(
        c,
        1 | 2 | 3 | 11 | 12 | 14 | 15 | 16 | 17 | 18 | 21 | 22 | 23
    )
}

/// Control codes taking eight units without a record.
fn inline(c: u16) -> bool {
    matches!(c, 4..=9 | 19 | 20)
}

struct Body<'a, 'b> {
    t: &'a Tables,
    r: &'b mut Report,
    what: String,
    /// BinData ids pictures and OLE objects refer to.
    used_bins: &'a mut Vec<u16>,
    paras: usize,
    /// Field starts minus field ends in the current list stack.
    fields: Vec<i64>,
}

impl Body<'_, '_> {
    fn err(&mut self, m: String) {
        let w = &self.what;
        self.r.error(BODY, format!("{w}: {m}"));
    }

    fn warn(&mut self, m: String) {
        let w = &self.what;
        self.r.warn(BODY, format!("{w}: {m}"));
    }

    /// HWPX leftover markup copied into an HWP record (a control taken
    /// from an HWPX document without conversion).
    fn markup(&mut self, recs: &[Rec<'_>], from: usize, to: usize) {
        for x in &recs[from..to] {
            if x.data.windows(7).any(|w| w == b"IDRef=\"")
                || x.data.windows(4).any(|w| w == b"<hp:")
            {
                self.err(format!("record {:#x} holds HWPX markup", x.tag));
                return;
            }
        }
    }

    /// The paragraphs at `level` starting at `*i`; returns how many.
    fn list(&mut self, recs: &[Rec<'_>], i: &mut usize, level: u16) -> usize {
        let mut n = 0;
        let mut last_flag = None;
        self.fields.push(0);
        while *i < recs.len() && recs[*i].level == level && recs[*i].tag == tag::PARA_HEADER {
            if last_flag == Some(true) {
                self.err(format!(
                    "paragraph {} follows one marked last in its list",
                    self.paras
                ));
            }
            last_flag = Some(self.para(recs, i, level));
            n += 1;
        }
        if last_flag == Some(false) {
            self.err(format!(
                "list ending at paragraph {} lacks its last-paragraph bit",
                self.paras
            ));
        }
        let open = self.fields.pop().unwrap_or(0);
        if open != 0 {
            self.warn(format!(
                "list ending at paragraph {} has {open:+} unclosed fields",
                self.paras
            ));
        }
        n
    }

    /// One paragraph and its children; returns its last-paragraph bit.
    fn para(&mut self, recs: &[Rec<'_>], i: &mut usize, level: u16) -> bool {
        let h = recs[*i].data;
        let index = self.paras;
        self.paras += 1;
        *i += 1;
        if h.len() < 22 {
            self.err(format!("paragraph {index} header is {} bytes", h.len()));
        }
        let raw = u32_at(h, 0).unwrap_or(0);
        let last = raw & 0x8000_0000 != 0;
        let nchars = (raw & 0x7FFF_FFFF) as usize;
        let mask = u32_at(h, 4).unwrap_or(0);
        let para_shape = u16_at(h, 8).unwrap_or(0) as usize;
        let style = h.get(10).copied().unwrap_or(0) as usize;
        let runs_declared = u16_at(h, 12).unwrap_or(0) as usize;
        let tags_declared = u16_at(h, 14).unwrap_or(0) as usize;
        let segs_declared = u16_at(h, 16).unwrap_or(0) as usize;
        if para_shape >= self.t.para_shapes {
            self.err(format!(
                "paragraph {index} uses para shape {para_shape} of {}",
                self.t.para_shapes
            ));
        }
        if style >= self.t.styles {
            self.err(format!(
                "paragraph {index} uses style {style} of {}",
                self.t.styles
            ));
        }
        let (mut text, mut runs, mut segs, mut tags) = (None, None, None, None);
        let mut ctrl_ids = Vec::new();
        while *i < recs.len() && recs[*i].level > level {
            let x = &recs[*i];
            if x.level != level + 1 {
                self.err(format!(
                    "paragraph {index} has record {:#x} at level {} without a parent",
                    x.tag, x.level
                ));
                *i += 1;
                continue;
            }
            match x.tag {
                tag::PARA_TEXT => text = Some(x.data),
                tag::PARA_CHAR_SHAPE => runs = Some(x.data),
                tag::PARA_LINE_SEG => segs = Some(x.data),
                tag::PARA_RANGE_TAG => tags = Some(x.data),
                tag::CTRL_HEADER => {
                    ctrl_ids.push(u32_at(x.data, 0).unwrap_or(0));
                    self.control(recs, i, level + 1);
                    continue;
                }
                // Memos hang under the last paragraph of the last section:
                // a memo list record, then each memo's list header and its
                // paragraphs one level below that paragraph.
                tag::LIST_HEADER => {
                    let declared = u32_at(x.data, 0).unwrap_or(0) as usize;
                    *i += 1;
                    let n = self.list(recs, i, level + 1);
                    if n != declared {
                        self.err(format!(
                            "paragraph {index} list declares {declared} paragraphs, has {n}"
                        ));
                    }
                    continue;
                }
                tag::PARA_HEADER => {
                    self.err(format!(
                        "paragraph {index} has a paragraph child outside a list"
                    ));
                }
                _ => {}
            }
            *i += 1;
            while *i < recs.len() && recs[*i].level > level + 1 {
                *i += 1;
            }
        }

        // Text, control blocks and the control mask.
        let mut ext_ids = Vec::new();
        let mut seen_mask = 0u32;
        match text {
            None if nchars != 1 => self.err(format!(
                "paragraph {index} declares {nchars} units but has no text"
            )),
            None => {}
            Some(t) => {
                if t.len() != 2 * nchars {
                    self.err(format!(
                        "paragraph {index} declares {nchars} units, its text has {} bytes",
                        t.len()
                    ));
                }
                let u: Vec<u16> = t
                    .chunks_exact(2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .collect();
                let mut pos = 0;
                while pos < u.len() {
                    let c = u[pos];
                    if c >= 32 {
                        if (0xD800..0xDC00).contains(&c)
                            && !u.get(pos + 1).is_some_and(|n| (0xDC00..0xE000).contains(n))
                        {
                            self.warn(format!(
                                "paragraph {index} has a lone high surrogate at {pos}"
                            ));
                        }
                        if (0xDC00..0xE000).contains(&c)
                            && !(pos > 0 && (0xD800..0xDC00).contains(&u[pos - 1]))
                        {
                            self.warn(format!(
                                "paragraph {index} has a lone low surrogate at {pos}"
                            ));
                        }
                        pos += 1;
                        continue;
                    }
                    seen_mask |= 1 << c;
                    if extended(c) || inline(c) {
                        if pos + 8 > u.len() {
                            self.err(format!(
                                "paragraph {index} control {c} at {pos} is cut by the text end"
                            ));
                            break;
                        }
                        if u[pos + 7] != c {
                            self.err(format!(
                                "paragraph {index} control block at {pos} starts with {c}, ends with {}",
                                u[pos + 7]
                            ));
                        }
                        if extended(c) {
                            ext_ids.push(u[pos + 1] as u32 | (u[pos + 2] as u32) << 16);
                        }
                        if c == 3
                            && let Some(f) = self.fields.last_mut()
                        {
                            *f += 1;
                        }
                        if c == 4
                            && let Some(f) = self.fields.last_mut()
                        {
                            *f -= 1;
                        }
                        pos += 8;
                        continue;
                    }
                    if c == 13 && pos + 1 != u.len() {
                        self.err(format!("paragraph {index} has a paragraph end at {pos}"));
                    }
                    if !matches!(c, 10 | 13 | 24 | 30 | 31) {
                        self.err(format!(
                            "paragraph {index} has reserved character {c} at {pos}"
                        ));
                    }
                    pos += 1;
                }
                if u.last() != Some(&13) {
                    self.err(format!(
                        "paragraph {index} text does not end with a paragraph end"
                    ));
                }
            }
        }
        // Hancom leaves the paragraph end out of the mask and sets the
        // hyphen bit only sometimes.
        let ignore = (1 << 13) | (1 << 24);
        if (seen_mask & !ignore) & !mask != 0 {
            self.err(format!(
                "paragraph {index} control mask {mask:#x} lacks codes {:#x}",
                (seen_mask & !ignore) & !mask
            ));
        }
        if (mask & !ignore) & !seen_mask != 0 {
            self.warn(format!(
                "paragraph {index} control mask {mask:#x} has codes {:#x} its text lacks",
                (mask & !ignore) & !seen_mask
            ));
        }
        if ext_ids.len() != ctrl_ids.len() {
            self.err(format!(
                "paragraph {index} has {} extended controls and {} control headers",
                ext_ids.len(),
                ctrl_ids.len()
            ));
        } else if let Some(k) = (0..ext_ids.len()).find(|&k| ext_ids[k] != ctrl_ids[k]) {
            // Hancom stores some fields as `%unk` in their header.
            let (a, b) = (fourcc(ext_ids[k]), fourcc(ctrl_ids[k]));
            let m =
                format!("paragraph {index} control {k} is {a:?} in the text, {b:?} in its header");
            if a.starts_with('%') && b.starts_with('%') {
                self.warn(m);
            } else {
                self.err(m);
            }
        }

        // Character shape runs.
        match runs {
            None => self.err(format!("paragraph {index} has no char shape run")),
            Some(b) => {
                if b.len() % 8 != 0 || b.len() / 8 != runs_declared {
                    self.err(format!(
                        "paragraph {index} declares {runs_declared} char shape runs, has {} bytes",
                        b.len()
                    ));
                }
                let mut prev: Option<u32> = None;
                for c in b.chunks_exact(8) {
                    let (pos, id) = (u32_at(c, 0).unwrap(), u32_at(c, 4).unwrap());
                    match prev {
                        None if pos != 0 => self.err(format!(
                            "paragraph {index} first char shape run starts at {pos}"
                        )),
                        Some(p) if pos <= p => self.err(format!(
                            "paragraph {index} char shape runs out of order at {pos}"
                        )),
                        _ => {}
                    }
                    if pos as usize >= nchars.max(1) {
                        self.err(format!(
                            "paragraph {index} char shape run at {pos} is past its {nchars} units"
                        ));
                    }
                    if id as usize >= self.t.char_shapes {
                        self.err(format!(
                            "paragraph {index} uses char shape {id} of {}",
                            self.t.char_shapes
                        ));
                    }
                    prev = Some(pos);
                }
            }
        }
        match segs {
            None if segs_declared != 0 => self.err(format!(
                "paragraph {index} declares {segs_declared} line segments, has none"
            )),
            None => {}
            Some(b) => {
                if b.len() % 36 != 0 || b.len() / 36 != segs_declared {
                    self.err(format!(
                        "paragraph {index} declares {segs_declared} line segments, has {} bytes",
                        b.len()
                    ));
                }
                let mut prev = None;
                for c in b.chunks_exact(36) {
                    let pos = u32_at(c, 0).unwrap();
                    if pos as usize >= nchars.max(1) {
                        self.warn(format!(
                            "paragraph {index} line segment at {pos} is past its {nchars} units"
                        ));
                    }
                    if prev.is_some_and(|p| pos < p) {
                        self.warn(format!(
                            "paragraph {index} line segments out of order at {pos}"
                        ));
                    }
                    prev = Some(pos);
                }
                if b.len() >= 36 && u32_at(b, 0) != Some(0) {
                    self.warn(format!(
                        "paragraph {index} first line segment does not start at 0"
                    ));
                }
            }
        }
        match tags {
            None if tags_declared != 0 => self.err(format!(
                "paragraph {index} declares {tags_declared} range tags, has none"
            )),
            None => {}
            Some(b) => {
                if b.len() % 12 != 0 || b.len() / 12 != tags_declared {
                    self.err(format!(
                        "paragraph {index} declares {tags_declared} range tags, has {} bytes",
                        b.len()
                    ));
                }
                for c in b.chunks_exact(12) {
                    let (s, e) = (u32_at(c, 0).unwrap(), u32_at(c, 4).unwrap());
                    if s > e || e as usize > nchars {
                        self.err(format!(
                            "paragraph {index} range tag {s}..{e} outside its {nchars} units"
                        ));
                    }
                }
            }
        }
        last
    }

    /// A control header at `level` and its subtree.
    fn control(&mut self, recs: &[Rec<'_>], i: &mut usize, level: u16) {
        let head = recs[*i].data;
        let id = u32_at(head, 0).unwrap_or(0);
        let name = fourcc(id);
        let first = *i;
        let end = (first + 1..recs.len())
            .find(|&k| recs[k].level <= level)
            .unwrap_or(recs.len());
        self.markup(recs, first, end);
        *i += 1;
        let mut table: Option<Grid> = None;
        let mut cells: Vec<Cell> = Vec::new();
        // A caption is the list header right below the control; Hancom
        // marks captioned objects with bit 29 of their attributes.
        if matches!(name.as_str(), "gso " | "tbl ") {
            let caption = recs
                .get(*i)
                .is_some_and(|x| x.tag == tag::LIST_HEADER && x.level == level + 1);
            if let Some(attr) = u32_at(head, 4)
                && (attr & (1 << 29) != 0) != caption
            {
                self.err(format!(
                    "{name} caption bit is {}, the control {} a caption",
                    attr >> 29 & 1,
                    if caption { "has" } else { "has no" }
                ));
            }
        }
        if name == "secd"
            && let Some(n) = u16_at(head, 18)
            && n as usize > self.t.numberings
        {
            self.err(format!(
                "section uses numbering {n} of {}",
                self.t.numberings
            ));
        }
        while *i < recs.len() && recs[*i].level > level {
            let x = &recs[*i];
            match x.tag {
                tag::LIST_HEADER => {
                    let declared = u32_at(x.data, 0).unwrap_or(0) as usize;
                    let list_level = x.level;
                    let in_table = table.is_some() && list_level == level + 1;
                    let cell = in_table.then(|| Cell::read(x.data));
                    *i += 1;
                    // Subrecords of the list header itself come first.
                    while *i < recs.len() && recs[*i].level > list_level {
                        *i += 1;
                    }
                    let n = self.list(recs, i, list_level);
                    if declared == 0 || n == 0 {
                        self.err(format!("{name} has an empty paragraph list"));
                    }
                    if n != declared {
                        self.err(format!(
                            "{name} list declares {declared} paragraphs, has {n}"
                        ));
                    }
                    if let Some(c) = cell {
                        match c {
                            Some(c) => cells.push(c),
                            None => self.err("table cell header is truncated".into()),
                        }
                    }
                    continue;
                }
                tag::TABLE if x.level == level + 1 => table = Grid::read(x.data),
                tag::SHAPE_COMPONENT => self.drawing(recs, *i),
                tag::SHAPE_PICTURE => {
                    if let Some(b) = u16_at(x.data, 71) {
                        self.bin(b, "picture");
                    }
                }
                tag::SHAPE_OLE => {
                    if let Some(b) = u16_at(x.data, 12) {
                        self.bin(b, "OLE object");
                    }
                }
                tag::PAGE_BORDER_FILL => {
                    if let Some(bf) = u16_at(x.data, 12)
                        && bf as usize > self.t.border_fills
                    {
                        self.err(format!(
                            "page border uses border fill {bf} of {}",
                            self.t.border_fills
                        ));
                    }
                }
                tag::PARA_HEADER => {
                    self.err(format!("{name} has a paragraph outside a list"));
                    self.para(recs, i, x.level);
                    continue;
                }
                tag::CTRL_HEADER => {
                    self.err(format!("{name} has a control header child"));
                }
                _ => {}
            }
            *i += 1;
        }
        if name == "tbl " {
            match table {
                None => self.err("table control without a TABLE record".into()),
                Some(g) => self.table(&g, &cells),
            }
        }
    }

    /// A drawing that holds text: Hancom wants its list before the
    /// geometry record and bit 24 of the component's flip word set.
    fn drawing(&mut self, recs: &[Rec<'_>], at: usize) {
        let c = &recs[at];
        let mut kids = Vec::new();
        for x in &recs[at + 1..] {
            if x.level <= c.level {
                break;
            }
            if x.level == c.level + 1 {
                kids.push(x.tag);
            }
        }
        let Some(list) = kids.iter().position(|&t| t == tag::LIST_HEADER) else {
            return;
        };
        if kids[..list].iter().any(|t| (0x4E..=0x53).contains(t)) {
            self.err("drawing's geometry record comes before its text list".into());
        }
        let doubled = c.data.get(0..4) == c.data.get(4..8);
        let at = if doubled { 36 } else { 32 };
        if u32_at(c.data, at).is_some_and(|f| f & (1 << 24) == 0) {
            self.err("drawing with text lacks bit 24 of its flip word".into());
        }
    }

    fn bin(&mut self, id: u16, what: &str) {
        // Hancom opens a picture without an image (id 0).
        if id == 0 {
            self.warn(format!("{what} has no BinData"));
            return;
        }
        if id as usize > self.t.bins {
            self.err(format!("{what} uses BinData {id} of {}", self.t.bins));
            return;
        }
        if self.t.bin_kinds.get(id as usize - 1) == Some(&0) {
            return;
        }
        self.used_bins.push(id);
    }

    fn table(&mut self, g: &Grid, cells: &[Cell]) {
        if g.rows == 0 || g.cols == 0 {
            self.err(format!("table is {}x{}", g.rows, g.cols));
            return;
        }
        let declared: usize = g.row_cells.iter().map(|&n| n as usize).sum();
        if declared != cells.len() {
            self.err(format!(
                "table row counts list {declared} cells, it has {}",
                cells.len()
            ));
        }
        for (row, &n) in g.row_cells.iter().enumerate() {
            let have = cells.iter().filter(|c| c.row as usize == row).count();
            if have != n as usize {
                self.err(format!("table row {row} declares {n} cells, has {have}"));
            }
        }
        if g.border_fill as usize > self.t.border_fills {
            self.err(format!(
                "table uses border fill {} of {}",
                g.border_fill, self.t.border_fills
            ));
        }
        let (rows, cols) = (g.rows as usize, g.cols as usize);
        let mut grid = vec![0u8; rows * cols];
        for c in cells {
            let (r0, c0) = (c.row as usize, c.col as usize);
            let (r1, c1) = (r0 + c.row_span as usize, c0 + c.col_span as usize);
            if c.row_span == 0 || c.col_span == 0 || r1 > rows || c1 > cols {
                self.err(format!(
                    "cell at {r0},{c0} spans {}x{} outside a {rows}x{cols} table",
                    c.row_span, c.col_span
                ));
                continue;
            }
            for rr in r0..r1 {
                for cc in c0..c1 {
                    grid[rr * cols + cc] += 1;
                }
            }
            if c.border_fill as usize > self.t.border_fills {
                self.err(format!(
                    "cell at {r0},{c0} uses border fill {} of {}",
                    c.border_fill, self.t.border_fills
                ));
            }
        }
        if let Some(k) = grid.iter().position(|&n| n != 1) {
            self.err(format!(
                "table grid slot {},{} is covered {} times",
                k / cols,
                k % cols,
                grid[k]
            ));
        }
        for z in &g.zones {
            if z[0] > z[2] || z[1] > z[3] || z[2] as usize >= rows || z[3] as usize >= cols {
                self.err(format!("table cell zone {z:?} lies outside the grid"));
            }
            if z[4] as usize > self.t.border_fills {
                self.err(format!(
                    "table cell zone uses border fill {} of {}",
                    z[4], self.t.border_fills
                ));
            }
        }
    }
}

struct Grid {
    rows: u16,
    cols: u16,
    row_cells: Vec<u16>,
    border_fill: u16,
    /// Start row, start column, end row, end column, border fill.
    zones: Vec<[u16; 5]>,
}

impl Grid {
    fn read(b: &[u8]) -> Option<Grid> {
        let rows = u16_at(b, 4)?;
        let cols = u16_at(b, 6)?;
        let row_cells = (0..rows as usize)
            .map(|k| u16_at(b, 18 + 2 * k))
            .collect::<Option<Vec<_>>>()?;
        let at = 18 + 2 * rows as usize;
        let border_fill = u16_at(b, at)?;
        let n = u16_at(b, at + 2).unwrap_or(0) as usize;
        let zones = (0..n)
            .filter_map(|k| {
                let o = at + 4 + 10 * k;
                Some([
                    u16_at(b, o)?,
                    u16_at(b, o + 2)?,
                    u16_at(b, o + 4)?,
                    u16_at(b, o + 6)?,
                    u16_at(b, o + 8)?,
                ])
            })
            .collect();
        Some(Grid {
            rows,
            cols,
            row_cells,
            border_fill,
            zones,
        })
    }
}

struct Cell {
    col: u16,
    row: u16,
    col_span: u16,
    row_span: u16,
    border_fill: u16,
}

impl Cell {
    fn read(b: &[u8]) -> Option<Cell> {
        Some(Cell {
            col: u16_at(b, 8)?,
            row: u16_at(b, 10)?,
            col_span: u16_at(b, 12)?,
            row_span: u16_at(b, 14)?,
            border_fill: u16_at(b, 32)?,
        })
    }
}

fn fourcc(id: u32) -> String {
    id.to_be_bytes()
        .iter()
        .map(|&b| {
            if b.is_ascii_graphic() || b == b' ' {
                b as char
            } else {
                '?'
            }
        })
        .collect()
}

fn find<'a>(streams: &'a [Stream], path: &str) -> Option<&'a Stream> {
    streams.iter().find(|s| s.path.eq_ignore_ascii_case(path))
}

/// The plain bytes of a body stream: decrypted for distribution documents,
/// inflated when the document is compressed.
fn body_stream(s: &Stream, view: bool, compressed: bool, r: &mut Report) -> Option<Vec<u8>> {
    let raw = if view {
        match hm_hwp5::distribute::open(&s.data) {
            Some((_, payload)) => payload,
            None => {
                r.error(BODY, format!("{}: distribution data does not open", s.path));
                return None;
            }
        }
    } else {
        s.data.clone()
    };
    if compressed {
        inflate(&raw, &s.path, BODY, r)
    } else {
        Some(raw)
    }
}

pub fn check(data: &[u8], r: &mut Report) {
    let Some(streams) = cfb::read(data, r) else {
        return;
    };
    let paths: Vec<String> = streams.iter().map(|s| s.path.clone()).collect();
    let Some(fh) = find(&streams, "FileHeader") else {
        r.error("header", "no FileHeader stream");
        return;
    };
    if fh.data.len() < 256 || !fh.data.starts_with(b"HWP Document File") {
        r.error("header", "FileHeader lacks its signature or is short");
        return;
    }
    let version = u32_at(&fh.data, 32).unwrap();
    if version >> 24 != 5 {
        r.error("header", format!("FileHeader version {version:#010x}"));
    }
    let flags = u32_at(&fh.data, 36).unwrap();
    if flags & 2 != 0 {
        r.warn(
            "header",
            "password-protected document; contents not checked",
        );
        return;
    }
    let compressed = flags & 1 != 0;
    let Some(info) = find(&streams, "DocInfo") else {
        r.error(DOC, "no DocInfo stream");
        return;
    };
    let info = if compressed {
        inflate(&info.data, "DocInfo", DOC, r)
    } else {
        Some(info.data.clone())
    };
    let Some(info) = info else {
        return;
    };
    let recs = records(&info, "DocInfo", DOC, r);
    let t = docinfo(&recs, &paths, r);

    let view = flags & 4 != 0 && find(&streams, "ViewText/Section0").is_some();
    let dir = if view { "ViewText" } else { "BodyText" };
    let sections: Vec<&Stream> = (0..)
        .map_while(|k| find(&streams, &format!("{dir}/Section{k}")))
        .collect();
    let stray = streams
        .iter()
        .filter(|s| {
            s.path
                .to_ascii_lowercase()
                .starts_with(&format!("{}/section", dir.to_ascii_lowercase()))
        })
        .count();
    if stray != sections.len() {
        r.error(
            BODY,
            format!(
                "{dir} has {stray} section streams, {} numbered from 0",
                sections.len()
            ),
        );
    }
    if sections.is_empty() {
        r.error(BODY, "no body sections");
    }
    if let Some(n) = t.sections
        && n != sections.len()
    {
        r.error(
            DOC,
            format!(
                "DOCUMENT_PROPERTIES lists {n} sections, the file has {}",
                sections.len()
            ),
        );
    }
    let mut used_bins = Vec::new();
    for (k, s) in sections.iter().enumerate() {
        let Some(plain) = body_stream(s, view, compressed, r) else {
            continue;
        };
        let what = format!("section {k}");
        let recs = records(&plain, &what, BODY, r);
        let mut b = Body {
            t: &t,
            r: &mut *r,
            what,
            used_bins: &mut used_bins,
            paras: 0,
            fields: Vec::new(),
        };
        let mut i = 0;
        let n = b.list(&recs, &mut i, 0);
        if n == 0 {
            b.err("section has no paragraphs".into());
        }
        // After the body: extended master pages and memos, each a list
        // header with its paragraphs, or records Hancom adds there.
        while i < recs.len() {
            let x = &recs[i];
            if x.tag == tag::LIST_HEADER {
                let declared = u32_at(x.data, 0).unwrap_or(0) as usize;
                let level = x.level;
                i += 1;
                while i < recs.len() && recs[i].level > level {
                    i += 1;
                }
                let got = b.list(&recs, &mut i, level);
                if got != declared {
                    b.err(format!(
                        "trailing list declares {declared} paragraphs, has {got}"
                    ));
                }
                continue;
            }
            if x.tag == tag::PARA_HEADER {
                let level = x.level;
                b.err(format!("stray paragraph at level {level} after the body"));
                b.para(&recs, &mut i, level);
                continue;
            }
            i += 1;
        }
    }

    // BinData streams: each referenced stream inflates, none is orphaned.
    used_bins.sort_unstable();
    used_bins.dedup();
    let entries: Vec<(usize, String, u16)> = recs
        .iter()
        .filter(|x| x.tag == tag::BIN_DATA)
        .enumerate()
        .filter_map(|(k, x)| {
            let attr = u16_at(x.data, 0)?;
            (attr & 0xF != 0).then_some(())?;
            let storage = u16_at(x.data, 2)?;
            let ext = hwp_string(x.data, 4).map(|(u, _)| String::from_utf16_lossy(&u))?;
            Some((k + 1, format!("BinData/BIN{storage:04X}.{ext}"), attr))
        })
        .collect();
    for (id, name, attr) in &entries {
        let Some(s) = find(&streams, name) else {
            continue;
        };
        let packed = match (attr >> 4) & 3 {
            1 => true,
            2 => false,
            _ => compressed,
        };
        if packed && attr & 0xF == 1 {
            let mut scratch = Report::default();
            if inflate(&s.data, name, BIN, &mut scratch).is_none() {
                r.error(BIN, format!("BinData {id} stream {name} does not inflate"));
            }
        }
        if s.data.is_empty() {
            r.error(BIN, format!("BinData {id} stream {name} is empty"));
        }
    }
    for s in &streams {
        let lower = s.path.to_ascii_lowercase();
        if lower.starts_with("bindata/")
            && !entries
                .iter()
                .any(|(_, n, _)| n.eq_ignore_ascii_case(&s.path))
        {
            r.warn(BIN, format!("stream {} has no BinData entry", s.path));
        }
    }
    let _ = used_bins;
}

/// Record counts by tag over DocInfo and every section, for comparing what
/// a save kept of records the engine does not interpret.
pub fn tag_counts(data: &[u8]) -> Option<BTreeMap<u16, usize>> {
    let mut r = Report::default();
    let streams = cfb::read(data, &mut r)?;
    let fh = find(&streams, "FileHeader")?;
    let flags = u32_at(&fh.data, 36)?;
    let compressed = flags & 1 != 0;
    let view = flags & 4 != 0 && find(&streams, "ViewText/Section0").is_some();
    let dir = if view { "ViewText" } else { "BodyText" };
    let mut out = BTreeMap::new();
    let mut add = |plain: &[u8], r: &mut Report| {
        for x in records(plain, "", BODY, r) {
            *out.entry(x.tag).or_insert(0) += 1;
        }
    };
    let info = find(&streams, "DocInfo")?;
    let info = if compressed {
        inflate(&info.data, "", DOC, &mut r)?
    } else {
        info.data.clone()
    };
    add(&info, &mut r);
    for k in 0.. {
        let Some(s) = find(&streams, &format!("{dir}/Section{k}")) else {
            break;
        };
        let plain = body_stream(s, view, compressed, &mut r)?;
        add(&plain, &mut r);
    }
    Some(out)
}
