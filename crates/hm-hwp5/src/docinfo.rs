//! DocInfo: parsing records into [`DocInfo`] and encoding them back.

use hm_model::{
    BinDataEntry, BorderFill, BorderLine, Bullet, CharShape, CompatibleDocument, Diagnostic,
    DocInfo, DocProperties, FaceName, IdMappings, InfoItem, InfoSlot, LayoutCompatibility,
    Numbering, NumberingLevel, ParaHead, ParaShape, RawRecord, Script, SharedBytes, Style, TabDef,
    TabStop,
};

use crate::Error;
use crate::record::{Reader, Record, Writer, read_records, tag, write_record};

/// Parses a decompressed DocInfo stream.
pub fn parse(stream: &SharedBytes, diags: &mut Vec<Diagnostic>) -> DocInfo {
    let (records, err) = read_records(stream);
    if let Some(e) = err {
        diags.push(Diagnostic {
            message: format!("DocInfo: {e}"),
        });
    }
    let mut info = DocInfo::default();
    let mut face_counts = [0usize; 7];
    let mut faces_seen = 0usize;
    for rec in &records {
        let src = Some(stream.slice(rec.start..rec.end));
        let level = rec.level;
        let item = match rec.tag {
            tag::DOCUMENT_PROPERTIES => typed(rec, diags, props, |v| {
                info.properties = DocProperties { src, ..v };
                InfoItem::Properties
            }),
            tag::ID_MAPPINGS => typed(rec, diags, id_mappings, |v| {
                for (s, slot) in face_counts.iter_mut().enumerate() {
                    *slot = v.count(IdMappings::FACES + s);
                }
                info.id_mappings = IdMappings { src, ..v };
                InfoItem::IdMappings
            }),
            tag::BIN_DATA => typed(rec, diags, bin_data, |v| {
                info.bin_data.push(BinDataEntry { src, ..v });
                InfoItem::BinData(info.bin_data.len() as u32 - 1)
            }),
            tag::FACE_NAME => typed(rec, diags, face_name, |v| {
                // Faces fill the script slots in order, by the counts in ID_MAPPINGS.
                let mut acc = 0;
                let mut slot = 6;
                for (s, &n) in face_counts.iter().enumerate() {
                    acc += n;
                    if faces_seen < acc {
                        slot = s;
                        break;
                    }
                }
                faces_seen += 1;
                let list = &mut info.faces[slot];
                list.push(FaceName { src, ..v });
                InfoItem::Face(Script::from_index(slot), list.len() as u32 - 1)
            }),
            tag::BORDER_FILL => typed(rec, diags, border_fill, |v| {
                info.border_fills.push(BorderFill { src, ..v });
                InfoItem::BorderFill(info.border_fills.len() as u32 - 1)
            }),
            tag::CHAR_SHAPE => typed(rec, diags, char_shape, |v| {
                info.char_shapes.push(CharShape { src, ..v });
                InfoItem::CharShape(info.char_shapes.len() as u32 - 1)
            }),
            tag::TAB_DEF => typed(rec, diags, tab_def, |v| {
                info.tab_defs.push(TabDef { src, ..v });
                InfoItem::TabDef(info.tab_defs.len() as u32 - 1)
            }),
            tag::NUMBERING => typed(rec, diags, numbering, |v| {
                info.numberings.push(Numbering { src, ..v });
                InfoItem::Numbering(info.numberings.len() as u32 - 1)
            }),
            tag::BULLET => typed(rec, diags, bullet, |v| {
                info.bullets.push(Bullet { src, ..v });
                InfoItem::Bullet(info.bullets.len() as u32 - 1)
            }),
            tag::PARA_SHAPE => typed(rec, diags, para_shape, |v| {
                info.para_shapes.push(ParaShape { src, ..v });
                InfoItem::ParaShape(info.para_shapes.len() as u32 - 1)
            }),
            tag::STYLE => typed(rec, diags, style, |v| {
                info.styles.push(Style { src, ..v });
                InfoItem::Style(info.styles.len() as u32 - 1)
            }),
            tag::COMPATIBLE_DOCUMENT => typed(rec, diags, compatible, |v| {
                info.compatible = Some(CompatibleDocument { src, ..v });
                InfoItem::Compatible
            }),
            tag::LAYOUT_COMPATIBILITY => typed(rec, diags, layout_compat, |v| {
                info.layout_compat = Some(LayoutCompatibility { src, ..v });
                InfoItem::LayoutCompat
            }),
            _ => None,
        };
        let item = item.unwrap_or_else(|| {
            info.raw.push(RawRecord {
                tag: rec.tag,
                level,
                payload: rec.payload.clone(),
            });
            InfoItem::Raw(info.raw.len() as u32 - 1)
        });
        info.order.push(InfoSlot { item, level });
    }
    info
}

