//! The stored file mapped into the shared model, the way Hancom converts
//! an HWP 3 document to HWP 5: lengths times four (1/1800 inch to HWPUNIT),
//! shapes collected into deduplicated tables, special characters turned
//! into controls.

use std::collections::HashMap;

use hm_model::{
    AutoNumber, BinStore, BorderFill, BorderLine, CharRun, CharShape, ColumnDef,
    CompatibleDocument, Compose, Control, ControlKind, CtrlId, DocInfo, DocMeta, Document,
    FaceName, Fill, Format, HeaderFooter, HwpString, LayoutCompatibility, NewNumber, Note,
    NoteShape, PageBorderFill, PageDef, PageHide, PageNumberPos, PageParity, ParaHeader, ParaList,
    ParaShape, Paragraph, Section, SectionChild, SectionDef, SharedBytes, SolidFill, Style, TabDef,
    TabStop, Version, break_kind,
};

use crate::file::{self, File, Item, Para, Special, i16_at, u16_at};
use crate::object;

/// 1/1800 inch to HWPUNIT.
pub fn hu(v: impl Into<i32>) -> i32 {
    v.into() * 4
}

/// Character and line colors by index: black, blue, green, cyan, red,
/// magenta, yellow, white, as COLORREF.
pub const COLORS: [u32; 8] = [
    0x000000, 0xFF0000, 0x00FF00, 0xFFFF00, 0x0000FF, 0xFF00FF, 0x00FFFF, 0xFFFFFF,
];

pub fn color(i: impl Into<u32>) -> u32 {
    COLORS[(i.into() & 7) as usize]
}

/// White blended toward `c` by `pct` percent.
pub fn tint(c: u32, pct: u8) -> u32 {
    let pct = pct.min(100) as u32;
    let mix = |shift: u32| {
        let v = (c >> shift) & 0xFF;
        (255 - (255 - v) * pct / 100) << shift
    };
    mix(0) | mix(8) | mix(16)
}

/// Font names Hancom renames when it converts: the Hangul 97 faces became
/// the Hanyang faces of the same design.
fn face_name(n: &str) -> &str {
    match n {
        "신명조" => "한양신명조",
        "중고딕" => "한양중고딕",
        "견고딕" => "한양견고딕",
        "견명조" => "한양견명조",
        "궁서" => "한양궁서",
        "그래픽" => "한양그래픽",
        "해서" => "한양해서",
        n => n,
    }
}

pub struct Conv<'a> {
    pub file: &'a File,
    pub info: DocInfo,
    char_shapes: HashMap<file::CharShape, u32>,
    para_shapes: HashMap<(usize, u8), u16>,
    tabs: HashMap<Vec<file::Tab>, u16>,
    border_fills: HashMap<BorderKey, u16>,
    /// Embedded pictures by stored name: BinData id.
    pub images: HashMap<String, u16>,
    pub bins: Vec<SharedBytes>,
    pub z: i32,
    pub instance: u32,
    /// Columns of the body flow: count, separator line, gap.
    columns: (u8, u8, u16),
}

/// A border fill as the converter builds them: four sides (kind, width
/// index), a fill color and diagonals.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct BorderKey {
    pub sides: [(u8, u8); 4],
    pub fill: Option<u32>,
    /// 0 none, 1 backslash, 2 slash, 3 both.
    pub diagonal: u8,
}

impl<'a> Conv<'a> {
    pub fn new(file: &'a File) -> Self {
        let mut c = Conv {
            file,
            info: DocInfo::default(),
            char_shapes: HashMap::new(),
            para_shapes: HashMap::new(),
            tabs: HashMap::new(),
            border_fills: HashMap::new(),
            images: HashMap::new(),
            bins: Vec::new(),
            z: 0,
            instance: 0x4000_0000,
            columns: (1, 0, 0),
        };
        for (slot, names) in file.fonts.iter().enumerate() {
            c.info.faces[slot] = names
                .iter()
                .map(|n| FaceName {
                    name: HwpString::from_text(face_name(n)),
                    ..Default::default()
                })
                .collect();
            if c.info.faces[slot].is_empty() {
                c.info.faces[slot].push(FaceName {
                    name: HwpString::from_text("한양신명조"),
                    ..Default::default()
                });
            }
        }
        // The first border fill draws nothing; paragraphs and pages use it.
        c.border_fill(BorderKey::default());
        for (id, data) in &file.extras {
            if *id == 1 && data.len() >= 32 {
                let name = crate::text::kchars(&data[..16]).to_ascii_lowercase();
                c.bins.push(SharedBytes::from(data[32..].to_vec()));
                c.info.bin_data.push(hm_model::BinDataEntry {
                    attr: 1,
                    storage_id: Some(c.bins.len() as u16),
                    extension: Some(HwpString::from_text(&crate::text::kchars(&data[16..32]))),
                    ..Default::default()
                });
                c.images.insert(name, c.bins.len() as u16);
            }
        }
        c
    }

    pub fn next_instance(&mut self) -> u32 {
        self.instance += 1;
        self.instance
    }

