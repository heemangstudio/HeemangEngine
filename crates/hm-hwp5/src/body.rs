//! BodyText sections: parsing records into paragraphs and encoding them back.

use hm_model::{
    CharRun, ColumnDef, Control, ControlKind, CtrlId, Diagnostic, ExtraRecord, LineSeg, NoteShape,
    PageBorderFill, PageDef, ParaHeader, Paragraph, RangeTag, RawControl, RawRecord, SectionChild,
    SectionDef, SharedBytes, is_extended_control,
};

use crate::Error;
use crate::record::{Reader, Record, Writer, copy_shifted, read_records, tag, write_record};

/// Parses a decompressed section stream into its body paragraphs.
pub fn parse_section(stream: &SharedBytes, diags: &mut Vec<Diagnostic>) -> Vec<Paragraph> {
    parse_paragraphs(stream, 0, diags)
}

/// Parses a paragraph list whose PARA_HEADER records sit at `level`, from a
/// framed record stream (a section, or the records of a list inside a
/// control, such as a table cell, sliced from its source span).
pub fn parse_paragraphs(
    stream: &SharedBytes,
    level: u16,
    diags: &mut Vec<Diagnostic>,
) -> Vec<Paragraph> {
    let (records, err) = read_records(stream);
    if let Some(e) = err {
        diags.push(Diagnostic {
            message: format!("section: {e}"),
        });
    }
    let mut p = Parser {
        stream,
        records: &records,
        i: 0,
        diags,
    };
    p.para_list(level)
}

/// Parses up to `count` consecutive paragraphs at `level` from the start of
/// `records` (a slice of one framed stream), for paragraph lists inside
/// controls. Returns them with the number of records consumed.
pub(crate) fn paragraph_run(
    stream: &SharedBytes,
    records: &[Record],
    level: u16,
    count: usize,
    diags: &mut Vec<Diagnostic>,
) -> (Vec<Paragraph>, usize) {
    let mut p = Parser {
        stream,
        records,
        i: 0,
        diags,
    };
    let mut out = Vec::new();
    while out.len() < count
        && p.i < records.len()
        && records[p.i].tag == tag::PARA_HEADER
        && records[p.i].level == level
    {
        out.push(p.paragraph());
    }
    (out, p.i)
}

struct Parser<'a, 'd> {
    stream: &'a SharedBytes,
    records: &'a [Record],
    i: usize,
    diags: &'d mut Vec<Diagnostic>,
}

impl Parser<'_, '_> {
    fn span(&self, from: usize, to: usize) -> SharedBytes {
        let start = self.records[from].start;
        let end = if to > from {
            self.records[to - 1].end
        } else {
            start
        };
        self.stream.slice(start..end)
    }

    /// Index just past the subtree of the record at `at` (all following
    /// records with a deeper level).
    fn subtree_end(&self, at: usize) -> usize {
        let level = self.records[at].level;
        let mut j = at + 1;
        while j < self.records.len() && self.records[j].level > level {
            j += 1;
        }
        j
    }

    /// Paragraphs at `level`. Records at that level that are not paragraph
    /// headers attach to the preceding paragraph as extra records.
    fn para_list(&mut self, level: u16) -> Vec<Paragraph> {
        let mut paras: Vec<Paragraph> = Vec::new();
        while self.i < self.records.len() && self.records[self.i].level >= level {
            let rec = &self.records[self.i];
            if rec.tag == tag::PARA_HEADER && rec.level == level {
                paras.push(self.paragraph());
            } else {
                // A stray record: keep it with the paragraph before it, whose
                // source span grows to cover it.
                let end = self.subtree_end(self.i);
                match paras.last_mut() {
                    Some(prev) => {
                        for r in &self.records[self.i..end] {
                            let rel = r.level as i32 - level as i32;
                            prev.extra.push(ExtraRecord {
                                slot: u32::MAX,
                                record: RawRecord {
                                    tag: r.tag,
                                    level: rel.max(0) as u16,
                                    payload: r.payload.clone(),
                                },
                            });
                        }
                        if let Some(src) = &prev.src {
                            let start = src.offset() - self.stream.offset();
                            prev.src = Some(self.stream.slice(start..self.records[end - 1].end));
                        }
                    }
                    None => self.diags.push(Diagnostic {
                        message: format!(
                            "record {:#x} before the first paragraph dropped",
                            rec.tag
                        ),
                    }),
                }
                self.i = end;
            }
        }
        paras
    }

