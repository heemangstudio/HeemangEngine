//! Object controls (`gso `, `eqed`): reading their record trees into the
//! model and encoding them back.
//!
//! Every typed record keeps the bytes after its fields as a tail, and a
//! record that does not parse stays raw, so re-encoding reproduces the
//! source exactly.

use hm_model::{
    ArcGeom, Caption, CtrlId, CurveGeom, Diagnostic, EllipseGeom, Equation, ImageRef, LineGeom,
    LineStyle, Matrix, ObjectCommon, ObjectControl, ObjectHeader, ObjectPart, Ole, Paragraph,
    Picture, PolyGeom, RawRecord, RectGeom, Shadow, Shape, ShapeComponent, ShapePart, ShapeStyle,
    SharedBytes, TextBox,
};

use crate::Error;
use crate::body::{encode_list, paragraph_run};
use crate::fill::{read_fill, write_fill};
use crate::record::{Reader, Record, Writer, tag, write_record};

/// Object record tags (HWPTAG_BEGIN + 60 and up).
pub mod otag {
    pub const SHAPE_COMPONENT: u16 = 0x4C;
    pub const LINE: u16 = 0x4E;
    pub const RECTANGLE: u16 = 0x4F;
    pub const ELLIPSE: u16 = 0x50;
    pub const ARC: u16 = 0x51;
    pub const POLYGON: u16 = 0x52;
    pub const CURVE: u16 = 0x53;
    pub const OLE: u16 = 0x54;
    pub const PICTURE: u16 = 0x55;
    pub const EQEDIT: u16 = 0x58;
}

const LINE_ID: CtrlId = CtrlId::from_chars(b"$lin");
const RECT_ID: CtrlId = CtrlId::from_chars(b"$rec");
const ELLIPSE_ID: CtrlId = CtrlId::from_chars(b"$ell");
const ARC_ID: CtrlId = CtrlId::from_chars(b"$arc");
const POLYGON_ID: CtrlId = CtrlId::from_chars(b"$pol");
const CURVE_ID: CtrlId = CtrlId::from_chars(b"$cur");
const GROUP_ID: CtrlId = CtrlId::from_chars(b"$con");

/// Drawing kinds whose component carries an outline, a fill and a shadow.
fn has_style(kind: CtrlId) -> bool {
    matches!(
        kind,
        LINE_ID | RECT_ID | ELLIPSE_ID | ARC_ID | POLYGON_ID | CURVE_ID
    )
}

/// Reads an object control from its CTRL_HEADER payload and its descendant
/// records (absolute levels; the control sits at `level`). Returns `None`
/// when the header is too short to be an object.
pub(crate) fn parse_object(
    stream: &SharedBytes,
    payload: &SharedBytes,
    records: &[Record],
    level: u16,
    diags: &mut Vec<Diagnostic>,
) -> Option<ObjectControl> {
    let id = CtrlId(u32::from_le_bytes(payload.get(0..4)?.try_into().ok()?));
    let header = object_header(payload)?;
    let mut parts = Vec::new();
    let mut i = 0;
    let mut seen_body = false;
    while i < records.len() {
        let rec = &records[i];
        let end = subtree_end(records, i);
        if rec.level == level + 1 {
            match rec.tag {
                tag::LIST_HEADER if !seen_body => {
                    if let Some(c) = caption_head(&rec.payload) {
                        let (paras, used) = paragraph_run(
                            stream,
                            &records[i + 1..],
                            level + 1,
                            c.count as usize,
                            diags,
                        );
                        parts.push(ObjectPart::Caption(Caption { paras, ..c }));
                        i += 1 + used;
                        continue;
                    }
                }
                otag::SHAPE_COMPONENT => {
                    seen_body = true;
                    if let Some(shape) = parse_shape(stream, &records[i..end], true, diags) {
                        parts.push(ObjectPart::Shape(Box::new(shape)));
                        i = end;
                        continue;
                    }
                }
                otag::EQEDIT if end == i + 1 => {
                    seen_body = true;
                    if let Ok(e) = equation(&rec.payload) {
                        parts.push(ObjectPart::Equation(e));
                        i = end;
                        continue;
                    }
                }
                _ => {}
            }
        }
        parts.push(ObjectPart::Raw(raw(rec, level)));
        i += 1;
    }
    Some(ObjectControl { id, header, parts })
}

