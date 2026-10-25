//! Objects and tables as a host selects and edits them: every box a page
//! draws, and the geometry edits (moving and resizing an object, sizing
//! table cells) as paragraph replacements in the native history.

use std::collections::HashMap;

use hm_edit::{ListPath, Op, Step};
use hm_model::{
    Control, ControlKind, CtrlId, Document, Matrix, ObjectCommon, Paragraph, Shape, ShapePart,
};
use serde::{Deserialize, Serialize};

use crate::types::{EngineError, Result, StepJs, TableAddress};

/// The smallest object side an edit leaves, about a millimetre.
const MIN_OBJECT: i32 = 283;
/// The smallest cell side an edit leaves.
const MIN_CELL: i32 = 200;

/// An object or table drawn on a page.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageObject {
    pub section: u32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<StepJs>,
    pub para: u32,
    pub control: u32,
    pub kind: &'static str,
    pub page: u32,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    /// Set as a character in its line.
    pub inline: bool,
    pub layer: &'static str,
    pub z: i32,
    /// In a header, footer, note or master page.
    pub furniture: bool,
    /// The size may not change (`크기 보호`).
    pub locked: bool,
    /// Holds a text box.
    pub text: bool,
    /// Degrees, clockwise.
    #[serde(skip_serializing_if = "is_zero")]
    pub rotation: i32,
    /// A line's start and end on the page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<[i32; 4]>,
}

fn is_zero(v: &i32) -> bool {
    *v == 0
}

/// Where each control (each table, by its table) sits in the document.
fn addresses(doc: &Document) -> HashMap<usize, (ListPath, u32, u32, bool)> {
    fn walk(
        paras: &[Paragraph],
        list: &ListPath,
        furniture: bool,
        out: &mut HashMap<usize, (ListPath, u32, u32, bool)>,
    ) {
        for (pi, p) in paras.iter().enumerate() {
            for (ci, c) in p.controls.iter().enumerate() {
                let key = match c.as_table() {
                    Some(t) => t as *const hm_model::Table as usize,
                    None => c as *const Control as usize,
                };
                out.insert(key, (list.clone(), pi as u32, ci as u32, furniture));
                let inner = furniture
                    || matches!(
                        c.kind,
                        ControlKind::HeaderFooter(_)
                            | ControlKind::Note(_)
                            | ControlKind::Section(_)
                    );
                for (k, l) in hm_edit::ops::control_lists(c).into_iter().enumerate() {
                    let mut path = list.clone();
                    path.steps.push(Step {
                        para: pi as u32,
                        control: ci as u32,
                        index: k as u32,
                    });
                    walk(l, &path, inner, out);
                }
            }
        }
    }
    let mut out = HashMap::new();
    for (si, s) in doc.sections.iter().enumerate() {
        let list = ListPath {
            section: si as u32,
            steps: Vec::new(),
        };
        walk(&s.paras, &list, false, &mut out);
    }
    out
}

/// The paragraph list of every table cell, by the cell's model address.
pub(super) fn cell_lists(doc: &Document) -> HashMap<usize, ListPath> {
    fn walk(paras: &[Paragraph], list: &ListPath, out: &mut HashMap<usize, ListPath>) {
        for (pi, p) in paras.iter().enumerate() {
            for (ci, c) in p.controls.iter().enumerate() {
                for (k, l) in hm_edit::ops::control_lists(c).into_iter().enumerate() {
                    let mut path = list.clone();
                    path.steps.push(Step {
                        para: pi as u32,
                        control: ci as u32,
                        index: k as u32,
                    });
                    if let Some(cell) = c.as_table().and_then(|t| t.cells.get(k)) {
                        out.insert(cell as *const hm_model::Cell as usize, path.clone());
                    }
                    walk(l, &path, out);
                }
            }
        }
    }
    let mut out = HashMap::new();
    for (si, s) in doc.sections.iter().enumerate() {
        let list = ListPath {
            section: si as u32,
            steps: Vec::new(),
        };
        walk(&s.paras, &list, &mut out);
    }
    out
}