    fn paragraph(&mut self) -> Paragraph {
        let head_at = self.i;
        let level = self.records[head_at].level;
        let end = self.subtree_end(head_at);
        let mut para = Paragraph::default();
        let mut nchars = 0u32;
        match para_header(&self.records[head_at].payload) {
            Ok((h, n)) => {
                para.header = h;
                nchars = n;
            }
            Err(e) => self.diags.push(Diagnostic {
                message: format!("paragraph header: {e}"),
            }),
        }
        let mut seen_text = false;
        let mut direct = 0u32;
        let mut j = head_at + 1;
        while j < end {
            let rec = &self.records[j];
            let sub_end = self.subtree_end(j);
            if rec.level != level + 1 {
                // Deeper records without a known parent ride along as extras.
                para.extra.push(ExtraRecord {
                    slot: direct,
                    record: RawRecord {
                        tag: rec.tag,
                        level: rec.level - level,
                        payload: rec.payload.clone(),
                    },
                });
                j += 1;
                continue;
            }
            match rec.tag {
                tag::PARA_TEXT if !seen_text => {
                    seen_text = true;
                    para.text = rec
                        .payload
                        .chunks_exact(2)
                        .map(|c| u16::from_le_bytes([c[0], c[1]]))
                        .collect();
                }
                tag::PARA_CHAR_SHAPE if para.runs.is_empty() => {
                    para.runs = rec
                        .payload
                        .chunks_exact(8)
                        .map(|c| CharRun {
                            pos: le32(c, 0),
                            shape: le32(c, 4),
                        })
                        .collect();
                }
                tag::PARA_LINE_SEG if para.line_segs.is_empty() => {
                    para.line_segs = rec.payload.chunks_exact(36).map(line_seg).collect();
                }
                tag::PARA_RANGE_TAG if para.range_tags.is_empty() => {
                    para.range_tags = rec
                        .payload
                        .chunks_exact(12)
                        .map(|c| RangeTag {
                            start: le32(c, 0),
                            end: le32(c, 4),
                            tag: le32(c, 8),
                        })
                        .collect();
                }
                tag::CTRL_HEADER => {
                    let ctl = self.control(j, sub_end);
                    para.controls.push(ctl);
                    direct += 1;
                    j = sub_end;
                    continue;
                }
                _ => {
                    // Unknown children keep the count of known children before them.
                    for r in &self.records[j..sub_end] {
                        para.extra.push(ExtraRecord {
                            slot: direct,
                            record: RawRecord {
                                tag: r.tag,
                                level: r.level - level,
                                payload: r.payload.clone(),
                            },
                        });
                    }
                    j = sub_end;
                    continue;
                }
            }
            direct += 1;
            j += 1;
        }
        if !seen_text {
            para.text = vec![hm_model::PARA_END];
        }
        if nchars as usize != para.text.len() || !self.controls_match(&para) {
            self.diags.push(Diagnostic {
                message: format!(
                    "paragraph at {} has a text length or control count mismatch",
                    self.records[head_at].start
                ),
            });
        }
        para.src = Some(self.span(head_at, end));
        self.i = end;
        para
    }

    fn controls_match(&self, para: &Paragraph) -> bool {
        let n = para
            .units()
            .filter(|(_, u)| matches!(u, hm_model::Unit::Extended { .. }))
            .count();
        n == para.controls.len()
    }