    pub fn char_shape(&mut self, s: &file::CharShape) -> u32 {
        if let Some(&id) = self.char_shapes.get(s) {
            return id;
        }
        let mut attr = 0u32;
        let a = s.attr;
        if a & 1 != 0 {
            attr |= 1;
        }
        if a & 2 != 0 {
            attr |= 2;
        }
        if a & 4 != 0 {
            attr |= 1 << 2;
        }
        if a & 8 != 0 {
            attr |= 1 << 8;
        }
        if a & 16 != 0 {
            attr |= 1 << 11;
        }
        if a & 32 != 0 {
            attr |= 1 << 15;
        }
        if a & 64 != 0 {
            attr |= 1 << 16;
        }
        let faces = std::array::from_fn(|k| {
            let n = self.info.faces[k].len().max(1);
            (s.faces[k] as usize).min(n - 1) as u16
        });
        let shape = CharShape {
            faces,
            ratios: s.ratios.map(|r| r.clamp(50, 200)),
            spacings: s.spacings.map(|v| v.clamp(-50, 50)),
            rel_sizes: [100; 7],
            offsets: [0; 7],
            base_size: hu(s.size as i32),
            attr,
            shadow_offset: [10, 10],
            color: color(s.color),
            underline_color: color(s.color),
            shade_color: if s.shade == 0 {
                SolidFill::NO_COLOR
            } else {
                tint(color(s.shade_color), s.shade)
            },
            shadow_color: 0xC0C0C0,
            border_fill: Some(1),
            strike_color: Some(0),
            ..Default::default()
        };
        self.info.char_shapes.push(shape);
        let id = self.info.char_shapes.len() as u32 - 1;
        self.char_shapes.insert(*s, id);
        id
    }

    pub fn border_fill(&mut self, k: BorderKey) -> u16 {
        if let Some(&id) = self.border_fills.get(&k) {
            return id;
        }
        let line = |(kind, width): (u8, u8)| BorderLine {
            kind,
            width,
            color: 0,
        };
        let fill = Fill {
            kind: k.fill.map_or(0, |_| Fill::SOLID),
            solid: k.fill.map(|c| SolidFill {
                background: c,
                pattern_color: 0,
                pattern: -1,
            }),
            ..Default::default()
        };
        let mut attr = 0u16;
        if k.diagonal & 1 != 0 {
            attr |= 2 << 5;
        }
        if k.diagonal & 2 != 0 {
            attr |= 2 << 2;
        }
        self.info.border_fills.push(BorderFill {
            attr,
            sides: k.sides.map(line),
            diagonal: BorderLine {
                kind: (k.diagonal != 0) as u8,
                width: 1,
                color: 0,
            },
            fill: Some(fill),
            ..Default::default()
        });
        let id = self.info.border_fills.len() as u16;
        self.border_fills.insert(k, id);
        id
    }

    fn tab_def(&mut self, tabs: &[file::Tab]) -> u16 {
        if let Some(&id) = self.tabs.get(tabs) {
            return id;
        }
        self.info.tab_defs.push(TabDef {
            attr: 0,
            tabs: tabs
                .iter()
                .map(|t| TabStop {
                    position: hu(t.pos as i32) * 2,
                    kind: t.kind.min(3),
                    fill: if t.leader != 0 { 3 } else { 0 },
                    reserved: 0,
                })
                .collect(),
            ..Default::default()
        });
        let id = self.info.tab_defs.len() as u16 - 1;
        self.tabs.insert(tabs.to_vec(), id);
        id
    }

    /// The model shape of stored paragraph shape `idx` with the paragraph
    /// flags that Hancom keeps in the shape.
    pub fn para_shape(&mut self, s: &file::ParaShape, key: (usize, u8)) -> u16 {
        if let Some(&id) = self.para_shapes.get(&key) {
            return id;
        }
        let flags = key.1;
        // Stored alignments: 0 left breaking anywhere, 1 left, 2 right,
        // 3 center, 4 distribute, 5 divide, 6 justify, 7 divide.
        let (align, breaks) = match s.align {
            0 => (1, (2 << 5) | (1 << 7)),
            1 => (1, 0),
            2 => (2, 0),
            3 => (3, 0),
            4 => (4, 0),
            5 | 7 => (5, 1 << 7),
            _ => (0, 1 << 7),
        };
        let mut attr1 = (align << 2) | breaks | ((s.word_spacing.min(75) as u32) << 9);
        let fixed = s.line_spacing & 0x8000 != 0;
        let spacing = if fixed {
            hu((s.line_spacing & 0x7FFF) as i32)
        } else {
            s.line_spacing as i32
        };
        if fixed {
            attr1 |= 1;
        }
        if flags & 0x40 != 0 {
            attr1 |= 1 << 16;
        }
        if s.border_connect != 0 {
            attr1 |= 1 << 28;
        }
        let border_fill = if s.border != 0 || s.shade != 0 {
            let side = if s.border != 0 { (1, 1) } else { (0, 0) };
            self.border_fill(BorderKey {
                sides: [side; 4],
                fill: (s.shade != 0).then(|| tint(0, s.shade)),
                diagonal: 0,
            })
        } else {
            1
        };
        let tab_def = self.tab_def(&s.tabs);
        self.info.para_shapes.push(ParaShape {
            attr1,
            // A hanging indent hangs from the stored left margin: the
            // first line starts there and the rest move right.
            margin_left: hu(s.left as i32 + (s.indent as i32).min(0)) * 2,
            margin_right: hu(s.right as i32) * 2,
            indent: hu(s.indent as i32) * 2,
            space_before: hu(s.above as i32) * 2,
            space_after: hu(s.below as i32) * 2,
            line_spacing_legacy: spacing,
            tab_def,
            numbering: 0,
            border_fill,
            border_offsets: [0; 4],
            attr2: Some(0),
            attr3: Some(fixed as u32),
            line_spacing: Some(spacing as u32),
            ..Default::default()
        });
        let id = self.info.para_shapes.len() as u16 - 1;
        self.para_shapes.insert(key, id);
        id
    }

