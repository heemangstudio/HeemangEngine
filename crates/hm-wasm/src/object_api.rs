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
    pub flip_horizontal: Option<bool>,
    pub flip_vertical: Option<bool>,
    pub z_order: Option<i32>,
    pub size_protect: Option<bool>,
    pub restrict_in_page: Option<bool>,
    pub allow_overlap: Option<bool>,
    pub keep_with_anchor: Option<bool>,
    pub border: Option<BorderPatch>,
    pub crop: Option<[i32; 4]>,
}
/// One border line: an OWPML line type (0 none, 1 solid, ...), a width
/// index into [`BORDER_WIDTHS`] and a COLORREF.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LinePatch {
    pub style: u8,
    pub width: u8,
    pub color: u32,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum FillPatch {
    None,
    #[serde(rename_all = "camelCase")]
    Solid {
        background: u32,
        #[serde(default)]
        pattern_color: u32,
        /// Hatch: 0 none, 1 horizontal ... 6 diagonal cross.
        #[serde(default)]
        pattern: i32,
    },
}
/// Changes to a border fill; what is left out stays.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BorderFillPatch {
    pub left: Option<LinePatch>,
    pub right: Option<LinePatch>,
    pub top: Option<LinePatch>,
    pub bottom: Option<LinePatch>,
    pub diagonal: Option<LinePatch>,
    /// Slash and backslash shape bits (0 none, 2 corner to corner, ...).
    pub slash: Option<u16>,
    pub backslash: Option<u16>,
    /// `none`, `horizontal`, `vertical` or `cross`; a center line takes
    /// the place of the diagonals.
    pub center_line: Option<String>,
    pub fill: Option<FillPatch>,
}
impl BorderFillPatch {
    fn sides(&self) -> [Option<LinePatch>; 4] {
        [self.left, self.right, self.top, self.bottom]
    }
}
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TablePatch {
    pub repeat_header: Option<bool>,
    pub split: Option<String>,
    pub spacing: Option<i16>,
    pub padding: Option<[i16; 4]>,
    pub border_fill: Option<BorderFillPatch>,
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
    /// 0 across, 1 down with Latin turned, 2 down upright.
    pub text_direction: Option<u32>,
    pub editable_in_form: Option<bool>,
    /// The cell's border fill; its changed sides carry over to the
    /// neighbors sharing them.
    pub border_fill: Option<BorderFillPatch>,
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
fn parts_mut(o: &mut ObjectControl) -> Result<(&mut ObjectCommon, &mut Shape)> {
    let shape = o
        .parts
        .iter_mut()
        .find_map(|p| match p {
            ObjectPart::Shape(s) => Some(&mut **s),
            _ => None,
        })
        .ok_or_else(|| bad("the object has no shape"))?;
    Ok((&mut o.header.common, shape))
}
/// The drawing's own box mapped into its object's box: left, top, right,
/// bottom.
fn drawn_bounds(c: &ShapeComponent) -> (f64, f64, f64, f64) {
    let (w, h) = local_size(c);
    let m = c.matrix();
    let pts = [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)].map(|(x, y)| m.apply(x, y));
    let xs = pts.map(|p| p.0);
    let ys = pts.map(|p| p.1);
    let min = |v: [f64; 4]| v.into_iter().fold(f64::INFINITY, f64::min);
    let max = |v: [f64; 4]| v.into_iter().fold(f64::NEG_INFINITY, f64::max);
    (min(xs), min(ys), max(xs), max(ys))
}
fn local_size(c: &ShapeComponent) -> (f64, f64) {
    let (w, h) = if c.initial_size.0 > 0 && c.initial_size.1 > 0 {
        c.initial_size
    } else {
        c.size
    };
    (w as f64, h as f64)
}
/// Turns a drawing to `angle` degrees and mirrors it, both about the
/// center of its scaled box. With `framed` (pictures, as Hancom saves
/// them) the object's box becomes the turned drawing's bounds, the
/// drawing kept where it is on the page.
fn orient(
    o: &mut ObjectCommon,
    c: &mut ShapeComponent,
    angle: i16,
    flip: (bool, bool),
    framed: bool,
) {
    let (w, h) = local_size(c);
    let scale = c.transforms.first().map_or(Matrix::IDENTITY, |(s, _)| *s);
    let (cx, cy) = scale.apply(w / 2.0, h / 2.0);
    let (sin, cos) = (angle as f64).to_radians().sin_cos();
    let (fx, fy) = (
        if flip.0 { -1.0 } else { 1.0 },
        if flip.1 { -1.0 } else { 1.0 },
    );
    // Center, rotate, mirror, back: T(c) R F T(-c).
    let (a, b, d, e) = (cos * fx, -sin * fy, sin * fx, cos * fy);
    let turn = Matrix([a, b, cx - a * cx - b * cy, d, e, cy - d * cx - e * cy]);
    match c.transforms.first_mut() {
        Some((_, r)) => *r = turn,
        None => c.transforms.push((Matrix::IDENTITY, turn)),
    }
    c.rotation = angle;
    c.flip = (c.flip & !3) | flip.0 as u32 | (flip.1 as u32) << 1;
    if framed {
        let (x0, y0, x1, y1) = drawn_bounds(c);
        let (dx, dy) = (x0.round(), y0.round());
        c.translation.0[2] -= dx;
        c.translation.0[5] -= dy;
        o.x_offset += dx as i32;
        o.y_offset += dy as i32;
        o.width = ((x1 - x0).round() as i32).max(1);
        o.height = ((y1 - y0).round() as i32).max(1);
        c.rotation_center = (o.width / 2, o.height / 2);
    } else {
        c.rotation_center = (cx.round() as i32, cy.round() as i32);
    }
}
/// Gives a turned picture a new frame: the drawing size whose turned
/// bounds are `frame`, solved from the frame (proportional near 45
/// degrees, where the frame does not fix the drawing's aspect).
fn resize_framed(c: &mut Control, frame: (i32, i32)) -> Result<()> {
    let ControlKind::Object(o) = &mut c.kind else {
        return Err(bad("the object is not a drawing"));
    };
    let (common, shape) = parts_mut(o)?;
    let k = &shape.component;
    let (angle, flip) = (k.rotation, (k.flip_horizontal(), k.flip_vertical()));
    let (w0, h0) = (k.size.0.max(1) as f64, k.size.1.max(1) as f64);
    let (sin, cos) = (angle as f64).to_radians().sin_cos();
    let (s, c2) = (sin.abs(), cos.abs());
    let (fw, fh) = (frame.0 as f64, frame.1 as f64);
    let det = c2 * c2 - s * s;
    let solved = (det.abs() > 0.1)
        .then(|| ((fw * c2 - fh * s) / det, (fh * c2 - fw * s) / det))
        .filter(|(w, h)| *w >= 283.0 && *h >= 283.0);
    let (w, h) = solved.unwrap_or_else(|| {
        let k = (fw + fh) / (w0 * c2 + h0 * s + w0 * s + h0 * c2);
        (w0 * k, h0 * k)
    });
    let new = ((w.round() as i32).max(283), (h.round() as i32).max(283));
    orient(common, &mut shape.component, 0, (false, false), true);
    crate::objects::resize_shape(shape, (common.width, common.height), new)?;
    crate::objects::absolute_size(common);
    common.width = new.0;
    common.height = new.1;
    orient(common, &mut shape.component, angle, flip, true);
    Ok(())
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
    let framed = c
        .as_object()
        .and_then(|o| o.shape())
        .is_some_and(|s| picture(s).is_some());
    let turned = c
        .as_object()
        .and_then(|o| o.shape())
        .is_some_and(|s| s.component.rotation.rem_euclid(360) != 0);
    if p.width.is_some() || p.height.is_some() {
        let w = positive(p.width.unwrap_or(old.width))?;
        let h = positive(p.height.unwrap_or(old.height))?;
        if w < 283 || h < 283 {
            return Err(bad("object property dimensions must be at least 283 HU"));
        }
        if framed && turned {
            // A turned picture's box is its frame: size the drawing so its
            // turned bounds fill the new frame.
            resize_framed(c, (w, h))?;
        } else {
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
    if let Some(v) = p.restrict_in_page {
        flag(&mut o.attr, 13, v);
    }
    if let Some(v) = p.allow_overlap {
        flag(&mut o.attr, 14, v);
    }
    if let Some(v) = p.keep_with_anchor {
        h.instance_id.get_or_insert(0);
        h.prevent_split = Some(v as i32);
    }
    if let Some(v) = p.description {
        h.instance_id.get_or_insert(0);
        h.prevent_split.get_or_insert(0);
        h.description = Some(HwpString::from_text(&v));
    }
    if p.rotation.is_some() || p.flip_horizontal.is_some() || p.flip_vertical.is_some() {
        let angle = p.rotation.unwrap_or(shape_mut(c)?.component.rotation);
        if !(-360..=360).contains(&angle) {
            return Err(bad("rotation must be between -360 and 360 degrees"));
        }
        let k = &shape_mut(c)?.component;
        let fh = p.flip_horizontal.unwrap_or(k.flip_horizontal());
        let fv = p.flip_vertical.unwrap_or(k.flip_vertical());
        let ControlKind::Object(o) = &mut c.kind else {
            return Err(bad("the object is not a drawing"));
        };
        let (common, shape) = parts_mut(o)?;
        orient(common, &mut shape.component, angle, (fh, fv), framed);
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
fn line_of(l: LinePatch) -> Result<BorderLine> {
    if l.width as usize >= BORDER_WIDTHS.len() || l.style > 17 {
        return Err(bad("unsupported border line"));
    }
    Ok(BorderLine {
        kind: l.style,
        width: l.width,
        color: l.color,
    })
}
/// `base` with a patch applied, as a new border fill to intern.
fn patched(base: Option<&BorderFill>, p: &BorderFillPatch) -> Result<BorderFill> {
    let mut b = base.cloned().unwrap_or_default();
    b.src = None;
    for (i, side) in p.sides().into_iter().enumerate() {
        if let Some(l) = side {
            b.sides[i] = line_of(l)?;
        }
    }
    if let Some(l) = p.diagonal {
        b.diagonal = line_of(l)?;
    }
    let mut attr = b.attr as u32;
    for (v, shift) in [(p.slash, 2), (p.backslash, 5)] {
        if let Some(v) = v {
            if v > 7 {
                return Err(bad("diagonal shape bits run 0 to 7"));
            }
            bits(&mut attr, shift, 7, v as u32);
        }
    }
    if let Some(c) = &p.center_line {
        match enum_code(c, &["none", "horizontal", "vertical", "cross"])? {
            0 => flag(&mut attr, 13, false),
            k => {
                flag(&mut attr, 13, true);
                bits(&mut attr, 8, 3, k);
                bits(&mut attr, 2, 0x3F, 0);
            }
        }
    }
    b.attr = attr as u16;
    if let Some(f) = &p.fill {
        let extra = b
            .fill
            .as_ref()
            .and_then(|f| f.extra.clone())
            .or_else(|| Some(SharedBytes::empty()));
        b.fill = Some(match *f {
            FillPatch::None => Fill {
                extra,
                ..Fill::default()
            },
            FillPatch::Solid {
                background,
                pattern_color,
                pattern,
            } => Fill {
                kind: Fill::SOLID,
                solid: Some(SolidFill {
                    background,
                    pattern_color,
                    pattern,
                }),
                extra,
                alpha: vec![0],
                ..Fill::default()
            },
        });
    }
    Ok(b)
}
fn same_fill(a: &BorderFill, b: &BorderFill) -> bool {
    a.attr == b.attr
        && a.sides == b.sides
        && a.diagonal == b.diagonal
        && a.fill == b.fill
        && a.tail == b.tail
}
/// The id (1-based) of a border fill equal to `b`, added when new.
fn intern(doc: &mut Document, b: BorderFill) -> Result<u16> {
    if let Some(i) = doc.info.border_fills.iter().position(|x| same_fill(x, &b)) {
        return Ok(i as u16 + 1);
    }
    if doc.info.border_fills.len() >= u16::MAX as usize {
        return Err(bad("too many border fills"));
    }
    doc.info.border_fills.push(b);
    doc.info.id_mappings.src = None;
    Ok(doc.info.border_fills.len() as u16)
}
fn border_fill_at(doc: &Document, id: u16) -> Option<&BorderFill> {
    id.checked_sub(1)
        .and_then(|i| doc.info.border_fills.get(i as usize))
}
fn apply_border_fill(doc: &mut Document, id: u16, p: &BorderFillPatch) -> Result<u16> {
    let b = patched(border_fill_at(doc, id), p)?;
    intern(doc, b)
}
/// Gives cell `k` a border fill patch and carries its changed sides over to
/// the neighbors that share them.
fn patch_cell_borders(
    doc: &mut Document,
    t: &mut Table,
    k: usize,
    p: &BorderFillPatch,
) -> Result<()> {
    let id = apply_border_fill(doc, t.cells[k].border_fill, p)?;
    t.cells[k].border_fill = id;
    let c = &t.cells[k];
    let (c0, c1, r0, r1) = (c.col, c.col + c.col_span, c.row, c.row + c.row_span);
    let sides = p.sides();
    for i in 0..t.cells.len() {
        if i == k {
            continue;
        }
        let n = &t.cells[i];
        let (n0, n1, m0, m1) = (n.col, n.col + n.col_span, n.row, n.row + n.row_span);
        let rows = m0 < r1 && m1 > r0;
        let cols = n0 < c1 && n1 > c0;
        let mut q = BorderFillPatch::default();
        if rows && n1 == c0 {
            q.right = sides[0];
        }
        if rows && n0 == c1 {
            q.left = sides[1];
        }
        if cols && m1 == r0 {
            q.bottom = sides[2];
        }
        if cols && m0 == r1 {
            q.top = sides[3];
        }
        if q.sides().iter().any(Option::is_some) {
            t.cells[i].border_fill = apply_border_fill(doc, t.cells[i].border_fill, &q)?;
        }
    }
    Ok(())
}
/// Applies a border fill to a block of cells as one cell (a table zone).
pub fn set_zone(
    doc: &mut Document,
    at: &TableAddress,
    (r0, c0, r1, c1): (u16, u16, u16, u16),
    p: &BorderFillPatch,
) -> Result<Op> {
    let mut para = replacement(doc, at)?;
    let t = table_mut(&mut para, at)?;
    if t.rows == 0 || t.cols == 0 {
        return Err(bad("an empty table has no cell zones"));
    }
    let (r0, r1) = (r0.min(r1), r0.max(r1).min(t.rows - 1));
    let (c0, c1) = (c0.min(c1), c0.max(c1).min(t.cols - 1));
    let zones = t.zones.get_or_insert_with(Vec::new);
    let found = zones
        .iter()
        .position(|z| (z.start_row, z.start_col, z.end_row, z.end_col) == (r0, c0, r1, c1));
    let base = found.map(|i| zones[i].border_fill).unwrap_or(0);
    let id = apply_border_fill(doc, base, p)?;
    let t = table_mut(&mut para, at)?;
    let zones = t.zones.get_or_insert_with(Vec::new);
    match found {
        Some(i) => zones[i].border_fill = id,
        None => zones.push(CellZone {
            start_col: c0,
            start_row: r0,
            end_col: c1,
            end_row: r1,
            border_fill: id,
        }),
    }
    Ok(replace(at, para))
}
/// An object's placement and drawing transform, kept to swap back in.
pub struct Transform {
    pub at: TableAddress,
    common: ObjectCommon,
    component: ShapeComponent,
    crop: Option<[i32; 4]>,
}
fn transform_parts(c: &mut Control) -> Result<(&mut ObjectCommon, &mut Shape)> {
    match &mut c.kind {
        ControlKind::Object(o) => parts_mut(o),
        _ => Err(bad("the object is not a drawing")),
    }
}
fn crop_mut(s: &mut Shape) -> Option<&mut [i32; 4]> {
    s.parts.iter_mut().find_map(|p| match p {
        ShapePart::Picture(p) => Some(&mut p.crop),
        _ => None,
    })
}
pub fn capture_transform(doc: &Document, at: &TableAddress) -> Result<Transform> {
    let mut c = control(doc, at)?.clone();
    let (common, shape) = transform_parts(&mut c)?;
    Ok(Transform {
        at: at.clone(),
        common: *common,
        crop: crop_mut(shape).map(|c| *c),
        component: shape.component.clone(),
    })
}
/// Puts a kept transform back on its object and keeps the one it had.
pub fn swap_transform(doc: &Document, t: &mut Transform) -> Result<Op> {
    let mut para = replacement(doc, &t.at)?;
    let (common, shape) = transform_parts(&mut para.controls[t.at.control as usize])?;
    std::mem::swap(common, &mut t.common);
    std::mem::swap(&mut shape.component, &mut t.component);
    if let (Some(now), Some(kept)) = (crop_mut(shape), t.crop.as_mut()) {
        std::mem::swap(now, kept);
    }
    Ok(replace(&t.at, para))
}
fn table_mut<'a>(p: &'a mut Paragraph, at: &TableAddress) -> Result<&'a mut Table> {
    match &mut p.controls[at.control as usize].kind {
        ControlKind::Table(t) => Ok(t),
        _ => Err(bad("the object is not a table")),
    }
}
pub fn set_table(
    doc: &mut Document,
    fonts: &hm_fonts::FontSystem,
    at: &TableAddress,
    p: TablePatch,
) -> Result<Op> {
    let mut para = replacement(doc, at)?;
    if let Some(b) = &p.border_fill {
        let id = table_mut(&mut para, at)?.border_fill;
        table_mut(&mut para, at)?.border_fill = apply_border_fill(doc, id, b)?;
    }
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
    doc: &mut Document,
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
    if let Some(v) = p.text_direction {
        if v > 2 {
            return Err(bad("text direction runs 0 to 2"));
        }
        bits(&mut c.list_attr, 0, 7, v);
    }
    if let Some(v) = p.editable_in_form {
        flag(&mut c.list_attr, 19, v);
    }
    for p in &mut c.paras {
        p.src = None;
        p.line_segs.clear();
    }
    if let Some(b) = &p.border_fill {
        let k = t
            .cells
            .iter()
            .position(|c| c.row == row && c.col == col)
            .unwrap();
        patch_cell_borders(doc, t, k, b)?;
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
    let mut out = json!({"kind":if c.as_table().is_some(){"table"}else{"object"},"width":o.width,"height":o.height,"horzOffset":o.x_offset,"vertOffset":o.y_offset,"treatAsChar":o.treat_as_char(),"vertRelTo":name(VREL,o.vert_rel_to()),"horzRelTo":name(HREL,(o.attr>>8)&3),"vertAlign":name(VALIGN,(o.attr>>5)&7),"horzAlign":name(HALIGN,(o.attr>>10)&7),"textWrap":name(WRAP,o.text_wrap()),"margins":o.margins,"zOrder":o.z_order,"restrictInPage":o.attr&(1<<13)!=0,"allowOverlap":o.attr&(1<<14)!=0,"keepWithAnchor":h.prevent_split.unwrap_or(0)!=0,"sizeProtect":c.as_table().is_none()&&o.attr&(1<<20)!=0,"description":h.description.as_ref().map(|s|s.to_string_lossy()).unwrap_or_default()});
    if let Some(s) = c.as_object().and_then(|o| o.shape()) {
        out["rotation"] = json!(s.component.rotation);
        out["size"] = json!([s.component.size.0, s.component.size.1]);
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
    let center = b.center_line().then(|| {
        name(
            &["none", "horizontal", "vertical", "cross"],
            (b.attr as u32 >> 8) & 3,
        )
    });
    let fill = b.fill.as_ref().map(|f| json!({"kind":f.kind,"solid":f.solid.map(|s|json!({"background":s.background,"patternColor":s.pattern_color,"pattern":s.pattern}))}));
    json!({"id":id,"left":line(b.sides[0]),"right":line(b.sides[1]),"top":line(b.sides[2]),"bottom":line(b.sides[3]),"diagonal":line(b.diagonal),"background":b.fill.as_ref().and_then(|f|f.background()),"slash":b.slash(),"backslash":b.backslash(),"centerLine":center.flatten().unwrap_or("none"),"fill":fill})
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
        json!({"row":c.row,"column":c.col,"rowSpan":c.row_span,"columnSpan":c.col_span,"width":c.width,"height":c.height,"padding":t.cell_padding(c),"ownPadding":c.own_padding(),"tablePadding":t.padding,"verticalAlign":name(&["top","center","bottom"],c.vertical_align()),"protected":c.protected(),"header":c.header(),"textDirection":c.list_attr&7,"editableInForm":c.list_attr&(1<<19)!=0,"borderFill":border_fill(doc,c.border_fill)}),
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