    fn control(&mut self, at: usize, end: usize) -> Control {
        let rec = &self.records[at];
        let src = Some(self.span(at, end));
        let id = CtrlId(
            rec.payload
                .get(0..4)
                .map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap())),
        );
        let level = rec.level;
        let records = self.records;
        if id == CtrlId::TABLE
            && let Some(t) = crate::table::parse(
                self.stream,
                &rec.payload,
                &records[at + 1..end],
                level,
                self.diags,
            )
        {
            return Control {
                kind: ControlKind::Table(Box::new(t)),
                src,
            };
        }
        if let Some(kind) = crate::annot::parse(id, &rec.payload, &records[at + 1..end], level) {
            return Control { kind, src };
        }
        let children: Vec<RawRecord> = records[at + 1..end]
            .iter()
            .map(|r| RawRecord {
                tag: r.tag,
                level: r.level - level,
                payload: r.payload.clone(),
            })
            .collect();
        let kind = match id {
            CtrlId::SHAPE | CtrlId::EQUATION => crate::object::parse_object(
                self.stream,
                &rec.payload,
                &self.records[at + 1..end],
                level,
                self.diags,
            )
            .map(|o| ControlKind::Object(Box::new(o)))
            .ok_or(Error::Truncated),
            CtrlId::SECTION => section_def(
                &rec.payload,
                self.stream,
                &self.records[at + 1..end],
                level,
                self.diags,
            )
            .map(|s| ControlKind::Section(Box::new(s))),
            CtrlId::HEADER
            | CtrlId::FOOTER
            | CtrlId::FOOTNOTE
            | CtrlId::ENDNOTE
            | CtrlId::AUTO_NUMBER
            | CtrlId::NEW_NUMBER
            | CtrlId::PAGE_NUMBER_POS
            | CtrlId::PAGE_HIDE
            | CtrlId::PAGE_PARITY => crate::furniture::parse(
                id,
                self.stream,
                &rec.payload,
                &self.records[at + 1..end],
                level,
                self.diags,
            )
            .ok_or(Error::Truncated),
            CtrlId::COLUMNS if children.is_empty() => {
                column_def(&rec.payload).map(ControlKind::Columns)
            }
            _ => Err(Error::Truncated),
        };
        let kind = kind.unwrap_or_else(|_| {
            ControlKind::Raw(RawControl {
                id,
                header: rec.payload.clone(),
                children,
                xml: None,
            })
        });
        Control { kind, src }
    }
}

fn le32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

fn line_seg(c: &[u8]) -> LineSeg {
    let i = |o| le32(c, o) as i32;
    LineSeg {
        text_start: le32(c, 0),
        vpos: i(4),
        height: i(8),
        text_height: i(12),
        baseline: i(16),
        spacing: i(20),
        col_start: i(24),
        width: i(28),
        flags: le32(c, 32),
    }
}

/// Returns the header and the stored character count (last-paragraph bit cleared).
fn para_header(payload: &SharedBytes) -> Result<(ParaHeader, u32), Error> {
    let mut r = Reader::new(payload);
    let nchars = r.u32()? & 0x7FFF_FFFF;
    let control_mask = r.u32()?;
    let para_shape = r.u16()?;
    let style = r.u8()?;
    let break_kind = r.u8()?;
    let _runs = r.u16()?;
    let _ranges = r.u16()?;
    let _segs = r.u16()?;
    let instance_id = r.u32()?;
    let merge_flag = if r.remaining() >= 2 {
        Some(r.u16()?)
    } else {
        None
    };
    let header = ParaHeader {
        para_shape,
        style,
        break_kind,
        instance_id,
        merge_flag,
        tail: r.tail(),
        control_mask,
    };
    Ok((header, nchars))
}

fn section_def(
    payload: &SharedBytes,
    stream: &SharedBytes,
    records: &[Record],
    level: u16,
    diags: &mut Vec<Diagnostic>,
) -> Result<SectionDef, Error> {
    let mut r = Reader::new(payload);
    r.u32()?;
    let mut s = SectionDef {
        attr: r.u32()?,
        column_gap: r.i16()?,
        vertical_grid: r.i16()?,
        horizontal_grid: r.i16()?,
        default_tab: r.i32()?,
        outline_numbering: r.u16()?,
        page_number: r.u16()?,
        figure_number: r.u16()?,
        table_number: r.u16()?,
        equation_number: r.u16()?,
        ..Default::default()
    };
    if r.remaining() >= 2 {
        s.language = Some(r.u16()?);
    }
    s.tail = r.tail();
    let mut i = 0;
    while i < records.len() {
        let c = &records[i];
        if c.level == level + 1 && c.tag == tag::LIST_HEADER {
            if let Some((list, used)) =
                crate::furniture::para_list(stream, &records[i..], level + 1, diags)
            {
                s.children.push(SectionChild::MasterPage(list));
                i += used;
                continue;
            }
        }
        let typed = if c.level == level + 1 {
            let mut r = Reader::new(&c.payload);
            match c.tag {
                tag::PAGE_DEF => page_def(&mut r).ok().map(SectionChild::Page),
                tag::FOOTNOTE_SHAPE => note_shape(&mut r).ok().map(SectionChild::NoteShape),
                tag::PAGE_BORDER_FILL => page_border(&mut r).ok().map(SectionChild::PageBorder),
                _ => None,
            }
        } else {
            None
        };
        s.children.push(typed.unwrap_or_else(|| {
            SectionChild::Raw(RawRecord {
                tag: c.tag,
                level: c.level - level,
                payload: c.payload.clone(),
            })
        }));
        i += 1;
    }
    Ok(s)
}

