//! Edit scripts: a JSON list of operations addressed by position or by a
//! quote of the text.
//!
//! ```json
//! {"ops": [
//!   {"op": "insert", "at": {"quote": "보도자료", "side": "after"}, "text": "(수정)"},
//!   {"op": "delete", "quote": "불필요한 말 "},
//!   {"op": "format", "quote": "중요", "bold": true, "color": "#FF0000", "size": 1200},
//!   {"op": "para", "at": {"section": 0, "para": 4}, "align": "center"},
//!   {"op": "split", "at": {"quote": "다음 장", "side": "before"}, "page_break": true},
//!   {"op": "page_break", "at": {"section": 0, "para": 9}, "on": true}
//! ]}
//! ```
//!
//! An address is `{"section", "para", "offset", "cells": [[para, control,
//! index], ...]}` in wire units (steps into control lists, see
//! [`crate::ops::Step`]), or `{"quote", "occurrence", "side"}`
//! searching body and cell paragraphs in document order (occurrence counts
//! from 1; side is `before` or `after` the quote).

use hm_model::{CharShape, Document, ParaShape, Unit};
use serde_json::Value;

use crate::Editor;
use crate::ops::{EditError, ListPath, Op, Pos, Step};

#[derive(Debug)]
pub struct ScriptError(pub String);

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ScriptError {}

fn err<T>(msg: impl Into<String>) -> Result<T, ScriptError> {
    Err(ScriptError(msg.into()))
}

/// Runs a script on an editor; each operation is one undo step.
pub fn run(ed: &mut Editor, script: &str) -> Result<usize, ScriptError> {
    let v: Value = serde_json::from_str(script).map_err(|e| ScriptError(e.to_string()))?;
    let ops = v
        .get("ops")
        .and_then(Value::as_array)
        .or_else(|| v.as_array())
        .ok_or_else(|| ScriptError("expected {\"ops\": [...]}".into()))?;
    for (i, o) in ops.iter().enumerate() {
        step(ed, o).map_err(|e| ScriptError(format!("op {}: {}", i + 1, e.0)))?;
    }
    Ok(ops.len())
}

fn step(ed: &mut Editor, o: &Value) -> Result<(), ScriptError> {
    let kind = o.get("op").and_then(Value::as_str).unwrap_or_default();
    let apply = |ed: &mut Editor, op: Op| ed.apply(op).map_err(|e| ScriptError(e.to_string()));
    match kind {
        "insert" => {
            let at = addr(&ed.doc, o.get("at"), false)?;
            let text = o.get("text").and_then(Value::as_str).unwrap_or_default();
            apply(
                ed,
                Op::InsertText {
                    at,
                    text: text.to_string(),
                },
            )
        }
        "delete" => {
            let (from, to) = range(&ed.doc, o)?;
            apply(ed, Op::Delete { from, to })
        }
        "split" => {
            let at = addr(&ed.doc, o.get("at"), false)?;
            let page_break = o
                .get("page_break")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            apply(ed, Op::Split { at, page_break })
        }
        "page_break" => {
            let at = addr(&ed.doc, o.get("at"), false)?;
            let on = o.get("on").and_then(Value::as_bool).unwrap_or(true);
            apply(
                ed,
                Op::SetPageBreak {
                    list: at.list,
                    para: at.para,
                    on,
                },
            )
        }
        "format" => {
            let (from, to) = range(&ed.doc, o)?;
            format(ed, o, from, to)
        }
        "para" => {
            let at = addr(&ed.doc, o.get("at"), false)?;
            let last = o
                .get("to")
                .map(|t| addr(&ed.doc, Some(t), false))
                .transpose()?
                .map_or(at.para, |t| t.para);
            para_format(ed, o, at.list, at.para, last)
        }
        other => err(format!("unknown op {other:?}")),
    }
}

