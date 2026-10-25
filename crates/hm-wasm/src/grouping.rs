//! Moving a line's end points, grouping drawings into one object and
//! ungrouping them again. Each is one model operation sequence, so undo
//! and redo restore the paragraphs exactly.

use crate::object_api::{component, local_size, replace, replacement, shape_control};
use crate::types::{EngineError, Result, TableAddress};
use hm_edit::{Op, Pos};
use hm_model::*;
use std::collections::HashMap;

/// An object's box on the page as laid out: page, x, y, width, height.
pub type Boxes = HashMap<(u32, u32, u32), LaidBox>;
/// Page, x, y, width, height.
pub type LaidBox = (usize, i32, i32, i32, i32);

const GROUP: CtrlId = CtrlId::from_chars(b"$con");

fn bad(s: impl std::fmt::Display) -> EngineError {
    EngineError::new("BAD_REQUEST", s.to_string())
}

/// Moves an object's box by `(dx, dy)` on the page and gives it the size
/// `(w, h)`, keeping its anchor: offsets count from the aligned edge, so
/// a centered or end-aligned box also makes up for the size change.
fn place(o: &mut ObjectCommon, dx: i32, dy: i32, w: i32, h: i32) {
    let shift = |code: u32, offset: i32, d: i32, old: i32, new: i32| match code {
        1 => offset + d + (new - old) / 2,
        2 => offset + (old - new) - d,
        _ => offset + d,
    };
    if !o.treat_as_char() {
        let v = if o.vert_rel_to() == 2 {
            0
        } else {
            (o.attr >> 5) & 7
        };
        o.x_offset = shift((o.attr >> 10) & 7, o.x_offset, dx, o.width, w);
        o.y_offset = shift(v, o.y_offset, dy, o.height, h);
    }
    crate::objects::absolute_size(o);
    o.width = w;
    o.height = h;
}

fn object_mut(c: &mut Control) -> Result<&mut ObjectControl> {
    match &mut c.kind {
        ControlKind::Object(o) => Ok(o),
        _ => Err(bad("the control is not a drawing object")),
    }
}

fn shape_part(o: &mut ObjectControl) -> Option<&mut Shape> {
    o.parts.iter_mut().find_map(|p| match p {
        ObjectPart::Shape(s) => Some(&mut **s),
        _ => None,
    })
}

/// Gives a line the end points `start` and `end`, relative to the top left
/// of its current box (as `Doc.pageObjects` reports it). The box becomes
/// the line's extent and the drawing loses any turn or mirroring.
pub fn set_line_ends(
    doc: &Document,
    at: &TableAddress,
    start: (i32, i32),
    end: (i32, i32),
) -> Result<Op> {
    let mut para = replacement(doc, at)?;
    let o = object_mut(&mut para.controls[at.control as usize])?;
    let (x0, y0) = (start.0.min(end.0), start.1.min(end.1));
    let w = (end.0 - start.0).abs().max(1);
    let h = (end.1 - start.1).abs().max(1);
    let shape = shape_part(o).ok_or_else(|| bad("the object has no drawing"))?;
    if !matches!(shape.kind(), Some(ShapePart::Line(_))) {
        return Err(bad("only a line has end points"));
    }
    for part in &mut shape.parts {
        if let ShapePart::Line(l) = part {
            l.start = (start.0 - x0, start.1 - y0);
            l.end = (end.0 - x0, end.1 - y0);
            if l.attr.is_some() {
                l.attr = Some(0);
            }
        }
    }
    let c = &mut shape.component;
    c.initial_size = (w as u32, h as u32);
    c.size = (w as u32, h as u32);
    c.translation = Matrix::IDENTITY;
    c.transforms = vec![(Matrix::IDENTITY, Matrix::IDENTITY)];
    c.rotation = 0;
    c.flip &= !3;
    c.rotation_center = (w / 2, h / 2);
    place(&mut o.header.common, x0, y0, w, h);
    Ok(replace(at, para))
}

fn translate(m: &mut Matrix, dx: f64, dy: f64) {
    m.0[2] += dx;
    m.0[5] += dy;
}

/// Makes `s` (a former top-level drawing, or one of its members) a member
/// one level deeper, shifted by `(dx, dy)` in the new group's box. Members
/// with more transform pairs than their parent map straight to the top
/// box, so only they move; the others stay local to their parent.
fn nest(s: &mut Shape, dx: f64, dy: f64, parent_pairs: Option<usize>) {
    let c = &mut s.component;
    let pairs = c.transforms.len();
    if parent_pairs.is_none_or(|p| pairs > p) {
        translate(&mut c.translation, dx, dy);
    }
    c.transforms.insert(0, (Matrix::IDENTITY, Matrix::IDENTITY));
    c.group_level += 1;
    if parent_pairs.is_none() {
        c.doubled = false;
        c.offset = (
            c.translation.0[2].round() as i32,
            c.translation.0[5].round() as i32,
        );
    }
    for p in &mut s.parts {
        if let ShapePart::Child(m) = p {
            nest(m, dx, dy, Some(pairs));
        }
    }
}

