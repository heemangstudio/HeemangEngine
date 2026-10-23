//! Copying a range as text, HTML and a fragment, and pasting fragments
//! into the same or another document.
//!
//! A fragment keeps the text of each paragraph with its character shapes
//! and face names and the paragraph's shape. Pasting inserts the text and
//! then applies the shapes: as they are within the document copied from,
//! otherwise as copies laid over the shape at the paste point (faces found
//! or added by name; border fills and other references stay the target's).
//! Objects and tables are not carried; a control becomes nothing.

use hm_edit::script::derive_char_shape;
use hm_edit::{EditError, Op, Pos};
use hm_model::{CharShape, Document, FaceName, Unit};

#[derive(Clone)]
pub struct Run {
    pub text: String,
    pub shape: u32,
    pub look: CharShape,
    pub faces: [Option<FaceName>; 7],
}

#[derive(Clone)]
pub struct FragPara {
    pub runs: Vec<Run>,
    pub shape: u16,
}

#[derive(Clone)]
pub struct Fragment {
    /// The document instance copied from.
    pub instance: u32,
    pub paras: Vec<FragPara>,
}

impl Fragment {
    pub fn text(&self) -> String {
        self.paras
            .iter()
            .map(|p| p.runs.iter().map(|r| r.text.as_str()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn html(&self) -> String {
        let mut out = String::new();
        for p in &self.paras {
            out.push_str("<p>");
            for r in &p.runs {
                let mut style = format!("font-size:{}pt", r.look.base_size as f64 / 100.0);
                if let Some(f) = &r.faces[0] {
                    style.push_str(&format!(
                        ";font-family:'{}'",
                        escape(&f.name.to_string_lossy())
                    ));
                }
                if r.look.bold() {
                    style.push_str(";font-weight:bold");
                }
                if r.look.italic() {
                    style.push_str(";font-style:italic");
                }
                if r.look.underline_kind() != 0 {
                    style.push_str(";text-decoration:underline");
                }
                let c = r.look.color;
                if c & 0xFF_FFFF != 0 {
                    style.push_str(&format!(
                        ";color:#{:02x}{:02x}{:02x}",
                        c & 0xFF,
                        (c >> 8) & 0xFF,
                        (c >> 16) & 0xFF
                    ));
                }
                let text = escape(&r.text).replace('\t', "&#9;");
                out.push_str(&format!("<span style=\"{style}\">{text}</span>"));
            }
            out.push_str("</p>");
        }
        out
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// The paragraphs of `[from, to)` as a fragment.
pub fn copy(doc: &Document, instance: u32, from: &Pos, to: &Pos) -> Result<Fragment, EditError> {
    if from.list != to.list || (to.para, to.offset) < (from.para, from.offset) {
        return Err(EditError::BadRange);
    }
    let paras = hm_edit::ops::list(doc, &from.list).ok_or(EditError::NoSuchList)?;
    let mut out = Vec::new();
    for pi in from.para..=to.para {
        let p = paras.get(pi as usize).ok_or(EditError::NoSuchParagraph)?;
        let start = if pi == from.para { from.offset } else { 0 };
        let end = if pi == to.para { to.offset } else { p.len() };
        let mut runs: Vec<Run> = Vec::new();
        for (pos, u) in p.units() {
            if pos < start || pos >= end {
                continue;
            }
            let ch = match u {
                Unit::Char(c) => c,
                Unit::Inline(9) => '\t',
                Unit::CharControl(30) | Unit::CharControl(31) => ' ',
                _ => continue,
            };
            let shape = p.shape_at(pos);
            match runs.last_mut() {
                Some(r) if r.shape == shape => r.text.push(ch),
                _ => {
                    let look = doc.info.char_shape(shape).cloned().unwrap_or_default();
                    let faces = std::array::from_fn(|k| {
                        doc.info.faces[k].get(look.faces[k] as usize).cloned()
                    });
                    runs.push(Run {
                        text: ch.to_string(),
                        shape,
                        look,
                        faces,
                    });
                }
            }
        }
        let shape = p.header.para_shape;
        out.push(FragPara { runs, shape });
    }
    Ok(Fragment {
        instance,
        paras: out,
    })
}

/// Wire units a pasted char takes: a tab is an 8-unit control.
fn units(c: char) -> u32 {
    match c {
        '\t' => 8,
        c => c.len_utf16() as u32,
    }
}

/// The shape of `r` in `doc`: its own id in the document it came from,
/// else a copy laid over `base` with faces found or added by name.
fn shape_for(doc: &mut Document, same: bool, r: &Run, base: u32) -> u32 {
    if same {
        return r.shape;
    }
    let mut faces = [0u16; 7];
    for (k, f) in r.faces.iter().enumerate() {
        let Some(f) = f else { continue };
        let list = &mut doc.info.faces[k];
        faces[k] = match list.iter().position(|e| e.name == f.name) {
            Some(i) => i as u16,
            None => {
                let mut f = f.clone();
                f.src = None;
                list.push(f);
                doc.info.id_mappings.src = None;
                (list.len() - 1) as u16
            }
        };
    }
    let l = &r.look;
    derive_char_shape(doc, base, &|c: &mut CharShape| {
        c.faces = faces;
        c.ratios = l.ratios;
        c.spacings = l.spacings;
        c.rel_sizes = l.rel_sizes;
        c.offsets = l.offsets;
        c.base_size = l.base_size;
        c.attr = l.attr;
        c.shadow_offset = l.shadow_offset;
        c.color = l.color;
        c.underline_color = l.underline_color;
        c.shade_color = l.shade_color;
        c.shadow_color = l.shadow_color;
        c.strike_color = l.strike_color;
    })
}

/// The operations pasting `frag` at `at`, and the position after it. The
/// first pasted paragraph keeps the target paragraph's shape; the others
/// take theirs within the same document.
pub fn paste(
    doc: &mut Document,
    instance: u32,
    frag: &Fragment,
    at: &Pos,
) -> Result<(Vec<Op>, Pos), EditError> {
    let paras = hm_edit::ops::list(doc, &at.list).ok_or(EditError::NoSuchList)?;
    let base = paras
        .get(at.para as usize)
        .ok_or(EditError::NoSuchParagraph)?
        .shape_at(at.offset.saturating_sub(1));
    let same = frag.instance == instance;
    let text = frag.text();
    let mut ops = vec![Op::InsertText {
        at: at.clone(),
        text,
    }];
    let mut pos = at.clone();
    for (k, p) in frag.paras.iter().enumerate() {
        if k > 0 {
            pos.para += 1;
            pos.offset = 0;
            if same {
                ops.push(Op::SetParaShape {
                    list: pos.list.clone(),
                    first: pos.para,
                    last: pos.para,
                    shape: p.shape,
                });
            }
        }
        for r in &p.runs {
            let len: u32 = r.text.chars().map(units).sum();
            let shape = shape_for(doc, same, r, base);
            let from = pos.clone();
            pos.offset += len;
            ops.push(Op::SetCharShape {
                from,
                to: pos.clone(),
                shape,
            });
        }
    }
    Ok((ops, pos))
}
