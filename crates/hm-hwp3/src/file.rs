//! The file as stored: fixed header blocks, font names, styles, nested
//! paragraph lists with their special characters, and the trailing
//! information blocks.

use crate::Error;

/// Little-endian reads over a byte slice.
pub struct Cur<'a> {
    b: &'a [u8],
    pub at: usize,
}

impl<'a> Cur<'a> {
    pub fn new(b: &'a [u8]) -> Self {
        Cur { b, at: 0 }
    }

    pub fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let end = self.at.checked_add(n).ok_or(Error::Truncated)?;
        let s = self.b.get(self.at..end).ok_or(Error::Truncated)?;
        self.at = end;
        Ok(s)
    }

    pub fn u8(&mut self) -> Result<u8, Error> {
        Ok(self.take(1)?[0])
    }

    pub fn u16(&mut self) -> Result<u16, Error> {
        let s = self.take(2)?;
        Ok(u16::from_le_bytes([s[0], s[1]]))
    }

    pub fn u32(&mut self) -> Result<u32, Error> {
        let s = self.take(4)?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }

    pub fn left(&self) -> usize {
        self.b.len().saturating_sub(self.at)
    }
}

pub fn u16_at(b: &[u8], o: usize) -> u16 {
    b.get(o..o + 2)
        .map_or(0, |s| u16::from_le_bytes([s[0], s[1]]))
}

pub fn i16_at(b: &[u8], o: usize) -> i16 {
    u16_at(b, o) as i16
}

pub fn u32_at(b: &[u8], o: usize) -> u32 {
    b.get(o..o + 4)
        .map_or(0, |s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

pub fn i32_at(b: &[u8], o: usize) -> i32 {
    u32_at(b, o) as i32
}

/// A character shape (31 bytes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct CharShape {
    /// Size in 1/1800 inch.
    pub size: u16,
    pub faces: [u8; 7],
    pub ratios: [u8; 7],
    pub spacings: [i8; 7],
    /// Colors are indexes 0-7: black, blue, green, cyan, red, magenta,
    /// yellow, white.
    pub shade_color: u8,
    pub color: u8,
    /// Shade percent.
    pub shade: u8,
    /// Bits: italic, bold, underline, outline, shadow, superscript,
    /// subscript, font-fitted blank.
    pub attr: u8,
}

impl CharShape {
    pub const LEN: usize = 31;

    fn parse(b: &[u8]) -> CharShape {
        let arr = |o: usize| -> [u8; 7] { b[o..o + 7].try_into().unwrap() };
        CharShape {
            size: u16_at(b, 0),
            faces: arr(2),
            ratios: arr(9),
            spacings: arr(16).map(|v| v as i8),
            shade_color: b[23],
            color: b[24],
            shade: b[25],
            attr: b[26],
        }
    }
}

/// A tab stop: kind (left, right, center, decimal), leader, position.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Tab {
    pub kind: u8,
    pub leader: u8,
    pub pos: u16,
}

/// A paragraph shape (187 bytes). Lengths in 1/1800 inch.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ParaShape {
    pub left: u16,
    pub right: u16,
    pub indent: i16,
    /// Percent, or an absolute length when bit 15 is set.
    pub line_spacing: u16,
    pub below: u16,
    pub word_spacing: u8,
    /// 0 justify, 1 left, 2 right, 3 center, 4 distribute, 5 divide.
    pub align: u8,
    pub tabs: Vec<Tab>,
    pub columns: u8,
    pub column_line: u8,
    pub column_gap: u16,
    pub shade: u8,
    pub border: u8,
    pub border_connect: u8,
    pub above: u16,
}

impl ParaShape {
    pub const LEN: usize = 187;

