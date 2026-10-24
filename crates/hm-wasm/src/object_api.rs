//! Native object creation, properties and embedded image reads.
use crate::types::{Address, EngineError, Result, TableAddress};
use hm_edit::{Op, Pos};
use hm_model::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

fn bad(s: impl std::fmt::Display) -> EngineError {
    EngineError::new("BAD_REQUEST", s)
}
fn address(s: &str) -> EngineError {
    EngineError::new("BAD_ADDRESS", s)
}
fn positive(v: i32) -> Result<i32> {
    if v > 0 {
        Ok(v)
    } else {
        Err(bad("dimensions must be positive HU"))
    }
}
fn enum_code(value: &str, values: &[&str]) -> Result<u32> {
    values
        .iter()
        .position(|s| *s == value)
        .map(|n| n as u32)
        .ok_or_else(|| bad(format!("unsupported value {value:?}")))
}
fn flag(attr: &mut u32, bit: u32, value: bool) {
    *attr = (*attr & !(1 << bit)) | ((value as u32) << bit);
}
fn bits(attr: &mut u32, shift: u32, mask: u32, value: u32) {
    *attr = (*attr & !(mask << shift)) | (value << shift);
}
const VREL: &[&str] = &["paper", "page", "para"];
const HREL: &[&str] = &["paper", "page", "column", "para"];
const VALIGN: &[&str] = &["top", "center", "bottom", "inside", "outside"];
const HALIGN: &[&str] = &["left", "center", "right", "inside", "outside"];
const WRAP: &[&str] = &["square", "topAndBottom", "behind", "inFront"];
fn name<'a>(values: &[&'a str], n: u32) -> Option<&'a str> {
    values.get(n as usize).copied()
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BorderPatch {
    pub color: Option<u32>,
    pub width: Option<i32>,
    pub style: Option<u32>,
}
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObjectPatch {
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub horz_offset: Option<i32>,
    pub vert_offset: Option<i32>,
    pub treat_as_char: Option<bool>,
    pub vert_rel_to: Option<String>,
    pub horz_rel_to: Option<String>,
    pub vert_align: Option<String>,
    pub horz_align: Option<String>,
    pub text_wrap: Option<String>,
    pub margins: Option<[i16; 4]>,
    pub description: Option<String>,
    pub rotation: Option<i16>,
    pub z_order: Option<i32>,
    pub size_protect: Option<bool>,
    pub border: Option<BorderPatch>,
    pub crop: Option<[i32; 4]>,
}
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TablePatch {
    pub repeat_header: Option<bool>,
    pub split: Option<String>,
    pub spacing: Option<i16>,
    pub padding: Option<[i16; 4]>,
}
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CellPatch {
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub padding: Option<[i16; 4]>,
    pub vertical_align: Option<String>,
    pub own_padding: Option<bool>,
    pub protected: Option<bool>,
    pub header: Option<bool>,
}
fn paragraph(doc: &Document, at: &Address) -> Result<Paragraph> {
    hm_edit::ops::list(doc, &at.pos().list)
        .and_then(|l| l.get(at.para as usize))
        .cloned()
        .ok_or_else(|| address("no such paragraph"))
}
pub fn control<'a>(doc: &'a Document, at: &TableAddress) -> Result<&'a Control> {
    let c = hm_edit::ops::list(doc, &crate::tables::path(at))
        .and_then(|l| l.get(at.para as usize))
        .and_then(|p| p.controls.get(at.control as usize))
        .ok_or_else(|| address("no such control"))?;
    if c.object_common().is_none() {
        return Err(bad("the control is not an object"));
    }
    Ok(c)
}
fn replacement(doc: &Document, at: &TableAddress) -> Result<Paragraph> {
    control(doc, at)?;
    Ok(hm_edit::ops::list(doc, &crate::tables::path(at)).unwrap()[at.para as usize].clone())
}
fn replace(at: &TableAddress, mut p: Paragraph) -> Op {
    p.src = None;
    p.line_segs.clear();
    p.controls[at.control as usize].src = None;
    Op::Replace {
        list: crate::tables::path(at),
        start: at.para,
        count: 1,
        paras: vec![p],
    }
}
pub fn insert(doc: &Document, at: &Address, c: Control) -> Result<Op> {
    hm_edit::ops::insert_control(doc, &at.pos(), c).map_err(|e| EngineError::new("BAD_ADDRESS", e))
}
pub fn delete(doc: &Document, at: &TableAddress) -> Result<(Op, Pos)> {
    control(doc, at)?;
    let list = crate::tables::path(at);
    let p = &hm_edit::ops::list(doc, &list).unwrap()[at.para as usize];
    let offset = p
        .units()
        .find_map(|(offset, u)| {
            matches!(u, Unit::Extended {index,..} if index==at.control).then_some(offset)
        })
        .ok_or_else(|| address("object has no anchor"))?;
    let from = Pos {
        list,
        para: at.para,
        offset,
    };
    Ok((
        Op::Delete {
            from: from.clone(),
            to: Pos {
                offset: offset + 8,
                ..from.clone()
            },
        },
        from,
    ))
}
pub fn relocate(doc: &Document, object: &TableAddress, at: &Address) -> Result<(Vec<Op>, Pos)> {
    let c = control(doc, object)?.clone();
    // Validate against the original document before changing any paragraph.
    insert(doc, at, c.clone())?;
    let (delete, from) = delete(doc, object)?;
    let mut dest = at.pos();
    if dest.list.section == from.list.section && dest.list.steps.starts_with(&from.list.steps) {
        let depth = from.list.steps.len();
        if let Some(s) = dest.list.steps.get_mut(depth) {
            if s.para == object.para && s.control == object.control {
                return Err(bad("an object cannot move inside itself"));
            }
            if s.para == object.para && s.control > object.control {
                s.control -= 1;
            }
        } else if dest.para == from.para {
            if dest.offset > from.offset && dest.offset < from.offset + 8 {
                return Err(bad("destination is inside the object anchor"));
            }
            if dest.offset >= from.offset + 8 {
                dest.offset -= 8;
            }
        }
    }
    let mut staged = doc.clone();
    hm_edit::apply(&mut staged, &delete).map_err(|e| EngineError::new("BAD_ADDRESS", e))?;
    let add = insert(&staged, &Address::of(&dest), c)?;
    let end = Pos {
        offset: dest.offset + 8,
        ..dest
    };
    Ok((vec![delete, add], end))
}
fn header(width: i32, height: i32, inline: bool, x: i32, y: i32) -> Result<ObjectHeader> {
    positive(width)?;
    positive(height)?;
    Ok(ObjectHeader {
        instance_id: Some(0),
        prevent_split: Some(0),
        common: ObjectCommon {
            attr: (inline as u32) | (2 << 3) | (2 << 8) | (4 << 15) | (2 << 18),
            width,
            height,
            x_offset: x,
            y_offset: y,
            ..Default::default()
        },
        ..Default::default()
    })
}
fn empty_para(base: &Paragraph) -> Paragraph {
    Paragraph {
        header: ParaHeader {
            para_shape: base.header.para_shape,
            style: base.header.style,
            ..Default::default()
        },
        text: vec![PARA_END],
        runs: vec![CharRun {
            pos: 0,
            shape: base.shape_at(0),
        }],
        ..Default::default()
    }
}
pub fn create_table(
    doc: &mut Document,
    at: &Address,
    rows: u16,
    cols: u16,
    widths: Option<Vec<i32>>,
    heights: Option<Vec<i32>>,
    inline: bool,
) -> Result<Control> {
    if rows == 0 || cols == 0 || (rows as usize) * (cols as usize) > 10000 {
        return Err(bad("a table needs 1 to 10000 cells"));
    }
    let base = paragraph(doc, at)?;
    let body_width = doc
        .sections
        .get(at.section as usize)
        .and_then(|s| s.definition())
        .and_then(|s| s.page_def())
        .map(|p| {
            p.width
                .saturating_sub(p.margin_left)
                .saturating_sub(p.margin_right) as i32
        })
        .unwrap_or(42520);
    let widths =
        widths.unwrap_or_else(|| vec![(body_width / i32::from(cols)).max(200); cols as usize]);
    let heights = heights.unwrap_or_else(|| vec![1800; rows as usize]);
    if widths.len() != cols as usize || heights.len() != rows as usize {
        return Err(bad("columnWidths and rowHeights must match the grid"));
    }
    for v in widths.iter().chain(&heights) {
        positive(*v)?;
    }
    let sum = |v: &Vec<i32>| {
        v.iter()
            .try_fold(0i32, |a, b| a.checked_add(*b))
            .ok_or_else(|| bad("table dimensions overflow"))
    };
    let h = header(sum(&widths)?, sum(&heights)?, inline, 0, 0)?;
    if doc.info.border_fills.len() >= u16::MAX as usize {
        return Err(bad("too many border fills"));
    }
    let border = BorderLine {
        kind: 1,
        width: 0,
        color: 0,
    };
    doc.info.border_fills.push(BorderFill {
        sides: [border; 4],
        ..Default::default()
    });
    let bf = doc.info.border_fills.len() as u16;
    doc.info.id_mappings.src = None;
    let mut cells = Vec::new();
    for row in 0..rows {
        for col in 0..cols {
            cells.push(Cell {
                row,
                col,
                row_span: 1,
                col_span: 1,
                width: widths[col as usize],
                height: heights[row as usize],
                border_fill: bf,
                paras: vec![empty_para(&base)],
                ..Default::default()
            });
        }
    }
    let table = Table {
        header: h,
        rows,
        cols,
        attr: 1,
        padding: [141, 141, 141, 141],
        row_cells: vec![cols; rows as usize],
        border_fill: bf,
        cells,
        ..Default::default()
    };
    Ok(Control {
        kind: ControlKind::Table(Box::new(table)),
        src: None,
    })
}
fn component(kind: CtrlId, w: i32, h: i32, style: Option<ShapeStyle>) -> ShapeComponent {
    ShapeComponent {
        kind,
        doubled: true,
        offset: (0, 0),
        group_level: 0,
        version: 1,
        initial_size: (w as u32, h as u32),
        size: (w as u32, h as u32),
        flip: 0,
        rotation: 0,
        rotation_center: (w / 2, h / 2),
        translation: Matrix::IDENTITY,
        transforms: vec![(Matrix::IDENTITY, Matrix::IDENTITY)],
        style,
        members: None,
        group_instance: None,
        tail: SharedBytes::default(),
    }
}
fn corners(w: i32, h: i32) -> [(i32, i32); 4] {
    [(0, 0), (w, 0), (w, h), (0, h)]
}
fn shape_control(header: ObjectHeader, shape: Shape) -> Control {
    Control {
        kind: ControlKind::Object(Box::new(ObjectControl {
            id: CtrlId::SHAPE,
            header,
            parts: vec![ObjectPart::Shape(Box::new(shape))],
        })),
        src: None,
    }
}
pub fn create_shape(
    doc: &Document,
    at: &Address,
    kind: &str,
    w: i32,
    h: i32,
    text: Option<String>,
    inline: bool,
    x: i32,
    y: i32,
) -> Result<Control> {
    let header = header(w, h, inline, x, y)?;
    let base = paragraph(doc, at)?;
    let (id, part) = match kind {
        "rect" | "textbox" => (
            CtrlId::from_chars(b"$rec"),
            ShapePart::Rect(RectGeom {
                corners: corners(w, h),
                ..Default::default()
            }),
        ),
        "ellipse" => (
            CtrlId::from_chars(b"$ell"),
            ShapePart::Ellipse(EllipseGeom {
                center: (w / 2, h / 2),
                axis1: (w, h / 2),
                axis2: (w / 2, 0),
                ..Default::default()
            }),
        ),
        "line" => (
            CtrlId::from_chars(b"$lin"),
            ShapePart::Line(LineGeom {
                end: (w, h),
                attr: Some(0),
                ..Default::default()
            }),
        ),
        _ => return Err(bad("shape kind must be rect, ellipse, line or textbox")),
    };
    if kind == "line" && text.is_some() {
        return Err(bad("lines do not hold text"));
    }
    let style = ShapeStyle {
        line: LineStyle {
            width: 28,
            attr: 1,
            outline: Some(0),
            ..Default::default()
        },
        ..Default::default()
    };
    let mut parts = vec![part];
    if kind == "textbox" || text.is_some() {
        let mut paras = vec![empty_para(&base)];
        if let Some(text) = text {
            let mut temp = doc.clone();
            temp.sections = vec![Section {
                paras,
                ..Default::default()
            }];
            hm_edit::apply(
                &mut temp,
                &Op::InsertText {
                    at: Pos {
                        list: hm_edit::ListPath::default(),
                        para: 0,
                        offset: 0,
                    },
                    text,
                },
            )
            .map_err(|e| EngineError::new("BAD_REQUEST", e))?;
            paras = temp.sections.remove(0).paras;
        }
        parts.push(ShapePart::Text(TextBox {
            count: paras.len() as u32,
            margins: Some([0; 4]),
            max_width: Some(w),
            paras,
            ..Default::default()
        }));
    }
    Ok(shape_control(
        header,
        Shape {
            component: component(id, w, h, Some(style)),
            parts,
        },
    ))
}
#[allow(clippy::too_many_arguments)]
pub fn create_picture(
    doc: &mut Document,
    bytes: Vec<u8>,
    extension: String,
    description: Option<String>,
    w: i32,
    h: i32,
    nw: Option<i32>,
    nh: Option<i32>,
    inline: bool,
    x: i32,
    y: i32,
) -> Result<Control> {
    let mut hder = header(w, h, inline, x, y)?;
    let nw = positive(nw.unwrap_or(w))?;
    let nh = positive(nh.unwrap_or(h))?;
    let ext = extension.trim_start_matches('.').to_ascii_lowercase();
    if !matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "tif" | "tiff" | "webp" | "svg" | "wmf" | "emf"
    ) {
        return Err(bad("unsupported picture extension"));
    }
    if bytes.is_empty() {
        return Err(bad("picture bytes are empty"));
    }
    if doc.info.bin_data.len() >= u16::MAX as usize {
        return Err(bad("too many embedded images"));
    }
    let id = doc.info.bin_data.len() as u16 + 1;
    let storage = doc
        .info
        .bin_data
        .iter()
        .filter_map(|b| b.storage_id)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| bad("no free BinData storage id"))?;
    doc.info.bin_data.push(BinDataEntry {
        attr: 1,
        storage_id: Some(storage),
        extension: Some(HwpString::from_text(&ext)),
        ..Default::default()
    });
    doc.info.id_mappings.src = None;
    let old = doc.bins.clone();
    let old_ole = old.clone();
    let bytes = SharedBytes::from(bytes);
    doc.bins = Some(
        BinStore::new(move |key| {
            if key == id {
                Some(bytes.clone())
            } else {
                old.as_ref().and_then(|b| b.get(key))
            }
        })
        .with_ole(move |key| old_ole.as_ref().and_then(|b| b.ole(key))),
    );
    hder.description = description.map(|s| HwpString::from_text(&s));
    let mut tail = vec![0; 4];
    tail.extend(nw.to_le_bytes());
    tail.extend(nh.to_le_bytes());
    tail.push(0);
    let picture = Picture {
        corners: corners(w, h),
        crop: [0, 0, nw, nh],
        image: ImageRef {
            bin_id: id,
            ..Default::default()
        },
        tail: SharedBytes::from(tail),
        ..Default::default()
    };
    Ok(shape_control(
        hder,
        Shape {
            component: component(CtrlId::from_chars(b"$pic"), w, h, None),
            parts: vec![ShapePart::Picture(picture)],
        },
    ))
}
fn header_mut(c: &mut Control) -> Result<&mut ObjectHeader> {
    match &mut c.kind {
        ControlKind::Table(t) => Ok(&mut t.header),
        ControlKind::Object(o) => Ok(&mut o.header),
        _ => Err(bad("unsupported object kind")),
    }
}
fn shape_mut(c: &mut Control) -> Result<&mut Shape> {
    let ControlKind::Object(o) = &mut c.kind else {
        return Err(bad("the object is not a drawing"));
    };
    o.parts
        .iter_mut()
        .find_map(|p| match p {
            ObjectPart::Shape(s) => Some(&mut **s),
            _ => None,
        })
        .ok_or_else(|| bad("the object has no shape"))
}
fn picture(s: &Shape) -> Option<&Picture> {
    s.parts.iter().find_map(|p| match p {
        ShapePart::Picture(p) => Some(p),
        _ => None,
    })
}
fn line_mut(s: &mut Shape) -> Result<&mut LineStyle> {
    if s.parts.iter().any(|p| matches!(p, ShapePart::Picture(_))) {
        return s
            .parts
            .iter_mut()
            .find_map(|p| match p {
                ShapePart::Picture(p) => Some(&mut p.border),
                _ => None,
            })
            .ok_or_else(|| bad("no picture border"));
    }
    s.component
        .style
        .as_mut()
        .map(|s| &mut s.line)
        .ok_or_else(|| bad("the drawing has no line style"))
}
pub fn set_object(
    doc: &Document,
    fonts: &hm_fonts::FontSystem,
    at: &TableAddress,
    p: ObjectPatch,
) -> Result<Op> {
    let mut para = replacement(doc, at)?;
    let c = &mut para.controls[at.control as usize];
    let old = c.object_common().unwrap();
    if p.width.is_some() || p.height.is_some() {
        let w = positive(p.width.unwrap_or(old.width))?;
        let h = positive(p.height.unwrap_or(old.height))?;
        if w < 283 || h < 283 {
            return Err(bad("object property dimensions must be at least 283 HU"));
        }
        let Op::Replace { paras, .. } = crate::objects::edit(
            doc,
            fonts,
            at,
            crate::objects::Action::Resize {
                width: w,
                height: h,
            },
        )?
        else {
            unreachable!()
        };
        para = paras.into_iter().next().unwrap();
    }
    let c = &mut para.controls[at.control as usize];
    let is_table = matches!(c.kind, ControlKind::Table(_));
    if is_table && p.size_protect.is_some() {
        return Err(bad("table size protection is not supported"));
    }
    let h = header_mut(c)?;
    let o = &mut h.common;
    if let Some(v) = p.horz_offset {
        o.x_offset = v;
    }
    if let Some(v) = p.vert_offset {
        o.y_offset = v;
    }
    if let Some(v) = p.treat_as_char {
        flag(&mut o.attr, 0, v);
    }
    if let Some(v) = p.vert_rel_to {
        bits(&mut o.attr, 3, 3, enum_code(&v, VREL)?);
    }
    if let Some(v) = p.horz_rel_to {
        bits(&mut o.attr, 8, 3, enum_code(&v, HREL)?);
    }
    if let Some(v) = p.vert_align {
        bits(&mut o.attr, 5, 7, enum_code(&v, VALIGN)?);
    }
    if let Some(v) = p.horz_align {
        bits(&mut o.attr, 10, 7, enum_code(&v, HALIGN)?);
    }
    if let Some(v) = p.text_wrap {
        bits(&mut o.attr, 21, 7, enum_code(&v, WRAP)?);
    }
    if let Some(v) = p.margins {
        o.margins = v;
    }
    if let Some(v) = p.z_order {
        o.z_order = v;
    }
    if let Some(v) = p.size_protect {
        flag(&mut o.attr, 20, v);
    }
    if let Some(v) = p.description {
        h.instance_id.get_or_insert(0);
        h.prevent_split.get_or_insert(0);
        h.description = Some(HwpString::from_text(&v));
    }
    if let Some(v) = p.rotation {
        if !(-360..=360).contains(&v) {
            return Err(bad("rotation must be between -360 and 360 degrees"));
        }
        let s = shape_mut(c)?;
        let (w, h) = s.component.size;
        let r = (v as f64).to_radians();
        let (sin, cos) = r.sin_cos();
        let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
        s.component.rotation = v;
        s.component.rotation_center = (w as i32 / 2, h as i32 / 2);
        let rotation = Matrix([
            cos,
            -sin,
            cx - cos * cx + sin * cy,
            sin,
            cos,
            cy - sin * cx - cos * cy,
        ]);
        match s.component.transforms.first_mut() {
            Some((_, r)) => *r = rotation,
            None => s.component.transforms.push((Matrix::IDENTITY, rotation)),
        }
    }
    if let Some(v) = p.border {
        let l = line_mut(shape_mut(c)?)?;
        if let Some(v) = v.color {
            l.color = v;
        }
        if let Some(v) = v.width {
            if v < 0 {
                return Err(bad("border width cannot be negative"));
            }
            l.width = v;
        }
        if let Some(v) = v.style {
            if v > 17 {
                return Err(bad("unsupported border style"));
            }
            bits(&mut l.attr, 0, 63, v);
        }
    }
    if let Some(v) = p.crop {
        let s = shape_mut(c)?;
        let pic = s
            .parts
            .iter_mut()
            .find_map(|p| match p {
                ShapePart::Picture(p) => Some(p),
                _ => None,
            })
            .ok_or_else(|| bad("crop is only supported for pictures"))?;
        if v[0] < 0 || v[1] < 0 || v[2] <= v[0] || v[3] <= v[1] {
            return Err(bad("crop must describe a positive image rectangle"));
        }
        pic.crop = v;
    }
    Ok(replace(at, para))
}
fn table_mut<'a>(p: &'a mut Paragraph, at: &TableAddress) -> Result<&'a mut Table> {
    match &mut p.controls[at.control as usize].kind {
        ControlKind::Table(t) => Ok(t),
        _ => Err(bad("the object is not a table")),
    }
}
pub fn set_table(
    doc: &Document,
    fonts: &hm_fonts::FontSystem,
    at: &TableAddress,
    p: TablePatch,
) -> Result<Op> {
    let mut para = replacement(doc, at)?;
    let t = table_mut(&mut para, at)?;
    if let Some(v) = p.repeat_header {
        flag(&mut t.attr, 2, v);
    }
    if let Some(v) = p.split {
        bits(&mut t.attr, 0, 3, enum_code(&v, &["none", "row", "cell"])?);
    }
    if let Some(v) = p.spacing {
        if v < 0 {
            return Err(bad("table spacing cannot be negative"));
        }
        t.spacing = v;
    }
    if let Some(v) = p.padding {
        if v.iter().any(|v| *v < 0) {
            return Err(bad("padding cannot be negative"));
        }
        t.padding = v;
    }
    for c in &mut t.cells {
        for p in &mut c.paras {
            p.src = None;
            p.line_segs.clear();
        }
    }
    crate::objects::fit_table(doc, fonts, at.section, t);
    Ok(replace(at, para))
}
pub fn set_cell(
    doc: &Document,
    fonts: &hm_fonts::FontSystem,
    at: &TableAddress,
    row: u16,
    col: u16,
    p: CellPatch,
) -> Result<Op> {
    let mut para = replacement(doc, at)?;
    let t = table_mut(&mut para, at)?;
    let c = t
        .cells
        .iter_mut()
        .find(|c| c.row == row && c.col == col)
        .ok_or_else(|| address("no cell origin at this row and column"))?;
    if let Some(v) = p.width {
        crate::objects::set_width(c, positive(v)?);
    }
    if let Some(v) = p.height {
        c.height = positive(v)?;
    }
    if let Some(v) = p.padding {
        if v.iter().any(|v| *v < 0) {
            return Err(bad("padding cannot be negative"));
        }
        c.padding = v;
        flag(&mut c.list_attr, 16, true);
    }
    if let Some(v) = p.own_padding {
        flag(&mut c.list_attr, 16, v);
    }
    if let Some(v) = p.vertical_align {
        bits(
            &mut c.list_attr,
            5,
            3,
            enum_code(&v, &["top", "center", "bottom"])?,
        );
    }
    if let Some(v) = p.protected {
        flag(&mut c.list_attr, 17, v);
    }
    if let Some(v) = p.header {
        flag(&mut c.list_attr, 18, v);
    }
    for p in &mut c.paras {
        p.src = None;
        p.line_segs.clear();
    }
    crate::objects::fit_table(doc, fonts, at.section, t);
    Ok(replace(at, para))
}
fn border(l: &LineStyle) -> Value {
    json!({"color":l.color,"width":l.width,"style":l.kind()})
}
pub fn object_properties(doc: &Document, at: &TableAddress) -> Result<Value> {
    let c = control(doc, at)?;
    let o = c.object_common().unwrap();
    let h = match &c.kind {
        ControlKind::Table(t) => &t.header,
        ControlKind::Object(o) => &o.header,
        _ => return Err(bad("unsupported object kind")),
    };
    let mut out = json!({"kind":if c.as_table().is_some(){"table"}else{"object"},"width":o.width,"height":o.height,"horzOffset":o.x_offset,"vertOffset":o.y_offset,"treatAsChar":o.treat_as_char(),"vertRelTo":name(VREL,o.vert_rel_to()),"horzRelTo":name(HREL,(o.attr>>8)&3),"vertAlign":name(VALIGN,(o.attr>>5)&7),"horzAlign":name(HALIGN,(o.attr>>10)&7),"textWrap":name(WRAP,o.text_wrap()),"margins":o.margins,"zOrder":o.z_order,"sizeProtect":c.as_table().is_none()&&o.attr&(1<<20)!=0,"description":h.description.as_ref().map(|s|s.to_string_lossy()).unwrap_or_default()});
    if let Some(s) = c.as_object().and_then(|o| o.shape()) {
        out["rotation"] = json!(s.component.rotation);
        out["flipHorizontal"] = json!(s.component.flip_horizontal());
        out["flipVertical"] = json!(s.component.flip_vertical());
        out["kind"] = json!(match s.kind() {
            Some(ShapePart::Picture(_)) => "picture",
            Some(ShapePart::Line(_)) => "line",
            Some(ShapePart::Rect(_)) => "rect",
            Some(ShapePart::Ellipse(_)) => "ellipse",
            _ => "shape",
        });
        if let Some(t) = s.text() {
            out["text"] = json!(
                t.paras
                    .iter()
                    .map(|p| p.plain_text())
                    .collect::<Vec<_>>()
                    .join("\n")
            );
        }
        if let Some(pic) = picture(s) {
            let (w, h) = pic.original_size().unwrap_or((
                s.component.initial_size.0 as i32,
                s.component.initial_size.1 as i32,
            ));
            out["crop"] = json!(pic.crop);
            out["border"] = border(&pic.border);
            out["pictureMargins"] = json!(pic.margins);
            out["naturalWidth"] = json!(w);
            out["naturalHeight"] = json!(h);
            out["brightness"] = json!(pic.image.brightness);
            out["contrast"] = json!(pic.image.contrast);
            out["effect"] = json!(pic.image.effect);
            out["transparency"] = json!(pic.alpha());
        } else if let Some(st) = s.style() {
            out["border"] = border(&st.line);
            out["fill"] = json!({"kind":st.fill.kind,"color":st.fill.background(),"solid":st.fill.solid.map(|s|json!({"background":s.background,"patternColor":s.pattern_color,"pattern":s.pattern})),"gradient":st.fill.gradient.as_ref().map(|g|json!({"kind":g.kind,"angle":g.angle,"centerX":g.center_x,"centerY":g.center_y,"step":g.step,"positions":g.positions,"colors":g.colors})),"image":st.fill.image.map(|i|json!({"mode":i.mode,"binId":i.bin_id,"brightness":i.brightness,"contrast":i.contrast,"effect":i.effect})),"alpha":st.fill.alpha});
        }
    }
    Ok(out)
}
fn border_fill(doc: &Document, id: u16) -> Value {
    let Some(b) = id
        .checked_sub(1)
        .and_then(|i| doc.info.border_fills.get(i as usize))
    else {
        return Value::Null;
    };
    let line = |l: BorderLine| json!({"style":l.kind,"width":l.width_hu(),"widthIndex":l.width,"color":l.color});
    json!({"id":id,"left":line(b.sides[0]),"right":line(b.sides[1]),"top":line(b.sides[2]),"bottom":line(b.sides[3]),"diagonal":line(b.diagonal),"background":b.fill.as_ref().and_then(|f|f.background())})
}
fn table<'a>(doc: &'a Document, at: &TableAddress) -> Result<&'a Table> {
    control(doc, at)?
        .as_table()
        .ok_or_else(|| bad("the object is not a table"))
}
pub fn table_properties(doc: &Document, at: &TableAddress) -> Result<Value> {
    let t = table(doc, at)?;
    Ok(
        json!({"rows":t.rows,"columns":t.cols,"repeatHeader":t.repeat_header(),"split":name(&["none","row","cell"],t.split()),"spacing":t.spacing,"padding":t.padding,"borderFill":border_fill(doc,t.border_fill)}),
    )
}
pub fn cell_properties(doc: &Document, at: &TableAddress, row: u16, col: u16) -> Result<Value> {
    let t = table(doc, at)?;
    let c = t
        .cells
        .iter()
        .find(|c| c.row == row && c.col == col)
        .ok_or_else(|| address("no cell origin at this row and column"))?;
    Ok(
        json!({"row":c.row,"column":c.col,"rowSpan":c.row_span,"columnSpan":c.col_span,"width":c.width,"height":c.height,"padding":t.cell_padding(c),"ownPadding":c.own_padding(),"tablePadding":t.padding,"verticalAlign":name(&["top","center","bottom"],c.vertical_align()),"protected":c.protected(),"header":c.header(),"borderFill":border_fill(doc,c.border_fill)}),
    )
}
pub fn picture_data(doc: &Document, at: &TableAddress) -> Result<Value> {
    let c = control(doc, at)?;
    let s = c
        .as_object()
        .and_then(|o| o.shape())
        .ok_or_else(|| bad("the object has no picture"))?;
    let p = picture(s).ok_or_else(|| bad("the object is not a picture"))?;
    let bytes = doc
        .bins
        .as_ref()
        .and_then(|b| b.get(p.image.bin_id))
        .ok_or_else(|| bad("picture bytes are unavailable"))?;
    let ext = doc
        .info
        .bin_data
        .get(p.image.bin_id.saturating_sub(1) as usize)
        .and_then(|e| e.extension.as_ref())
        .map(|s| s.to_string_lossy())
        .unwrap_or_default();
    let mime = match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "tif" | "tiff" => "image/tiff",
        "wmf" => "image/x-wmf",
        "emf" => "image/x-emf",
        _ => "application/octet-stream",
    };
    let (w, h) = p.original_size().unwrap_or((
        s.component.initial_size.0 as i32,
        s.component.initial_size.1 as i32,
    ));
    Ok(json!({"bytes":&*bytes,"extension":ext,"mime":mime,"naturalWidth":w,"naturalHeight":h}))
}