/// Groups the drawings `objects` (two or more top-level objects of one
/// section shown on one page) into one group object. The group takes the
/// place and anchor of the first member in document order; the members
/// keep where they are on the page. Positions come from the layout before
/// the transaction.
pub fn group(doc: &Document, objects: &[TableAddress], boxes: &Boxes) -> Result<(Vec<Op>, Pos)> {
    if objects.len() < 2 {
        return Err(bad("grouping needs at least two objects"));
    }
    let section = objects[0].section;
    let mut members: Vec<(&TableAddress, &ObjectControl, LaidBox)> = Vec::new();
    for a in objects {
        if a.section != section || !a.steps.is_empty() {
            return Err(bad(
                "grouped objects must be top-level objects of one section",
            ));
        }
        if members
            .iter()
            .any(|(m, ..)| m.para == a.para && m.control == a.control)
        {
            return Err(bad("an object is named twice"));
        }
        let o = match &crate::object_api::control(doc, a)?.kind {
            ControlKind::Object(o) => &**o,
            _ => return Err(bad("only drawings and pictures can be grouped")),
        };
        if o.shape().is_none() || o.equation().is_some() || o.caption().is_some() {
            return Err(bad(
                "only drawings and pictures without captions can be grouped",
            ));
        }
        if o.header.common.treat_as_char() {
            return Err(bad("an object set as a character cannot be grouped"));
        }
        let b = *boxes
            .get(&(a.section, a.para, a.control))
            .ok_or_else(|| bad("a grouped object is not laid out"))?;
        members.push((a, o, b));
    }
    if members.iter().any(|m| m.2.0 != members[0].2.0) {
        return Err(bad("grouped objects must be on one page"));
    }
    members.sort_by_key(|(a, ..)| (a.para, a.control));
    let x0 = members.iter().map(|m| m.2.1).min().unwrap();
    let y0 = members.iter().map(|m| m.2.2).min().unwrap();
    let x1 = members.iter().map(|m| m.2.1 + m.2.3).max().unwrap();
    let y1 = members.iter().map(|m| m.2.2 + m.2.4).max().unwrap();
    let (w, h) = ((x1 - x0).max(1), (y1 - y0).max(1));
    // Members draw bottom to top: by z-order, then document order.
    let mut stack: Vec<_> = members.iter().collect();
    stack.sort_by_key(|(a, o, _)| (o.header.common.z_order, a.para, a.control));
    let mut kinds = Vec::new();
    let mut parts = Vec::new();
    for (_, o, b) in &stack {
        let mut s = o.shape().unwrap().clone();
        nest(&mut s, (b.1 - x0) as f64, (b.2 - y0) as f64, None);
        kinds.push(s.component.kind);
        parts.push(ShapePart::Child(Box::new(s)));
    }
    let (anchor, first, ab) = &members[0];
    let mut header = first.header.clone();
    place(&mut header.common, x0 - ab.1, y0 - ab.2, w, h);
    header.common.z_order = members
        .iter()
        .map(|m| m.1.header.common.z_order)
        .max()
        .unwrap();
    header.description = None;
    let mut comp = component(GROUP, w, h, None);
    comp.members = Some(kinds);
    comp.group_instance = Some(0);
    let group = shape_control(
        header,
        Shape {
            component: comp,
            parts,
        },
    );
    let mut para = replacement(doc, anchor)?;
    para.controls[anchor.control as usize] = group;
    let mut ops = vec![replace(anchor, para)];
    for (a, ..) in members.iter().skip(1).rev() {
        ops.push(crate::object_api::delete(doc, a)?.0);
    }
    let (_, at) = crate::object_api::delete(doc, anchor)?;
    Ok((ops, at))
}

fn inverse(m: &Matrix) -> Matrix {
    let [a, b, c, d, e, f] = m.0;
    let det = a * e - b * d;
    if det.abs() < 1e-12 {
        return Matrix::IDENTITY;
    }
    Matrix([
        e / det,
        -b / det,
        (b * f - c * e) / det,
        -d / det,
        a / det,
        (c * d - a * f) / det,
    ])
}

fn chain(pairs: &[(Matrix, Matrix)]) -> Matrix {
    pairs
        .iter()
        .fold(Matrix::IDENTITY, |m, (scale, rot)| m.mul(rot).mul(scale))
}