/// What a host calls an object control.
fn kind(c: &Control) -> &'static str {
    match &c.kind {
        ControlKind::Table(_) => "table",
        ControlKind::Form(_) => "form",
        ControlKind::Object(o) if o.equation().is_some() => "equation",
        ControlKind::Object(o) => match o.shape() {
            Some(s) if s.children().next().is_some() => "group",
            Some(s) => match s.kind() {
                Some(ShapePart::Picture(_)) => "picture",
                Some(ShapePart::Ole(_)) => "ole",
                Some(ShapePart::Line(_)) if s.component.kind == CtrlId::from_chars(b"$col") => {
                    "connector"
                }
                Some(ShapePart::Line(_)) => "line",
                _ => "shape",
            },
            None => "object",
        },
        _ => "object",
    }
}

/// The boxes of the objects and tables page `index` draws, bottom to top.
pub fn page_objects(
    doc: &Document,
    layout: &hm_layout::DocLayout,
    fonts: &hm_fonts::FontSystem,
    index: usize,
) -> Vec<PageObject> {
    let boxes = hm_render::page_objects(doc, layout, fonts, index);
    if boxes.is_empty() {
        return Vec::new();
    }
    let at = addresses(doc);
    let mut out: Vec<PageObject> = Vec::new();
    for b in boxes {
        let Some((list, para, control, furniture)) = at.get(&b.key) else {
            continue;
        };
        let Some(c) = hm_edit::ops::list(doc, list)
            .and_then(|l| l.get(*para as usize))
            .and_then(|p| p.controls.get(*control as usize))
        else {
            continue;
        };
        let Some(o) = c.object_common() else { continue };
        let steps: Vec<StepJs> = list
            .steps
            .iter()
            .map(|s| StepJs {
                para: s.para,
                control: s.control,
                index: s.index,
            })
            .collect();
        // A table split over several draws on one page (a repeated header
        // and the band) is one box.
        if let Some(prev) = out.iter_mut().find(|p| {
            p.kind == "table"
                && p.section == list.section
                && p.steps == steps
                && p.para == *para
                && p.control == *control
        }) {
            let (x0, y0) = (prev.x.min(b.x), prev.y.min(b.y));
            let x1 = (prev.x + prev.width).max(b.x + b.width);
            let y1 = (prev.y + prev.height).max(b.y + b.height);
            (prev.x, prev.y, prev.width, prev.height) = (x0, y0, x1 - x0, y1 - y0);
            continue;
        }
        let inline = o.treat_as_char();
        let layer = if inline {
            "text"
        } else {
            match hm_layout::objects::layer_of(&o) {
                hm_layout::objects::Layer::Behind => "behind",
                hm_layout::objects::Layer::Text => "text",
                hm_layout::objects::Layer::Front => "front",
            }
        };
        let shape = c.as_object().and_then(|o| o.shape());
        let line = shape.and_then(|s| match s.kind() {
            Some(ShapePart::Line(l)) => {
                let m =
                    Matrix([1.0, 0.0, b.x as f64, 0.0, 1.0, b.y as f64]).mul(&s.component.matrix());
                let (x1, y1) = m.apply(l.start.0 as f64, l.start.1 as f64);
                let (x2, y2) = m.apply(l.end.0 as f64, l.end.1 as f64);
                Some([x1, y1, x2, y2].map(|v| v.round() as i32))
            }
            _ => None,
        });
        out.push(PageObject {
            section: list.section,
            steps,
            para: *para,
            control: *control,
            kind: kind(c),
            page: index as u32,
            x: b.x,
            y: b.y,
            width: b.width,
            height: b.height,
            inline,
            layer,
            z: o.z_order,
            furniture: *furniture,
            locked: o.attr & (1 << 20) != 0 && !matches!(c.kind, ControlKind::Table(_)),
            text: shape.is_some_and(has_text),
            rotation: shape.map_or(0, |s| s.component.rotation as i32),
            line,
        });
    }
    out
}

fn has_text(s: &Shape) -> bool {
    s.text().is_some() || s.children().any(has_text)
}

/// One cell's new size: deltas from its size as laid out.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CellSize {
    pub index: u32,
    #[serde(default)]
    pub width_delta: i32,
    #[serde(default)]
    pub height_delta: i32,
}

/// The geometry edits.
pub enum Action {
    /// Moves a floating object by a page distance.
    Move { dx: i32, dy: i32 },
    /// Gives an object a new size, its top left kept.
    Resize { width: i32, height: i32 },
    /// Resizes cells of a table from their laid-out sizes.
    Cells(Vec<CellSize>),
}

fn bad(message: &str) -> EngineError {
    EngineError::new("BAD_REQUEST", message)
}

