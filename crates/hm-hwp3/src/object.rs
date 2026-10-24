//! Boxes (tables, text boxes, equations, buttons), pictures, drawings and
//! lines mapped into object controls.

use hm_model::{
    ArcGeom, Caption, Cell, ControlKind, CtrlId, CurveGeom, EllipseGeom, Equation, Fill, HwpString,
    ImageRef, LineGeom, LineStyle, Matrix, ObjectCommon, ObjectControl, ObjectHeader, ObjectPart,
    Paragraph, Picture, PolyGeom, RectGeom, Shape, ShapeComponent, ShapePart, ShapeStyle,
    SharedBytes, SolidFill, Table, TextBox,
};

use crate::convert::{BorderKey, Conv, hu};
use crate::file::{Cur, Special, i16_at, i32_at, u16_at, u32_at};

/// The control a box, picture or line special character becomes.
pub fn convert(c: &mut Conv<'_>, s: &Special) -> Option<(CtrlId, ControlKind)> {
    match s.code {
        10 => match u16_at(&s.data, 78) {
            0 => table(c, s),
            2 => equation(c, s),
            _ => text_box(c, s),
        },
        11 => picture(c, s),
        14 => line(c, s),
        _ => None,
    }
}

/// Placement fields shared by boxes and pictures (offsets 8 to 75).
struct Place {
    anchor: u8,
    wrap: u8,
    x: i16,
    y: i16,
    /// Outside margins: left, right, top, bottom.
    outer: [i16; 4],
    width: i32,
    height: i32,
}

fn place(d: &[u8]) -> Place {
    Place {
        anchor: d[8],
        wrap: d[9],
        x: i16_at(d, 10),
        y: i16_at(d, 12),
        outer: std::array::from_fn(|k| hu(u16_at(d, 18 + 2 * k) as i32) as i16),
        width: hu(u16_at(d, 42) as i32),
        height: hu(u16_at(d, 44) as i32),
    }
}

/// The common object attributes of a placement, numbered as `kind`
/// (0 none, 1 figure, 2 table, 3 equation).
fn common(c: &mut Conv<'_>, p: &Place, kind: u32) -> ObjectCommon {
    let (vrel, hrel) = match p.anchor {
        0 | 1 => (2, 2),
        2 => (1, 1),
        _ => (0, 0),
    };
    let axis = |v: i16| match v {
        -1 => (0, 0),
        -2 => (2, 0),
        -3 => (1, 0),
        v => (0, hu(v as i32)),
    };
    let (valign, y) = axis(p.y);
    let (halign, x) = axis(p.x);
    let wrap = match p.wrap {
        1 => 3,
        2 => 0,
        _ => 1,
    };
    let attr = (p.anchor == 0) as u32
        | vrel << 3
        | valign << 5
        | hrel << 8
        | halign << 10
        | ((p.anchor == 1) as u32) << 13
        | 4 << 15
        | 2 << 18
        | wrap << 21
        | kind << 26;
    let z = c.z;
    c.z += 1;
    ObjectCommon {
        attr,
        y_offset: y,
        x_offset: x,
        width: p.width,
        height: p.height,
        z_order: z,
        margins: p.outer,
    }
}

fn header(c: &mut Conv<'_>, common: ObjectCommon) -> ObjectHeader {
    ObjectHeader {
        common,
        instance_id: Some(c.next_instance()),
        prevent_split: Some(0),
        description: Some(HwpString::default()),
        tail: SharedBytes::empty(),
    }
}

/// A caption list when it holds text.
fn caption(c: &mut Conv<'_>, s: &Special, width: i32) -> Option<Caption> {
    let list = s.lists.last()?;
    let has_text = list.iter().any(|p| {
        p.items
            .iter()
            .any(|(_, it)| !matches!(it, crate::file::Item::Char(13 | 32)))
    });
    if !has_text {
        return None;
    }
    let paras = c.paras(list, false, None);
    let side = match u16_at(&s.data, 70) {
        0 => 3,
        1 => 2,
        2..=4 => 0,
        _ => 1,
    };
    Some(Caption {
        count: paras.len() as u32,
        attr: Some(side),
        width: Some(8504),
        gap: Some(852),
        max_width: Some(width),
        paras,
        ..Default::default()
    })
}