fn raw(rec: &Record, base: u16) -> RawRecord {
    RawRecord {
        tag: rec.tag,
        level: rec.level.saturating_sub(base),
        payload: rec.payload.clone(),
    }
}

fn subtree_end(records: &[Record], at: usize) -> usize {
    let level = records[at].level;
    let mut j = at + 1;
    while j < records.len() && records[j].level > level {
        j += 1;
    }
    j
}

pub(crate) fn object_header(payload: &SharedBytes) -> Option<ObjectHeader> {
    let common = ObjectCommon::parse(payload.get(4..)?)?;
    let mut r = Reader::new(payload);
    r.array::<36>().ok()?;
    let mut h = ObjectHeader {
        common,
        ..Default::default()
    };
    if r.remaining() >= 4 {
        h.instance_id = r.u32().ok();
    }
    if r.remaining() >= 4 {
        h.prevent_split = r.i32().ok();
    }
    if r.remaining() >= 2 {
        let at = r.pos();
        match r.string() {
            Ok(s) => h.description = Some(s),
            Err(_) => {
                h.tail = payload.tail_from(at);
                return Some(h);
            }
        }
    }
    h.tail = r.tail();
    Some(h)
}

pub(crate) fn caption_head(payload: &SharedBytes) -> Option<Caption> {
    let mut r = Reader::new(payload);
    let mut c = Caption {
        count: r.u32().ok()?,
        list_attr: r.u32().ok()?,
        ..Default::default()
    };
    if r.remaining() >= 14 {
        c.attr = Some(r.u32().ok()?);
        c.width = Some(r.i32().ok()?);
        c.gap = Some(r.i16().ok()?);
        c.max_width = Some(r.i32().ok()?);
    }
    c.tail = r.tail();
    Some(c)
}

/// Parses a shape component and its subtree (`records[0]` is the component).
fn parse_shape(
    stream: &SharedBytes,
    records: &[Record],
    top: bool,
    diags: &mut Vec<Diagnostic>,
) -> Option<Shape> {
    let comp = &records[0];
    let level = comp.level;
    let component = component(&comp.payload, top).ok()?;
    let mut parts = Vec::new();
    let mut i = 1;
    while i < records.len() {
        let rec = &records[i];
        let end = subtree_end(records, i);
        if rec.level == level + 1 {
            let typed = match rec.tag {
                otag::SHAPE_COMPONENT => parse_shape(stream, &records[i..end], false, diags)
                    .map(|s| (ShapePart::Child(Box::new(s)), end)),
                tag::LIST_HEADER => text_head(&rec.payload).map(|t| {
                    let (paras, used) = paragraph_run(
                        stream,
                        &records[i + 1..],
                        level + 1,
                        t.count as usize,
                        diags,
                    );
                    (ShapePart::Text(TextBox { paras, ..t }), i + 1 + used)
                }),
                _ if end == i + 1 => kind_record(
                    rec.tag,
                    &rec.payload,
                    component.kind == CtrlId::from_chars(b"$col"),
                )
                .map(|p| (p, end)),
                _ => None,
            };
            if let Some((part, next)) = typed {
                parts.push(part);
                i = next;
                continue;
            }
        }
        parts.push(ShapePart::Raw(raw(rec, level)));
        i += 1;
    }
    Some(Shape { component, parts })
}

fn point(r: &mut Reader<'_>) -> Result<(i32, i32), Error> {
    Ok((r.i32()?, r.i32()?))
}

fn matrix(r: &mut Reader<'_>) -> Result<Matrix, Error> {
    let mut m = [0f64; 6];
    for v in &mut m {
        *v = f64::from_le_bytes(r.array::<8>()?);
    }
    Ok(Matrix(m))
}