/// The edit as a replacement of the paragraph holding the control.
pub fn edit(
    doc: &Document,
    fonts: &hm_fonts::FontSystem,
    object: &TableAddress,
    action: Action,
) -> Result<Op> {
    let list = crate::tables::path(object);
    let mut para = hm_edit::ops::list(doc, &list)
        .and_then(|p| p.get(object.para as usize))
        .cloned()
        .ok_or_else(|| EngineError::new("BAD_ADDRESS", "no such object paragraph"))?;
    let control = para
        .controls
        .get_mut(object.control as usize)
        .ok_or_else(|| EngineError::new("BAD_ADDRESS", "no such object control"))?;
    match action {
        Action::Move { dx, dy } => {
            let common = common_mut(control)?;
            if common.treat_as_char() {
                return Err(bad("an object set as a character does not move by offset"));
            }
            move_by(common, dx, dy);
        }
        Action::Resize { width, height } => {
            let (width, height) = (width.max(MIN_OBJECT), height.max(MIN_OBJECT));
            match &mut control.kind {
                ControlKind::Object(o) => {
                    if o.equation().is_some() {
                        return Err(bad("an equation takes its size from its script"));
                    }
                    let old = (o.header.common.width, o.header.common.height);
                    let shape = o
                        .parts
                        .iter_mut()
                        .find_map(|p| match p {
                            hm_model::ObjectPart::Shape(s) => Some(&mut **s),
                            _ => None,
                        })
                        .ok_or_else(|| bad("the object has no shape"))?;
                    resize_shape(shape, old, (width, height))?;
                    let c = &mut o.header.common;
                    absolute_size(c);
                    c.width = width;
                    c.height = height;
                }
                ControlKind::Table(t) => {
                    let laid = laid_out(doc, fonts, object.section, t);
                    let (sx, sy) = (
                        width as f64 / laid.width.max(1) as f64,
                        height as f64 / laid.height.max(1) as f64,
                    );
                    for (c, l) in t.cells.iter_mut().zip(&laid.cells) {
                        set_width(c, ((l.w as f64 * sx).round() as i32).max(MIN_CELL));
                        c.height = ((l.h as f64 * sy).round() as i32).max(MIN_CELL);
                    }
                    fit_table(doc, fonts, object.section, t);
                }
                _ => return Err(bad("the control is not an object")),
            }
        }
        Action::Cells(sizes) => {
            let ControlKind::Table(t) = &mut control.kind else {
                return Err(bad("the control is not a table"));
            };
            let laid = laid_out(doc, fonts, object.section, t);
            for s in &sizes {
                let (Some(c), Some(l)) = (
                    t.cells.get_mut(s.index as usize),
                    laid.cells.get(s.index as usize),
                ) else {
                    return Err(EngineError::new("BAD_ADDRESS", "no such cell"));
                };
                if s.width_delta != 0 {
                    set_width(c, (l.w + s.width_delta).max(MIN_CELL));
                }
                if s.height_delta != 0 {
                    c.height = (l.h + s.height_delta).max(MIN_CELL);
                }
            }
            fit_table(doc, fonts, object.section, t);
        }
    }
    // Only the control changed: the paragraph keeps its stored spans, as
    // after a cell text edit, and layout lets an object's new size break
    // its lines again.
    control.src = None;
    Ok(Op::Replace {
        list,
        start: object.para,
        count: 1,
        paras: vec![para],
    })
}

fn common_mut(c: &mut Control) -> Result<&mut ObjectCommon> {
    match &mut c.kind {
        ControlKind::Table(t) => Ok(&mut t.header.common),
        ControlKind::Object(o) => Ok(&mut o.header.common),
        _ => Err(bad("the control is not an object")),
    }
}

/// Offsets count inward from the aligned edge: an object aligned to the
/// end moves right as its offset shrinks. A paragraph-relative object
/// always hangs from the paragraph's top.
fn move_by(o: &mut ObjectCommon, dx: i32, dy: i32) {
    let h = (o.attr >> 10) & 7;
    o.x_offset = if h == 2 {
        o.x_offset.saturating_sub(dx)
    } else {
        o.x_offset.saturating_add(dx)
    };
    let v = if o.vert_rel_to() == 2 {
        0
    } else {
        (o.attr >> 5) & 7
    };
    o.y_offset = if v == 2 {
        o.y_offset.saturating_sub(dy)
    } else {
        o.y_offset.saturating_add(dy)
    };
}