/// Border kind and width index of a stored cell or box line kind.
pub fn line_kind(k: u8) -> (u8, u8) {
    match k {
        1 => (1, 1),
        2 => (1, 6),
        3 => (3, 1),
        4 => (8, 7),
        _ => (0, 0),
    }
}

fn table(c: &mut Conv<'_>, s: &Special) -> Option<(CtrlId, ControlKind)> {
    let d = &s.data;
    let p = place(d);
    let n = u16_at(d, 80) as usize;
    if n == 0 || s.lists.len() < n {
        return None;
    }
    struct Raw {
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        center: bool,
        lines: [u8; 4],
        shade: u8,
        diagonal: u8,
    }
    let raw: Vec<Raw> = (0..n)
        .map(|k| {
            let o = 84 + 27 * k;
            Raw {
                x: u16_at(d, o + 4) as i32,
                y: u16_at(d, o + 6) as i32,
                w: u16_at(d, o + 8) as i32,
                h: u16_at(d, o + 10) as i32,
                center: d[o + 19] != 0,
                lines: [d[o + 20], d[o + 21], d[o + 22], d[o + 23]],
                shade: d[o + 24],
                diagonal: d[o + 25] & 3,
            }
        })
        .collect();
    // The grid lines are the distinct cell edges.
    let mut xs: Vec<i32> = raw.iter().flat_map(|r| [r.x, r.x + r.w]).collect();
    let mut ys: Vec<i32> = raw.iter().flat_map(|r| [r.y, r.y + r.h]).collect();
    xs.sort_unstable();
    xs.dedup();
    ys.sort_unstable();
    ys.dedup();
    let index = |v: &[i32], at: i32| v.partition_point(|&e| e < at);
    let cols = xs.len().saturating_sub(1).max(1);
    let rows = ys.len().saturating_sub(1).max(1);
    let pad: [i16; 4] = std::array::from_fn(|k| hu(u16_at(d, 34 + 2 * k) as i32) as i16);
    let mut cells: Vec<(usize, usize, Cell)> = Vec::with_capacity(n);
    for (k, r) in raw.iter().enumerate() {
        let (col, row) = (index(&xs, r.x), index(&ys, r.y));
        let col_span = index(&xs, r.x + r.w).saturating_sub(col).max(1);
        let row_span = index(&ys, r.y + r.h).saturating_sub(row).max(1);
        // Stored order is left, right, top, bottom.
        let border_fill = c.border_fill(BorderKey {
            sides: r.lines.map(line_kind),
            fill: (r.shade != 0).then(|| crate::convert::tint(0, r.shade)),
            diagonal: r.diagonal,
        });
        let inner = hu(r.w) - pad[0] as i32 - pad[1] as i32;
        let paras = c.paras(&s.lists[k], false, Some(inner));
        cells.push((
            row,
            col,
            Cell {
                list_attr: if r.center { 1 << 5 } else { 0 },
                col: col as u16,
                row: row as u16,
                col_span: col_span as u16,
                row_span: row_span as u16,
                width: hu(r.w),
                height: hu(r.h),
                padding: pad,
                border_fill,
                paras,
                ..Default::default()
            },
        ));
    }
    cells.sort_by_key(|&(r, c, _)| (r, c));
    let mut row_cells = vec![0u16; rows];
    for (r, _, _) in &cells {
        if let Some(n) = row_cells.get_mut(*r) {
            *n += 1;
        }
    }
    let common = common(c, &p, 2);
    let caption = caption(c, s, p.width);
    let t = Table {
        header: header(c, common),
        caption,
        attr: 4,
        rows: rows as u16,
        cols: cols as u16,
        spacing: 0,
        padding: pad,
        row_cells,
        border_fill: 1,
        zones: Some(Vec::new()),
        cells: cells.into_iter().map(|(_, _, c)| c).collect(),
        ..Default::default()
    };
    Some((CtrlId::TABLE, ControlKind::Table(Box::new(t))))
}

fn component(kind: &[u8; 4], w: i32, h: i32, offset: (i32, i32), level: u16) -> ShapeComponent {
    ShapeComponent {
        kind: CtrlId::from_chars(kind),
        doubled: level == 0,
        offset,
        group_level: level,
        version: 1,
        initial_size: (w.max(0) as u32, h.max(0) as u32),
        size: (w.max(0) as u32, h.max(0) as u32),
        flip: 0,
        rotation: 0,
        rotation_center: (w / 2, h / 2),
        translation: Matrix([1.0, 0.0, offset.0 as f64, 0.0, 1.0, offset.1 as f64]),
        transforms: vec![(Matrix::IDENTITY, Matrix::IDENTITY)],
        style: None,
        members: None,
        group_instance: None,
        tail: SharedBytes::empty(),
    }
}

