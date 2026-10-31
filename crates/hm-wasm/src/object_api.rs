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
const LINE_WRAP: &[&str] = &["break", "squeeze", "keep"];
fn name<'a>(values: &[&'a str], n: u32) -> Option<&'a str> {
    values.get(n as usize).copied()
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BorderPatch {
    pub color: Option<u32>,
    pub width: Option<i32>,
    pub style: Option<u32>,
    pub end_cap: Option<u32>,
    pub head: Option<u32>,
    pub tail: Option<u32>,
    pub head_size: Option<u32>,
    pub tail_size: Option<u32>,
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
    /// Pictures: percent, -100 to 100.
    pub brightness: Option<i8>,
    pub contrast: Option<i8>,
    /// Pictures: 0 real picture, 1 grayscale, 2 black and white.
    pub effect: Option<u8>,
    /// Pictures: 0 opaque to 255 transparent.
    pub transparency: Option<u8>,
    /// Pictures: inner margins, left, right, top, bottom.
    pub picture_margins: Option<[i16; 4]>,
    /// Drawings: the face fill, and its alpha (0 opaque to 255 clear).
    pub fill: Option<FillPatch>,
    pub fill_alpha: Option<u8>,
    pub caption: Option<CaptionPatch>,
    pub text_margins: Option<[i16; 4]>,
    pub text_vertical_align: Option<String>,
    pub round: Option<u8>,
    pub shadow: Option<ShadowPatch>,
}
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShadowPatch {
    pub kind: Option<u32>,
    pub color: Option<u32>,
    pub x: Option<i32>,
    pub y: Option<i32>,
}
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptionPatch {
    pub enabled: Option<bool>,
    pub side: Option<String>,
    pub vertical_align: Option<String>,
    pub width: Option<i32>,
    pub gap: Option<i16>,
    pub max_width: Option<i32>,
    pub include_margin: Option<bool>,
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
    Gradient {
        gradient_type: Option<u8>,
        angle: Option<i32>,
        center_x: Option<i32>,
        center_y: Option<i32>,
        step: Option<i32>,
    },
    Image,
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
fn fill_patch(base: &Fill, patch: &FillPatch) -> Fill {
    let extra = base.extra.clone().or_else(|| Some(SharedBytes::empty()));
    let alpha = vec![base.alpha.first().copied().unwrap_or(0)];
    match *patch {
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
            alpha,
            ..Fill::default()
        },
        FillPatch::Gradient {
            gradient_type,
            angle,
            center_x,
            center_y,
            step,
        } => {
            let mut g = base.gradient.clone().unwrap_or(Gradient {
                kind: 1,
                colors: vec![0xffffff, 0],
                ..Gradient::default()
            });
            if let Some(v) = gradient_type {
                g.kind = v;
            }
            if let Some(v) = angle {
                g.angle = v;
            }
            if let Some(v) = center_x {
                g.center_x = v;
            }
            if let Some(v) = center_y {
                g.center_y = v;
            }
            if let Some(v) = step {
                g.step = v;
            }
            Fill {
                kind: Fill::GRADIENT,
                gradient: Some(g),
                extra,
                alpha,
                ..Fill::default()
            }
        }
        FillPatch::Image => Fill {
            kind: Fill::IMAGE,
            image: Some(base.image.unwrap_or_default()),
            extra,
            alpha,
            ..Fill::default()
        },
    }
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
    /// 개체 보호하기: the table cannot be selected as an object.
    pub lock: Option<bool>,
    /// 크기 고정.
    pub size_protect: Option<bool>,
    pub repeat_header: Option<bool>,
    pub split: Option<String>,
    pub spacing: Option<i16>,
    pub padding: Option<[i16; 4]>,
    pub border_fill: Option<BorderFillPatch>,
    pub border_fill_id: Option<u16>,
}
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CellPatch {
    pub field_name: Option<String>,
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
    /// 한 줄로 입력: `break` (wrap lines), `squeeze` (one line, narrowing
    /// the text) or `keep` (one line as it is).
    pub line_wrap: Option<String>,
    /// The cell's border fill; its changed sides carry over to the
    /// neighbors sharing them.
    pub border_fill: Option<BorderFillPatch>,
    pub border_fill_id: Option<u16>,
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
pub(crate) fn replacement(doc: &Document, at: &TableAddress) -> Result<Paragraph> {
    control(doc, at)?;
    Ok(hm_edit::ops::list(doc, &crate::tables::path(at)).unwrap()[at.para as usize].clone())
}
pub(crate) fn replace(at: &TableAddress, mut p: Paragraph) -> Op {
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
/// A new object's header: beside the paragraph and the column, absolute
/// size. An object in the text also flows with it and keeps the text above
/// and below it, as Hancom inserts pictures (objects case pic-insert), so
/// turning treat-as-char off leaves it on its own line.
fn header(width: i32, height: i32, inline: bool, x: i32, y: i32) -> ObjectHeader {
    let inline = if inline { 1 | (1 << 13) | (1 << 21) } else { 0 };
    ObjectHeader {
        instance_id: Some(0),
        prevent_split: Some(0),
        common: ObjectCommon {
            attr: inline | (2 << 3) | (2 << 8) | (4 << 15) | (2 << 18),
            width,
            height,
            x_offset: x,
            y_offset: y,
            ..Default::default()
        },
        ..Default::default()
    }
}
/// The z-order Hancom gives a new object: one above every object of the
/// section's text, so the newest is drawn on top (objects case pic-insert:
/// 0, 1, 2 for successive pictures).
pub(crate) fn next_z(doc: &Document, section: u32) -> i32 {
    doc.sections.get(section as usize).map_or(0, |s| {
        s.paras
            .iter()
            .flat_map(|p| &p.controls)
            .filter_map(|c| c.object_common())
            .map(|o| o.z_order.saturating_add(1))
            .max()
            .unwrap_or(0)
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
/// The body width of a section's pages, HU.
fn body_width(doc: &Document, section: u32) -> i32 {
    doc.sections
        .get(section as usize)
        .and_then(|s| s.definition())
        .and_then(|s| s.page_def())
        .map_or(42520, |p| p.body().2)
}

/// The width a new table fills, as Hancom sizes it: the column of the body
/// text at `at` (the first column of unequal ones), the inside of the cell
/// or text box holding it, a header's or footer's text width.
fn container_width(doc: &Document, at: &Address) -> i32 {
    let pos = at.pos();
    let section = &doc.sections[pos.list.section as usize];
    let Some((last, outer)) = pos.list.steps.split_last() else {
        let columns = section.paras[..=pos.para as usize]
            .iter()
            .flat_map(|p| &p.controls)
            .filter_map(|c| match &c.kind {
                ControlKind::Columns(def) => Some(def),
                _ => None,
            })
            .next_back();
        let width = body_width(doc, pos.list.section);
        return columns.map_or(width, |c| c.layout(width)[0].1);
    };
    let outer = hm_edit::ListPath {
        section: pos.list.section,
        steps: outer.to_vec(),
    };
    let host = hm_edit::ops::list(doc, &outer)
        .and_then(|l| l.get(last.para as usize))
        .and_then(|p| p.controls.get(last.control as usize));
    match host.map(|c| &c.kind) {
        Some(ControlKind::Table(t)) => match t.cells.get(last.index as usize) {
            Some(cell) => {
                let [l, r, _, _] = t.cell_padding(cell);
                cell.width - l - r
            }
            None => t.header.common.width,
        },
        Some(ControlKind::Object(o)) => {
            fn boxes<'a>(s: &'a Shape, out: &mut Vec<(&'a TextBox, i32)>) {
                if let Some(t) = s.text() {
                    out.push((t, s.component.size.0 as i32));
                }
                for c in s.children() {
                    boxes(c, out);
                }
            }
            let mut found = Vec::new();
            if let Some(s) = o.shape() {
                boxes(s, &mut found);
            }
            match found.get(last.index as usize) {
                Some((t, w)) => {
                    let [l, r, _, _] = t.margins.unwrap_or([0; 4]).map(i32::from);
                    t.max_width.filter(|&m| m > 0).unwrap_or(*w) - l - r
                }
                None => o.header.common.width,
            }
        }
        _ => body_width(doc, pos.list.section),
    }
}

/// The most cells a new table may have: Studio's 표 만들기 dialog allows
/// 256 rows by 256 columns (Hancom makes 110 by 100).
const MAX_CELLS: usize = 256 * 256;

pub fn create_table(
    doc: &mut Document,
    at: &Address,
    rows: u16,
    cols: u16,
    widths: Option<Vec<i32>>,
    heights: Option<Vec<i32>>,
    inline: bool,
) -> Result<Control> {
    if rows == 0 || cols == 0 || (rows as usize) * (cols as usize) > MAX_CELLS {
        return Err(bad(format!("a table needs 1 to {MAX_CELLS} cells")));
    }
    let base = paragraph(doc, at)?;
    // Hancom's new table: 1 mm outside margins, cells 1.8 mm by 0.5 mm
    // inside, one line of text high, the column, cell or text box it goes
    // in filled between the outside margins.
    const OUTSIDE: i32 = 283;
    let fill = container_width(doc, at);
    // A cell is never narrower than its side margins (Hancom's 100 columns
    // in one page take 1020 each).
    let widths = widths.unwrap_or_else(|| {
        vec![((fill - 2 * OUTSIDE) / i32::from(cols)).max(510 + 510); cols as usize]
    });
    // Rows store only the cell margins and grow to their text, in the text
    // or on their own line; they draw one line high.
    let line = 1282;
    let stored = 282;
    let explicit = heights.is_some();
    let heights = heights.unwrap_or_else(|| vec![stored; rows as usize]);
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
    let drawn = if explicit {
        sum(&heights)?
    } else {
        line * i32::from(rows)
    };
    let mut h = header(sum(&widths)?, drawn, inline, 0, 0);
    // Hancom's new table, in the text or not: placed from the paragraph
    // and the column, kept inside the page, above and below the text, and
    // numbered as a table.
    h.common.attr = (inline as u32)
        | (2 << 3)
        | (2 << 8)
        | (1 << 13)
        | (4 << 15)
        | (2 << 18)
        | (1 << 21)
        | (2 << 26);
    h.common.margins = [OUTSIDE as i16; 4];
    // Hancom's table border: 0.12 mm solid black on every side, no fill,
    // shared with an equal existing fill.
    let border = BorderLine {
        kind: 1,
        width: 1,
        color: 0,
    };
    let bf = intern(
        doc,
        BorderFill {
            sides: [border; 4],
            diagonal: BorderLine {
                kind: 1,
                width: 0,
                color: 0,
            },
            ..Default::default()
        },
    )?;
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
                // Centred vertically.
                list_attr: 1 << 5,
                padding: [510, 510, 141, 141],
                paras: vec![empty_para(&base)],
                ..Default::default()
            });
        }
    }
    let table = Table {
        header: h,
        rows,
        cols,
        // Split at page boundaries between rows, its title rows repeated
        // on each page, as Hancom starts it.
        attr: 2 | 4,
        padding: [510, 510, 141, 141],
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
pub(crate) fn component(kind: CtrlId, w: i32, h: i32, style: Option<ShapeStyle>) -> ShapeComponent {
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
pub(crate) fn shape_control(header: ObjectHeader, shape: Shape) -> Control {
    Control {
        kind: ControlKind::Object(Box::new(ObjectControl {
            id: CtrlId::SHAPE,
            header,
            parts: vec![ObjectPart::Shape(Box::new(shape))],
        })),
        src: None,
    }
}
const MAX_POLYGON_POINTS: usize = 4096;
/// Hancom's outline for a new drawing (objects cases shape-defaults c1/c2,
/// textbox c1): 0.12 mm solid black with flat ends and filled medium
/// arrowheads.
const NEW_LINE: LineStyle = LineStyle {
    color: 0,
    width: 33,
    attr: 1 | (1 << 6) | (4 << 22) | (4 << 26) | (1 << 30) | (1 << 31),
    outline: Some(0),
};
/// Hancom's shadow for a new drawing: none yet, in its grey.
const NEW_SHADOW: Shadow = Shadow {
    kind: 0,
    color: 0xB2B2B2,
    offset: (0, 0),
};
/// Inner margins of the text in a new drawing, 1 mm on every side; the text
/// is centred vertically (textbox c1, verify h06).
const NEW_TEXT_MARGIN: i16 = 283;

#[allow(clippy::too_many_arguments)]
pub fn create_shape(
    doc: &Document,
    at: &Address,
    kind: &str,
    w: i32,
    h: i32,
    points: Option<Vec<(i32, i32)>>,
    text: Option<String>,
    inline: bool,
    x: i32,
    y: i32,
) -> Result<Control> {
    if points.is_some() && kind != "polygon" {
        return Err(bad("only polygons take points"));
    }
    if kind == "line" {
        // A horizontal or vertical line has no extent across it, as
        // Hancom stores one (verify h05).
        if w < 0 || h < 0 || (w == 0 && h == 0) {
            return Err(bad("a line needs a non-negative extent, not both zero"));
        }
    } else {
        positive(w)?;
        positive(h)?;
    }
    let mut header = header(w, h, inline, x, y);
    let instance = crate::connectors::next_id(doc);
    header.instance_id = Some(instance);
    header.common.z_order = next_z(doc, at.section);
    let base = paragraph(doc, at)?;
    // The description Hancom gives each kind.
    let (id, part, comment) = match kind {
        "rect" | "textbox" => (
            CtrlId::from_chars(b"$rec"),
            ShapePart::Rect(RectGeom {
                corners: corners(w, h),
                ..Default::default()
            }),
            "사각형입니다.",
        ),
        "ellipse" => (
            CtrlId::from_chars(b"$ell"),
            ShapePart::Ellipse(EllipseGeom {
                center: (w / 2, h / 2),
                axis1: (w, h / 2),
                axis2: (w / 2, 0),
                ..Default::default()
            }),
            "타원입니다.",
        ),
        "line" => (
            CtrlId::from_chars(b"$lin"),
            ShapePart::Line(LineGeom {
                end: (w, h),
                attr: Some(0),
                ..Default::default()
            }),
            "선입니다.",
        ),
        "arc" => (
            CtrlId::from_chars(b"$arc"),
            // A quarter of the ellipse filling the box, from its right to
            // its top.
            ShapePart::Arc(ArcGeom {
                center: (w / 2, h / 2),
                axis1: (w, h / 2),
                axis2: (w / 2, 0),
                ..Default::default()
            }),
            "호입니다.",
        ),
        "polygon" => {
            // Without vertices, a triangle standing in the box.
            let points = points.unwrap_or_else(|| vec![(w / 2, 0), (w, h), (0, h)]);
            if !(2..=MAX_POLYGON_POINTS).contains(&points.len()) {
                return Err(bad(format!(
                    "a polygon takes 2 to {MAX_POLYGON_POINTS} points"
                )));
            }
            (
                CtrlId::from_chars(b"$pol"),
                ShapePart::Polygon(PolyGeom {
                    points,
                    ..Default::default()
                }),
                "다각형입니다.",
            )
        }
        _ => {
            return Err(bad(
                "shape kind must be rect, ellipse, line, textbox, polygon or arc",
            ));
        }
    };
    if kind == "line" && text.is_some() {
        return Err(bad("lines do not hold text"));
    }
    // Hancom puts a drawing in front of the text it may overlap, numbers
    // drawings as pictures and fills closed ones white; a line has no
    // caption number or fill.
    if !inline {
        header.common.attr |= 3 << 21;
    }
    header.common.attr |= 1 << 14;
    if kind != "line" {
        header.common.attr |= 1 << 26;
    }
    header.description = Some(HwpString::from_text(comment));
    let fill = if kind == "line" {
        Fill::default()
    } else {
        fill_patch(
            &Fill::default(),
            &FillPatch::Solid {
                background: 0xFFFFFF,
                pattern_color: 0,
                pattern: -1,
            },
        )
    };
    let style = ShapeStyle {
        instance_id: Some(instance),
        line: NEW_LINE,
        fill,
        shadow: Some(NEW_SHADOW),
        shadow_alpha: Some(0),
        ..Default::default()
    };
    let mut parts = vec![part];
    if kind == "textbox" || text.is_some() {
        let mut part = text_part(&base, w);
        if let Some(text) = text {
            let paras = std::mem::take(&mut part.paras);
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
            part.paras = temp.sections.remove(0).paras;
            part.count = part.paras.len() as u32;
        }
        parts.push(ShapePart::Text(part));
    }
    Ok(shape_control(
        header,
        Shape {
            component: component(id, w, h, Some(style)),
            parts,
        },
    ))
}
/// The text part Hancom gives a drawing: one empty paragraph in the
/// anchor paragraph's shapes, inside the 1 mm margins and centred.
fn text_part(base: &Paragraph, width: i32) -> TextBox {
    TextBox {
        count: 1,
        // Centred vertically.
        attr: 1 << 5,
        margins: Some([NEW_TEXT_MARGIN; 4]),
        max_width: Some(width),
        paras: vec![empty_para(base)],
        ..Default::default()
    }
}
/// Gives a closed drawing without text an empty text part, as Hancom does
/// when Enter or typing goes into a selected rectangle (objects verify
/// h06): the paragraph to type into, at the start of the drawing's first
/// list. A drawing that has text keeps it and nothing changes; lines,
/// pictures, groups and other objects take no text.
pub fn ensure_text(doc: &Document, at: &TableAddress) -> Result<(Option<Op>, Pos)> {
    let mut para = replacement(doc, at)?;
    let list = crate::tables::path(at);
    let first = |list: hm_edit::ListPath| {
        let mut list = list;
        list.steps.push(hm_edit::Step {
            para: at.para,
            control: at.control,
            index: 0,
        });
        Pos {
            list,
            para: 0,
            offset: 0,
        }
    };
    let c = &mut para.controls[at.control as usize];
    let frame = c.object_common().map_or(0, |o| o.width);
    let shape = shape_mut(c)?;
    // A turned drawing's box is its frame; the text runs across the
    // drawing itself.
    let width = match shape.component.size.0 {
        0 => frame,
        w => i32::try_from(w).unwrap_or(frame),
    };
    if shape.text().is_some() {
        return Ok((None, first(list)));
    }
    if !matches!(
        shape.kind(),
        Some(
            ShapePart::Rect(_)
                | ShapePart::Ellipse(_)
                | ShapePart::Polygon(_)
                | ShapePart::Arc(_)
                | ShapePart::Curve(_)
        )
    ) {
        return Err(bad("only a closed drawing holds text"));
    }
    let base = paragraph(
        doc,
        &Address::of(&Pos {
            list: list.clone(),
            para: at.para,
            offset: 0,
        }),
    )?;
    shape.parts.push(ShapePart::Text(text_part(&base, width)));
    Ok((Some(replace(at, para)), first(list)))
}
#[allow(clippy::too_many_arguments)]
pub fn create_picture(
    doc: &mut Document,
    section: u32,
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
    positive(w)?;
    positive(h)?;
    let mut hder = header(w, h, inline, x, y);
    // Numbered as a picture and drawn above the section's older objects,
    // as Hancom inserts one.
    hder.common.attr |= 1 << 26;
    hder.common.z_order = next_z(doc, section);
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
/// Gives an object or table control the z-order `z`.
pub(crate) fn set_z(c: &mut Control, z: i32) {
    if let Ok(h) = header_mut(c) {
        h.common.z_order = z;
    }
}
/// Moves an object or table control's stored offsets.
pub(crate) fn shift(c: &mut Control, dx: i32, dy: i32) {
    if let Ok(h) = header_mut(c) {
        h.common.x_offset = h.common.x_offset.saturating_add(dx);
        h.common.y_offset = h.common.y_offset.saturating_add(dy);
    }
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

/// Replace a picture's resource while retaining its frame, styling and identity.
pub fn assign_picture_image(
    doc: &mut Document,
    at: &TableAddress,
    bytes: Vec<u8>,
    extension: String,
    natural_width: i32,
    natural_height: i32,
) -> Result<Op> {
    let mut para = replacement(doc, at)?;
    let control = &mut para.controls[at.control as usize];
    let common = control.object_common().unwrap().clone();
    let shape = shape_mut(control)?;
    let target = shape
        .parts
        .iter_mut()
        .find_map(|part| match part {
            ShapePart::Picture(picture) => Some(picture),
            _ => None,
        })
        .ok_or_else(|| bad("the object is not a picture"))?;
    let fresh = create_picture(
        doc,
        at.section,
        bytes,
        extension,
        None,
        common.width,
        common.height,
        Some(natural_width),
        Some(natural_height),
        false,
        0,
        0,
    )?;
    let image = picture(fresh.as_object().unwrap().shape().unwrap()).unwrap();
    target.image.bin_id = image.image.bin_id;
    target.crop = image.crop;
    let mut tail = target.tail.to_vec();
    if tail.len() >= 13 {
        let at = tail.len() - 9;
        tail[at..at + 4].copy_from_slice(&natural_width.to_le_bytes());
        tail[at + 4..at + 8].copy_from_slice(&natural_height.to_le_bytes());
        target.tail = SharedBytes::from(tail);
    } else {
        target.tail = image.tail.clone();
    }
    Ok(replace(at, para))
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
pub(crate) fn local_size(c: &ShapeComponent) -> (f64, f64) {
    let (w, h) = if c.initial_size.0 > 0 && c.initial_size.1 > 0 {
        c.initial_size
    } else {
        c.size
    };
    (w as f64, h as f64)
}
/// Turns a drawing to `angle` degrees and mirrors it, both about the
/// center of its scaled box. With `framed` (every drawing but a group, as
/// Hancom saves them: objects case shape-transform c2, a 5500x6050
/// rectangle turned 90 degrees, is 6050x5500 with its drawing shifted by
/// 275, -275) the object's box becomes the turned drawing's bounds, the
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
        // Hancom stores the shift as the drawing's offset too.
        c.offset = (
            c.translation.0[2].round() as i32,
            c.translation.0[5].round() as i32,
        );
        o.x_offset += dx as i32;
        o.y_offset += dy as i32;
        o.width = ((x1 - x0).round() as i32).max(1);
        o.height = ((y1 - y0).round() as i32).max(1);
        c.rotation_center = (o.width / 2, o.height / 2);
    } else {
        c.rotation_center = (cx.round() as i32, cy.round() as i32);
    }
}
/// Gives a turned drawing a new frame: the drawing size whose turned
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
    if let Some(cap) = &p.caption {
        let captions = match &mut c.kind {
            ControlKind::Object(o) => {
                if cap.enabled == Some(false) {
                    o.parts
                        .retain(|part| !matches!(part, ObjectPart::Caption(_)));
                    o.header.common.attr &= !(1 << 29);
                    None
                } else {
                    o.parts.iter_mut().find_map(|part| {
                        if let ObjectPart::Caption(c) = part {
                            Some(c)
                        } else {
                            None
                        }
                    })
                }
            }
            ControlKind::Table(t) => {
                if cap.enabled == Some(false) {
                    t.caption = None;
                    t.header.common.attr &= !(1 << 29);
                }
                t.caption.as_mut()
            }
            _ => None,
        };
        if let Some(c) = captions {
            if let Some(side) = &cap.side {
                bits(
                    c.attr.get_or_insert(3),
                    0,
                    3,
                    enum_code(side, &["left", "right", "top", "bottom"])?,
                );
            }
            if let Some(align) = &cap.vertical_align {
                bits(
                    &mut c.list_attr,
                    5,
                    3,
                    enum_code(align, &["top", "center", "bottom"])?,
                );
            }
            if let Some(v) = cap.include_margin {
                flag(c.attr.get_or_insert(3), 2, v);
            }
            if let Some(v) = cap.width {
                c.width = Some(v);
            }
            if let Some(v) = cap.gap {
                c.gap = Some(v);
            }
            if let Some(v) = cap.max_width {
                c.max_width = Some(v);
            }
        } else if cap.enabled != Some(false) {
            return Err(bad("the object has no caption"));
        }
    }
    let old = c.object_common().unwrap();
    // A group's members place themselves against its box; it turns
    // without a frame.
    let framed = c
        .as_object()
        .and_then(|o| o.shape())
        .is_some_and(|s| s.children().next().is_none());
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
            // A turned drawing's box is its frame: size the drawing so its
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
    if let (ControlKind::Table(t), Some(v)) = (&mut c.kind, p.size_protect) {
        protect_size(doc, fonts, at.section, t, v);
    }
    let is_table = matches!(c.kind, ControlKind::Table(_));
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
    if let Some(v) = p.size_protect.filter(|_| !is_table) {
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
        for (value, shift, mask) in [
            (v.end_cap, 6, 15),
            (v.head, 10, 63),
            (v.tail, 16, 63),
            (v.head_size, 22, 15),
            (v.tail_size, 26, 15),
        ] {
            if let Some(value) = value {
                if value > mask {
                    return Err(bad("invalid line end property"));
                }
                bits(&mut l.attr, shift, mask, value);
            }
        }
        if let Some(v) = v.style {
            if v > 17 {
                return Err(bad("unsupported border style"));
            }
            bits(&mut l.attr, 0, 63, v);
        }
    }
    if p.text_margins.is_some() || p.text_vertical_align.is_some() {
        let text = shape_mut(c)?
            .parts
            .iter_mut()
            .find_map(|p| {
                if let ShapePart::Text(t) = p {
                    Some(t)
                } else {
                    None
                }
            })
            .ok_or_else(|| bad("this drawing has no text box"))?;
        if let Some(v) = p.text_margins {
            text.margins = Some(v);
        }
        if let Some(v) = &p.text_vertical_align {
            bits(
                &mut text.attr,
                5,
                3,
                enum_code(v, &["top", "center", "bottom"])?,
            );
        }
    }
    if let Some(v) = p.round {
        if v > 50 {
            return Err(bad("rectangle rounding runs from 0 to 50"));
        }
        let rect = shape_mut(c)?
            .parts
            .iter_mut()
            .find_map(|p| {
                if let ShapePart::Rect(r) = p {
                    Some(r)
                } else {
                    None
                }
            })
            .ok_or_else(|| bad("this drawing is not a rectangle"))?;
        rect.round = v;
    }
    if let Some(v) = p.shadow {
        let shadow = shape_mut(c)?
            .component
            .style
            .as_mut()
            .ok_or_else(|| bad("this drawing takes no shadow"))?
            .shadow
            .get_or_insert_with(Shadow::default);
        if let Some(v) = v.kind {
            shadow.kind = v;
        }
        if let Some(v) = v.color {
            shadow.color = v;
        }
        if let Some(v) = v.x {
            shadow.offset.0 = v;
        }
        if let Some(v) = v.y {
            shadow.offset.1 = v;
        }
    }
    if p.brightness.is_some()
        || p.contrast.is_some()
        || p.effect.is_some()
        || p.transparency.is_some()
        || p.picture_margins.is_some()
    {
        let pic = shape_mut(c)?
            .parts
            .iter_mut()
            .find_map(|p| match p {
                ShapePart::Picture(p) => Some(p),
                _ => None,
            })
            .ok_or_else(|| bad("image effects are only supported for pictures"))?;
        for v in [p.brightness, p.contrast].into_iter().flatten() {
            if !(-100..=100).contains(&v) {
                return Err(bad("brightness and contrast run -100 to 100"));
            }
        }
        if let Some(v) = p.brightness {
            pic.image.brightness = v;
        }
        if let Some(v) = p.contrast {
            pic.image.contrast = v;
        }
        if let Some(v) = p.effect {
            if !matches!(v, 0 | 1 | 2 | 4) {
                return Err(bad("unsupported picture effect"));
            }
            pic.image.effect = v;
        }
        if let Some(v) = p.picture_margins {
            if v.iter().any(|m| *m < 0) {
                return Err(bad("picture margins cannot be negative"));
            }
            pic.margins = v;
        }
        if let Some(v) = p.transparency {
            if pic.tail.len() < 13 {
                return Err(bad("this picture keeps no transparency"));
            }
            let mut tail = pic.tail.to_vec();
            *tail.last_mut().unwrap() = v;
            pic.tail = SharedBytes::from(tail);
        }
    }
    if p.fill.is_some() || p.fill_alpha.is_some() {
        let style = shape_mut(c)?
            .component
            .style
            .as_mut()
            .ok_or_else(|| bad("this drawing takes no fill"))?;
        if let Some(f) = &p.fill {
            style.fill = fill_patch(&style.fill, f);
        }
        if let Some(v) = p.fill_alpha {
            if style.fill.kind == 0 {
                return Err(bad("an empty fill has no alpha"));
            }
            let parts = (style.fill.kind & 7).count_ones() as usize;
            style.fill.alpha = vec![v; parts];
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
        b.fill = Some(fill_patch(b.fill.as_ref().unwrap_or(&Fill::default()), f));
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
pub(crate) fn apply_border_fill(doc: &mut Document, id: u16, p: &BorderFillPatch) -> Result<u16> {
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
/// 크기 고정: a size-protected table keeps its stored heights, so Hancom
/// stores the height each cell's text needs when protecting it (and marks
/// every cell in HWP 5 too); the rows draw as before.
fn protect_size(
    doc: &Document,
    fonts: &hm_fonts::FontSystem,
    section: u32,
    t: &mut Table,
    on: bool,
) {
    if on && t.header.common.attr & (1 << 20) == 0 {
        let laid = crate::objects::laid_out(doc, fonts, section, t);
        for (c, l) in t.cells.iter_mut().zip(&laid.cells) {
            if !l.vertical {
                c.height = c.height.max(l.content + l.padding[2] + l.padding[3]);
            }
        }
    }
    flag(&mut t.header.common.attr, 20, on);
    for c in &mut t.cells {
        flag(&mut c.list_attr, 24, on);
    }
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
    if let Some(id) = p.border_fill_id {
        if id as usize > doc.info.border_fills.len() {
            return Err(bad("no such border fill"));
        }
        table_mut(&mut para, at)?.border_fill = id;
    }
    if let Some(b) = &p.border_fill {
        let id = table_mut(&mut para, at)?.border_fill;
        table_mut(&mut para, at)?.border_fill = apply_border_fill(doc, id, b)?;
    }
    let t = table_mut(&mut para, at)?;
    if let Some(v) = p.lock {
        flag(&mut t.header.common.attr, 30, v);
    }
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
    // The heights a protected table keeps come from the text as the rest
    // of the patch leaves it (its padding too).
    if let Some(v) = p.size_protect {
        protect_size(doc, fonts, at.section, t, v);
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
    if let Some(v) = p.line_wrap {
        bits(&mut c.list_attr, 3, 3, enum_code(&v, LINE_WRAP)?);
    }
    if let Some(name) = p.field_name {
        c.field_name = (!name.is_empty()).then(|| HwpString::from_text(&name));
    }
    if let Some(id) = p.border_fill_id {
        if id as usize > doc.info.border_fills.len() {
            return Err(bad("no such border fill"));
        }
        c.border_fill = id;
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
    json!({"color":l.color,"width":l.width,"style":l.kind(),"attr":l.attr,"outline":l.outline,"endCap":(l.attr>>6)&15,"head":(l.attr>>10)&63,"tail":(l.attr>>16)&63,"headSize":(l.attr>>22)&15,"tailSize":(l.attr>>26)&15})
}
pub fn object_properties(doc: &Document, at: &TableAddress) -> Result<Value> {
    let c = control(doc, at)?;
    let o = c.object_common().unwrap();
    let h = match &c.kind {
        ControlKind::Table(t) => &t.header,
        ControlKind::Object(o) => &o.header,
        _ => return Err(bad("unsupported object kind")),
    };
    let instance_id = c
        .as_object()
        .and_then(|o| o.shape())
        .and_then(|s| match s.kind() {
            Some(ShapePart::Ole(ole)) => ole.instance_id,
            _ => s
                .component
                .style
                .as_ref()
                .and_then(|s| s.instance_id)
                .filter(|id| *id != 0),
        })
        .or(h.instance_id)
        .unwrap_or(0);
    let mut out = json!({"kind":if c.as_table().is_some(){"table"}else{"object"},"width":o.width,"height":o.height,"horzOffset":o.x_offset,"vertOffset":o.y_offset,"treatAsChar":o.treat_as_char(),"vertRelTo":name(VREL,o.vert_rel_to()),"horzRelTo":name(HREL,(o.attr>>8)&3),"vertAlign":name(VALIGN,(o.attr>>5)&7),"horzAlign":name(HALIGN,(o.attr>>10)&7),"textWrap":name(WRAP,o.text_wrap()),"margins":o.margins,"zOrder":o.z_order,"restrictInPage":o.attr&(1<<13)!=0,"allowOverlap":o.attr&(1<<14)!=0,"keepWithAnchor":h.prevent_split.unwrap_or(0)!=0,"sizeProtect":o.attr&(1<<20)!=0,"objectId":h.instance_id.unwrap_or(0),"instanceId":instance_id,"description":h.description.as_ref().map(|s|s.to_string_lossy()).unwrap_or_default()});
    if let Some(o) = c.as_object() {
        // The caption list follows the drawing's own text lists.
        let lists = hm_edit::ops::control_lists(c).len();
        out["caption"] = json!(o.caption().map(|c| json!({"side":name(&["left","right","top","bottom"],c.side()),"verticalAlign":name(&["top","center","bottom"],(c.list_attr>>5)&3),"includeMargin":c.attr.unwrap_or(0)&4!=0,"width":c.width.unwrap_or(0),"gap":c.gap.unwrap_or(0),"maxWidth":c.max_width.unwrap_or(0),"paragraphs":c.paras.len(),"list":lists.saturating_sub(1)})));
    }
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
            out["textMargins"] = json!(t.margins.unwrap_or([0; 4]));
            out["textVerticalAlign"] = json!(name(&["top", "center", "bottom"], (t.attr >> 5) & 3));
            out["text"] = json!(
                t.paras
                    .iter()
                    .map(|p| p.plain_text())
                    .collect::<Vec<_>>()
                    .join("\n")
            );
        }
        if let Some(ShapePart::Rect(r)) = s.kind() {
            out["round"] = json!(r.round);
        }
        if let Some(st) = s.style() {
            out["shadow"] = json!(
                st.shadow
                    .map(|s| json!({"kind":s.kind,"color":s.color,"x":s.offset.0,"y":s.offset.1}))
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
    let caption = t.caption.as_ref().map(|c| {
        json!({"side":name(&["left","right","top","bottom"],c.side()),"verticalAlign":name(&["top","center","bottom"],(c.list_attr>>5)&3),"includeMargin":c.attr.unwrap_or(0)&4!=0,"width":c.width.unwrap_or(0),"gap":c.gap.unwrap_or(0),"maxWidth":c.max_width.unwrap_or(0),"paragraphs":c.paras.len()})
    });
    let zones: Vec<Value> = t.zones.iter().flatten().map(|z| json!({"startRow":z.start_row,"startColumn":z.start_col,"endRow":z.end_row,"endColumn":z.end_col,"borderFill":border_fill(doc,z.border_fill)})).collect();
    Ok(
        json!({"rows":t.rows,"columns":t.cols,"lock":t.header.common.attr&(1<<30)!=0,"sizeProtect":t.header.common.attr&(1<<20)!=0,"repeatHeader":t.repeat_header(),"split":name(&["none","row","cell"],t.split()),"spacing":t.spacing,"padding":t.padding,"borderFill":border_fill(doc,t.border_fill),"caption":caption,"zones":zones}),
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
        json!({"row":c.row,"column":c.col,"rowSpan":c.row_span,"columnSpan":c.col_span,"width":c.width,"height":c.height,"padding":t.cell_padding(c),"ownPadding":c.own_padding(),"tablePadding":t.padding,"verticalAlign":name(&["top","center","bottom"],c.vertical_align()),"protected":c.protected(),"header":c.header(),"textDirection":c.list_attr&7,"editableInForm":c.list_attr&(1<<19)!=0,"lineWrap":name(LINE_WRAP,(c.list_attr>>3)&3),"fieldName":c.field_name.as_ref().map(|n|n.to_string_lossy()).unwrap_or_default(),"borderFill":border_fill(doc,c.border_fill)}),
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
    Ok(
        json!({"bytes":&*bytes,"extension":ext,"mime":mime,"naturalWidth":w,"naturalHeight":h,"binDataId":p.image.bin_id}),
    )
}