    /// Converts a paragraph list. `body` marks the section's own list,
    /// where column changes become column definitions.
    ///
    /// `width` is the width the list's lines are laid out in (the column
    /// or cell interior), when known: the stored lines then become line
    /// segments the layout replays, as Hancom does when it opens the file.
    pub fn paras(&mut self, list: &[Para], body: bool, width: Option<i32>) -> Vec<Paragraph> {
        let mut out = Vec::with_capacity(list.len());
        for (k, p) in list.iter().enumerate() {
            let width = match body {
                true => Some(self.column_width(p.shape)),
                false => width,
            };
            let mut para = self.para(p, width);
            // A stored page or column break ends its paragraph: the next
            // one starts the new page, as Hancom converts it.
            if let Some(prev) = k.checked_sub(1).map(|i| list[i].flags) {
                if prev & 2 != 0 {
                    para.header.break_kind |= break_kind::PAGE;
                }
                if prev & 1 != 0 {
                    para.header.break_kind |= break_kind::COLUMN;
                }
            }
            if body {
                let s = &self.file.para_shapes[p.shape];
                let cols = (s.columns.max(1), s.column_line, s.column_gap);
                if k == 0 {
                    self.columns = cols;
                    let lead = [self.section_def(), self.column_def(cols)];
                    prepend(&mut para, lead);
                    para.header.break_kind |= break_kind::SECTION | break_kind::MULTI_COLUMN;
                } else if cols != self.columns {
                    self.columns = cols;
                    let c = self.column_def(cols);
                    prepend(&mut para, [c]);
                    para.header.break_kind |= break_kind::MULTI_COLUMN;
                }
            }
            out.push(para);
        }
        out
    }

    /// The width of one column of the body under stored shape `shape`.
    fn column_width(&self, shape: usize) -> i32 {
        let s = &self.file.para_shapes[shape];
        let n = s.columns.max(1) as i32;
        let body = self.body_width();
        (body - hu(s.column_gap as i32) * (n - 1)) / n
    }

    pub fn body_width(&self) -> i32 {
        self.page_size().0 - self.margin(14) - self.margin(16) - self.margin(22)
    }

    fn para(&mut self, p: &Para, width: Option<i32>) -> Paragraph {
        let shape = self.para_shape(&self.file.para_shapes[p.shape].clone(), (p.shape, p.flags));
        let mut text: Vec<u16> = Vec::with_capacity(p.len as usize + 8);
        let mut controls = Vec::new();
        // Model position of each stored unit position where an item starts.
        let mut starts: Vec<(u16, u32)> = Vec::with_capacity(p.items.len());
        for (at, item) in &p.items {
            starts.push((*at, text.len() as u32));
            match item {
                Item::Char(13) => {}
                Item::Char(c) => match crate::text::hchar(*c) {
                    Some(ch) => {
                        let mut buf = [0u16; 2];
                        text.extend_from_slice(ch.encode_utf16(&mut buf));
                    }
                    None => text.push(0xFFFD),
                },
                Item::Special(s) => self.special(s, &mut text, &mut controls),
            }
        }
        text.push(hm_model::PARA_END);
        let model_pos = |at: u16| -> u32 {
            match starts.partition_point(|s| s.0 <= at) {
                0 => 0,
                i => starts[i - 1].1,
            }
        };
        let mut runs: Vec<CharRun> = Vec::new();
        for (at, cs) in &p.runs {
            let mut cs = *cs;
            // In nonempty legacy paragraphs, import caps a CR-only shape
            // at the last stored line's height. Text runs keep their size.
            if p.len > 1
                && *at == p.len - 1
                && p.items
                    .last()
                    .is_some_and(|(_, item)| matches!(item, Item::Char(13)))
                && let Some(line) = p.lines.last().filter(|l| l.height > 0 && l.start <= *at)
            {
                cs.size = cs.size.min(line.height);
            }
            let shape = self.char_shape(&cs);
            let pos = model_pos(*at);
            match runs.last_mut() {
                Some(r) if r.pos == pos => r.shape = shape,
                Some(r) if r.shape == shape => {}
                _ => runs.push(CharRun { pos, shape }),
            }
        }
        if runs.is_empty() {
            runs.push(CharRun { pos: 0, shape: 0 });
        }
        let shape_ref = &self.file.para_shapes[p.shape];
        let mut para = Paragraph {
            header: ParaHeader {
                para_shape: shape,
                style: p.style,
                ..Default::default()
            },
            text,
            runs,
            controls,
            ..Default::default()
        };
        para.line_segs = width.map_or_else(Vec::new, |w| {
            let left = hu(shape_ref.left as i32 + (shape_ref.indent as i32).min(0));
            let right = hu(shape_ref.right as i32);
            segments(
                p,
                &model_pos,
                left,
                w - left - right,
                shape_ref,
                &para,
                &self.info.char_shapes,
            )
        });
        set_mask(&mut para);
        para
    }

