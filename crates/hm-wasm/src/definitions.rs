//! Host-authored style and list definitions, recorded as normal history edits.
use hm_edit::{ListPath, Op};
use hm_model::{DocInfo, Document, Paragraph};

use crate::types::{EngineError, ListDefinition, Result, StylePatch};

fn bad(message: &str) -> EngineError {
    EngineError::new("BAD_REQUEST", message)
}

fn replace(mut info: DocInfo) -> Op {
    // Counts and definition indexes must be re-encoded even on deletion.
    info.id_mappings.src = None;
    Op::SetDocInfo {
        info: Box::new(info),
    }
}

pub fn style(
    doc: &mut Document,
    fonts: &hm_fonts::FontSystem,
    id: Option<u32>,
    patch: StylePatch,
) -> Result<Vec<Op>> {
    let original_info = doc.info.clone();
    let creating = id.is_none();
    let id = id.unwrap_or(doc.info.styles.len() as u32);
    if id > 255 || (!creating && id as usize >= doc.info.styles.len()) {
        return Err(bad("no such style or style table is full"));
    }
    if creating && patch.name.as_ref().is_none_or(|s| s.trim().is_empty()) {
        return Err(bad("new style needs a name"));
    }
    let mut s = if creating {
        hm_model::Style {
            char_shape: doc.info.styles.first().map_or(0, |s| s.char_shape),
            para_shape: doc.info.styles.first().map_or(0, |s| s.para_shape),
            next: id as u8,
            ..Default::default()
        }
    } else {
        doc.info.styles[id as usize].clone()
    };
    if let Some(name) = patch.name {
        if name.trim().is_empty() {
            return Err(bad("style name is empty"));
        }
        s.local_name = hm_model::HwpString::from_text(&name);
    }
    if let Some(name) = patch.english_name {
        s.english_name = hm_model::HwpString::from_text(&name);
    }
    if let Some(kind) = patch.kind {
        let kind = match kind.as_str() {
            "paragraph" => 0,
            "character" => 1,
            _ => return Err(bad("unknown style kind")),
        };
        s.attr = (s.attr & !7) | kind;
    }
    if let Some(next) = patch.next {
        s.next = next;
    }
    if creating && patch.next.is_none() {
        s.next = id as u8;
    }
    if let Some(shape) = patch.char_shape_id {
        s.char_shape = shape;
    }
    if let Some(shape) = patch.para_shape_id {
        s.para_shape = shape;
    }
    if s.char_shape as usize >= doc.info.char_shapes.len()
        || s.para_shape as usize >= doc.info.para_shapes.len()
        || s.next as usize >= doc.info.styles.len() + usize::from(creating)
    {
        return Err(bad("style references a missing shape or next style"));
    }
    if let Some(patch) = patch.characters {
        let patch = crate::Inner::char_patch(doc, fonts, &patch)?;
        let id = hm_edit::script::derive_char_shape(doc, s.char_shape as u32, &|c| patch.apply(c));
        s.char_shape = u16::try_from(id).map_err(|_| bad("style character shape table is full"))?;
    }
    if let Some(patch) = patch.paragraphs {
        let (mut patch, tabs) = crate::Inner::para_patch(doc, &patch)?;
        if let Some(tabs) = tabs {
            patch.tab_def = Some(crate::Inner::tab_def(doc, s.para_shape, &tabs)?);
        }
        s.para_shape = hm_edit::script::derive_para_shape(doc, s.para_shape, &|p| patch.apply(p));
    }
    s.src = None;
    // Text in a redefined paragraph style follows it attribute by attribute, keeping what
    // it sets itself (Hancom's StyleChangeToCurrentShape).
    let mut restyle = Vec::new();
    if let Some(old) = original_info.styles.get(id as usize).filter(|_| !creating)
        && old.attr & 7 == 0
        && (old.para_shape, old.char_shape) != (s.para_shape, s.char_shape)
    {
        let from = (old.para_shape, old.char_shape as u32);
        let to = (s.para_shape, s.char_shape as u32);
        for (list, para, p) in styled(doc, id) {
            let q = hm_edit::script::restyled(doc, &p, id as u8, from, to);
            if q.src.is_none() {
                restyle.push(Op::Replace {
                    list,
                    start: para,
                    count: 1,
                    paras: vec![q],
                });
            }
        }
    }
    let mut info = doc.info.clone();
    if creating {
        info.styles.push(s);
    } else {
        info.styles[id as usize] = s;
    }
    doc.info = original_info;
    let mut ops = vec![replace(info)];
    ops.extend(restyle);
    Ok(ops)
}