fn component(payload: &SharedBytes, top: bool) -> Result<ShapeComponent, Error> {
    let mut r = Reader::new(payload);
    let kind = CtrlId(r.u32()?);
    let doubled = top && payload.get(4..8) == payload.get(0..4);
    if doubled {
        r.u32()?;
    }
    let offset = point(&mut r)?;
    let group_level = r.u16()?;
    let version = r.u16()?;
    let initial_size = (r.u32()?, r.u32()?);
    let size = (r.u32()?, r.u32()?);
    let flip = r.u32()?;
    let rotation = r.i16()?;
    let rotation_center = point(&mut r)?;
    let count = r.u16()? as usize;
    if count * 96 + 48 > r.remaining() {
        return Err(Error::Truncated);
    }
    let translation = matrix(&mut r)?;
    let mut transforms = Vec::with_capacity(count);
    for _ in 0..count {
        transforms.push((matrix(&mut r)?, matrix(&mut r)?));
    }
    let mut c = ShapeComponent {
        kind,
        doubled,
        offset,
        group_level,
        version,
        initial_size,
        size,
        flip,
        rotation,
        rotation_center,
        translation,
        transforms,
        style: None,
        members: None,
        group_instance: None,
        tail: SharedBytes::empty(),
    };
    let at = r.pos();
    if has_style(kind) {
        match shape_style(&mut r) {
            Ok(s) => c.style = Some(s),
            Err(_) => r = skip_to(payload, at),
        }
    } else if kind == GROUP_ID {
        let ok = (|| -> Result<(), Error> {
            let n = r.u16()? as usize;
            if n * 4 > r.remaining() {
                return Err(Error::Truncated);
            }
            c.members = Some(
                (0..n)
                    .map(|_| r.u32().map(CtrlId))
                    .collect::<Result<_, _>>()?,
            );
            if r.remaining() >= 4 {
                c.group_instance = Some(r.u32()?);
            }
            Ok(())
        })();
        if ok.is_err() {
            c.members = None;
            c.group_instance = None;
            r = skip_to(payload, at);
        }
    }
    c.tail = r.tail();
    Ok(c)
}

fn skip_to(payload: &SharedBytes, at: usize) -> Reader<'_> {
    let mut r = Reader::new(payload);
    for _ in 0..at {
        let _ = r.u8();
    }
    r
}

fn line_style(r: &mut Reader<'_>, outline: bool) -> Result<LineStyle, Error> {
    Ok(LineStyle {
        color: r.u32()?,
        width: r.i32()?,
        attr: r.u32()?,
        outline: if outline { Some(r.u8()?) } else { None },
    })
}

fn shape_style(r: &mut Reader<'_>) -> Result<ShapeStyle, Error> {
    let line = line_style(r, true)?;
    let fill = read_fill(r)?;
    // A fill cut short leaves the rest of the record to the tail.
    if fill.extra.is_none() {
        return Err(Error::Truncated);
    }
    let mut s = ShapeStyle {
        line,
        fill,
        ..Default::default()
    };
    if r.remaining() >= 16 {
        s.shadow = Some(Shadow {
            kind: r.u32()?,
            color: r.u32()?,
            offset: point(r)?,
        });
        if r.remaining() >= 4 {
            s.instance_id = Some(r.u32()?);
        }
        if r.remaining() >= 1 {
            s.reserved = Some(r.u8()?);
        }
        if r.remaining() >= 1 {
            s.shadow_alpha = Some(r.u8()?);
        }
    }
    Ok(s)
}

fn image_ref(r: &mut Reader<'_>) -> Result<ImageRef, Error> {
    Ok(ImageRef {
        contrast: r.i8()?,
        brightness: r.i8()?,
        effect: r.u8()?,
        bin_id: r.u16()?,
    })
}