    fn parse(b: &[u8]) -> ParaShape {
        let tabs = (0..40)
            .map(|k| {
                let o = 12 + k * 4;
                Tab {
                    kind: b[o],
                    leader: b[o + 1],
                    pos: u16_at(b, o + 2),
                }
            })
            .filter(|t| t.pos != 0)
            .collect();
        ParaShape {
            left: u16_at(b, 0),
            right: u16_at(b, 2),
            indent: i16_at(b, 4),
            line_spacing: u16_at(b, 6),
            below: u16_at(b, 8),
            word_spacing: b[10],
            align: b[11],
            tabs,
            columns: b[172],
            column_line: b[173],
            column_gap: u16_at(b, 174),
            shade: b[180],
            border: b[181],
            border_connect: b[182],
            above: u16_at(b, 183),
        }
    }
}

/// One stored line.
#[derive(Clone, Copy, Debug, Default)]
pub struct Line {
    /// First unit of the line.
    pub start: u16,
    pub space_adjust: i16,
    pub height: u16,
    /// Vertical position Hangul computed, from the top of the flow.
    pub y: u16,
    /// Left exclusion boundary in column coordinates; zero means none.
    pub exclusion: u16,
    /// Nonzero low words store an inclusive right boundary in flow coordinates.
    /// With bit 15 set, bits 0/1 mark page/column boundaries. Zero is unknown.
    pub breaks: u16,
}

/// A special character with the data stored after its code.
#[derive(Clone, Debug)]
pub struct Special {
    pub code: u16,
    /// The data after the leading code: the fixed block of fixed-size
    /// codes, the information block of structured ones.
    pub data: Vec<u8>,
    /// The paragraph lists the code carries (cells, caption, note text).
    pub lists: Vec<Vec<Para>>,
}

#[derive(Clone, Debug)]
pub enum Item {
    Char(u16),
    Special(Box<Special>),
}

#[derive(Clone, Debug)]
pub struct Para {
    pub flags: u8,
    pub style: u8,
    /// Index into [`File::para_shapes`].
    pub shape: usize,
    pub lines: Vec<Line>,
    /// Character shape changes by unit position; the first is at 0.
    pub runs: Vec<(u16, CharShape)>,
    /// Items with the unit position each starts at.
    pub items: Vec<(u16, Item)>,
    /// Units, the final CR included.
    pub len: u16,
}

pub struct Style {
    pub name: String,
    pub char_shape: CharShape,
    pub para_shape: ParaShape,
}

/// The document information block (128 bytes).
pub struct Info {
    pub raw: Vec<u8>,
}

impl Info {
    pub fn paper(&self) -> [u16; 9] {
        std::array::from_fn(|k| u16_at(&self.raw, 6 + k * 2))
    }
    pub fn landscape(&self) -> bool {
        self.raw[5] != 0
    }
    pub fn compressed(&self) -> bool {
        self.raw[124] != 0
    }
    pub fn info_len(&self) -> usize {
        u16_at(&self.raw, 126) as usize
    }
    pub fn password(&self) -> bool {
        u16_at(&self.raw, 96) != 0
    }
    pub fn page_start(&self) -> u16 {
        u16_at(&self.raw, 98)
    }
}

pub struct File {
    pub info: Info,
    pub fonts: [Vec<String>; 7],
    pub styles: Vec<Style>,
    pub para_shapes: Vec<ParaShape>,
    pub body: Vec<Para>,
    /// Trailing information blocks: id and content.
    pub extras: Vec<(u32, Vec<u8>)>,
}

pub const SIGNATURE: &[u8; 30] = b"HWP Document File V3.00 \x1a\x01\x02\x03\x04\x05";

/// Largest inflated body accepted.
const LIMIT: usize = 512 << 20;