    fn special(&mut self, s: &Special, text: &mut Vec<u16>, controls: &mut Vec<Control>) {
        let d = &s.data;
        let ext =
            |text: &mut Vec<u16>, controls: &mut Vec<Control>, code: u16, id: CtrlId, kind| {
                text.extend_from_slice(&[code, id.0 as u16, (id.0 >> 16) as u16, 0, 0, 0, 0, code]);
                controls.push(Control { kind, src: None });
            };
        match s.code {
            9 => {
                let w = hu(u16_at(d, 0) as i32) as u32;
                let leader = if u16_at(d, 2) != 0 { 3 } else { 0 };
                text.extend_from_slice(&[9, w as u16, (w >> 16) as u16, leader, 0, 0, 0, 9]);
            }
            10 | 11 | 14 => {
                if let Some((id, kind)) = object::convert(self, s) {
                    ext(text, controls, 11, id, kind);
                }
            }
            16 => {
                let footer = d.get(8) == Some(&1);
                let height = if footer {
                    self.margin(20)
                } else {
                    self.margin(18)
                };
                let body_w = self.body_width();
                let paras = self.paras(&s.lists[0], false, Some(body_w));
                let hf = HeaderFooter {
                    footer,
                    attr: d.get(9).copied().unwrap_or(0).min(2) as u32,
                    tail: SharedBytes::empty(),
                    list: ParaList {
                        count: paras.len() as u32,
                        attr: if footer { 2 << 5 } else { 0 },
                        text_size: Some((body_w, height)),
                        paras,
                        ..Default::default()
                    },
                };
                let id = if footer {
                    CtrlId::FOOTER
                } else {
                    CtrlId::HEADER
                };
                ext(
                    text,
                    controls,
                    16,
                    id,
                    ControlKind::HeaderFooter(Box::new(hf)),
                );
            }
            17 => {
                let endnote = u16_at(d, 10) == 1;
                let width = hu(u16_at(d, 12) as i32);
                let paras = self.paras(&s.lists[0], false, (width > 0).then_some(width));
                let note = Note {
                    endnote,
                    number: u16_at(d, 8) as u32 + 1,
                    list: ParaList {
                        count: paras.len() as u32,
                        paras,
                        ..Default::default()
                    },
                    ..Default::default()
                };
                let id = if endnote {
                    CtrlId::ENDNOTE
                } else {
                    CtrlId::FOOTNOTE
                };
                ext(text, controls, 17, id, ControlKind::Note(Box::new(note)));
            }
            18 => {
                let n = AutoNumber {
                    attr: (u16_at(d, 0) & 0xF) as u32,
                    number: u16_at(d, 2),
                    ..Default::default()
                };
                ext(
                    text,
                    controls,
                    18,
                    CtrlId::AUTO_NUMBER,
                    ControlKind::AutoNumber(n),
                );
            }
            19 => {
                let n = NewNumber {
                    attr: (u16_at(d, 0) & 0xF) as u32,
                    number: u16_at(d, 2),
                    ..Default::default()
                };
                ext(
                    text,
                    controls,
                    21,
                    CtrlId::NEW_NUMBER,
                    ControlKind::NewNumber(n),
                );
            }
            20 => {
                let pos = u16_at(d, 0).min(10) as u32;
                let form = u16_at(d, 2);
                let shape = match form % 3 {
                    1 => 2,
                    2 => 3,
                    _ => 0,
                };
                let n = PageNumberPos {
                    attr: shape | pos << 8,
                    side: if form >= 3 { b'-' as u16 } else { 0 },
                    ..Default::default()
                };
                ext(
                    text,
                    controls,
                    21,
                    CtrlId::PAGE_NUMBER_POS,
                    ControlKind::PageNumber(n),
                );
            }
            21 => {
                if u16_at(d, 0) == 1 {
                    let h = u16_at(d, 2) as u32;
                    let attr = (h & 3) | (h & 4) << 3 | (h & 8);
                    let p = PageHide {
                        attr,
                        ..Default::default()
                    };
                    ext(
                        text,
                        controls,
                        21,
                        CtrlId::PAGE_HIDE,
                        ControlKind::PageHide(p),
                    );
                } else {
                    let p = PageParity {
                        attr: 2,
                        ..Default::default()
                    };
                    ext(
                        text,
                        controls,
                        21,
                        CtrlId::PAGE_PARITY,
                        ControlKind::PageParity(p),
                    );
                }
            }
            23 => {
                let chars: Vec<u16> = (0..3)
                    .map(|k| u16_at(d, k * 2))
                    .filter(|&c| c != 0)
                    .filter_map(crate::text::hchar)
                    .flat_map(|c| {
                        let mut b = [0u16; 2];
                        c.encode_utf16(&mut b).to_vec()
                    })
                    .collect();
                let c = Compose {
                    text: HwpString(chars),
                    frame: None,
                    tail: SharedBytes::empty(),
                };
                ext(
                    text,
                    controls,
                    23,
                    CtrlId::COMPOSE,
                    ControlKind::Compose(Box::new(c)),
                );
            }
            28 => {
                for c in outline(d).encode_utf16() {
                    text.push(c);
                }
            }
            24 => text.push(24),
            30 => text.push(30),
            31 => text.push(31),
            _ => {}
        }
    }

