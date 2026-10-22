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
//! cell], ...]}` in wire units, or `{"quote", "occurrence", "side"}`
//! searching body and cell paragraphs in document order (occurrence counts
//! from 1; side is `before` or `after` the quote).

use hm_model::{CharShape, Document, ParaShape, Unit};
use serde_json::Value;

use crate::Editor;
use crate::ops::{CellStep, ListPath, Op, Pos};

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
                    Some(CellStep {
                        para: s.first()?.as_u64()? as u32,
                        control: s.get(1)?.as_u64()? as u32,
                        cell: s.get(2)?.as_u64()? as u32,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Pos {
        list: ListPath {
            section: num("section").unwrap_or(0),
            cells,
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
            cells: Vec::new(),
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
                sub.cells.push(CellStep {
                    para: pi as u32,
                    control: ci as u32,
                    cell: k as u32,
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

/// Applies character formatting to `[from, to)`, run by run.
fn format(ed: &mut Editor, o: &Value, from: Pos, to: Pos) -> Result<(), ScriptError> {
    let bold = o.get("bold").and_then(Value::as_bool);
    let italic = o.get("italic").and_then(Value::as_bool);
    let underline = o.get("underline").and_then(Value::as_bool);
    let size = o.get("size").and_then(Value::as_i64).map(|v| v as i32);
    let col = o.get("color").and_then(color);
    let change = move |c: &mut CharShape| {
        let mut set = |bit: u32, on: Option<bool>| match on {
            Some(true) => c.attr |= 1 << bit,
            Some(false) => c.attr &= !(1 << bit),
            None => {}
        };
        set(0, italic);
        set(1, bold);
        if let Some(u) = underline {
            c.attr &= !(3 << 2);
            if u {
                c.attr |= 1 << 2;
            }
        }
        if let Some(s) = size {
            c.base_size = s;
        }
        if let Some(k) = col {
            c.color = k;
        }
    };
    // The shapes in the range, each derived once.
    let paras =
        crate::ops::list(&ed.doc, &from.list).ok_or_else(|| ScriptError("no such list".into()))?;
    let mut pieces = Vec::new();
    for pi in from.para..=to.para {
        let p = paras
            .get(pi as usize)
            .ok_or_else(|| ScriptError("no such paragraph".into()))?;
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
    for (pi, s, e, base) in pieces {
        let shape = derive_char_shape(&mut ed.doc, base, &change);
        let at = |offset| Pos {
            list: from.list.clone(),
            para: pi,
            offset,
        };
        ed.apply(Op::SetCharShape {
            from: at(s),
            to: at(e),
            shape,
        })
        .map_err(|e| ScriptError(e.to_string()))?;
    }
    Ok(())
}

/// Applies paragraph formatting to paragraphs `first..=last`.
fn para_format(
    ed: &mut Editor,
    o: &Value,
    list: ListPath,
    first: u32,
    last: u32,
) -> Result<(), ScriptError> {
    let align = match o.get("align").and_then(Value::as_str) {
        None => None,
        Some("justify") => Some(0),
        Some("left") => Some(1),
        Some("right") => Some(2),
        Some("center") => Some(3),
        Some("distribute") => Some(4),
        Some("divide") => Some(5),
        Some(a) => return err(format!("unknown align {a:?}")),
    };
    // Distances in HU, stored packed (shifted left once).
    let dist = |k: &str| o.get(k).and_then(Value::as_i64).map(|v| (v as i32) << 1);
    let (before, after, left, indent) = (
        dist("space_before"),
        dist("space_after"),
        dist("left"),
        dist("indent"),
    );
    let line = o.get("line_spacing").and_then(Value::as_i64);
    let change = move |s: &mut ParaShape| {
        if let Some(a) = align {
            s.attr1 = s.attr1 & !(7 << 2) | (a as u32) << 2;
        }
        if let Some(v) = before {
            s.space_before = v;
        }
        if let Some(v) = after {
            s.space_after = v;
        }
        if let Some(v) = left {
            s.margin_left = v;
        }
        if let Some(v) = indent {
            s.indent = v;
        }
        if let Some(v) = line {
            // Percent spacing in both the current and the legacy fields.
            s.line_spacing_legacy = v as i32;
            s.attr1 &= !3;
            if s.line_spacing.is_some() {
                s.line_spacing = Some(v as u32);
                s.attr3 = s.attr3.map(|a| a & !0x1F);
            }
        }
    };
    let bases: Vec<u16> = {
        let paras =
            crate::ops::list(&ed.doc, &list).ok_or_else(|| ScriptError("no such list".into()))?;
        (first..=last)
            .map(|i| paras.get(i as usize).map(|p| p.header.para_shape))
            .collect::<Option<_>>()
            .ok_or_else(|| ScriptError("no such paragraph".into()))?
    };
    for (i, base) in (first..=last).zip(bases) {
        let shape = derive_para_shape(&mut ed.doc, base, &change);
        ed.apply(Op::SetParaShape {
            list: list.clone(),
            first: i,
            last: i,
            shape,
        })
        .map_err(|e| ScriptError(e.to_string()))?;
    }
    Ok(())
}