fn text_head(payload: &SharedBytes) -> Option<TextBox> {
    let mut r = Reader::new(payload);
    let mut t = TextBox {
        count: r.u32().ok()?,
        attr: r.u32().ok()?,
        ..Default::default()
    };
    if r.remaining() >= 12 {
        t.margins = Some([r.i16().ok()?, r.i16().ok()?, r.i16().ok()?, r.i16().ok()?]);
        t.max_width = Some(r.i32().ok()?);
    }
    t.tail = r.tail();
    Some(t)
}

fn kind_record(t: u16, payload: &SharedBytes, connector: bool) -> Option<ShapePart> {
    let mut r = Reader::new(payload);
    let r = &mut r;
    let part = match t {
        otag::LINE => {
            let start = point(r).ok()?;
            let end = point(r).ok()?;
            let (attr, connector) = if connector {
                let kind = r.u32().ok()?;
                let start = (r.u32().ok()?, r.u32().ok()?);
                let end = (r.u32().ok()?, r.u32().ok()?);
                let count = r.u32().ok()? as usize;
                if count > r.remaining() / 10 {
                    return None;
                }
                let mut points = Vec::with_capacity(count);
                for _ in 0..count {
                    points.push((r.i32().ok()?, r.i32().ok()?, r.u16().ok()?));
                }
                (
                    Some(kind as u16),
                    Some(hm_model::Connector { start, end, points }),
                )
            } else {
                (
                    if r.remaining() >= 2 {
                        r.u16().ok()
                    } else {
                        None
                    },
                    None,
                )
            };
            ShapePart::Line(LineGeom {
                start,
                end,
                attr,
                connector,
                tail: r.tail(),
            })
        }
        otag::RECTANGLE => {
            let round = r.u8().ok()?;
            let mut corners = [(0, 0); 4];
            for c in &mut corners {
                *c = point(r).ok()?;
            }
            ShapePart::Rect(RectGeom {
                round,
                corners,
                tail: r.tail(),
            })
        }
        otag::ELLIPSE => ShapePart::Ellipse(EllipseGeom {
            attr: r.u32().ok()?,
            center: point(r).ok()?,
            axis1: point(r).ok()?,
            axis2: point(r).ok()?,
            start: point(r).ok()?,
            end: point(r).ok()?,
            start2: point(r).ok()?,
            end2: point(r).ok()?,
            tail: r.tail(),
        }),
        otag::ARC => ShapePart::Arc(ArcGeom {
            attr: r.u32().ok()?,
            center: point(r).ok()?,
            axis1: point(r).ok()?,
            axis2: point(r).ok()?,
            tail: r.tail(),
        }),
        otag::POLYGON => {
            let points = points(r)?;
            ShapePart::Polygon(PolyGeom {
                points,
                tail: r.tail(),
            })
        }
        otag::CURVE => {
            let points = points(r)?;
            let n = points.len().saturating_sub(1);
            if r.remaining() < n {
                return None;
            }
            let segments = (0..n).map(|_| r.u8()).collect::<Result<_, _>>().ok()?;
            ShapePart::Curve(CurveGeom {
                points,
                segments,
                tail: r.tail(),
            })
        }
        otag::PICTURE => {
            if payload.len() < 78 {
                return None;
            }
            let border = line_style(r, false).ok()?;
            let mut corners = [(0, 0); 4];
            for c in &mut corners {
                *c = point(r).ok()?;
            }
            let mut crop = [0; 4];
            for v in &mut crop {
                *v = r.i32().ok()?;
            }
            let mut margins = [0; 4];
            for v in &mut margins {
                *v = r.i16().ok()?;
            }
            ShapePart::Picture(Picture {
                border,
                corners,
                crop,
                margins,
                image: image_ref(r).ok()?,
                border_alpha: r.u8().ok()?,
                instance_id: r.u32().ok()?,
                tail: r.tail(),
            })
        }
        otag::OLE => {
            let attr = r.u32().ok()?;
            let extent = point(r).ok()?;
            let bin_id = r.u16().ok()?;
            let border = if r.remaining() >= 12 {
                Some(line_style(r, false).ok()?)
            } else {
                None
            };
            ShapePart::Ole(Ole {
                attr,
                extent,
                bin_id,
                border,
                tail: r.tail(),
            })
        }
        _ => return None,
    };
    Some(part)
}