    fn margin(&self, offset: usize) -> i32 {
        hu(u16_at(&self.file.info.raw, offset) as i32)
    }

    /// Paper width and height as laid out.
    fn page_size(&self) -> (i32, i32) {
        let (w, h) = (self.margin(8), self.margin(6));
        if self.file.info.landscape() {
            (h, w)
        } else {
            (w, h)
        }
    }

    fn section_def(&mut self) -> Control {
        let raw = &self.file.info.raw;
        let m = |o| self.margin(o) as u32;
        let page = PageDef {
            width: m(8),
            height: m(6),
            margin_top: m(10),
            margin_bottom: m(12),
            margin_left: m(14),
            margin_right: m(16),
            margin_header: m(18),
            margin_footer: m(20),
            gutter: m(22),
            attr: self.file.info.landscape() as u32,
            ..Default::default()
        };
        let (pw, _) = self.page_size();
        let body_w = pw - (m(14) + m(16) + m(22)) as i32;
        let (line_kind, separator_length) = match raw[111] {
            1 => (1, body_w / 3),
            2 => (1, body_w),
            3 => (0, 0),
            _ => (1, -1),
        };
        let footnote = NoteShape {
            suffix: if raw[110] == 0 { 0 } else { raw[110] as u16 },
            start: u16_at(raw, 100).max(1),
            separator_length,
            space_above: hu(i16_at(raw, 104)) as i16,
            space_below: hu(i16_at(raw, 106)) as i16,
            space_between: hu(i16_at(raw, 108)) as i16,
            line_kind,
            line_width: 1,
            ..Default::default()
        };
        let endnote = NoteShape {
            suffix: b')' as u16,
            start: 1,
            space_above: 864,
            space_below: 576,
            line_width: 1,
            ..Default::default()
        };
        let border = PageBorderFill {
            attr: 1,
            offsets: [1417; 4],
            border_fill: 1,
            ..Default::default()
        };
        let line = crate::object::line_kind(u16_at(raw, 120).min(255) as u8);
        let first = if line.0 != 0 {
            let bf = self.border_fill(BorderKey {
                sides: [line; 4],
                ..Default::default()
            });
            PageBorderFill {
                offsets: std::array::from_fn(|k| hu(i16_at(raw, 112 + 2 * k)) as i16),
                border_fill: bf,
                ..border.clone()
            }
        } else {
            border.clone()
        };
        let def = SectionDef {
            column_gap: 1134,
            default_tab: 8000,
            page_number: self.file.info.page_start(),
            children: vec![
                SectionChild::Page(page),
                SectionChild::NoteShape(footnote),
                SectionChild::NoteShape(endnote),
                SectionChild::PageBorder(first),
                SectionChild::PageBorder(border.clone()),
                SectionChild::PageBorder(border),
            ],
            ..Default::default()
        };
        Control {
            kind: ControlKind::Section(Box::new(def)),
            src: None,
        }
    }

    fn column_def(&mut self, (count, line, gap): (u8, u8, u16)) -> Control {
        let count = count.clamp(1, 255) as u16;
        let def = ColumnDef {
            attr: (count << 2) | (1 << 12),
            gap: hu(gap as i32) as i16,
            line_kind: if line > 0 { 1 } else { 0 },
            line_width: 1,
            ..Default::default()
        };
        Control {
            kind: ControlKind::Columns(def),
            src: None,
        }
    }
}

/// Puts section-level controls before a paragraph's text.
fn prepend<const N: usize>(para: &mut Paragraph, controls: [Control; N]) {
    let mut text = Vec::with_capacity(para.text.len() + 8 * N);
    for c in &controls {
        let id = c.id();
        text.extend_from_slice(&[2, id.0 as u16, (id.0 >> 16) as u16, 0, 0, 0, 0, 2]);
    }
    let shift = text.len() as u32;
    text.extend_from_slice(&para.text);
    para.text = text;
    for r in para.runs.iter_mut().skip(1) {
        r.pos += shift;
    }
    for line in para.line_segs.iter_mut().skip(1) {
        line.text_start += shift;
    }
    let mut all: Vec<Control> = controls.into_iter().collect();
    all.append(&mut para.controls);
    para.controls = all;
    set_mask(para);
}