/// Takes the first `n` transform pairs (the ungrouped group's) off `s` and
/// its members, folding what they did into the translation and the next
/// pair so every drawing keeps its place. `parent` maps the parent's
/// coordinates to the old group's box; `origin` is the new box's top left
/// there.
fn lift(s: &mut Shape, n: usize, parent: (&Matrix, usize), origin: (f64, f64)) {
    let c = &mut s.component;
    let pairs = c.transforms.len();
    c.group_level = c.group_level.saturating_sub(1);
    if pairs <= parent.1 {
        // Local to its parent: it holds none of the group's pairs.
        let full = parent.0.mul(&c.matrix());
        for p in &mut s.parts {
            if let ShapePart::Child(m) = p {
                lift(m, n, (&full, pairs), origin);
            }
        }
        return;
    }
    let full = c.matrix();
    let own: Vec<_> = c.transforms.iter().skip(n.min(pairs)).cloned().collect();
    // full = L × own; L's linear part goes into the first own pair.
    let l = full.mul(&inverse(&chain(&own)));
    let mut kept = own;
    let lin = Matrix([l.0[0], l.0[1], 0.0, l.0[3], l.0[4], 0.0]);
    if lin != Matrix::IDENTITY {
        match kept.first_mut() {
            Some((scale, rot)) => *scale = inverse(rot).mul(&lin).mul(rot).mul(scale),
            None => kept.push((lin, Matrix::IDENTITY)),
        }
    }
    if kept.is_empty() {
        kept.push((Matrix::IDENTITY, Matrix::IDENTITY));
    }
    c.translation = Matrix([1.0, 0.0, l.0[2] - origin.0, 0.0, 1.0, l.0[5] - origin.1]);
    c.transforms = kept;
    for p in &mut s.parts {
        if let ShapePart::Child(m) = p {
            lift(m, n, (&full, pairs), origin);
        }
    }
}

/// Splits the group `at` into its members, each its own object in place of
/// the group, kept where it is on the page.
pub fn ungroup(doc: &Document, at: &TableAddress) -> Result<(Vec<Op>, Pos)> {
    let control = crate::object_api::control(doc, at)?;
    let ControlKind::Object(o) = &control.kind else {
        return Err(bad("the control is not a group"));
    };
    let g = o
        .shape()
        .filter(|s| s.component.kind == GROUP && s.children().next().is_some());
    let g = g.ok_or_else(|| bad("the object is not a group"))?;
    let n = g.component.transforms.len();
    let top = g.component.matrix();
    let mut controls = Vec::new();
    for (k, child) in g.children().enumerate() {
        let mut s = child.clone();
        let pairs = s.component.transforms.len();
        let full = if pairs > n {
            s.component.matrix()
        } else {
            top.mul(&s.component.matrix())
        };
        let (w, h) = local_size(&s.component);
        let pts = [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)].map(|(x, y)| full.apply(x, y));
        let bx0 = pts.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
        let by0 = pts.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
        let bx1 = pts.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
        let by1 = pts.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
        let (ox, oy) = (bx0.round(), by0.round());
        lift(&mut s, n, (&top, n), (ox, oy));
        s.component.doubled = true;
        s.component.offset = (0, 0);
        let mut header = o.header.clone();
        let (cw, ch) = (
            ((bx1 - ox).round() as i32).max(1),
            ((by1 - oy).round() as i32).max(1),
        );
        place(&mut header.common, ox as i32, oy as i32, cw, ch);
        header.common.z_order = o.header.common.z_order.saturating_add(k as i32);
        header.description = None;
        controls.push(shape_control(header, s));
    }
    let list = crate::tables::path(at);
    let p = &hm_edit::ops::list(doc, &list).unwrap()[at.para as usize];
    let offset = p
        .units()
        .find_map(|(offset, u)| {
            matches!(u, Unit::Extended { index, .. } if index == at.control).then_some(offset)
        })
        .ok_or_else(|| bad("the group has no anchor"))?;
    let mut rest = controls.into_iter();
    let mut para = replacement(doc, at)?;
    para.controls[at.control as usize] = rest.next().unwrap();
    let mut ops = vec![replace(at, para)];
    let mut staged = doc.clone();
    hm_edit::apply(&mut staged, &ops[0]).map_err(|e| EngineError::new("BAD_ADDRESS", e))?;
    let mut pos = Pos {
        list,
        para: at.para,
        offset: offset + 8,
    };
    for c in rest {
        let op = hm_edit::ops::insert_control(&staged, &pos, c)
            .map_err(|e| EngineError::new("BAD_ADDRESS", e))?;
        hm_edit::apply(&mut staged, &op).map_err(|e| EngineError::new("BAD_ADDRESS", e))?;
        ops.push(op);
        pos.offset += 8;
    }
    Ok((ops, pos))
}