/// A point count (stored as a 32-bit word) followed by `(x, y)` pairs.
fn points(r: &mut Reader<'_>) -> Option<Vec<(i32, i32)>> {
    let n = r.i32().ok()?;
    if n < 0 || n as usize * 8 > r.remaining() {
        return None;
    }
    (0..n).map(|_| point(r).ok()).collect()
}

fn equation(payload: &SharedBytes) -> Result<Equation, Error> {
    let mut r = Reader::new(payload);
    let mut e = Equation {
        attr: r.u32()?,
        script: r.string()?,
        size: r.u32()?,
        color: r.u32()?,
        baseline: r.i16()?,
        ..Default::default()
    };
    if r.remaining() >= 2 {
        e.reserved = Some(r.u16()?);
    }
    let mut strings = [None, None];
    for slot in &mut strings {
        if r.remaining() >= 2 {
            let at = r.pos();
            match r.string() {
                Ok(s) => *slot = Some(s),
                Err(_) => {
                    let [version, font] = strings;
                    return Ok(Equation {
                        version,
                        font,
                        tail: payload.tail_from(at),
                        ..e
                    });
                }
            }
        }
    }
    let [version, font] = strings;
    e.version = version;
    e.font = font;
    e.tail = r.tail();
    Ok(e)
}

// ---- encoding ----

/// Encodes an object control at `level`.
pub(crate) fn encode_object(out: &mut Vec<u8>, o: &ObjectControl, level: u16) {
    let mut w = Writer::default();
    w.u32(o.id.0);
    encode_header(&mut w, &o.header);
    write_record(out, tag::CTRL_HEADER, level, &w.0);
    for part in &o.parts {
        match part {
            ObjectPart::Caption(c) => encode_caption(out, c, level + 1),
            ObjectPart::Shape(s) => encode_shape(out, s, level + 1),
            ObjectPart::Equation(e) => {
                let mut w = Writer::default();
                encode_equation(&mut w, e);
                write_record(out, otag::EQEDIT, level + 1, &w.0);
            }
            ObjectPart::Raw(r) => write_record(out, r.tag, level + r.level, &r.payload),
        }
    }
}

/// Writes a caption's list header and paragraphs at `level`.
pub(crate) fn encode_caption(out: &mut Vec<u8>, c: &Caption, level: u16) {
    let mut w = Writer::default();
    w.u32(c.paras.len() as u32);
    w.u32(c.list_attr);
    if let (Some(a), Some(wd), Some(g), Some(m)) = (c.attr, c.width, c.gap, c.max_width) {
        w.u32(a);
        w.i32(wd);
        w.i16(g);
        w.i32(m);
    }
    w.bytes(&c.tail);
    write_record(out, tag::LIST_HEADER, level, &w.0);
    encode_list(out, &c.paras, level);
}

pub(crate) fn encode_header(w: &mut Writer, h: &ObjectHeader) {
    let c = &h.common;
    w.u32(c.attr);
    w.i32(c.y_offset);
    w.i32(c.x_offset);
    w.i32(c.width);
    w.i32(c.height);
    w.i32(c.z_order);
    c.margins.iter().for_each(|&m| w.i16(m));
    if let Some(v) = h.instance_id {
        w.u32(v);
    }
    if let Some(v) = h.prevent_split {
        w.i32(v);
    }
    if let Some(d) = &h.description {
        w.string(d);
    }
    w.bytes(&h.tail);
}

fn encode_equation(w: &mut Writer, e: &Equation) {
    w.u32(e.attr);
    w.string(&e.script);
    w.u32(e.size);
    w.u32(e.color);
    w.i16(e.baseline);
    if let Some(v) = e.reserved {
        w.u16(v);
    }
    for s in [&e.version, &e.font].into_iter().flatten() {
        w.string(s);
    }
    w.bytes(&e.tail);
}

fn put_point(w: &mut Writer, p: (i32, i32)) {
    w.i32(p.0);
    w.i32(p.1);
}