/// Parses one typed record into a default value. A record that ends early
/// keeps the fields read so far, its source bytes (so it still saves byte for
/// byte) and its table position, and is reported.
fn typed<T: Default>(
    rec: &Record,
    diags: &mut Vec<Diagnostic>,
    parse: impl FnOnce(&mut Reader<'_>, &mut T) -> Result<(), Error>,
    store: impl FnOnce(T) -> InfoItem,
) -> Option<InfoItem> {
    let mut r = Reader::new(&rec.payload);
    let mut v = T::default();
    if let Err(e) = parse(&mut r, &mut v) {
        diags.push(Diagnostic {
            message: format!("DocInfo tag {:#x} at {}: {e}", rec.tag, rec.start),
        });
    }
    Some(store(v))
}

// Each parser fills `v` field by field and sets the tail last, so a record
// that ends early keeps everything read before the cut.

fn props(r: &mut Reader<'_>, v: &mut DocProperties) -> Result<(), Error> {
    v.section_count = r.u16()?;
    v.page_start = r.u16()?;
    v.footnote_start = r.u16()?;
    v.endnote_start = r.u16()?;
    v.figure_start = r.u16()?;
    v.table_start = r.u16()?;
    v.equation_start = r.u16()?;
    v.caret_list = r.u32()?;
    v.caret_para = r.u32()?;
    v.caret_pos = r.u32()?;
    v.tail = r.tail();
    Ok(())
}

fn id_mappings(r: &mut Reader<'_>, v: &mut IdMappings) -> Result<(), Error> {
    while r.remaining() >= 4 {
        v.counts.push(r.i32()?);
    }
    v.tail = r.tail();
    Ok(())
}

fn bin_data(r: &mut Reader<'_>, v: &mut BinDataEntry) -> Result<(), Error> {
    v.attr = r.u16()?;
    if v.attr & 0xF == 0 {
        v.link_absolute = Some(r.string()?);
        v.link_relative = Some(r.string()?);
    } else {
        v.storage_id = Some(r.u16()?);
        if r.remaining() >= 2 {
            v.extension = Some(r.string()?);
        }
    }
    v.tail = r.tail();
    Ok(())
}

fn face_name(r: &mut Reader<'_>, v: &mut FaceName) -> Result<(), Error> {
    v.attr = r.u8()?;
    v.name = r.string()?;
    if v.attr & 0x80 != 0 {
        v.substitute = Some((r.u8()?, r.string()?));
    }
    if v.attr & 0x40 != 0 {
        v.panose = Some(r.array::<10>()?);
    }
    if v.attr & 0x20 != 0 {
        v.default_name = Some(r.string()?);
    }
    v.tail = r.tail();
    Ok(())
}

fn border_line(r: &mut Reader<'_>) -> Result<BorderLine, Error> {
    Ok(BorderLine {
        kind: r.u8()?,
        width: r.u8()?,
        color: r.u32()?,
    })
}

fn border_fill(r: &mut Reader<'_>, v: &mut BorderFill) -> Result<(), Error> {
    v.attr = r.u16()?;
    for side in &mut v.sides {
        *side = border_line(r)?;
    }
    v.diagonal = border_line(r)?;
    if r.remaining() >= 4 {
        v.fill = Some(crate::fill::read_fill(r)?);
    }
    v.tail = r.tail();
    Ok(())
}

fn char_shape(r: &mut Reader<'_>, c: &mut CharShape) -> Result<(), Error> {
    for f in &mut c.faces {
        *f = r.u16()?;
    }
    c.ratios = r.array()?;
    c.spacings = r.array::<7>()?.map(|b| b as i8);
    c.rel_sizes = r.array()?;
    c.offsets = r.array::<7>()?.map(|b| b as i8);
    c.base_size = r.i32()?;
    c.attr = r.u32()?;
    c.shadow_offset = [r.i8()?, r.i8()?];
    c.color = r.u32()?;
    c.underline_color = r.u32()?;
    c.shade_color = r.u32()?;
    c.shadow_color = r.u32()?;
    if r.remaining() >= 2 {
        c.border_fill = Some(r.u16()?);
    }
    if r.remaining() >= 4 {
        c.strike_color = Some(r.u32()?);
    }
    c.tail = r.tail();
    Ok(())
}

fn tab_def(r: &mut Reader<'_>, v: &mut TabDef) -> Result<(), Error> {
    v.attr = r.u32()?;
    let count = r.i32()?.max(0) as usize;
    for _ in 0..count.min(r.remaining() / 8 + 1) {
        v.tabs.push(TabStop {
            position: r.i32()?,
            kind: r.u8()?,
            fill: r.u8()?,
            reserved: r.u16()?,
        });
    }
    v.tail = r.tail();
    Ok(())
}

fn para_head(r: &mut Reader<'_>) -> Result<ParaHead, Error> {
    Ok(ParaHead {
        attr: r.u32()?,
        width_adjust: r.i16()?,
        text_distance: r.i16()?,
        char_shape: r.u32()?,
    })
}

fn numbering(r: &mut Reader<'_>, n: &mut Numbering) -> Result<(), Error> {
    for _ in 0..7 {
        let head = para_head(r)?;
        let format = r.string()?;
        n.levels.push(NumberingLevel { head, format });
    }
    if r.remaining() >= 2 {
        n.start = Some(r.u16()?);
    }
    if r.remaining() >= 28 {
        let mut s = [0u32; 7];
        for v in &mut s {
            *v = r.u32()?;
        }
        n.level_starts = Some(s);
    }
    n.tail = r.tail();
    Ok(())
}

fn bullet(r: &mut Reader<'_>, b: &mut Bullet) -> Result<(), Error> {
    b.head = para_head(r)?;
    b.bullet_char = r.u16()?;
    if r.remaining() >= 4 {
        b.image = Some(r.i32()?);
    }
    if r.remaining() >= 4 {
        b.image_info = Some(r.array()?);
    }
    if r.remaining() >= 2 {
        b.check_char = Some(r.u16()?);
    }
    b.tail = r.tail();
    Ok(())
}

fn para_shape(r: &mut Reader<'_>, p: &mut ParaShape) -> Result<(), Error> {
    p.attr1 = r.u32()?;
    p.margin_left = r.i32()?;
    p.margin_right = r.i32()?;
    p.indent = r.i32()?;
    p.space_before = r.i32()?;
    p.space_after = r.i32()?;
    p.line_spacing_legacy = r.i32()?;
    p.tab_def = r.u16()?;
    p.numbering = r.u16()?;
    p.border_fill = r.u16()?;
    for o in &mut p.border_offsets {
        *o = r.i16()?;
    }
    if r.remaining() >= 4 {
        p.attr2 = Some(r.u32()?);
    }
    if r.remaining() >= 4 {
        p.attr3 = Some(r.u32()?);
    }
    if r.remaining() >= 4 {
        p.line_spacing = Some(r.u32()?);
    }
    p.tail = r.tail();
    Ok(())
}

fn style(r: &mut Reader<'_>, s: &mut Style) -> Result<(), Error> {
    s.local_name = r.string()?;
    s.english_name = r.string()?;
    s.attr = r.u8()?;
    s.next = r.u8()?;
    s.language = r.i16()?;
    s.para_shape = r.u16()?;
    s.char_shape = r.u16()?;
    s.tail = r.tail();
    Ok(())
}

fn compatible(r: &mut Reader<'_>, v: &mut CompatibleDocument) -> Result<(), Error> {
    v.target = r.u32()?;
    v.tail = r.tail();
    Ok(())
}

fn layout_compat(r: &mut Reader<'_>, v: &mut LayoutCompatibility) -> Result<(), Error> {
    for f in &mut v.flags {
        *f = r.u32()?;
    }
    v.tail = r.tail();
    Ok(())
}

// ---- encoding ----

/// Encodes DocInfo, copying clean records and re-encoding the rest, in the
/// source order followed by entries added since reading.
pub fn encode(info: &DocInfo) -> Vec<u8> {
    let order = full_order(info);
    let counts = id_counts(info, &order);
    let mut out = Vec::new();
    for slot in order {
        if slot.item == InfoItem::IdMappings {
            // Hancom rejects a file whose counts disagree with its records,
            // so the stored record is copied only while it still agrees.
            let m = &info.id_mappings;
            match &m.src {
                Some(b) if m.counts == counts => out.extend_from_slice(b),
                _ => {
                    let mut w = Writer::default();
                    counts.iter().for_each(|&c| w.i32(c));
                    w.bytes(&m.tail);
                    write_record(&mut out, tag::ID_MAPPINGS, slot.level, &w.0);
                }
            }
            continue;
        }
        encode_item(info, slot, &mut out);
    }
    out
}

/// The ID_MAPPINGS counts of the records `order` writes: every typed entry
/// and every record of the same kind kept raw. Memo shape and track change
/// counts, which the model keeps only as raw records, count those records
/// when the stored array holds them.
fn id_counts(info: &DocInfo, order: &[InfoSlot]) -> Vec<i32> {
    let raw_tag = |s: &InfoSlot| match s.item {
        InfoItem::Raw(i) => info.raw.get(i as usize).map(|r| r.tag),
        _ => None,
    };
    let raw = |t: u16| order.iter().filter(|s| raw_tag(s) == Some(t)).count();
    // A raw face record belongs to the script slot of the face before it.
    let mut faces = [0usize; 7];
    let mut slot = 0;
    for s in order {
        match s.item {
            InfoItem::Face(sc, _) => slot = sc.index(),
            _ if raw_tag(s) == Some(tag::FACE_NAME) => faces[slot] += 1,
            _ => {}
        }
    }
    for sc in Script::ALL {
        faces[sc.index()] += info.faces[sc.index()].len();
    }
    let mut counts = info.id_mappings.counts.clone();
    if counts.len() < IdMappings::STYLES + 1 {
        counts.resize(18, 0);
    }
    let mut set = |i: usize, n: usize| counts[i] = n as i32;
    set(
        IdMappings::BIN_DATA,
        info.bin_data.len() + raw(tag::BIN_DATA),
    );
    for (k, n) in faces.iter().enumerate() {
        set(IdMappings::FACES + k, *n);
    }
    set(
        IdMappings::BORDER_FILLS,
        info.border_fills.len() + raw(tag::BORDER_FILL),
    );
    set(
        IdMappings::CHAR_SHAPES,
        info.char_shapes.len() + raw(tag::CHAR_SHAPE),
    );
    set(
        IdMappings::TAB_DEFS,
        info.tab_defs.len() + raw(tag::TAB_DEF),
    );
    set(
        IdMappings::NUMBERINGS,
        info.numberings.len() + raw(tag::NUMBERING),
    );
    set(IdMappings::BULLETS, info.bullets.len() + raw(tag::BULLET));
    set(
        IdMappings::PARA_SHAPES,
        info.para_shapes.len() + raw(tag::PARA_SHAPE),
    );
    set(IdMappings::STYLES, info.styles.len() + raw(tag::STYLE));
    for (i, t) in [
        (15, tag::MEMO_SHAPE),
        (16, tag::TRACK_CHANGE),
        (17, tag::TRACK_CHANGE_AUTHOR),
    ] {
        let n = raw(t);
        if i < counts.len() {
            counts[i] = n as i32;
        } else if n > 0 {
            counts.resize(i + 1, 0);
            counts[i] = n as i32;
        }
    }
    counts
}

/// The source order with entries added after reading inserted after the
/// last entry of their kind.
fn full_order(info: &DocInfo) -> Vec<InfoSlot> {
    let mut order = info.order.clone();
    // Sparse documents may omit DOCUMENT_PROPERTIES. An edited value still
    // needs a record, while an untouched default must retain its source bytes.
    let p = &info.properties;
    let nondefault = [
        p.section_count,
        p.page_start,
        p.footnote_start,
        p.endnote_start,
        p.figure_start,
        p.table_start,
        p.equation_start,
    ]
    .iter()
    .any(|v| *v != 0)
        || [p.caret_list, p.caret_para, p.caret_pos]
            .iter()
            .any(|v| *v != 0);
    // A DocInfo built from another format has no source order at all.
    let built = order.is_empty();
    if (nondefault || built) && !order.iter().any(|s| matches!(s.item, InfoItem::Properties)) {
        order.insert(
            0,
            InfoSlot {
                item: InfoItem::Properties,
                level: 0,
            },
        );
    }
    // Hancom reads the tables by these counts and rejects a DocInfo without
    // them; they follow the document properties. A source without them
    // keeps its records as they were.
    if built && !order.iter().any(|s| s.item == InfoItem::IdMappings) {
        let at = order
            .iter()
            .position(|s| matches!(s.item, InfoItem::Properties))
            .map_or(0, |p| p + 1);
        order.insert(
            at,
            InfoSlot {
                item: InfoItem::IdMappings,
                level: 0,
            },
        );
    }
    grow(
        &mut order,
        info.bin_data.len(),
        |i| matches!(i, InfoItem::BinData(_)),
        InfoItem::BinData,
    );
    for s in Script::ALL {
        let ours = move |i: &InfoItem| matches!(i, InfoItem::Face(x, _) if *x == s);
        let have = order.iter().filter(|o| ours(&o.item)).count();
        let total = info.faces[s.index()].len();
        if total > have {
            let at = match order
                .iter()
                .rposition(|o| matches!(o.item, InfoItem::Face(x, _) if x <= s))
            {
                Some(p) => p + 1,
                // Before the faces of later scripts, else after the BinData.
                None => order
                    .iter()
                    .position(|o| matches!(o.item, InfoItem::Face(..)))
                    .unwrap_or_else(|| first_slot(&order, 3)),
            };
            let new = (have..total).map(|i| InfoSlot {
                item: InfoItem::Face(s, i as u32),
                level: 1,
            });
            order.splice(at..at, new);
        }
    }
    grow(
        &mut order,
        info.border_fills.len(),
        |i| matches!(i, InfoItem::BorderFill(_)),
        InfoItem::BorderFill,
    );
    grow(
        &mut order,
        info.char_shapes.len(),
        |i| matches!(i, InfoItem::CharShape(_)),
        InfoItem::CharShape,
    );
    grow(
        &mut order,
        info.tab_defs.len(),
        |i| matches!(i, InfoItem::TabDef(_)),
        InfoItem::TabDef,
    );
    grow(
        &mut order,
        info.numberings.len(),
        |i| matches!(i, InfoItem::Numbering(_)),
        InfoItem::Numbering,
    );
    grow(
        &mut order,
        info.bullets.len(),
        |i| matches!(i, InfoItem::Bullet(_)),
        InfoItem::Bullet,
    );
    grow(
        &mut order,
        info.para_shapes.len(),
        |i| matches!(i, InfoItem::ParaShape(_)),
        InfoItem::ParaShape,
    );
    grow(
        &mut order,
        info.styles.len(),
        |i| matches!(i, InfoItem::Style(_)),
        InfoItem::Style,
    );
    in_hancom_order(info, &mut order);
    order
}

/// Puts the records in Hancom's order when they break it: an HWPX header
/// may list its tables in another order, or place the compatibility
/// before them. An order that keeps it (every HWP 5 source) is left as is.
fn in_hancom_order(info: &DocInfo, order: &mut [InfoSlot]) {
    let key = |s: &InfoSlot| -> Option<u8> {
        if let Some(r) = rank(&s.item) {
            return Some(r);
        }
        Some(match s.item {
            InfoItem::Compatible => 12,
            InfoItem::LayoutCompat => 13,
            InfoItem::Raw(i) => match info.raw.get(i as usize)?.tag {
                tag::BIN_DATA => 2,
                tag::FACE_NAME => 3,
                tag::BORDER_FILL => 4,
                tag::CHAR_SHAPE => 5,
                tag::TAB_DEF => 6,
                tag::NUMBERING => 7,
                tag::BULLET => 8,
                tag::PARA_SHAPE => 9,
                tag::STYLE => 10,
                tag::MEMO_SHAPE | tag::TRACK_CHANGE | tag::TRACK_CHANGE_AUTHOR | 0x1B | 0x5E => 11,
                tag::COMPATIBLE_DOCUMENT => 12,
                tag::LAYOUT_COMPATIBILITY => 13,
                0x1C | 0x20 => 14,
                _ => return None,
            },
            _ => return None,
        })
    };
    // Records of unknown kinds travel with the record before them.
    let mut keys = Vec::with_capacity(order.len());
    let mut last = 0;
    for s in order.iter() {
        last = key(s).unwrap_or(last);
        keys.push(last);
    }
    if keys.windows(2).all(|w| w[0] <= w[1]) {
        return;
    }
    let mut indexed: Vec<(u8, InfoSlot)> = keys.into_iter().zip(order.iter().copied()).collect();
    indexed.sort_by_key(|(k, _)| *k);
    for (slot, (_, s)) in order.iter_mut().zip(indexed) {
        *slot = s;
    }
}

/// Where a table's records sit in Hancom's DocInfo order: properties, id
/// mappings, BinData, faces, border fills, char shapes, tab defs,
/// numberings, bullets, para shapes, styles. The compatibility and other
/// records come after them.
fn rank(i: &InfoItem) -> Option<u8> {
    Some(match i {
        InfoItem::Properties => 0,
        InfoItem::IdMappings => 1,
        InfoItem::BinData(_) => 2,
        InfoItem::Face(..) => 3,
        InfoItem::BorderFill(_) => 4,
        InfoItem::CharShape(_) => 5,
        InfoItem::TabDef(_) => 6,
        InfoItem::Numbering(_) => 7,
        InfoItem::Bullet(_) => 8,
        InfoItem::ParaShape(_) => 9,
        InfoItem::Style(_) => 10,
        _ => return None,
    })
}

/// Where the first record of a kind of `rank` goes: after the last record
/// of an earlier kind. Hancom rejects a DocInfo whose BinData, bullets or
/// other tables follow the compatibility records.
fn first_slot(order: &[InfoSlot], rank_of_kind: u8) -> usize {
    order
        .iter()
        .rposition(|s| rank(&s.item).is_some_and(|r| r < rank_of_kind))
        .map_or(0, |p| p + 1)
}

fn grow(
    order: &mut Vec<InfoSlot>,
    total: usize,
    is_kind: impl Fn(&InfoItem) -> bool,
    make: impl Fn(u32) -> InfoItem,
) {
    let have = order.iter().filter(|s| is_kind(&s.item)).count();
    if total <= have {
        return;
    }
    let item = make(0);
    let at = order.iter().rposition(|s| is_kind(&s.item)).map_or_else(
        || first_slot(order, rank(&item).unwrap_or(u8::MAX)),
        |p| p + 1,
    );
    let new: Vec<InfoSlot> = (have..total)
        .map(|i| InfoSlot {
            item: make(i as u32),
            level: 1,
        })
        .collect();
    order.splice(at..at, new);
}

fn emit(
    out: &mut Vec<u8>,
    src: &Option<SharedBytes>,
    tag: u16,
    level: u16,
    payload: impl FnOnce() -> Vec<u8>,
) {
    match src {
        Some(b) => out.extend_from_slice(b),
        None => write_record(out, tag, level, &payload()),
    }
}

fn encode_item(info: &DocInfo, slot: InfoSlot, out: &mut Vec<u8>) {
    let level = slot.level;
    match slot.item {
        InfoItem::Properties => {
            let p = &info.properties;
            emit(out, &p.src, tag::DOCUMENT_PROPERTIES, level, || {
                let mut w = Writer::default();
                for v in [
                    p.section_count,
                    p.page_start,
                    p.footnote_start,
                    p.endnote_start,
                    p.figure_start,
                    p.table_start,
                    p.equation_start,
                ] {
                    w.u16(v);
                }
                w.u32(p.caret_list);
                w.u32(p.caret_para);
                w.u32(p.caret_pos);
                w.bytes(&p.tail);
                w.0
            });
        }
        // Written by `encode`, which knows the counts.
        InfoItem::IdMappings => {}
        InfoItem::BinData(i) => {
            let e = &info.bin_data[i as usize];
            emit(out, &e.src, tag::BIN_DATA, level, || {
                let mut w = Writer::default();
                w.u16(e.attr);
                if let Some(s) = &e.link_absolute {
                    w.string(s);
                }
                if let Some(s) = &e.link_relative {
                    w.string(s);
                }
                if let Some(id) = e.storage_id {
                    w.u16(id);
                }
                if let Some(s) = &e.extension {
                    w.string(s);
                }
                w.bytes(&e.tail);
                w.0
            });
        }
        InfoItem::Face(s, i) => {
            let f = &info.faces[s.index()][i as usize];
            emit(out, &f.src, tag::FACE_NAME, level, || {
                let mut w = Writer::default();
                w.u8(f.attr);
                w.string(&f.name);
                if let Some((k, n)) = &f.substitute {
                    w.u8(*k);
                    w.string(n);
                }
                if let Some(p) = &f.panose {
                    w.bytes(p);
                }
                if let Some(n) = &f.default_name {
                    w.string(n);
                }
                w.bytes(&f.tail);
                w.0
            });
        }
        InfoItem::BorderFill(i) => {
            let b = &info.border_fills[i as usize];
            emit(out, &b.src, tag::BORDER_FILL, level, || {
                let mut w = Writer::default();
                w.u16(b.attr);
                for l in b.sides.iter().chain([&b.diagonal]) {
                    w.u8(l.kind);
                    w.u8(l.width);
                    w.u32(l.color);
                }
                if let Some(f) = &b.fill {
                    crate::fill::write_fill(&mut w, f);
                }
                w.bytes(&b.tail);
                w.0
            });
        }
        InfoItem::CharShape(i) => {
            let c = &info.char_shapes[i as usize];
            emit(out, &c.src, tag::CHAR_SHAPE, level, || {
                let mut w = Writer::default();
                c.faces.iter().for_each(|&f| w.u16(f));
                w.bytes(&c.ratios);
                c.spacings.iter().for_each(|&v| w.i8(v));
                w.bytes(&c.rel_sizes);
                c.offsets.iter().for_each(|&v| w.i8(v));
                w.i32(c.base_size);
                w.u32(c.attr);
                w.i8(c.shadow_offset[0]);
                w.i8(c.shadow_offset[1]);
                for v in [c.color, c.underline_color, c.shade_color, c.shadow_color] {
                    w.u32(v);
                }
                if let Some(v) = c.border_fill {
                    w.u16(v);
                }
                if let Some(v) = c.strike_color {
                    w.u32(v);
                }
                w.bytes(&c.tail);
                w.0
            });
        }
        InfoItem::TabDef(i) => {
            let t = &info.tab_defs[i as usize];
            emit(out, &t.src, tag::TAB_DEF, level, || {
                let mut w = Writer::default();
                w.u32(t.attr);
                w.i32(t.tabs.len() as i32);
                for s in &t.tabs {
                    w.i32(s.position);
                    w.u8(s.kind);
                    w.u8(s.fill);
                    w.u16(s.reserved);
                }
                w.bytes(&t.tail);
                w.0
            });
        }
        InfoItem::Numbering(i) => {
            let n = &info.numberings[i as usize];
            emit(out, &n.src, tag::NUMBERING, level, || {
                let mut w = Writer::default();
                for l in &n.levels {
                    write_head(&mut w, &l.head);
                    w.string(&l.format);
                }
                if let Some(s) = n.start {
                    w.u16(s);
                }
                if let Some(s) = n.level_starts {
                    s.iter().for_each(|&v| w.u32(v));
                }
                w.bytes(&n.tail);
                w.0
            });
        }
        InfoItem::Bullet(i) => {
            let b = &info.bullets[i as usize];
            emit(out, &b.src, tag::BULLET, level, || {
                let mut w = Writer::default();
                write_head(&mut w, &b.head);
                w.u16(b.bullet_char);
                if let Some(v) = b.image {
                    w.i32(v);
                }
                if let Some(v) = b.image_info {
                    w.bytes(&v);
                }
                if let Some(v) = b.check_char {
                    w.u16(v);
                }
                w.bytes(&b.tail);
                w.0
            });
        }
        InfoItem::ParaShape(i) => {
            let p = &info.para_shapes[i as usize];
            emit(out, &p.src, tag::PARA_SHAPE, level, || {
                let mut w = Writer::default();
                w.u32(p.attr1);
                for v in [
                    p.margin_left,
                    p.margin_right,
                    p.indent,
                    p.space_before,
                    p.space_after,
                    p.line_spacing_legacy,
                ] {
                    w.i32(v);
                }
                w.u16(p.tab_def);
                w.u16(p.numbering);
                w.u16(p.border_fill);
                p.border_offsets.iter().for_each(|&v| w.i16(v));
                for v in [p.attr2, p.attr3, p.line_spacing].into_iter().flatten() {
                    w.u32(v);
                }
                w.bytes(&p.tail);
                w.0
            });
        }
        InfoItem::Style(i) => {
            let s = &info.styles[i as usize];
            emit(out, &s.src, tag::STYLE, level, || {
                let mut w = Writer::default();
                w.string(&s.local_name);
                w.string(&s.english_name);
                w.u8(s.attr);
                w.u8(s.next);
                w.i16(s.language);
                w.u16(s.para_shape);
                w.u16(s.char_shape);
                w.bytes(&s.tail);
                w.0
            });
        }
        InfoItem::Compatible => {
            if let Some(c) = &info.compatible {
                emit(out, &c.src, tag::COMPATIBLE_DOCUMENT, level, || {
                    let mut w = Writer::default();
                    w.u32(c.target);
                    w.bytes(&c.tail);
                    w.0
                });
            }
        }
        InfoItem::LayoutCompat => {
            if let Some(c) = &info.layout_compat {
                emit(out, &c.src, tag::LAYOUT_COMPATIBILITY, level, || {
                    let mut w = Writer::default();
                    c.flags.iter().for_each(|&f| w.u32(f));
                    w.bytes(&c.tail);
                    w.0
                });
            }
        }
        InfoItem::Raw(i) => {
            let r = &info.raw[i as usize];
            write_record(out, r.tag, r.level, &r.payload);
        }
    }
}

fn write_head(w: &mut Writer, h: &ParaHead) {
    w.u32(h.attr);
    w.i16(h.width_adjust);
    w.i16(h.text_distance);
    w.u32(h.char_shape);
}