pub fn parse(bytes: &[u8]) -> Result<File, Error> {
    if !bytes.starts_with(SIGNATURE) {
        return Err(Error::NotHwp3);
    }
    let mut c = Cur::new(bytes);
    c.take(30)?;
    let info = Info {
        raw: c.take(128)?.to_vec(),
    };
    if info.password() {
        return Err(Error::Encrypted);
    }
    c.take(1008)?;
    c.take(info.info_len())?;
    let rest = &bytes[c.at..];
    let inflated;
    let body: &[u8] = if info.compressed() {
        inflated = inflate(rest)?;
        &inflated
    } else {
        rest
    };
    let mut c = Cur::new(body);
    let mut fonts: [Vec<String>; 7] = Default::default();
    for list in &mut fonts {
        let n = c.u16()?;
        for _ in 0..n {
            list.push(crate::text::kchars(c.take(40)?));
        }
    }
    let n = c.u16()?;
    let mut styles = Vec::with_capacity(n as usize);
    for _ in 0..n {
        let b = c.take(20 + CharShape::LEN + ParaShape::LEN)?;
        styles.push(Style {
            name: crate::text::kchars(&b[..20]),
            char_shape: CharShape::parse(&b[20..]),
            para_shape: ParaShape::parse(&b[20 + CharShape::LEN..]),
        });
    }
    let mut p = Parser {
        c,
        shapes: Vec::new(),
        depth: 0,
    };
    let body_list = p.list()?;
    let mut extras = Vec::new();
    while p.c.left() >= 8 {
        let id = p.c.u32()?;
        let len = p.c.u32()? as usize;
        if id == 0 && len == 0 {
            break;
        }
        let n = len.min(p.c.left());
        extras.push((id, p.c.take(n)?.to_vec()));
    }
    Ok(File {
        info,
        fonts,
        styles,
        para_shapes: p.shapes,
        body: body_list,
        extras,
    })
}

fn inflate(data: &[u8]) -> Result<Vec<u8>, Error> {
    use miniz_oxide::inflate::{self, TINFLStatus};
    match inflate::decompress_to_vec_with_limit(data, LIMIT) {
        Ok(v) => Ok(v),
        Err(e) if e.status == TINFLStatus::HasMoreOutput => Err(Error::TooLarge),
        Err(e) if !e.output.is_empty() => Ok(e.output),
        Err(e) => Err(Error::Corrupt(format!("deflate: {:?}", e.status))),
    }
}

/// A special character's data, its paragraph lists and its units.
type Stored = (Vec<u8>, Vec<Vec<Para>>, u16);

struct Parser<'a> {
    c: Cur<'a>,
    shapes: Vec<ParaShape>,
    depth: u32,
}

/// Size of the fixed block, leading code included, of codes that carry
/// no paragraph list and no length.
fn fixed_len(code: u16) -> Option<usize> {
    Some(match code {
        7 => 84,
        8 => 96,
        9 => 8,
        18..=21 => 8,
        22 => 24,
        23 => 10,
        24 | 25 => 6,
        26 => 246,
        28 => 64,
        30 | 31 => 4,
        _ => return None,
    })
}

