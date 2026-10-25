//! Connector attachment IDs and endpoint updates from the current laid-out boxes.
use super::*;
use hm_model::{Connector, Control, ControlKind, CtrlId, Document, ObjectPart, ShapePart};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct ConnectorSpec {
    pub kind: u16,
    pub start: (u32, u32),
    pub end: (u32, u32),
}

pub fn next_id(doc: &Document) -> u32 {
    let mut max = 0;
    fn walk(paras: &[hm_model::Paragraph], max: &mut u32) {
        for p in paras {
            for c in &p.controls {
                if let Some(o) = c.as_object() {
                    *max = (*max).max(o.header.instance_id.unwrap_or(0));
                    if let Some(s) = o.shape() {
                        *max = (*max).max(
                            s.component
                                .style
                                .as_ref()
                                .and_then(|s| s.instance_id)
                                .unwrap_or(0),
                        );
                    }
                }
                for list in hm_edit::ops::control_lists(c) {
                    walk(list, max);
                }
            }
        }
    }
    for sec in &doc.sections {
        walk(&sec.paras, &mut max);
    }
    max.saturating_add(1).max(1)
}

pub fn attach(c: &mut Control, spec: ConnectorSpec) -> Result<()> {
    if spec.kind > 8 || spec.start.1 > 3 || spec.end.1 > 3 {
        return Err(EngineError::new(
            "BAD_REQUEST",
            "invalid connector attachment",
        ));
    }
    let ControlKind::Object(o) = &mut c.kind else {
        unreachable!()
    };
    for p in &mut o.parts {
        if let ObjectPart::Shape(s) = p {
            s.component.kind = CtrlId::from_chars(b"$col");
            if let Some(style) = &mut s.component.style {
                if spec.kind % 3 != 0 {
                    style.line.attr |= 1 << 16;
                }
                if spec.kind % 3 == 2 {
                    style.line.attr |= 1 << 10;
                }
            }
            for p in &mut s.parts {
                if let ShapePart::Line(l) = p {
                    l.attr = Some(spec.kind);
                    let points = if spec.kind >= 3 {
                        let mid = (l.start.0 + l.end.0) / 2;
                        vec![
                            (l.start.0, l.start.1, 0),
                            (mid, l.start.1, 0),
                            (mid, l.end.1, 0),
                            (l.end.0, l.end.1, 0),
                        ]
                    } else {
                        Vec::new()
                    };
                    l.connector = Some(Connector {
                        start: spec.start,
                        end: spec.end,
                        points,
                    });
                }
            }
        }
    }
    Ok(())
}

fn point(box_: grouping::LaidBox, index: u32) -> (i32, i32) {
    let (_, x, y, w, h) = box_;
    match index {
        0 => (x + w / 2, y),
        1 => (x + w, y + h / 2),
        2 => (x + w / 2, y + h),
        _ => (x, y + h / 2),
    }
}

#[wasm_bindgen]
impl Doc {
    /// Rebinds the section's attached connector endpoints as one undoable edit.
    #[wasm_bindgen(js_name = updateConnectors)]
    pub fn update_connectors(&mut self, section: u32) -> Result<String> {
        let d = self.inner()?;
        d.not_composing()?;
        let sec = d
            .editor
            .doc
            .sections
            .get(section as usize)
            .ok_or_else(|| EngineError::new("BAD_ADDRESS", "no such section"))?;
        let boxes = d.object_boxes();
        let mut subjects = std::collections::HashMap::new();
        for (pi, p) in sec.paras.iter().enumerate() {
            for (ci, c) in p.controls.iter().enumerate() {
                let Some(box_) = boxes.get(&(section, pi as u32, ci as u32)) else {
                    continue;
                };
                if let Some(o) = c.as_object() {
                    for id in [
                        o.header.instance_id,
                        o.shape()
                            .and_then(|s| s.component.style.as_ref().and_then(|s| s.instance_id)),
                    ]
                    .into_iter()
                    .flatten()
                    .filter(|id| *id != 0)
                    {
                        subjects.insert(id, *box_);
                    }
                }
            }
        }
        let mut changes: std::collections::BTreeMap<usize, hm_model::Paragraph> =
            std::collections::BTreeMap::new();
        for (pi, p) in sec.paras.iter().enumerate() {
            for (ci, c) in p.controls.iter().enumerate() {
                let Some(s) = c.as_object().and_then(|o| o.shape()) else {
                    continue;
                };
                let Some(ShapePart::Line(line)) = s.kind() else {
                    continue;
                };
                let Some(refs) = &line.connector else {
                    continue;
                };
                let Some(&(page, x, y, _, _)) = boxes.get(&(section, pi as u32, ci as u32)) else {
                    continue;
                };
                let endpoint = |subject: (u32, u32), old: (i32, i32)| {
                    subjects
                        .get(&subject.0)
                        .filter(|b| b.0 == page)
                        .map(|b| point(*b, subject.1))
                        .unwrap_or_else(|| {
                            let (sx, sy) = s.component.matrix().apply(old.0 as f64, old.1 as f64);
                            (x + sx.round() as i32, y + sy.round() as i32)
                        })
                };
                let a = endpoint(refs.start, line.start);
                let b = endpoint(refs.end, line.end);
                let current = |old: (i32, i32)| {
                    let (ox, oy) = s.component.matrix().apply(old.0 as f64, old.1 as f64);
                    (x + ox.round() as i32, y + oy.round() as i32)
                };
                if a == current(line.start) && b == current(line.end) {
                    continue;
                }
                let at = types::TableAddress {
                    section,
                    steps: Vec::new(),
                    para: pi as u32,
                    control: ci as u32,
                };
                let mut working = d.editor.doc.clone();
                if let Some(para) = changes.get(&pi) {
                    working.sections[section as usize].paras[pi] = para.clone();
                }
                let op =
                    grouping::set_line_ends(&working, &at, (a.0 - x, a.1 - y), (b.0 - x, b.1 - y))?;
                let Op::Replace { mut paras, .. } = op else {
                    unreachable!()
                };
                let mut para = paras.remove(0);
                if let ControlKind::Object(o) = &mut para.controls[ci].kind {
                    for p in &mut o.parts {
                        if let ObjectPart::Shape(s) = p {
                            for p in &mut s.parts {
                                if let ShapePart::Line(l) = p {
                                    l.attr = line.attr;
                                    if let Some(c) = &mut l.connector {
                                        c.points = if line.attr.unwrap_or(0) >= 3 {
                                            let mid = (l.start.0 + l.end.0) / 2;
                                            vec![
                                                (l.start.0, l.start.1, 0),
                                                (mid, l.start.1, 0),
                                                (mid, l.end.1, 0),
                                                (l.end.0, l.end.1, 0),
                                            ]
                                        } else {
                                            Vec::new()
                                        };
                                    }
                                }
                            }
                        }
                    }
                }
                changes.insert(pi, para);
            }
        }
        if changes.is_empty() {
            return Ok("null".into());
        }
        let ops = changes
            .into_iter()
            .map(|(pi, p)| Op::Replace {
                list: hm_edit::ListPath {
                    section,
                    steps: Vec::new(),
                },
                start: pi as u32,
                count: 1,
                paras: vec![p],
            })
            .collect();
        let before = d.page_sigs();
        d.editor
            .transact(ops)
            .map_err(|(i, e)| Inner::edit_error(i, e))?;
        d.content_epoch += 1;
        d.relayout();
        let scopes = d.editor.changed().map(<[_]>::to_vec);
        Ok(json(&d.receipt_after(
            None,
            &before,
            scopes.as_deref(),
            None,
        )))
    }
}