fn solid_line(color: u32, width: i32) -> LineStyle {
    LineStyle {
        color,
        width,
        attr: 1,
        outline: Some(0),
    }
}

fn no_fill() -> Fill {
    Fill::default()
}

fn object(id: CtrlId, header: ObjectHeader, parts: Vec<ObjectPart>) -> (CtrlId, ControlKind) {
    (
        id,
        ControlKind::Object(Box::new(ObjectControl { id, header, parts })),
    )
}

fn text_box(c: &mut Conv<'_>, s: &Special) -> Option<(CtrlId, ControlKind)> {
    let d = &s.data;
    let p = place(d);
    let list = s.lists.first()?;
    let o = 84;
    let lines = d.get(o + 20..o + 24)?;
    let visible = lines.iter().any(|&k| k != 0);
    let pad: [i16; 4] = std::array::from_fn(|k| hu(u16_at(d, 26 + 2 * k) as i32) as i16);
    let center = d.get(o + 19).is_some_and(|&v| v != 0);
    let paras = c.paras(list, false, None);
    let (w, h) = (p.width, p.height);
    let mut comp = component(b"$rec", w, h, (0, 0), 0);
    let width = if lines.contains(&2) { 113 } else { 33 };
    comp.style = Some(ShapeStyle {
        line: if visible {
            solid_line(0, width)
        } else {
            LineStyle::default()
        },
        fill: match d.get(o + 24) {
            Some(&shade) if shade > 0 => solid_fill(crate::convert::tint(0, shade)),
            _ => no_fill(),
        },
        ..Default::default()
    });
    let text = TextBox {
        count: paras.len() as u32,
        attr: if center { 1 << 5 } else { 0 },
        margins: Some(pad),
        max_width: Some(w),
        paras,
        ..Default::default()
    };
    let shape = Shape {
        component: comp,
        parts: vec![
            ShapePart::Text(text),
            ShapePart::Rect(RectGeom {
                round: 0,
                corners: [(0, 0), (w, 0), (w, h), (0, h)],
                tail: SharedBytes::empty(),
            }),
        ],
    };
    let common = common(c, &p, 1);
    let mut parts = vec![ObjectPart::Shape(Box::new(shape))];
    if let Some(cap) = caption(c, s, w) {
        parts.insert(0, ObjectPart::Caption(cap));
    }
    let h = header(c, common);
    Some(object(CtrlId::SHAPE, h, parts))
}

fn solid_fill(color: u32) -> Fill {
    Fill {
        kind: Fill::SOLID,
        solid: Some(SolidFill {
            background: color,
            pattern_color: 0,
            pattern: -1,
        }),
        ..Default::default()
    }
}

fn equation(c: &mut Conv<'_>, s: &Special) -> Option<(CtrlId, ControlKind)> {
    let d = &s.data;
    let p = place(d);
    let list = s.lists.first()?;
    let mut script = String::new();
    let mut size = 1000;
    for (k, para) in list.iter().enumerate() {
        if k > 0 {
            script.push(' ');
        }
        if let Some((_, cs)) = para.runs.first() {
            size = hu(cs.size as i32);
        }
        for (_, it) in &para.items {
            if let crate::file::Item::Char(ch) = it
                && *ch != 13
            {
                script.push(crate::text::hchar(*ch).unwrap_or(' '));
            }
        }
    }
    let common = common(c, &p, 3);
    let baseline = u16_at(d, 76) as i32 * 4;
    let eq = Equation {
        attr: 0,
        script: HwpString::from_text(script.trim()),
        size: size.max(100) as u32,
        color: 0,
        baseline: if p.height > 0 {
            (baseline * 100 / p.height).clamp(0, 100) as i16
        } else {
            85
        },
        ..Default::default()
    };
    let h = header(c, common);
    Some(object(CtrlId::EQUATION, h, vec![ObjectPart::Equation(eq)]))
}