fn page_def(r: &mut Reader<'_>) -> Result<PageDef, Error> {
    Ok(PageDef {
        width: r.u32()?,
        height: r.u32()?,
        margin_left: r.u32()?,
        margin_right: r.u32()?,
        margin_top: r.u32()?,
        margin_bottom: r.u32()?,
        margin_header: r.u32()?,
        margin_footer: r.u32()?,
        gutter: r.u32()?,
        attr: r.u32()?,
        tail: r.tail(),
    })
}

fn note_shape(r: &mut Reader<'_>) -> Result<NoteShape, Error> {
    Ok(NoteShape {
        attr: r.u32()?,
        user_char: r.u16()?,
        prefix: r.u16()?,
        suffix: r.u16()?,
        start: r.u16()?,
        separator_length: r.i32()?,
        space_above: r.i16()?,
        space_below: r.i16()?,
        space_between: r.i16()?,
        line_kind: r.u8()?,
        line_width: r.u8()?,
        line_color: r.u32()?,
        tail: r.tail(),
    })
}

fn page_border(r: &mut Reader<'_>) -> Result<PageBorderFill, Error> {
    Ok(PageBorderFill {
        attr: r.u32()?,
        offsets: [r.i16()?, r.i16()?, r.i16()?, r.i16()?],
        border_fill: r.u16()?,
        tail: r.tail(),
    })
}

fn column_def(payload: &SharedBytes) -> Result<ColumnDef, Error> {
    let mut r = Reader::new(payload);
    r.u32()?;
    let attr = r.u16()?;
    let mut c = ColumnDef {
        attr,
        ..Default::default()
    };
    if !c.equal_widths() && c.count() > 1 {
        c.attr2 = r.u16()?;
        for _ in 0..c.count() {
            c.columns.push((r.i16()?, r.i16()?));
        }
    } else {
        c.gap = r.i16()?;
        c.attr2 = r.u16()?;
    }
    c.line_kind = r.u8()?;
    c.line_width = r.u8()?;
    c.line_color = r.u32()?;
    c.tail = r.tail();
    Ok(c)
}

// ---- encoding ----

/// Encodes a section's paragraphs at level 0.
pub fn encode_section(paras: &[Paragraph]) -> Vec<u8> {
    let mut out = Vec::new();
    let (memos, clean) = crate::annot::encode_memos(paras);
    if memos.is_empty() {
        encode_list(&mut out, paras, 0);
        return out;
    }
    let (last, rest) = paras.split_last().unwrap();
    for p in rest {
        encode_paragraph(&mut out, p, 0, false, false);
    }
    // The memo lists follow the last paragraph's records; its source holds
    // them while they are unchanged.
    let fresh = !(clean && last.is_clean());
    encode_paragraph(&mut out, last, 0, true, fresh);
    if fresh {
        out.extend_from_slice(&memos);
    }
    out
}

/// Encodes a paragraph list at `level`, copying clean paragraphs.
pub fn encode_list(out: &mut Vec<u8>, paras: &[Paragraph], level: u16) {
    for (k, p) in paras.iter().enumerate() {
        let last = k + 1 == paras.len();
        encode_paragraph(out, p, level, last, false);
    }
}

fn source_level(src: &[u8]) -> i32 {
    src.get(0..4).map_or(0, |b| {
        ((u32::from_le_bytes(b.try_into().unwrap()) >> 10) & 0x3FF) as i32
    })
}