fn set_mask(p: &mut Paragraph) {
    let mut mask = 0u32;
    for (_, u) in p.units() {
        match u {
            hm_model::Unit::CharControl(13) | hm_model::Unit::Char(_) => {}
            hm_model::Unit::CharControl(c) | hm_model::Unit::Inline(c) => mask |= 1 << c,
            hm_model::Unit::Extended { code, .. } => mask |= 1 << code,
        }
    }
    p.header.control_mask = mask;
}

pub fn document(f: &File) -> Document {
    let mut c = Conv::new(f);
    let paras = c.paras(&f.body, true, None);
    for s in &f.styles {
        let cs = c.char_shape(&s.char_shape);
        // Style shapes are keyed past the body's so they never collide.
        let key = (usize::MAX - c.info.styles.len(), 0);
        let ps = c.para_shape(&s.para_shape, key);
        c.info.styles.push(Style {
            local_name: HwpString::from_text(&s.name),
            english_name: HwpString::default(),
            next: c.info.styles.len() as u8,
            para_shape: ps,
            char_shape: cs as u16,
            ..Default::default()
        });
    }
    if c.info.char_shapes.is_empty() {
        c.char_shape(&file::CharShape {
            size: 250,
            ratios: [100; 7],
            ..Default::default()
        });
    }
    let raw = &f.info.raw;
    c.info.properties.section_count = 1;
    c.info.properties.page_start = f.info.page_start().max(1);
    c.info.properties.footnote_start = u16_at(raw, 100).max(1);
    c.info.properties.endnote_start = 1;
    c.info.properties.figure_start = 1;
    c.info.properties.table_start = 1;
    c.info.properties.equation_start = 1;
    c.info.compatible = Some(CompatibleDocument::default());
    c.info.layout_compat = Some(LayoutCompatibility::default());
    let bins = std::mem::take(&mut c.bins);
    let info = c.info;
    Document {
        meta: DocMeta {
            format: Format::Hwp3,
            version: Version(0x0300_0000),
            flags: 0,
        },
        info,
        sections: vec![Section {
            paras,
            src: None,
            source_index: None,
            last_page_master: None,
        }],
        source: None,
        bins: Some(BinStore::new(move |id| {
            bins.get((id as usize).checked_sub(1)?).cloned()
        })),
        diagnostics: Vec::new(),
    }
}

/// Line segments from the stored lines: Hangul's own line breaks and
/// membership and object-inclusive height. Spacing comes from the paragraph
/// mode and imported character sizes, independently of paragraph placement.
fn segments(
    p: &Para,
    model_pos: &dyn Fn(u16) -> u32,
    left: i32,
    width: i32,
    shape: &file::ParaShape,
    para: &Paragraph,
    char_shapes: &[CharShape],
) -> Vec<hm_model::LineSeg> {
    if width <= 0 || p.lines.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<hm_model::LineSeg> = Vec::with_capacity(p.lines.len());
    let mut items = para.units().peekable();
    let mut run = 0;
    for (k, l) in p.lines.iter().enumerate() {
        let height = hu(l.height as i32);
        let start = model_pos(l.start);
        if out.last().is_some_and(|s| s.text_start >= start) {
            return Vec::new();
        }
        let end = p.lines.get(k + 1).map_or(u32::MAX, |n| model_pos(n.start));
        // The object's character style contributes to percent spacing, not
        // its box height. Zero-width controls do not contribute a text style.
        let mut text_size: Option<i32> = None;
        let mut tab_size = 0;
        while let Some(&(pos, unit)) = items.peek() {
            if pos >= end {
                break;
            }
            items.next();
            while para.runs.get(run + 1).is_some_and(|r| r.pos <= pos) {
                run += 1;
            }
            let contributes = match unit {
                hm_model::Unit::Char(_) | hm_model::Unit::CharControl(10 | 13 | 30 | 31) => true,
                hm_model::Unit::Extended { index, .. } => {
                    let c = &para.controls[index as usize];
                    match &c.kind {
                        ControlKind::AutoNumber(n) => matches!(
                            n.kind(),
                            hm_model::NumberKind::Page
                                | hm_model::NumberKind::Figure
                                | hm_model::NumberKind::Table
                                | hm_model::NumberKind::Equation
                        ),
                        _ => c.object_common().is_some_and(|o| o.treat_as_char()),
                    }
                }
                _ => false,
            };
            if let Some(s) = char_shapes.get(para.runs[run].shape as usize) {
                if contributes {
                    text_size = Some(text_size.unwrap_or(0).max(s.base_size));
                } else if matches!(unit, hm_model::Unit::Inline(9)) {
                    tab_size = tab_size.max(s.base_size);
                }
            }
        }
        let text_size = text_size.unwrap_or(tab_size);
        let spacing = if shape.line_spacing & 0x8000 == 0 {
            hu(hm_model::units::mul_div(
                text_size / 4,
                shape.line_spacing as i32 - 100,
                100,
            ))
        } else {
            hu((shape.line_spacing & 0x7FFF) as i32) - height
        };
        // The saved exclusion is column-relative. The preceding stored word
        // is the aligned text origin and must not narrow the segment.
        let col_start = if l.exclusion == 0 {
            left
        } else {
            left.max(hu(l.exclusion))
        };
        out.push(hm_model::LineSeg {
            text_start: start,
            vpos: hu(l.y as i32),
            height,
            text_height: height,
            baseline: (height * 85 + 50) / 100,
            spacing,
            col_start,
            width: width - (col_start - left),
            flags: hm_model::LineSeg::FIRST_SEGMENT | hm_model::LineSeg::LAST_SEGMENT,
        });
    }
    out
}