fn picture(c: &mut Conv<'_>, s: &Special) -> Option<(CtrlId, ControlKind)> {
    let d = &s.data;
    let p = place(d);
    let kind = d[74];
    let opt = u16_at(d, 14);
    let (w, h) = (p.width, p.height);
    let shape = match kind {
        3 => drawing(c, &d[348..], &s.lists, w, h)?,
        0 | 2 => {
            let name = crate::text::kchars(&d[83..339]).to_ascii_lowercase();
            let bin_id = if kind == 2 {
                c.images.get(&name).copied().unwrap_or(0)
            } else {
                // Linked assets are not read from the filesystem. An absent
                // payload still leaves the picture's frame in document flow.
                0
            };
            let mut comp = component(b"$pic", w, h, (0, 0), 0);
            comp.style = None;
            let border = if opt & 1 != 0 {
                LineStyle {
                    color: 0,
                    width: 1,
                    attr: 1,
                    outline: None,
                }
            } else {
                LineStyle::default()
            };
            let pic = Picture {
                border,
                corners: [(0, 0), (w, 0), (w, h), (0, h)],
                crop: [0; 4],
                margins: [0; 4],
                image: ImageRef {
                    brightness: d[339] as i8,
                    contrast: d[340] as i8,
                    effect: d[341],
                    bin_id,
                },
                border_alpha: 0,
                instance_id: 0,
                tail: SharedBytes::empty(),
            };
            Shape {
                component: comp,
                parts: vec![ShapePart::Picture(pic)],
            }
        }
        _ => return None,
    };
    let common = common(c, &p, 1);
    let mut parts = vec![ObjectPart::Shape(Box::new(shape))];
    if let Some(cap) = caption(c, s, w) {
        parts.insert(0, ObjectPart::Caption(cap));
    }
    let h = header(c, common);
    Some(object(CtrlId::SHAPE, h, parts))
}

fn line(c: &mut Conv<'_>, s: &Special) -> Option<(CtrlId, ControlKind)> {
    let d = &s.data;
    let x0 = i16_at(d, 70) as i32;
    let y0 = i16_at(d, 72) as i32;
    let x1 = i16_at(d, 74) as i32;
    let y1 = i16_at(d, 76) as i32;
    let p = Place {
        anchor: d[8],
        wrap: 0,
        x: x0.min(x1) as i16,
        y: y0.min(y1) as i16,
        outer: [0; 4],
        width: hu((x1 - x0).abs()),
        height: hu((y1 - y0).abs()),
    };
    let start = (hu(x0 - x0.min(x1)), hu(y0 - y0.min(y1)));
    let end = (hu(x1 - x0.min(x1)), hu(y1 - y0.min(y1)));
    let mut comp = component(b"$lin", p.width, p.height, (0, 0), 0);
    comp.style = Some(ShapeStyle {
        line: solid_line(
            crate::convert::color(u16_at(d, 82) as u32),
            hu(u16_at(d, 78) as i32).max(1),
        ),
        fill: no_fill(),
        ..Default::default()
    });
    let shape = Shape {
        component: comp,
        parts: vec![ShapePart::Line(LineGeom {
            start,
            end,
            attr: None,
            tail: SharedBytes::empty(),
        })],
    };
    let mut common = common(c, &p, 0);
    // Lines float in front of the text.
    common.attr = (common.attr & !(7 << 21)) | 3 << 21;
    let h = header(c, common);
    Some(object(
        CtrlId::SHAPE,
        h,
        vec![ObjectPart::Shape(Box::new(shape))],
    ))
}

/// A drawing: the frame header, then the objects in preorder.
fn drawing(
    c: &mut Conv<'_>,
    b: &[u8],
    lists: &[Vec<crate::file::Para>],
    w: i32,
    h: i32,
) -> Option<Shape> {
    let mut cur = Cur::new(b);
    let head_len = cur.u32().ok()? as usize;
    let head = cur.take(head_len).ok()?;
    let count = u32_at(head, 4);
    if count == 0 {
        return None;
    }
    let mut texts = lists.iter();
    let roots = objects(c, &mut cur, &mut texts, 0).ok()?;
    if roots.len() == 1 {
        let mut s = roots.into_iter().next()?;
        s.component.doubled = true;
        return Some(s);
    }
    let mut comp = component(b"$con", w, h, (0, 0), 0);
    comp.members = Some(roots.iter().map(|s| s.component.kind).collect());
    Some(Shape {
        component: comp,
        parts: roots
            .into_iter()
            .map(|s| ShapePart::Child(Box::new(s)))
            .collect(),
    })
}