fn encode_paragraph(out: &mut Vec<u8>, p: &Paragraph, level: u16, last: bool, rebuild: bool) {
    if let Some(src) = p.src.as_ref().filter(|_| p.is_clean() && !rebuild) {
        let delta = level as i32 - source_level(src);
        let start = out.len();
        copy_shifted(out, src, delta);
        // The last-paragraph bit depends on position, which may have changed.
        if let Some(b) = out.get_mut(start + 4..start + 8) {
            let n = u32::from_le_bytes((&*b).try_into().unwrap());
            let fixed = if last {
                n | 0x8000_0000
            } else {
                n & 0x7FFF_FFFF
            };
            b.copy_from_slice(&fixed.to_le_bytes());
        }
        return;
    }
    let child = level + 1;
    let has_text = p.text.len() > 1;
    let mut w = Writer::default();
    let n = p.text.len() as u32;
    w.u32(if last { n | 0x8000_0000 } else { n });
    w.u32(control_mask(&p.text, p.header.control_mask));
    w.u16(p.header.para_shape);
    w.u8(p.header.style);
    w.u8(p.header.break_kind);
    w.u16(p.runs.len() as u16);
    w.u16(p.range_tags.len() as u16);
    w.u16(p.line_segs.len() as u16);
    w.u32(p.header.instance_id);
    if let Some(m) = p.header.merge_flag {
        w.u16(m);
    }
    w.bytes(&p.header.tail);
    write_record(out, tag::PARA_HEADER, level, &w.0);

    // Direct children in canonical order; extras re-enter at their slot.
    let mut known: Vec<(u16, Vec<u8>)> = Vec::new();
    if has_text {
        known.push((
            tag::PARA_TEXT,
            p.text.iter().flat_map(|u| u.to_le_bytes()).collect(),
        ));
    }
    if !p.runs.is_empty() {
        let mut w = Writer::default();
        for r in &p.runs {
            w.u32(r.pos);
            w.u32(r.shape);
        }
        known.push((tag::PARA_CHAR_SHAPE, w.0));
    }
    if !p.line_segs.is_empty() {
        let mut w = Writer::default();
        for s in &p.line_segs {
            w.u32(s.text_start);
            for v in [
                s.vpos,
                s.height,
                s.text_height,
                s.baseline,
                s.spacing,
                s.col_start,
                s.width,
            ] {
                w.i32(v);
            }
            w.u32(s.flags);
        }
        known.push((tag::PARA_LINE_SEG, w.0));
    }
    if !p.range_tags.is_empty() {
        let mut w = Writer::default();
        for t in &p.range_tags {
            w.u32(t.start);
            w.u32(t.end);
            w.u32(t.tag);
        }
        known.push((tag::PARA_RANGE_TAG, w.0));
    }
    let mut next_extra = 0usize;
    let flush = |out: &mut Vec<u8>, next: &mut usize, upto: u32| {
        while let Some(e) = p.extra.get(*next).filter(|e| e.slot <= upto) {
            write_record(out, e.record.tag, level + e.record.level, &e.record.payload);
            *next += 1;
        }
    };
    let mut direct = 0u32;
    for (t, payload) in &known {
        flush(out, &mut next_extra, direct);
        write_record(out, *t, child, payload);
        direct += 1;
    }
    for c in &p.controls {
        flush(out, &mut next_extra, direct);
        encode_control(out, c, child);
        direct += 1;
    }
    flush(out, &mut next_extra, u32::MAX);
}

/// The control mask: one bit per control code present in the text.
///
/// Hancom leaves the paragraph end out. It sets the soft hyphen bit (24) in
/// some paragraphs and not in others, so that bit follows `stored` while a
/// soft hyphen is present.
pub fn control_mask(text: &[u16], stored: u32) -> u32 {
    const HYPHEN: u32 = 1 << 24;
    let mut mask = 0u32;
    let mut i = 0;
    while i < text.len() {
        let u = text[i];
        if u < 32 {
            if u != hm_model::PARA_END {
                mask |= 1 << u;
            }
            i += hm_model::control_width(u) as usize;
        } else {
            i += 1;
        }
    }
    if mask & HYPHEN != 0 {
        mask = (mask & !HYPHEN) | (stored & HYPHEN);
    }
    mask
}