/// The visible text of an outline number (special character 28): the
/// level's number in its format, between its decoration characters.
fn outline(d: &[u8]) -> String {
    if u16_at(d, 0) != 1 {
        return String::new();
    }
    let level = (d.get(3).copied().unwrap_or(0) as usize).min(6);
    let value = u16_at(d, 4 + level * 2) as u32 + 1;
    // Hangul's own outline: 1. 가. 1) 가) (1) (가) ①.
    const DEFAULT: [(u16, u16, u16); 7] = [
        (0, 0, b'.' as u16),
        (5, 0, b'.' as u16),
        (0, 0, b')' as u16),
        (5, 0, b')' as u16),
        (0, b'(' as u16, b')' as u16),
        (5, b'(' as u16, b')' as u16),
        (7, 0, 0),
    ];
    let (format, before, after) = match d.get(2).copied().unwrap_or(0) {
        0 | 128 => (
            u16_at(d, 18 + level * 2),
            u16_at(d, 32 + level * 4),
            u16_at(d, 34 + level * 4),
        ),
        _ => DEFAULT[level],
    };
    let mut s = String::new();
    let push = |s: &mut String, c: u16| {
        if c != 0
            && let Some(ch) = crate::text::hchar(c)
        {
            s.push(ch);
        }
    };
    push(&mut s, before);
    match format {
        128 => {}
        0..=12 => s.push_str(&number_text(format, value)),
        bullet => push(&mut s, bullet),
    }
    push(&mut s, after);
    s
}