fn put_matrix(w: &mut Writer, m: &Matrix) {
    for v in m.0 {
        w.bytes(&v.to_le_bytes());
    }
}

fn put_line(w: &mut Writer, l: &LineStyle) {
    w.u32(l.color);
    w.i32(l.width);
    w.u32(l.attr);
    if let Some(o) = l.outline {
        w.u8(o);
    }
}

fn put_image(w: &mut Writer, i: &ImageRef) {
    w.i8(i.contrast);
    w.i8(i.brightness);
    w.u8(i.effect);
    w.u16(i.bin_id);
}

fn encode_component(w: &mut Writer, c: &ShapeComponent) {
    w.u32(c.kind.0);
    if c.doubled {
        w.u32(c.kind.0);
    }
    put_point(w, c.offset);
    w.u16(c.group_level);
    w.u16(c.version);
    w.u32(c.initial_size.0);
    w.u32(c.initial_size.1);
    w.u32(c.size.0);
    w.u32(c.size.1);
    w.u32(c.flip);
    w.i16(c.rotation);
    put_point(w, c.rotation_center);
    w.u16(c.transforms.len() as u16);
    put_matrix(w, &c.translation);
    for (s, r) in &c.transforms {
        put_matrix(w, s);
        put_matrix(w, r);
    }
    if let Some(s) = &c.style {
        put_line(w, &s.line);
        write_fill(w, &s.fill);
        if let Some(sh) = &s.shadow {
            w.u32(sh.kind);
            w.u32(sh.color);
            put_point(w, sh.offset);
        }
        if let Some(v) = s.instance_id {
            w.u32(v);
        }
        if let Some(v) = s.reserved {
            w.u8(v);
        }
        if let Some(v) = s.shadow_alpha {
            w.u8(v);
        }
    }
    if let Some(m) = &c.members {
        w.u16(m.len() as u16);
        m.iter().for_each(|id| w.u32(id.0));
        if let Some(v) = c.group_instance {
            w.u32(v);
        }
    }
    w.bytes(&c.tail);
}

fn encode_shape(out: &mut Vec<u8>, s: &Shape, level: u16) {
    let mut w = Writer::default();
    encode_component(&mut w, &s.component);
    write_record(out, otag::SHAPE_COMPONENT, level, &w.0);
    let child = level + 1;
    for part in &s.parts {
        let mut w = Writer::default();
        let t = match part {
            ShapePart::Child(c) => {
                encode_shape(out, c, child);
                continue;
            }
            ShapePart::Text(t) => {
                w.u32(t.paras.len() as u32);
                w.u32(t.attr);
                if let (Some(m), Some(mw)) = (t.margins, t.max_width) {
                    m.iter().for_each(|&v| w.i16(v));
                    w.i32(mw);
                }
                w.bytes(&t.tail);
                write_record(out, tag::LIST_HEADER, child, &w.0);
                encode_list(out, &t.paras, child);
                continue;
            }
            ShapePart::Raw(r) => {
                write_record(out, r.tag, level + r.level, &r.payload);
                continue;
            }
            ShapePart::Line(l) => {
                put_point(&mut w, l.start);
                put_point(&mut w, l.end);
                if let Some(c) = &l.connector {
                    w.u32(l.attr.unwrap_or(0) as u32);
                    for v in [
                        c.start.0,
                        c.start.1,
                        c.end.0,
                        c.end.1,
                        c.points.len() as u32,
                    ] {
                        w.u32(v);
                    }
                    for &(x, y, kind) in &c.points {
                        w.i32(x);
                        w.i32(y);
                        w.u16(kind);
                    }
                } else if let Some(a) = l.attr {
                    w.u16(a);
                }
                w.bytes(&l.tail);
                otag::LINE
            }
            ShapePart::Rect(rc) => {
                w.u8(rc.round);
                rc.corners.iter().for_each(|&p| put_point(&mut w, p));
                w.bytes(&rc.tail);
                otag::RECTANGLE
            }
            ShapePart::Ellipse(e) => {
                w.u32(e.attr);
                for p in [e.center, e.axis1, e.axis2, e.start, e.end, e.start2, e.end2] {
                    put_point(&mut w, p);
                }
                w.bytes(&e.tail);
                otag::ELLIPSE
            }
            ShapePart::Arc(a) => {
                w.u32(a.attr);
                for p in [a.center, a.axis1, a.axis2] {
                    put_point(&mut w, p);
                }
                w.bytes(&a.tail);
                otag::ARC
            }
            ShapePart::Polygon(p) => {
                w.i32(p.points.len() as i32);
                p.points.iter().for_each(|&q| put_point(&mut w, q));
                w.bytes(&p.tail);
                otag::POLYGON
            }
            ShapePart::Curve(c) => {
                w.i32(c.points.len() as i32);
                c.points.iter().for_each(|&q| put_point(&mut w, q));
                w.bytes(&c.segments);
                w.bytes(&c.tail);
                otag::CURVE
            }
            ShapePart::Picture(p) => {
                put_line(&mut w, &p.border);
                p.corners.iter().for_each(|&q| put_point(&mut w, q));
                p.crop.iter().for_each(|&v| w.i32(v));
                p.margins.iter().for_each(|&v| w.i16(v));
                put_image(&mut w, &p.image);
                w.u8(p.border_alpha);
                w.u32(p.instance_id);
                w.bytes(&p.tail);
                otag::PICTURE
            }
            ShapePart::Ole(o) => {
                w.u32(o.attr);
                put_point(&mut w, o.extent);
                w.u16(o.bin_id);
                if let Some(b) = &o.border {
                    put_line(&mut w, b);
                }
                w.bytes(&o.tail);
                otag::OLE
            }
        };
        write_record(out, t, child, &w.0);
    }
}