fn encode_control(out: &mut Vec<u8>, c: &Control, level: u16) {
    if let Some(src) = &c.src {
        copy_shifted(out, src, level as i32 - source_level(src));
        return;
    }
    match &c.kind {
        ControlKind::Raw(r) => {
            write_record(out, tag::CTRL_HEADER, level, &r.header);
            for ch in &r.children {
                write_record(out, ch.tag, level + ch.level, &ch.payload);
            }
        }
        ControlKind::Section(s) => {
            let mut w = Writer::default();
            w.u32(CtrlId::SECTION.0);
            w.u32(s.attr);
            w.i16(s.column_gap);
            w.i16(s.vertical_grid);
            w.i16(s.horizontal_grid);
            w.i32(s.default_tab);
            for v in [
                s.outline_numbering,
                s.page_number,
                s.figure_number,
                s.table_number,
                s.equation_number,
            ] {
                w.u16(v);
            }
            if let Some(l) = s.language {
                w.u16(l);
            }
            w.bytes(&s.tail);
            write_record(out, tag::CTRL_HEADER, level, &w.0);
            for ch in &s.children {
                let mut w = Writer::default();
                let t = match ch {
                    SectionChild::Page(p) => {
                        for v in [
                            p.width,
                            p.height,
                            p.margin_left,
                            p.margin_right,
                            p.margin_top,
                            p.margin_bottom,
                            p.margin_header,
                            p.margin_footer,
                            p.gutter,
                            p.attr,
                        ] {
                            w.u32(v);
                        }
                        w.bytes(&p.tail);
                        tag::PAGE_DEF
                    }
                    SectionChild::NoteShape(n) => {
                        w.u32(n.attr);
                        for v in [n.user_char, n.prefix, n.suffix, n.start] {
                            w.u16(v);
                        }
                        w.i32(n.separator_length);
                        for v in [n.space_above, n.space_below, n.space_between] {
                            w.i16(v);
                        }
                        w.u8(n.line_kind);
                        w.u8(n.line_width);
                        w.u32(n.line_color);
                        w.bytes(&n.tail);
                        tag::FOOTNOTE_SHAPE
                    }
                    SectionChild::PageBorder(b) => {
                        w.u32(b.attr);
                        b.offsets.iter().for_each(|&v| w.i16(v));
                        w.u16(b.border_fill);
                        w.bytes(&b.tail);
                        tag::PAGE_BORDER_FILL
                    }
                    SectionChild::Raw(r) => {
                        write_record(out, r.tag, level + r.level, &r.payload);
                        continue;
                    }
                    SectionChild::MasterPage(list) => {
                        crate::furniture::encode_para_list(out, list, level + 1);
                        continue;
                    }
                };
                write_record(out, t, level + 1, &w.0);
            }
        }
        ControlKind::Table(t) => crate::table::encode(out, t, level),
        ControlKind::Field(_)
        | ControlKind::Bookmark(_)
        | ControlKind::IndexMark(_)
        | ControlKind::Form(_)
        | ControlKind::Compose(_)
        | ControlKind::Dutmal(_) => {
            crate::annot::encode(out, &c.kind, level);
        }
        ControlKind::Object(o) => crate::object::encode_object(out, o, level),
        ControlKind::HeaderFooter(_)
        | ControlKind::Note(_)
        | ControlKind::AutoNumber(_)
        | ControlKind::NewNumber(_)
        | ControlKind::PageNumber(_)
        | ControlKind::PageHide(_)
        | ControlKind::PageParity(_) => {
            crate::furniture::encode(out, &c.kind, level);
        }
        ControlKind::Columns(c) => {
            let mut w = Writer::default();
            w.u32(CtrlId::COLUMNS.0);
            w.u16(c.attr);
            if !c.equal_widths() && c.count() > 1 {
                w.u16(c.attr2);
                for &(cw, g) in &c.columns {
                    w.i16(cw);
                    w.i16(g);
                }
            } else {
                w.i16(c.gap);
                w.u16(c.attr2);
            }
            w.u8(c.line_kind);
            w.u8(c.line_width);
            w.u32(c.line_color);
            w.bytes(&c.tail);
            write_record(out, tag::CTRL_HEADER, level, &w.0);
        }
    }
}

/// True for codes that need a control record; exposed for edit validation.
pub fn needs_record(code: u16) -> bool {
    is_extended_control(code)
}