/// A list of sibling objects at group `level`, with their children.
fn objects(
    c: &mut Conv<'_>,
    cur: &mut Cur<'_>,
    texts: &mut std::slice::Iter<'_, Vec<crate::file::Para>>,
    level: u16,
) -> Result<Vec<Shape>, crate::Error> {
    if level > 32 {
        return Err(crate::Error::Corrupt("drawing groups nest too deep".into()));
    }
    let mut out = Vec::new();
    loop {
        let len = cur.u32()? as usize;
        let head = cur.take(len)?;
        let kind = u16_at(head, 0);
        let link = u16_at(head, 2);
        let rel = (hu(i32_at(head, 4)), hu(i32_at(head, 8)));
        let (w, h) = (hu(i32_at(head, 12)), hu(i32_at(head, 16)));
        // Basic attributes start at 44 of the header after its length.
        let a = head.get(44..88).unwrap_or(&[0; 44]);
        let style_word = u32_at(a, 0);
        let line_color = u32_at(a, 12) & 0x00FF_FFFF;
        let line_width = hu(i32_at(a, 16));
        let fill_color = u32_at(a, 20);
        let pattern = u32_at(a, 24);
        let options = u32_at(a, 40);
        if std::env::var_os("HM_H3_DRAW").is_some() {
            eprintln!(
                "draw kind{kind} link{link} rel{rel:?} size{:?} style{:?} lc{line_color:#x} lw{line_width} fc{fill_color:#x} pat{pattern:#x} pc{:#x} opt{options:#x} hl{} extra{:?}",
                (w, h),
                (u32_at(a, 0), u32_at(a, 4), u32_at(a, 8)),
                u32_at(a, 28),
                head.len(),
                head.get(88..)
                    .map(|e| e.iter().take(40).collect::<Vec<_>>())
            );
        }
        let info = |cur: &mut Cur<'_>| -> Result<(Vec<u8>, Vec<u8>), crate::Error> {
            let n1 = cur.u32()? as usize;
            let i1 = cur.take(n1)?.to_vec();
            let n2 = cur.u32()? as usize;
            let i2 = cur.take(n2)?.to_vec();
            Ok((i1, i2))
        };
        let (i1, i2) = match kind {
            9 => (Vec::new(), Vec::new()),
            _ => info(cur)?,
        };
        let code: &[u8; 4] = match kind {
            0 => b"$con",
            1 => b"$lin",
            2 | 6 => b"$rec",
            3 | 8 => b"$ell",
            4 | 9 => b"$arc",
            5 => b"$pol",
            _ => b"$cur",
        };
        let mut comp = component(code, w, h, rel, level);
        // Stored line styles count from solid; the model's kinds count
        // from none.
        let line = LineStyle {
            color: line_color,
            width: line_width.max(0),
            attr: (style_word & 0x3F) + 1,
            outline: Some(0),
        };
        let ext = head.get(88..).unwrap_or(&[]);
        let gradient_at = if options & (1 << 17) != 0 { 32 } else { 0 };
        let bitmap_at = gradient_at + if options & (1 << 16) != 0 { 28 } else { 0 };
        let bitmap = (options & (1 << 18) != 0)
            .then(|| ext.get(bitmap_at..bitmap_at + 278))
            .flatten()
            .filter(|b| b[277] == 2)
            .and_then(|b| {
                let name = crate::text::kchars(&b[16..277]).to_ascii_lowercase();
                c.images.get(&name).copied()
            });
        let fill = if let Some(bin_id) = bitmap {
            Fill {
                kind: Fill::IMAGE,
                image: Some(hm_model::ImageFill {
                    mode: if options & 8 != 0 { 5 } else { 0 },
                    bin_id,
                    ..Default::default()
                }),
                ..Default::default()
            }
        } else if options & (1 << 16) != 0 && ext.len() >= gradient_at + 28 {
            let g = &ext[gradient_at..];
            Fill {
                kind: Fill::GRADIENT,
                gradient: Some(hm_model::Gradient {
                    kind: u32_at(g, 8).clamp(1, 4) as u8,
                    angle: i32_at(g, 12),
                    center_x: i32_at(g, 16),
                    center_y: i32_at(g, 20),
                    step: i32_at(g, 24),
                    positions: Vec::new(),
                    colors: vec![u32_at(g, 0) & 0xFF_FFFF, u32_at(g, 4) & 0xFF_FFFF],
                    step_center: Some(50),
                }),
                ..Default::default()
            }
        } else if fill_color >> 24 != 0 || matches!(kind, 1 | 4 | 7 | 9 | 10) {
            no_fill()
        } else {
            solid_fill(fill_color & 0x00FF_FFFF)
        };
        let _ = pattern;
        comp.style = (kind != 0).then(|| ShapeStyle {
            line,
            fill,
            ..Default::default()
        });
        let mut parts = Vec::new();
        let points = |i1: &[u8], i2: &[u8]| -> Vec<(i32, i32)> {
            let n = u32_at(i1, 0) as usize;
            (0..n.min(i2.len() / 8))
                .map(|k| (hu(i32_at(i2, k * 8)), hu(i32_at(i2, k * 8 + 4))))
                .collect()
        };
        match kind {
            1 => {
                let flip = u32_at(&i1, 0);
                let (x0, x1) = if flip & 1 != 0 { (w, 0) } else { (0, w) };
                let (y0, y1) = if flip & 2 != 0 { (h, 0) } else { (0, h) };
                parts.push(ShapePart::Line(LineGeom {
                    start: (x0, y0),
                    end: (x1, y1),
                    attr: None,
                    tail: SharedBytes::empty(),
                }));
            }
            2 | 6 => {
                let round = if options & 4 != 0 {
                    50
                } else if options & 1 != 0 {
                    20
                } else {
                    0
                };
                if kind == 6 || options & (1 << 19) != 0 {
                    let stored = (kind == 6 && !i2.is_empty())
                        .then(|| texts.next())
                        .flatten();
                    let paras = match stored {
                        Some(list) => c.paras(list, false, None),
                        None => Vec::new(),
                    };
                    let m = (hu(i32_at(a, 32)) as i16, hu(i32_at(a, 36)) as i16);
                    parts.push(ShapePart::Text(TextBox {
                        count: paras.len() as u32,
                        attr: if options & 16 != 0 { 1 << 5 } else { 0 },
                        margins: Some([m.0, m.0, m.1, m.1]),
                        max_width: Some(w),
                        paras,
                        ..Default::default()
                    }));
                }
                parts.push(ShapePart::Rect(RectGeom {
                    round,
                    corners: [(0, 0), (w, 0), (w, h), (0, h)],
                    tail: SharedBytes::empty(),
                }));
            }
            3 | 8 => parts.push(ShapePart::Ellipse(EllipseGeom {
                center: (w / 2, h / 2),
                axis1: (w, h / 2),
                axis2: (w / 2, 0),
                ..Default::default()
            })),
            4 | 9 => {
                let flip = u32_at(&i1, 0);
                let cx = if flip & 1 != 0 { w } else { 0 };
                let cy = if flip & 2 != 0 { 0 } else { h };
                parts.push(ShapePart::Arc(ArcGeom {
                    attr: 0,
                    center: (cx, cy),
                    axis1: (w - cx, cy),
                    axis2: (cx, h - cy),
                    tail: SharedBytes::empty(),
                }));
            }
            5 => parts.push(ShapePart::Polygon(PolyGeom {
                points: points(&i1, &i2),
                tail: SharedBytes::empty(),
            })),
            _ => {
                let pts = points(&i1, &i2);
                let n = pts.len();
                let segments = if kind == 10 {
                    let at = u32_at(&i1, 0) as usize * 8;
                    (0..n.saturating_sub(1))
                        .map(|k| i2.get(at + k).map_or(0, |&v| (v != 0) as u8))
                        .collect()
                } else {
                    vec![0; n.saturating_sub(1)]
                };
                parts.push(ShapePart::Curve(CurveGeom {
                    points: pts,
                    segments,
                    tail: SharedBytes::empty(),
                }));
            }
        }
        if link & 2 != 0 {
            let kids = objects(c, cur, texts, level + 1)?;
            comp.members = Some(kids.iter().map(|s| s.component.kind).collect());
            parts.extend(kids.into_iter().map(|s| ShapePart::Child(Box::new(s))));
        }
        out.push(Shape {
            component: comp,
            parts,
        });
        if link & 1 == 0 {
            break;
        }
    }
    Ok(out)
}

/// Paragraphs of a stored list, for tests.
#[allow(dead_code)]
fn plain(paras: &[Paragraph]) -> String {
    paras
        .iter()
        .map(|p| p.plain_text())
        .collect::<Vec<_>>()
        .join("\n")
}