/// A size kept relative to the paper, page, column or paragraph becomes
/// absolute once set by hand.
pub(super) fn absolute_size(o: &mut ObjectCommon) {
    o.attr = (o.attr & !(7 << 15)) | (4 << 15);
    o.attr = (o.attr & !(3 << 18)) | (2 << 18);
}

fn scale(sx: f64, sy: f64) -> Matrix {
    Matrix([sx, 0.0, 0.0, 0.0, sy, 0.0])
}

/// Scales a drawing from `old` to `new` in its own axes, its top left
/// kept: the outermost scale of the shape and of every group member, the
/// members' places in the group, and a rotation about the new center.
pub(super) fn resize_shape(shape: &mut Shape, old: (i32, i32), new: (i32, i32)) -> Result<()> {
    let c = &mut shape.component;
    let current = (
        if c.size.0 > 0 {
            c.size.0 as f64
        } else {
            old.0.max(1) as f64
        },
        if c.size.1 > 0 {
            c.size.1 as f64
        } else {
            old.1.max(1) as f64
        },
    );
    let (sx, sy) = (new.0 as f64 / current.0, new.1 as f64 / current.1);
    let grouped = shape.children().next().is_some();
    if grouped && shape.component.rotation != 0 {
        return Err(bad("a rotated group does not resize"));
    }
    let c = &mut shape.component;
    c.size = (new.0 as u32, new.1 as u32);
    let d = scale(sx, sy);
    // The translation places the drawing (flips included); it scales with it.
    c.translation.0[2] *= sx;
    c.translation.0[5] *= sy;
    match c.transforms.first_mut() {
        Some((s, r)) => {
            *s = d.mul(s);
            if c.rotation != 0 {
                let [a, b, _, cc, dd, _] = r.0;
                let (cx, cy) = (new.0 as f64 / 2.0, new.1 as f64 / 2.0);
                r.0[2] = cx - (a * cx + b * cy);
                r.0[5] = cy - (cc * cx + dd * cy);
            }
        }
        None => c.transforms.push((d, Matrix::IDENTITY)),
    }
    for p in &mut shape.parts {
        match p {
            ShapePart::Child(child) => scale_member(child, sx, sy),
            ShapePart::Text(t) => {
                if let Some(w) = &mut t.max_width {
                    *w = (*w as f64 * sx).round() as i32;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// A group member scales with its group: its place, and the group's pair
/// it carries first.
fn scale_member(shape: &mut Shape, sx: f64, sy: f64) {
    let c = &mut shape.component;
    c.translation.0[2] *= sx;
    c.translation.0[5] *= sy;
    if let Some((s, _)) = c.transforms.first_mut() {
        *s = scale(sx, sy).mul(s);
    }
    for p in &mut shape.parts {
        match p {
            ShapePart::Child(child) => scale_member(child, sx, sy),
            ShapePart::Text(t) => {
                if let Some(w) = &mut t.max_width {
                    *w = (*w as f64 * sx).round() as i32;
                }
            }
            _ => {}
        }
    }
}

fn tab(doc: &Document, section: u32) -> i32 {
    doc.sections
        .get(section as usize)
        .and_then(|s| s.definition())
        .map(|d| hm_model::units::unpack_distance(d.default_tab, || 1000))
        .filter(|&v| v > 0)
        .unwrap_or(4000)
}

fn laid_out(
    doc: &Document,
    fonts: &hm_fonts::FontSystem,
    section: u32,
    t: &hm_model::Table,
) -> hm_layout::table::TableLayout {
    let mut styles = hm_layout::style::Styles::new(&doc.info, fonts);
    hm_layout::table::layout(t, &doc.info, &mut styles, tab(doc, section))
}

/// A new width breaks the cell's lines again.
pub(super) fn set_width(c: &mut hm_model::Cell, width: i32) {
    if c.width != width {
        for p in &mut c.paras {
            p.src = None;
            p.line_segs.clear();
        }
    }
    c.width = width;
}

/// The table's box follows its grid, as Hancom sets it on open.
pub(super) fn fit_table(
    doc: &Document,
    fonts: &hm_fonts::FontSystem,
    section: u32,
    t: &mut hm_model::Table,
) {
    let laid = laid_out(doc, fonts, section, t);
    t.header.common.width = laid.width;
    t.header.common.height = laid.height;
}