impl Parser<'_> {
    /// A paragraph list up to and including its empty end paragraph.
    fn list(&mut self) -> Result<Vec<Para>, Error> {
        self.depth += 1;
        if self.depth > 64 {
            return Err(Error::Corrupt("paragraph lists nest too deep".into()));
        }
        let mut out = Vec::new();
        let mut current: Option<usize> = None;
        loop {
            let follow = self.c.u8()?;
            let len = self.c.u16()?;
            if len == 0 {
                self.c.take(40)?;
                break;
            }
            let nlines = self.c.u16()?;
            let has_shapes = self.c.u8()?;
            let flags = self.c.u8()?;
            let _special = self.c.u32()?;
            let style = self.c.u8()?;
            let rep = CharShape::parse(self.c.take(CharShape::LEN)?);
            let shape = match (follow, current) {
                (0, _) | (_, None) => {
                    let s = ParaShape::parse(self.c.take(ParaShape::LEN)?);
                    self.shapes.push(s);
                    self.shapes.len() - 1
                }
                (_, Some(k)) => k,
            };
            current = Some(shape);
            let mut lines = Vec::with_capacity(nlines as usize);
            for _ in 0..nlines {
                let b = self.c.take(14)?;
                lines.push(Line {
                    start: u16_at(b, 0),
                    space_adjust: i16_at(b, 2),
                    height: u16_at(b, 4),
                    y: u16_at(b, 6),
                    exclusion: u16_at(b, 10),
                    breaks: u16_at(b, 12),
                });
            }
            let mut runs = vec![(0u16, rep)];
            if has_shapes != 0 {
                runs.clear();
                let mut last = rep;
                for k in 0..len {
                    if self.c.u8()? != 1 {
                        last = CharShape::parse(self.c.take(CharShape::LEN)?);
                    }
                    if runs.last().is_none_or(|r| r.1 != last) {
                        runs.push((k, last));
                    }
                }
            }
            let items = self.items(len)?;
            out.push(Para {
                flags,
                style,
                shape,
                lines,
                runs,
                items,
                len,
            });
        }
        self.depth -= 1;
        Ok(out)
    }

    fn items(&mut self, len: u16) -> Result<Vec<(u16, Item)>, Error> {
        let mut items = Vec::new();
        let mut pos = 0u16;
        while pos < len {
            let code = self.c.u16()?;
            let at = pos;
            if code >= 32 || code == 13 {
                items.push((at, Item::Char(code)));
                pos += 1;
                continue;
            }
            let (data, lists, units) = self.special(code)?;
            items.push((at, Item::Special(Box::new(Special { code, data, lists }))));
            pos = pos.saturating_add(units);
        }
        Ok(items)
    }

    /// The data and lists of special `code`, whose code was read, and the
    /// units it takes in the paragraph.
    fn special(&mut self, code: u16) -> Result<Stored, Error> {
        if let Some(n) = fixed_len(code) {
            let data = self.c.take(n - 2)?.to_vec();
            return Ok((data, Vec::new(), (n / 2) as u16));
        }
        // An 8-byte header: code, a length or reserved word, code again.
        let word = self.c.u32()?;
        self.c.u16()?;
        let mut lists = Vec::new();
        let data = match code {
            10 => {
                let info = self.c.take(84)?.to_vec();
                let cells = u16_at(&info, 80) as usize;
                let mut data = info;
                data.extend_from_slice(self.c.take(27 * cells)?);
                for _ in 0..cells {
                    lists.push(self.list()?);
                }
                lists.push(self.list()?);
                data
            }
            11 => {
                let extra = self.c.take(4)?;
                let n = u32_at(extra, 0) as usize;
                let mut data = extra.to_vec();
                data.extend_from_slice(self.c.take(344)?);
                let start = self.c.at;
                let drawing = self.c.take(n)?;
                data.extend_from_slice(drawing);
                if data[74] == 3 {
                    // Text boxes inside a drawing keep paragraph lists in
                    // the same stream format; read them in order.
                    let end = self.c.at;
                    for (at, len) in drawing_texts(drawing) {
                        self.c.at = start + at;
                        let list = self.list()?;
                        if self.c.at > start + at + len {
                            return Err(Error::Corrupt("drawing text overruns".into()));
                        }
                        lists.push(list);
                    }
                    self.c.at = end;
                }
                lists.push(self.list()?);
                data
            }
            14 => self.c.take(84)?.to_vec(),
            15 => {
                let d = self.c.take(8)?.to_vec();
                lists.push(self.list()?);
                d
            }
            16 => {
                let d = self.c.take(10)?.to_vec();
                lists.push(self.list()?);
                d
            }
            17 => {
                let d = self.c.take(14)?.to_vec();
                lists.push(self.list()?);
                d
            }
            _ => self.c.take(word as usize)?.to_vec(),
        };
        Ok((data, lists, 4))
    }
}

/// Offsets and lengths of the paragraph lists of the text boxes in a
/// drawing (after the 348-byte picture block), in stored order.
pub fn drawing_texts(b: &[u8]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut c = Cur::new(b);
    let Ok(head) = c.u32() else { return out };
    if c.take(head as usize).is_err() {
        return out;
    }
    while c.left() >= 4 {
        let Ok(len) = c.u32() else { break };
        let Ok(head) = c.take(len as usize) else {
            break;
        };
        let kind = u16_at(head, 0);
        if kind == 9 {
            continue;
        }
        let Ok(n1) = c.u32() else { break };
        if c.take(n1 as usize).is_err() {
            break;
        }
        let Ok(n2) = c.u32() else { break };
        let at = c.at;
        if c.take(n2 as usize).is_err() {
            break;
        }
        if kind == 6 && n2 > 0 {
            out.push((at, n2 as usize));
        }
    }
    out
}