/// Every paragraph (nested lists included) in style `id`, with its list and index.
fn styled(doc: &Document, id: u32) -> Vec<(ListPath, u32, Paragraph)> {
    all_paragraphs(doc)
        .into_iter()
        .filter(|(_, _, p)| p.header.style as u32 == id)
        .map(|(list, para, p)| (list, para, p.clone()))
        .collect()
}

fn all_paragraphs(doc: &Document) -> Vec<(ListPath, u32, &Paragraph)> {
    let mut all = Vec::new();
    for (section, s) in doc.sections.iter().enumerate() {
        paragraphs(
            &s.paras,
            &ListPath {
                section: section as u32,
                steps: Vec::new(),
            },
            &mut all,
        );
    }
    all
}

fn paragraphs<'a>(
    paras: &'a [Paragraph],
    path: &ListPath,
    out: &mut Vec<(ListPath, u32, &'a Paragraph)>,
) {
    for (pi, p) in paras.iter().enumerate() {
        out.push((path.clone(), pi as u32, p));
        for (ci, c) in p.controls.iter().enumerate() {
            for (index, list) in hm_edit::ops::control_lists(c).into_iter().enumerate() {
                let mut child = path.clone();
                child.steps.push(hm_edit::Step {
                    para: pi as u32,
                    control: ci as u32,
                    index: index as u32,
                });
                paragraphs(list, &child, out);
            }
        }
    }
}

/// Deletes style `id`. Its paragraphs take style `replacement` (an id from before the
/// deletion), following it like a redefinition; without one, a style in use is refused.
/// Styles naming it as their next style name themselves instead, as in Hancom.
pub fn delete_style(doc: &mut Document, id: u32, replacement: Option<u32>) -> Result<Vec<Op>> {
    if id == 0 || id as usize >= doc.info.styles.len() {
        return Err(bad("cannot delete style zero or a missing style"));
    }
    let to = match replacement {
        Some(r) if r == id => return Err(bad("a style cannot replace itself")),
        Some(r) => match doc.info.styles.get(r as usize) {
            Some(s) if s.attr & 7 == 0 => Some((r, s.para_shape, s.char_shape as u32)),
            _ => return Err(bad("the replacement is not a paragraph style")),
        },
        None => None,
    };
    let shift = |i: u32| if i > id { i - 1 } else { i };
    // Paragraphs in this style or a later one, in document order: an enclosing paragraph
    // is replaced before the lists it holds are edited.
    let affected: Vec<(ListPath, u32, Paragraph)> = all_paragraphs(doc)
        .into_iter()
        .filter(|(_, _, p)| p.header.style as u32 >= id)
        .map(|(list, para, p)| (list, para, p.clone()))
        .collect();
    let used = affected.iter().any(|(_, _, p)| p.header.style as u32 == id);
    if used && to.is_none() {
        return Err(bad("style is still used; name a replacement"));
    }
    let original_info = doc.info.clone();
    let gone = &original_info.styles[id as usize];
    let from = (gone.para_shape, gone.char_shape as u32);
    let mut ops = Vec::new();
    for (list, para, p) in affected {
        let op = match to {
            Some((r, para_shape, char_shape)) if p.header.style as u32 == id => {
                let to = (para_shape, char_shape);
                let q = hm_edit::script::restyled(doc, &p, shift(r) as u8, from, to);
                Op::Replace {
                    list,
                    start: para,
                    count: 1,
                    paras: vec![q],
                }
            }
            _ => Op::SetStyle {
                list,
                first: para,
                last: para,
                style: p.header.style - 1,
                para_shape: p.header.para_shape,
                char_shape: None,
            },
        };
        ops.push(op);
    }
    let mut info = doc.info.clone();
    doc.info = original_info;
    info.styles.remove(id as usize);
    for (i, s) in info.styles.iter_mut().enumerate() {
        let i = i as u32;
        let next = s.next as u32;
        let next = if next == id { i } else { shift(next) };
        if next != s.next as u32 {
            s.next = next as u8;
            s.src = None;
        }
    }
    info.order
        .retain(|slot| !matches!(slot.item, hm_model::InfoItem::Style(i) if i == id));
    for slot in &mut info.order {
        if let hm_model::InfoItem::Style(i) = &mut slot.item {
            if *i > id {
                *i -= 1;
            }
        }
    }
    let mut out = vec![replace(info)];
    out.extend(ops);
    Ok(out)
}