fn number_text(format: u16, n: u32) -> String {
    const GANADA: &str = "가나다라마바사아자차카타파하";
    const JAMO: &str = "ㄱㄴㄷㄹㅁㅂㅅㅇㅈㅊㅋㅌㅍㅎ";
    const HANJA: &str = "一二三四五六七八九十";
    let cycle = |set: &str| {
        let v: Vec<char> = set.chars().collect();
        v[(n as usize - 1) % v.len()].to_string()
    };
    let roman = |upper: bool| {
        let mut n = n;
        let mut s = String::new();
        for (v, r) in [
            (1000, "m"),
            (900, "cm"),
            (500, "d"),
            (400, "cd"),
            (100, "c"),
            (90, "xc"),
            (50, "l"),
            (40, "xl"),
            (10, "x"),
            (9, "ix"),
            (5, "v"),
            (4, "iv"),
            (1, "i"),
        ] {
            while n >= v {
                s.push_str(r);
                n -= v;
            }
        }
        if upper { s.to_uppercase() } else { s }
    };
    let letter = |base: u8| ((base + ((n - 1) % 26) as u8) as char).to_string();
    match format {
        1 => roman(true),
        2 => roman(false),
        3 => letter(b'A'),
        4 => letter(b'a'),
        5 => cycle(GANADA),
        6 => cycle(JAMO),
        7 if n <= 20 => char::from_u32(0x2460 + n - 1).map_or(n.to_string(), String::from),
        8 => cycle(HANJA),
        _ => n.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_exclusion_intersects_margins_in_column_coordinates() {
        let p = Para {
            flags: 0,
            style: 0,
            shape: 0,
            lines: vec![
                file::Line {
                    height: 225,
                    exclusion: 9291,
                    ..Default::default()
                },
                file::Line {
                    start: 17,
                    height: 225,
                    y: 360,
                    ..Default::default()
                },
            ],
            runs: vec![(
                0,
                file::CharShape {
                    size: 225,
                    ..Default::default()
                },
            )],
            items: (0..18)
                .map(|at| (at, Item::Char(if at == 17 { 13 } else { 65 })))
                .collect(),
            len: 18,
        };
        for (margin, excluded, full) in [
            (0, (37164, 13860), (0, 51024)),
            (250, (37164, 13860), (1000, 50024)),
            (9300, (37200, 13824), (37200, 13824)),
        ] {
            let file = File {
                info: file::Info { raw: vec![0; 128] },
                fonts: Default::default(),
                styles: vec![],
                para_shapes: vec![file::ParaShape {
                    left: margin,
                    line_spacing: 160,
                    ..Default::default()
                }],
                body: vec![],
                extras: vec![],
            };
            let para = Conv::new(&file).para(&p, Some(51024));
            assert_eq!(
                para.line_segs
                    .iter()
                    .map(|l| l.text_start)
                    .collect::<Vec<_>>(),
                [0, 17]
            );
            assert_eq!(
                para.line_segs
                    .iter()
                    .map(|l| (l.col_start, l.width))
                    .collect::<Vec<_>>(),
                [excluded, full],
            );
            assert_eq!(para.text[17..], [13]);
            assert!(para.src.is_none());
        }
    }

    fn imported_end(count: u16, run: u16, size: u16, heights: &[u16]) -> CharShape {
        let content = file::CharShape {
            size: 200,
            ratios: [100; 7],
            ..Default::default()
        };
        let terminal = file::CharShape {
            size,
            attr: 2,
            shade: 15,
            ..content
        };
        let p = Para {
            flags: 0,
            style: 0,
            shape: 0,
            lines: heights
                .iter()
                .enumerate()
                .map(|(i, &height)| file::Line {
                    start: if i == 0 { 0 } else { count - 1 },
                    height,
                    ..Default::default()
                })
                .collect(),
            runs: vec![(0, content), (run, terminal)],
            items: (0..=count)
                .map(|at| (at, Item::Char(if at == count { 13 } else { 32 })))
                .collect(),
            len: count + 1,
        };
        let file = File {
            info: file::Info { raw: vec![0; 128] },
            fonts: Default::default(),
            styles: vec![],
            para_shapes: vec![file::ParaShape::default()],
            body: vec![],
            extras: vec![],
        };
        let mut c = Conv::new(&file);
        let para = c.para(&p, None);
        assert_eq!(para.text.last(), Some(&13));
        c.info.char_shapes[para.runs.last().unwrap().shape as usize].clone()
    }

    #[test]
    fn imported_cr_size_is_capped_by_the_last_positive_stored_line_height() {
        for (count, size, heights, expected) in [
            (128, 250, vec![200], 800),
            (129, 250, vec![200], 800),
            (128, 250, vec![225], 900),
            (128, 250, vec![250], 1000),
            (128, 150, vec![200], 600),
            (128, 300, vec![200], 800),
            (134, 250, vec![200, 225], 900),
            (134, 250, vec![200, 250], 1000),
            (134, 250, vec![200, 1987], 1000),
        ] {
            let shape = imported_end(count, count, size, &heights);
            assert_eq!(shape.base_size, expected);
            assert_eq!(shape.attr, 2);
            assert_eq!(shape.shade_color, tint(0, 15));
        }
    }

    #[test]
    fn imported_text_runs_and_missing_line_heights_keep_their_size() {
        assert_eq!(imported_end(0, 0, 250, &[200]).base_size, 1000);
        assert_eq!(imported_end(128, 127, 250, &[200]).base_size, 1000);
        assert_eq!(imported_end(134, 128, 250, &[200]).base_size, 1000);
        assert_eq!(imported_end(128, 128, 250, &[]).base_size, 1000);
        assert_eq!(imported_end(128, 128, 250, &[0]).base_size, 1000);
    }

    #[test]
    fn multiline_conversion_keeps_membership_after_section_and_column_controls() {
        let content = file::CharShape {
            size: 200,
            ratios: [100; 7],
            ..Default::default()
        };
        let p = Para {
            flags: 0,
            style: 0,
            shape: 0,
            lines: vec![
                file::Line {
                    start: 0,
                    height: 200,
                    y: 0,
                    ..Default::default()
                },
                file::Line {
                    start: 128,
                    height: 250,
                    y: 320,
                    ..Default::default()
                },
            ],
            runs: vec![
                (0, content),
                (
                    134,
                    file::CharShape {
                        size: 250,
                        ..content
                    },
                ),
            ],
            items: (0..135)
                .map(|at| (at, Item::Char(if at == 134 { 13 } else { 32 })))
                .collect(),
            len: 135,
        };
        let mut second = p.clone();
        second.shape = 1;
        let mut raw = vec![0; 128];
        raw[8..10].copy_from_slice(&12756u16.to_le_bytes());
        let shape = file::ParaShape {
            line_spacing: 0x8000 | 300,
            ..Default::default()
        };
        let f = File {
            info: file::Info { raw },
            fonts: Default::default(),
            styles: vec![],
            para_shapes: vec![
                shape.clone(),
                file::ParaShape {
                    columns: 2,
                    ..shape
                },
            ],
            body: vec![p, second],
            extras: vec![],
        };
        let d = document(&f);
        for (p, prefix) in d.sections[0].paras.iter().zip([16, 8]) {
            let lines = &p.line_segs;
            assert_eq!(
                lines.iter().map(|l| l.text_start).collect::<Vec<_>>(),
                [0, 128 + prefix]
            );
            assert_eq!(
                p.text[lines[1].text_start as usize..],
                [32, 32, 32, 32, 32, 32, 13]
            );
            assert_eq!(p.runs.last().unwrap().pos, 134 + prefix);
            assert_eq!((lines[0].spacing, lines[1].spacing), (400, 200));
        }
    }
}