/// `from`/`to` addresses, or a quote covering the range.
fn range(doc: &Document, o: &Value) -> Result<(Pos, Pos), ScriptError> {
    if o.get("quote").is_some() {
        let from = addr(doc, Some(&with_side(o, "before")), false)?;
        let to = addr(doc, Some(&with_side(o, "after")), false)?;
        return Ok((from, to));
    }
    Ok((
        addr(doc, o.get("from"), false)?,
        addr(doc, o.get("to"), false)?,
    ))
}

fn with_side(o: &Value, side: &str) -> Value {
    let mut v = serde_json::json!({ "quote": o["quote"], "side": side });
    if let Some(n) = o.get("occurrence") {
        v["occurrence"] = n.clone();
    }
    v
}

fn addr(doc: &Document, a: Option<&Value>, _end: bool) -> Result<Pos, ScriptError> {
    let Some(a) = a else {
        return err("missing address");
    };
    if let Some(q) = a.get("quote").and_then(Value::as_str) {
        let n = a
            .get("occurrence")
            .and_then(Value::as_u64)
            .unwrap_or(1)
            .max(1);
        let after = a.get("side").and_then(Value::as_str) == Some("after");
        return find(doc, q, n as usize, after)
            .ok_or_else(|| ScriptError(format!("quote {q:?} #{n} not found")));
    }
    let num = |k: &str| a.get(k).and_then(Value::as_u64).map(|v| v as u32);
    let cells = a
        .get("cells")
        .and_then(Value::as_array)
        .map(|steps| {
            steps
                .iter()
                .filter_map(|s| {
                    let s = s.as_array()?;
                    Some(Step {
                        para: s.first()?.as_u64()? as u32,
                        control: s.get(1)?.as_u64()? as u32,
                        index: s.get(2)?.as_u64()? as u32,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Pos {
        list: ListPath {
            section: num("section").unwrap_or(0),
            steps: cells,
        },
        para: num("para").ok_or_else(|| ScriptError("address needs para".into()))?,
        offset: num("offset").unwrap_or(0),
    })
}

/// The `n`th occurrence of `quote` in document order: body paragraphs,
/// descending into table cells where the table sits.
pub fn find(doc: &Document, quote: &str, n: usize, after: bool) -> Option<Pos> {
    let mut seen = 0;
    for (si, sec) in doc.sections.iter().enumerate() {
        let list = ListPath {
            section: si as u32,
            steps: Vec::new(),
        };
        if let Some(p) = find_in(&sec.paras, &list, quote, n, after, &mut seen) {
            return Some(p);
        }
    }
    None
}

fn find_in(
    paras: &[hm_model::Paragraph],
    list: &ListPath,
    quote: &str,
    n: usize,
    after: bool,
    seen: &mut usize,
) -> Option<Pos> {
    let q: Vec<char> = quote.chars().collect();
    for (pi, p) in paras.iter().enumerate() {
        // Characters with their wire positions and ends.
        let chars: Vec<(char, u32, u32)> = p
            .units()
            .filter_map(|(pos, u)| match u {
                Unit::Char(c) => Some((c, pos, pos + c.len_utf16() as u32)),
                Unit::Inline(9) => Some(('\t', pos, pos + 8)),
                _ => None,
            })
            .collect();
        if !q.is_empty() && chars.len() >= q.len() {
            for s in 0..=chars.len() - q.len() {
                if chars[s..s + q.len()]
                    .iter()
                    .map(|c| c.0)
                    .eq(q.iter().copied())
                {
                    *seen += 1;
                    if *seen == n {
                        let offset = if after {
                            chars[s + q.len() - 1].2
                        } else {
                            chars[s].1
                        };
                        return Some(Pos {
                            list: list.clone(),
                            para: pi as u32,
                            offset,
                        });
                    }
                }
            }
        }
        for (ci, c) in p.controls.iter().enumerate() {
            let Some(t) = c.as_table() else { continue };
            for (k, cell) in t.cells.iter().enumerate() {
                let mut sub = list.clone();
                sub.steps.push(Step {
                    para: pi as u32,
                    control: ci as u32,
                    index: k as u32,
                });
                if let Some(p) = find_in(&cell.paras, &sub, quote, n, after, seen) {
                    return Some(p);
                }
            }
        }
    }
    None
}

/// A char shape like `base` changed by `f`: an existing equal one, else a
/// new entry.
pub fn derive_char_shape(doc: &mut Document, base: u32, f: &dyn Fn(&mut CharShape)) -> u32 {
    let mut c = doc
        .info
        .char_shapes
        .get(base as usize)
        .cloned()
        .unwrap_or_default();
    f(&mut c);
    c.src = None;
    let key = |c: &CharShape| {
        format!(
            "{:?}",
            CharShape {
                src: None,
                ..c.clone()
            }
        )
    };
    let k = key(&c);
    if let Some(i) = doc.info.char_shapes.iter().position(|e| key(e) == k) {
        return i as u32;
    }
    doc.info.char_shapes.push(c);
    // The counts record must declare the new entry.
    doc.info.id_mappings.src = None;
    doc.info.char_shapes.len() as u32 - 1
}

/// A para shape like `base` changed by `f`, likewise.
pub fn derive_para_shape(doc: &mut Document, base: u16, f: &dyn Fn(&mut ParaShape)) -> u16 {
    let mut s = doc
        .info
        .para_shapes
        .get(base as usize)
        .cloned()
        .unwrap_or_default();
    f(&mut s);
    s.src = None;
    let key = |s: &ParaShape| {
        format!(
            "{:?}",
            ParaShape {
                src: None,
                ..s.clone()
            }
        )
    };
    let k = key(&s);
    if let Some(i) = doc.info.para_shapes.iter().position(|e| key(e) == k) {
        return i as u16;
    }
    doc.info.para_shapes.push(s);
    doc.info.id_mappings.src = None;
    doc.info.para_shapes.len() as u16 - 1
}

fn color(v: &Value) -> Option<u32> {
    let s = v.as_str()?.trim_start_matches('#');
    let rgb = u32::from_str_radix(s, 16).ok()?;
    // COLORREF is 0x00BBGGRR.
    Some((rgb & 0xFF) << 16 | (rgb & 0xFF00) | (rgb >> 16) & 0xFF)
}

/// Character formatting to apply; `None` leaves a property as it is.
#[derive(Clone, Copy, Debug, Default)]
pub struct CharPatch {
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    /// Base size in HU (1000 is 10 pt).
    pub size: Option<i32>,
    /// COLORREF.
    pub color: Option<u32>,
    pub strike: Option<bool>,
    /// Face ids for the seven script slots; `None` keeps a slot's face.
    pub faces: [Option<u16>; 7],
    /// Spacing percent (-50 to 50) for every script slot.
    pub spacing: Option<i8>,
    pub spacings: [Option<i8>; 7],
    pub ratios: [Option<u8>; 7],
    /// Relative size percent (10 to 250) per script slot.
    pub rel_sizes: [Option<u8>; 7],
    /// Vertical offset percent (-100 to 100) per script slot.
    pub offsets: [Option<i8>; 7],
    pub superscript: Option<bool>,
    pub subscript: Option<bool>,
    pub emboss: Option<bool>,
    pub engrave: Option<bool>,
    pub kerning: Option<bool>,
    /// Underline position: 0 none, 1 below, 3 above.
    pub underline_kind: Option<u32>,
    /// Line shape (0 to 15) of the underline.
    pub underline_shape: Option<u32>,
    pub underline_color: Option<u32>,
    /// Line shape (0 to 15) of the strikeout.
    pub strike_shape: Option<u32>,
    pub strike_color: Option<u32>,
    /// Outline kind (0 to 6).
    pub outline: Option<u32>,
    /// Shadow kind: 0 none, 1 discrete, 2 continuous.
    pub shadow: Option<u32>,
    pub shadow_color: Option<u32>,
    /// Shadow offset percent (-100 to 100), x then y.
    pub shadow_offset: Option<[i8; 2]>,
    /// Emphasis mark (0 to 12).
    pub emphasis: Option<u32>,
    /// COLORREF; 0xFFFFFFFF is no shading.
    pub shade_color: Option<u32>,
    pub border_fill: Option<u16>,
}

impl CharPatch {
    pub fn apply(&self, c: &mut CharShape) {
        if let Some(id) = self.border_fill {
            c.border_fill = Some(id);
        }
        let mut set = |bit: u32, on: Option<bool>| match on {
            Some(true) => c.attr |= 1 << bit,
            Some(false) => c.attr &= !(1 << bit),
            None => {}
        };
        set(0, self.italic);
        set(1, self.bold);
        match self.underline {
            Some(true) => c.attr = c.attr & !(3 << 2) | 1 << 2,
            // With a strikeout, a centered underline is the strikeout's.
            Some(false) if (c.attr >> 2) & 3 != 2 || (c.attr >> 18) & 7 == 0 => c.attr &= !(3 << 2),
            _ => {}
        }
        if let Some(s) = self.size {
            c.base_size = s;
        }
        if let Some(k) = self.color {
            c.color = k;
        }
        match self.strike {
            // A solid strikeout; Hancom also keeps it as a centered
            // underline when there is no other underline.
            Some(true) => {
                // A hidden centered 3D line is Hancom's no-strike sentinel.
                // A requested solid strike must replace its line shape too.
                if (c.attr >> 2) & 63 == (2 | (15 << 2)) {
                    c.attr &= !(15 << 4);
                }
                c.attr = c.attr & !(7 << 18 | 15 << 26) | 1 << 18;
                c.strike_color = Some(c.strike_color.unwrap_or(c.color));
                if (c.attr >> 2) & 3 == 0 {
                    c.attr = c.attr & !(0x3F << 2) | 2 << 2;
                    c.underline_color = c.strike_color.unwrap_or(c.color);
                }
            }
            Some(false) => {
                c.attr &= !(7 << 18 | 15 << 26);
                if (c.attr >> 2) & 3 == 2 {
                    c.attr &= !(0x3F << 2);
                }
            }
            None => {}
        }
        for (slot, face) in c.faces.iter_mut().zip(self.faces) {
            if let Some(f) = face {
                *slot = f;
            }
        }
        if let Some(v) = self.spacing {
            c.spacings = [v; 7];
        }
        for (slot, v) in c.spacings.iter_mut().zip(self.spacings) {
            if let Some(v) = v {
                *slot = v;
            }
        }
        for (slot, v) in c.ratios.iter_mut().zip(self.ratios) {
            if let Some(v) = v {
                *slot = v;
            }
        }
        for (slot, v) in c.rel_sizes.iter_mut().zip(self.rel_sizes) {
            if let Some(v) = v {
                *slot = v;
            }
        }
        for (slot, v) in c.offsets.iter_mut().zip(self.offsets) {
            if let Some(v) = v {
                *slot = v;
            }
        }
        let field = |attr: &mut u32, shift: u32, mask: u32, v: Option<u32>| {
            if let Some(v) = v {
                *attr = *attr & !(mask << shift) | (v & mask) << shift;
            }
        };
        // Superscript and subscript exclude each other.
        if self.superscript == Some(true) {
            c.attr &= !(1 << 16);
        }
        if self.subscript == Some(true) {
            c.attr &= !(1 << 15);
        }
        let mut set = |bit: u32, on: Option<bool>| match on {
            Some(true) => c.attr |= 1 << bit,
            Some(false) => c.attr &= !(1 << bit),
            None => {}
        };
        set(13, self.emboss);
        set(14, self.engrave);
        set(15, self.superscript);
        set(16, self.subscript);
        set(30, self.kerning);
        // Emboss and engrave exclude each other too.
        if self.emboss == Some(true) {
            c.attr &= !(1 << 14);
        }
        if self.engrave == Some(true) {
            c.attr &= !(1 << 13);
        }
        field(&mut c.attr, 2, 3, self.underline_kind);
        field(&mut c.attr, 4, 15, self.underline_shape);
        field(&mut c.attr, 8, 7, self.outline);
        field(&mut c.attr, 11, 3, self.shadow);
        field(&mut c.attr, 21, 15, self.emphasis);
        if let Some(k) = self.underline_color {
            c.underline_color = k;
        }
        if let Some(shape) = self.strike_shape {
            field(&mut c.attr, 26, 15, Some(shape));
            c.strike_color.get_or_insert(c.color);
        }
        if let Some(k) = self.strike_color {
            c.strike_color = Some(k);
        }
        if let Some(k) = self.shadow_color {
            c.shadow_color = k;
        }
        if let Some(o) = self.shadow_offset {
            c.shadow_offset = o;
        }
        if let Some(k) = self.shade_color {
            c.shade_color = k;
        }
    }
}

/// Paragraph formatting to apply; distances in HU.
#[derive(Clone, Copy, Debug, Default)]
pub struct ParaPatch {
    /// 0 justify, 1 left, 2 right, 3 center, 4 distribute, 5 divide.
    pub align: Option<u32>,
    pub space_before: Option<i32>,
    pub space_after: Option<i32>,
    pub left: Option<i32>,
    pub right: Option<i32>,
    pub indent: Option<i32>,
    /// Percent.
    pub line_spacing: Option<u32>,
    /// 0 percent, 1 fixed, 2 added, 3 at least.
    pub line_spacing_kind: Option<u32>,
    pub tab_def: Option<u16>,
}

impl ParaPatch {
    pub fn apply(&self, s: &mut ParaShape) {
        if let Some(id) = self.tab_def {
            s.tab_def = id;
        }
        // Distances are stored packed (shifted left once).
        if let Some(a) = self.align {
            s.attr1 = s.attr1 & !(7 << 2) | a << 2;
        }
        if let Some(v) = self.space_before {
            s.space_before = v << 1;
        }
        if let Some(v) = self.space_after {
            s.space_after = v << 1;
        }
        if let Some(v) = self.left {
            s.margin_left = v << 1;
        }
        if let Some(v) = self.right {
            s.margin_right = v << 1;
        }
        if let Some(v) = self.indent {
            s.indent = v << 1;
        }
        if self.line_spacing.is_some() || self.line_spacing_kind.is_some() {
            let (old_kind, old_value) = s.line_spacing();
            let old_kind = match old_kind {
                hm_model::LineSpacingKind::Percent => 0,
                hm_model::LineSpacingKind::Fixed => 1,
                hm_model::LineSpacingKind::Added => 2,
                hm_model::LineSpacingKind::AtLeast => 3,
            };
            // A value without an explicit kind retains the original API's percent meaning.
            let kind = self
                .line_spacing_kind
                .unwrap_or(if self.line_spacing.is_some() {
                    0
                } else {
                    old_kind
                });
            let value = self.line_spacing.unwrap_or(old_value as u32);
            s.line_spacing_legacy = value as i32;
            s.attr1 = (s.attr1 & !3) | kind.min(2);
            s.line_spacing = Some(value);
            s.attr3 = Some((s.attr3.unwrap_or(0) & !0x1F) | kind);
        }
    }
}

/// The operations applying `patch` to `[from, to)`, run by run, with the
/// shapes they need added to the document.
pub fn char_ops(
    doc: &mut Document,
    patch: &CharPatch,
    from: &Pos,
    to: &Pos,
) -> Result<Vec<Op>, EditError> {
    if from.list != to.list || (to.para, to.offset) < (from.para, from.offset) {
        return Err(EditError::BadRange);
    }
    let paras = crate::ops::list(doc, &from.list).ok_or(EditError::NoSuchList)?;
    let mut pieces = Vec::new();
    for pi in from.para..=to.para {
        let p = paras.get(pi as usize).ok_or(EditError::NoSuchParagraph)?;
        let s = if pi == from.para { from.offset } else { 0 };
        let e = if pi == to.para { to.offset } else { p.len() };
        let mut bounds: Vec<u32> = vec![s];
        bounds.extend(p.runs.iter().map(|r| r.pos).filter(|&q| q > s && q < e));
        bounds.push(e);
        for w in bounds.windows(2) {
            if w[1] > w[0] {
                pieces.push((pi, w[0], w[1], p.shape_at(w[0])));
            }
        }
    }
    let mut ops = Vec::new();
    for (pi, s, e, base) in pieces {
        let shape = derive_char_shape(doc, base, &|c| patch.apply(c));
        let at = |offset| Pos {
            list: from.list.clone(),
            para: pi,
            offset,
        };
        ops.push(Op::SetCharShape {
            from: at(s),
            to: at(e),
            shape,
        });
    }
    Ok(ops)
}

/// The operations applying `patch` to paragraphs `first..=last` of a list.
pub fn para_ops(
    doc: &mut Document,
    patch: &ParaPatch,
    list: &ListPath,
    first: u32,
    last: u32,
) -> Result<Vec<Op>, EditError> {
    let bases: Vec<u16> = {
        let paras = crate::ops::list(doc, list).ok_or(EditError::NoSuchList)?;
        (first..=last)
            .map(|i| paras.get(i as usize).map(|p| p.header.para_shape))
            .collect::<Option<_>>()
            .ok_or(EditError::NoSuchParagraph)?
    };
    Ok((first..=last)
        .zip(bases)
        .map(|(i, base)| Op::SetParaShape {
            list: list.clone(),
            first: i,
            last: i,
            shape: derive_para_shape(doc, base, &|s| patch.apply(s)),
        })
        .collect())
}

/// Applies character formatting from a script operation.
fn format(ed: &mut Editor, o: &Value, from: Pos, to: Pos) -> Result<(), ScriptError> {
    let patch = CharPatch {
        bold: o.get("bold").and_then(Value::as_bool),
        italic: o.get("italic").and_then(Value::as_bool),
        underline: o.get("underline").and_then(Value::as_bool),
        size: o.get("size").and_then(Value::as_i64).map(|v| v as i32),
        color: o.get("color").and_then(color),
        strike: o.get("strike").and_then(Value::as_bool),
        ..CharPatch::default()
    };
    let ops = char_ops(&mut ed.doc, &patch, &from, &to).map_err(|e| ScriptError(e.to_string()))?;
    ed.transact(ops)
        .map_err(|(_, e)| ScriptError(e.to_string()))
}

/// Applies paragraph formatting from a script operation.
fn para_format(
    ed: &mut Editor,
    o: &Value,
    list: ListPath,
    first: u32,
    last: u32,
) -> Result<(), ScriptError> {
    let align = match o.get("align").and_then(Value::as_str) {
        None => None,
        Some(a) => Some(align_code(a).ok_or_else(|| ScriptError(format!("unknown align {a:?}")))?),
    };
    let dist = |k: &str| o.get(k).and_then(Value::as_i64).map(|v| v as i32);
    let patch = ParaPatch {
        tab_def: None,
        line_spacing_kind: None,
        align,
        space_before: dist("space_before"),
        space_after: dist("space_after"),
        left: dist("left"),
        right: dist("right"),
        indent: dist("indent"),
        line_spacing: o
            .get("line_spacing")
            .and_then(Value::as_u64)
            .map(|v| v as u32),
    };
    let ops = para_ops(&mut ed.doc, &patch, &list, first, last)
        .map_err(|e| ScriptError(e.to_string()))?;
    ed.transact(ops)
        .map_err(|(_, e)| ScriptError(e.to_string()))
}

/// The stored alignment of a name.
pub fn align_code(name: &str) -> Option<u32> {
    Some(match name {
        "justify" => 0,
        "left" => 1,
        "right" => 2,
        "center" => 3,
        "distribute" => 4,
        "divide" => 5,
        _ => return None,
    })
}