/// Paragraphs of a caption or text box, for callers walking every list.
pub fn object_lists(o: &ObjectControl) -> Vec<&[Paragraph]> {
    fn shape<'a>(s: &'a Shape, out: &mut Vec<&'a [Paragraph]>) {
        for p in &s.parts {
            match p {
                ShapePart::Text(t) => out.push(t.paras.as_slice()),
                ShapePart::Child(c) => shape(c, out),
                _ => {}
            }
        }
    }
    let mut out: Vec<&[Paragraph]> = Vec::new();
    for p in &o.parts {
        match p {
            ObjectPart::Caption(c) => out.push(c.paras.as_slice()),
            ObjectPart::Shape(s) => shape(s, &mut out),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::read_records;

    fn component_payload(kind: &[u8; 4], doubled: bool, style: bool) -> Vec<u8> {
        let mut w = Writer::default();
        let id = CtrlId::from_chars(kind).0;
        w.u32(id);
        if doubled {
            w.u32(id);
        }
        put_point(&mut w, (0, 0));
        w.u16(0);
        w.u16(1);
        for v in [9000u32, 3000, 9070, 3117] {
            w.u32(v);
        }
        w.u32(0);
        w.i16(0);
        put_point(&mut w, (4535, 1558));
        w.u16(1);
        for _ in 0..3 {
            put_matrix(&mut w, &Matrix::IDENTITY);
        }
        if style {
            put_line(
                &mut w,
                &LineStyle {
                    color: 0,
                    width: 33,
                    attr: 0xC000_0041,
                    outline: Some(0),
                },
            );
            // A solid white fill with its alpha byte, then the shadow block.
            w.u32(1);
            w.u32(0x00FF_FFFF);
            w.u32(0);
            w.i32(-1);
            w.u32(0);
            w.u8(0);
            w.u32(0);
            w.u32(0x00B2_B2B2);
            put_point(&mut w, (0, 0));
            w.u32(0x1234_5678);
            w.u8(0);
            w.u8(0);
        }
        w.0
    }

    fn text_para() -> Vec<u8> {
        let mut out = Vec::new();
        let mut h = Writer::default();
        h.u32(2 | 0x8000_0000);
        h.u32(0);
        h.u16(0);
        h.u8(0);
        h.u8(0);
        h.u16(1);
        h.u16(0);
        h.u16(0);
        h.u32(0);
        write_record(&mut out, tag::PARA_HEADER, 2, &h.0);
        write_record(&mut out, tag::PARA_TEXT, 3, &[0x41, 0, 0x0D, 0]);
        write_record(&mut out, tag::PARA_CHAR_SHAPE, 3, &[0; 8]);
        out
    }

    #[test]
    fn rectangle_with_text_reads_typed_and_writes_back() {
        let mut stream = Vec::new();
        let mut header = Writer::default();
        header.u32(CtrlId::SHAPE.0);
        header.u32(0x0420_0000);
        for v in [100, 200, 9070, 3117, 5] {
            header.i32(v);
        }
        [0i16, 0, 0, 0].iter().for_each(|&m| header.i16(m));
        header.u32(77);
        header.i32(0);
        header.string(&hm_model::HwpString::from_text(""));
        write_record(&mut stream, tag::CTRL_HEADER, 0, &header.0);
        write_record(
            &mut stream,
            otag::SHAPE_COMPONENT,
            1,
            &component_payload(b"$rec", true, true),
        );
        let mut list = Writer::default();
        list.u32(1);
        list.u32(1 << 5);
        [141i16, 141, 141, 141].iter().for_each(|&m| list.i16(m));
        list.i32(9070);
        write_record(&mut stream, tag::LIST_HEADER, 2, &list.0);
        stream.extend(text_para());
        let mut rect = Writer::default();
        rect.u8(0);
        for p in [(0, 0), (9070, 0), (9070, 3117), (0, 3117)] {
            put_point(&mut rect, p);
        }
        write_record(&mut stream, otag::RECTANGLE, 2, &rect.0);

        let bytes = SharedBytes::from(stream.clone());
        let (records, err) = read_records(&bytes);
        assert!(err.is_none());
        let mut diags = Vec::new();
        let o = parse_object(&bytes, &records[0].payload, &records[1..], 0, &mut diags).unwrap();
        assert_eq!(o.header.common.width, 9070);
        assert_eq!(o.header.instance_id, Some(77));
        let shape = o.shape().unwrap();
        assert!(shape.component.doubled);
        assert_eq!(shape.component.size, (9070, 3117));
        let style = shape.style().unwrap();
        assert_eq!(style.line.width, 33);
        assert_eq!(style.fill.solid.unwrap().background, 0x00FF_FFFF);
        assert_eq!(style.instance_id, Some(0x1234_5678));
        let text = shape.text().unwrap();
        assert_eq!(text.vertical_align(), 1);
        assert_eq!(text.margins, Some([141; 4]));
        assert_eq!(text.paras.len(), 1);
        assert_eq!(text.paras[0].plain_text(), "A");
        match shape.kind() {
            Some(ShapePart::Rect(r)) => assert_eq!(r.corners[2], (9070, 3117)),
            other => panic!("expected a rectangle, got {other:?}"),
        }

        let mut out = Vec::new();
        encode_object(&mut out, &o, 0);
        assert_eq!(out, stream);
    }

    #[test]
    fn equation_fields_follow_the_script() {
        let mut w = Writer::default();
        w.u32(0);
        w.string(&hm_model::HwpString::from_text("1 over 2"));
        w.u32(1100);
        w.u32(0);
        w.i16(85);
        w.u16(0);
        w.string(&hm_model::HwpString::from_text("Equation Version 60"));
        w.string(&hm_model::HwpString::from_text("HYhwpEQ"));
        let payload = SharedBytes::from(w.0.clone());
        let e = equation(&payload).unwrap();
        assert_eq!(e.script.to_string_lossy(), "1 over 2");
        assert_eq!(e.size, 1100);
        assert_eq!(e.baseline, 85);
        assert_eq!(e.font.as_ref().unwrap().to_string_lossy(), "HYhwpEQ");
        let mut back = Writer::default();
        encode_equation(&mut back, &e);
        assert_eq!(back.0, w.0);
    }
}