pub fn ensure_list(doc: &Document, definition: ListDefinition) -> Result<Vec<Op>> {
    let mut info = doc.info.clone();
    match definition {
        ListDefinition::Bullet { bullet } => {
            let units: Vec<_> = bullet.encode_utf16().collect();
            if units.len() != 1 || bullet.chars().any(char::is_control) {
                return Err(bad("bullet must be one non-control BMP character"));
            }
            let b = hm_model::Bullet {
                bullet_char: units[0],
                ..Default::default()
            };
            if info.bullets.iter().any(|old| {
                old.bullet_char == b.bullet_char
                    && old.head == b.head
                    && old.image.is_none()
                    && old.check_char.is_none()
            }) {
                return Ok(Vec::new());
            }
            if info.bullets.len() >= u16::MAX as usize {
                return Err(bad("bullet table is full"));
            }
            info.bullets.push(b);
        }
        ListDefinition::Numbering { start, levels } => {
            if levels.is_empty() || levels.len() > 7 {
                return Err(bad("numbering needs one to seven levels"));
            }
            let start = start.unwrap_or(1);
            let mut starts = [start as u32; 7];
            let mut n = hm_model::Numbering {
                start: Some(start),
                ..Default::default()
            };
            for (i, level) in levels.into_iter().enumerate() {
                let kind = level.number_kind.unwrap_or(0);
                if kind > 15 || level.format.encode_utf16().count() > 1024 {
                    return Err(bad("invalid numbering format or numberKind"));
                }
                starts[i] = level.start.unwrap_or(start as u32);
                n.levels.push(hm_model::NumberingLevel {
                    head: hm_model::ParaHead {
                        attr: kind << 5,
                        ..Default::default()
                    },
                    format: hm_model::HwpString::from_text(&level.format),
                });
            }
            for i in n.levels.len()..7 {
                n.levels.push(hm_model::NumberingLevel {
                    format: hm_model::HwpString::from_text(&format!("^{}.", i + 1)),
                    ..Default::default()
                });
            }
            n.level_starts = Some(starts);
            if info.numberings.iter().any(|old| {
                old.start.unwrap_or(1) == start
                    && old
                        .level_starts
                        .unwrap_or([old.start.unwrap_or(1) as u32; 7])
                        == starts
                    && old.levels.len() == n.levels.len()
                    && old
                        .levels
                        .iter()
                        .zip(&n.levels)
                        .all(|(a, b)| a.head == b.head && a.format == b.format)
            }) {
                return Ok(Vec::new());
            }
            if info.numberings.len() >= u16::MAX as usize {
                return Err(bad("numbering table is full"));
            }
            info.numberings.push(n);
        }
    }
    Ok(vec![replace(info)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_patches_preserve_unexposed_fields_and_explicit_bases() {
        let mut doc = hm_hwpx::read(hm_model::SharedBytes::from(
            include_bytes!("../tests/fixtures/query-equations.hwpx").to_vec(),
        ))
        .unwrap();
        doc.info.char_shapes = vec![hm_model::CharShape {
            base_size: 1100,
            attr: 1 << 15,
            border_fill: Some(7),
            strike_color: Some(0x123456),
            ratios: [100; 7],
            ..Default::default()
        }];
        doc.info.para_shapes = vec![hm_model::ParaShape {
            attr1: 1 << 18,
            numbering: 3,
            ..Default::default()
        }];
        doc.info.styles = vec![hm_model::Style::default()];
        let patch = serde_json::from_value(serde_json::json!({
            "name": "Derived", "charShapeId": 0, "paraShapeId": 0,
            "characters": {"bold": true}, "paragraphs": {"right": 600}
        }))
        .unwrap();
        let ops = style(&mut doc, &hm_fonts::FontSystem::new(), None, patch).unwrap();
        // Translation leaves document tables untouched until the operation commits.
        assert_eq!(doc.info.char_shapes.len(), 1);
        assert_eq!(doc.info.styles.len(), 1);
        for op in ops {
            hm_edit::apply(&mut doc, &op).unwrap();
        }
        let s = &doc.info.styles[1];
        let c = &doc.info.char_shapes[s.char_shape as usize];
        assert!(c.bold());
        assert_eq!(c.attr & (1 << 15), 1 << 15);
        assert_eq!(c.base_size, 1100);
        assert_eq!(c.border_fill, Some(7));
        assert_eq!(c.strike_color, Some(0x123456));
        let p = &doc.info.para_shapes[s.para_shape as usize];
        assert!(p.keep_lines());
        assert_eq!(p.numbering, 3);
        assert_eq!(p.right(1100), 600);
    }
}
