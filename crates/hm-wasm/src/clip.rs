//! Copying a range as text, HTML and a fragment, and pasting fragments
//! into the same or another document.
//!
//! A fragment keeps the text of each paragraph with its character shapes
//! and face names and the paragraph's shape. Pasting inserts the text and
//! then applies the shapes: as they are within the document copied from,
//! otherwise with the definitions and binary resources they reference
//! imported into the destination document.
//! Line breaks and the nonbreaking and fixed-width spaces keep their
//! units (as U+2028, U+00A0 and U+2007 in a run); plain text shows a line
//! break as a newline and HTML as `<br>`. Tables, drawings, pictures,
//! equations and form objects travel whole, including to another document;
//! the source snapshot keeps their resources alive after source closure.

use hm_edit::{EditError, Op, Pos};
use hm_model::{CharShape, Control, ControlKind, Document, FaceName, Unit};

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
    /// Objects with their wire offsets in the pasted paragraph: the text
    /// before them plus 8 for each object before them.
    pub objects: Vec<(u32, Control)>,
}

#[derive(Clone)]
pub struct Fragment {
    /// The document instance copied from.
    pub instance: u32,
    pub source: std::sync::Arc<Document>,
    pub paras: Vec<FragPara>,
}

impl Fragment {
    /// How many objects the fragment carries.
    pub fn objects(&self) -> usize {
        self.paras.iter().map(|p| p.objects.len()).sum()
    }

    /// Plain text: a line break is a newline as between paragraphs.
    pub fn text(&self) -> String {
        self.typed().replace('\u{2028}', "\n")
    }

    /// The text to insert: paragraphs split at newlines, line breaks kept.
    fn typed(&self) -> String {
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
            let mut wire = 0;
            let mut objects = p.objects.iter().peekable();
            for r in &p.runs {
                let style = look_style(&r.look, r.faces[0].as_ref());
                out.push_str(&format!("<span style=\"{style}\">"));
                for ch in r.text.chars() {
                    while objects.peek().is_some_and(|(offset, _)| *offset == wire) {
                        let (_, control) = objects.next().unwrap();
                        out.push_str("</span>");
                        out.push_str(&control_html(&self.source, control));
                        out.push_str(&format!("<span style=\"{style}\">"));
                        wire += 8;
                    }
                    out.push_str(&html_char(ch));
                    wire += units(ch);
                }
                out.push_str("</span>");
            }
            for (_, control) in objects {
                out.push_str(&control_html(&self.source, control));
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

fn look_style(look: &CharShape, face: Option<&FaceName>) -> String {
    let mut style = format!("font-size:{}pt", look.base_size as f64 / 100.0);
    if let Some(f) = face {
        style.push_str(&format!(
            ";font-family:'{}'",
            escape(&f.name.to_string_lossy())
        ));
    }
    if look.bold() {
        style.push_str(";font-weight:bold");
    }
    if look.italic() {
        style.push_str(";font-style:italic");
    }
    let underline = matches!(look.underline_kind(), 1 | 3);
    let strike = look.strike() != 0 && (look.attr >> 26) & 15 != 15;
    if underline {
        style.push_str(";text-decoration:underline");
    }
    if strike {
        if underline {
            style.push_str(" line-through");
        } else {
            style.push_str(";text-decoration:line-through");
        }
    }
    let c = look.color;
    if c & 0xFF_FFFF != 0 {
        style.push_str(&format!(
            ";color:#{:02x}{:02x}{:02x}",
            c & 0xFF,
            (c >> 8) & 0xFF,
            (c >> 16) & 0xFF
        ));
    }
    style
}
fn html_char(ch: char) -> String {
    match ch {
        '\t' => "&#9;".into(),
        '\u{2028}' => "<br>".into(),
        '\u{a0}' => "&nbsp;".into(),
        '\u{2007}' => "&#8199;".into(),
        ch => escape(&ch.to_string()),
    }
}
fn paragraph_html(doc: &Document, p: &hm_model::Paragraph) -> String {
    let mut out = String::from("<p>");
    let mut shape = None;
    for (offset, unit) in p.units() {
        let ch = match unit {
            Unit::Char(ch) => Some(ch),
            Unit::Inline(9) => Some('\t'),
            Unit::CharControl(10) => Some('\u{2028}'),
            Unit::Extended { index, .. } => {
                if shape.take().is_some() {
                    out.push_str("</span>");
                }
                out.push_str(&control_html(doc, &p.controls[index as usize]));
                None
            }
            _ => None,
        };
        if let Some(ch) = ch {
            let next = p.shape_at(offset);
            if shape != Some(next) {
                if shape.is_some() {
                    out.push_str("</span>");
                }
                let look = doc.info.char_shape(next).cloned().unwrap_or_default();
                let face = doc.info.faces[0].get(look.faces[0] as usize);
                out.push_str(&format!("<span style=\"{}\">", look_style(&look, face)));
                shape = Some(next);
            }
            out.push_str(&html_char(ch));
        }
    }
    if shape.is_some() {
        out.push_str("</span>");
    }
    out.push_str("</p>");
    out
}

fn control_html(doc: &Document, c: &Control) -> String {
    if let ControlKind::Table(t) = &c.kind {
        let mut out = String::from("<table>");
        for row in 0..t.rows {
            out.push_str("<tr>");
            let mut cells: Vec<_> = t.cells.iter().filter(|c| c.row == row).collect();
            cells.sort_by_key(|c| c.col);
            for cell in cells {
                out.push_str(&format!(
                    "<td rowspan=\"{}\" colspan=\"{}\">",
                    cell.row_span, cell.col_span
                ));
                for p in &cell.paras {
                    out.push_str(&paragraph_html(doc, p));
                }
                out.push_str("</td>");
            }
            out.push_str("</tr>");
        }
        out.push_str("</table>");
        return out;
    }
    if let Some(o) = c.as_object() {
        if let Some(pic) = o.shape().and_then(|s| {
            s.parts.iter().find_map(|p| {
                if let hm_model::ShapePart::Picture(p) = p {
                    Some(p)
                } else {
                    None
                }
            })
        }) {
            if let Some(bytes) = doc.bins.as_ref().and_then(|b| b.get(pic.image.bin_id)) {
                let ext = doc
                    .info
                    .bin_data
                    .get(pic.image.bin_id as usize - 1)
                    .and_then(|e| e.extension.as_ref())
                    .map(|s| s.to_string_lossy())
                    .unwrap_or_else(|| "png".into());
                let mime = match ext.to_ascii_lowercase().as_str() {
                    "jpg" | "jpeg" => "jpeg".to_string(),
                    "svg" => "svg+xml".to_string(),
                    other => other.to_string(),
                };
                return format!(
                    "<img width=\"{}\" height=\"{}\" src=\"data:image/{};base64,{}\">",
                    o.header.common.width / 75,
                    o.header.common.height / 75,
                    escape(&mime),
                    hm_render::svg::base64(&bytes)
                );
            }
        }
    }
    String::new()
}

/// The paragraphs of `[from, to)` as a fragment.
pub fn copy(doc: &Document, instance: u32, from: &Pos, to: &Pos) -> Result<Fragment, EditError> {
    if from.list.section != to.list.section
        && from.list.steps.is_empty()
        && to.list.steps.is_empty()
    {
        if from.list.section > to.list.section {
            return Err(EditError::BadRange);
        }
        let mut paras = Vec::new();
        for section in from.list.section..=to.list.section {
            let list = hm_edit::ListPath {
                section,
                steps: Vec::new(),
            };
            let source = hm_edit::ops::list(doc, &list).ok_or(EditError::NoSuchList)?;
            let first = if section == from.list.section {
                from.para
            } else {
                0
            };
            let last = if section == to.list.section {
                to.para
            } else {
                source
                    .len()
                    .checked_sub(1)
                    .ok_or(EditError::NoSuchParagraph)? as u32
            };
            let a = Pos {
                list: list.clone(),
                para: first,
                offset: if section == from.list.section {
                    from.offset
                } else {
                    hm_edit::ops::lead(
                        source
                            .get(first as usize)
                            .ok_or(EditError::NoSuchParagraph)?,
                    )
                },
            };
            let b = Pos {
                list,
                para: last,
                offset: if section == to.list.section {
                    to.offset
                } else {
                    source
                        .get(last as usize)
                        .ok_or(EditError::NoSuchParagraph)?
                        .len()
                        .saturating_sub(1)
                },
            };
            paras.extend(copy(doc, instance, &a, &b)?.paras);
        }
        return Ok(Fragment {
            instance,
            source: std::sync::Arc::new(doc.clone()),
            paras,
        });
    }
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
        let mut objects = Vec::new();
        let mut text_units = 0u32;
        for (pos, u) in p.units() {
            if pos < start || pos >= end {
                continue;
            }
            if let Unit::Extended { index, .. } = u {
                let c = &p.controls[index as usize];
                if pos + 8 <= end && carried(c) {
                    let at = text_units + 8 * objects.len() as u32;
                    objects.push((at, c.clone()));
                }
                continue;
            }
            let ch = match u {
                Unit::Char(c) => c,
                Unit::Inline(9) => '\t',
                Unit::CharControl(10) => '\u{2028}',
                Unit::CharControl(30) => '\u{a0}',
                Unit::CharControl(31) => '\u{2007}',
                _ => continue,
            };
            text_units += units(ch);
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
        out.push(FragPara {
            runs,
            shape,
            objects,
        });
    }
    Ok(Fragment {
        instance,
        source: std::sync::Arc::new(doc.clone()),
        paras: out,
    })
}

/// Controls a fragment carries.
fn carried(c: &Control) -> bool {
    matches!(
        c.kind,
        ControlKind::Table(_) | ControlKind::Object(_) | ControlKind::Form(_)
    )
}

/// Wire units a pasted char takes: a tab is an 8-unit control.
fn units(c: char) -> u32 {
    match c {
        '\t' => 8,
        c => c.len_utf16() as u32,
    }
}

/// The id of face `f` in slot `k` of `doc`, found by name or added.
pub fn face_id(doc: &mut Document, k: usize, f: &FaceName) -> u16 {
    let list = &mut doc.info.faces[k];
    if let Some(i) = list.iter().position(|e| e.name == f.name) {
        return i as u16;
    }
    let mut f = f.clone();
    f.src = None;
    list.push(f);
    doc.info.id_mappings.src = None;
    (list.len() - 1) as u16
}

/// A face record named `name`: one the document has in any slot, else a
/// plain one.
pub fn face_named(doc: &Document, name: &str) -> FaceName {
    let name = hm_model::HwpString::from_text(name);
    doc.info
        .faces
        .iter()
        .flatten()
        .find(|f| f.name == name)
        .cloned()
        .unwrap_or(FaceName {
            name,
            ..FaceName::default()
        })
}

/// The operations pasting `frag` at `at` (after `cut`, which the caller
/// runs first), and the position after it. The first pasted paragraph
/// keeps the target paragraph's shape; the others take theirs within the
/// same document.
pub fn paste(
    doc: &mut Document,
    instance: u32,
    frag: &Fragment,
    at: &Pos,
    cut: Option<&Op>,
) -> Result<(Vec<Op>, Pos), EditError> {
    let imported;
    let frag = if frag.instance != instance {
        let mut copy = frag.clone();
        let mut import = crate::clip_import::Import::new(&frag.source, doc);
        for p in &mut copy.paras {
            p.shape = import
                .paragraph_shape(p.shape)
                .map_err(|_| EditError::BadRange)?;
            for r in &mut p.runs {
                r.shape = import.character(r.shape).map_err(|_| EditError::BadRange)?;
            }
            for (_, c) in &mut p.objects {
                import.control(c).map_err(|_| EditError::BadRange)?;
            }
        }
        copy.instance = instance;
        imported = copy;
        &imported
    } else {
        frag
    };
    let paras = hm_edit::ops::list(doc, &at.list).ok_or(EditError::NoSuchList)?;
    paras
        .get(at.para as usize)
        .ok_or(EditError::NoSuchParagraph)?;
    let same = frag.instance == instance;
    let text = frag.typed();
    let mut ops = Vec::new();
    if !text.is_empty() {
        ops.push(Op::InsertText {
            at: at.clone(),
            text,
        });
    }
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
            let shape = r.shape;
            let from = pos.clone();
            pos.offset += len;
            ops.push(Op::SetCharShape {
                from,
                to: pos.clone(),
                shape,
            });
        }
    }
    if !same || frag.objects() == 0 {
        return Ok((ops, pos));
    }
    // Objects go in after the text, each into the paragraph as it is then.
    let mut staged = doc.clone();
    for op in cut.into_iter().chain(&ops) {
        hm_edit::apply(&mut staged, op)?;
    }
    for (k, p) in frag.paras.iter().enumerate() {
        let base = if k == 0 { at.offset } else { 0 };
        for (offset, c) in &p.objects {
            let at = Pos {
                list: at.list.clone(),
                para: at.para + k as u32,
                offset: base + offset,
            };
            let op = hm_edit::ops::insert_control(&staged, &at, c.clone())?;
            hm_edit::apply(&mut staged, &op)?;
            ops.push(op);
        }
    }
    pos.offset += 8 * frag.paras.last().map_or(0, |p| p.objects.len() as u32);
    Ok((ops, pos))
}
