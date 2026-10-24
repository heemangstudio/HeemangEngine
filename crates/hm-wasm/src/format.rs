//! Effective formatting for the host: the characters and paragraphs at a
//! caret or over a range, styles, and list definitions.
//!
//! A collapsed caret reports what typing there would take: the character
//! shape of the unit before it, or of the paragraph's first unit at its
//! start. A range reports the values at its start and names in `mixed`
//! every property that differs somewhere in it.

use hm_model::{CharShape, Document, LineSpacingKind, ParaShape, Paragraph, Unit};
use serde::Serialize;

use crate::types::{CharPatchJs, ParaPatchJs};

#[derive(Debug, Serialize, PartialEq, Clone)]
pub struct ListRef {
    /// `bullet`, `numbering` or `outline`.
    pub kind: &'static str,
    /// The definition's id (1-based, as paragraphs refer to it).
    pub id: u32,
    pub level: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormatContext {
    pub characters: CharPatchJs,
    pub paragraphs: ParaPatchJs,
    pub style_id: u32,
    pub list: Option<ListRef>,
    pub mixed: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StyleInfo {
    pub id: u32,
    pub name: String,
    pub english_name: String,
    /// `paragraph` or `character`.
    pub kind: &'static str,
    pub next: u32,
    pub para_shape: u32,
    pub char_shape: u32,
    pub characters: CharPatchJs,
    pub paragraphs: ParaPatchJs,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LevelInfo {
    /// Text with `^n` standing for the number of level n.
    pub format: String,
    /// The number style code (0 digits, 1 circled digits, ...).
    pub number_kind: u32,
    pub start: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListInfo {
    pub kind: &'static str,
    pub id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bullet: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<u32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub levels: Vec<LevelInfo>,
}

/// A character shape as a full patch.
pub fn characters(doc: &Document, id: u32) -> CharPatchJs {
    let c = doc.info.char_shape(id).cloned().unwrap_or_default();
    characters_of(doc, &c)
}

fn characters_of(doc: &Document, c: &CharShape) -> CharPatchJs {
    CharPatchJs {
        bold: Some(c.bold()),
        italic: Some(c.italic()),
        underline: Some(matches!(c.underline_kind(), 1 | 3)),
        strike: Some(c.strike() != 0 && (c.attr >> 26) & 15 != 15),
        size: Some(c.base_size),
        color: Some(c.color),
        font_face: doc.info.faces[0]
            .get(c.faces[0] as usize)
            .map(|f| f.name.to_string_lossy()),
        font_faces: Some(
            (0..7)
                .map(|k| {
                    doc.info.faces[k]
                        .get(c.faces[k] as usize)
                        .map(|f| f.name.to_string_lossy())
                })
                .collect(),
        ),
        char_spacing: Some(c.spacings[0] as i32),
    }
}

/// A paragraph shape as a full patch; distances relative to characters
/// resolve against `basis` (HU).
pub fn paragraphs(doc: &Document, id: u16, basis: i32) -> ParaPatchJs {
    let s = doc
        .info
        .para_shapes
        .get(id as usize)
        .cloned()
        .unwrap_or_default();
    paragraphs_of(&s, basis)
}

fn paragraphs_of(s: &ParaShape, basis: i32) -> ParaPatchJs {
    let align = ["justify", "left", "right", "center", "distribute", "divide"]
        .get(((s.attr1 >> 2) & 7) as usize)
        .unwrap_or(&"justify");
    let (kind, spacing) = s.line_spacing();
    ParaPatchJs {
        align: Some(align.to_string()),
        space_before: Some(s.before(basis)),
        space_after: Some(s.after(basis)),
        left: Some(s.left(basis)),
        right: Some(s.right(basis)),
        indent: Some(s.first_indent(basis)),
        line_spacing: (kind == LineSpacingKind::Percent).then_some(spacing as u32),
    }
}

/// A paragraph's list membership.
pub fn list_of(doc: &Document, p: &Paragraph) -> Option<ListRef> {
    let s = doc.info.para_shapes.get(p.header.para_shape as usize)?;
    let kind = match s.head_kind() {
        1 => "outline",
        2 => "numbering",
        3 => "bullet",
        _ => return None,
    };
    Some(ListRef {
        kind,
        id: s.numbering as u32,
        level: s.level(),
    })
}

/// The character shape typing at `offset` takes.
pub fn caret_shape(p: &Paragraph, offset: u32) -> u32 {
    let lo = hm_edit::ops::lead(p);
    if offset > lo {
        p.shape_at(offset - 1)
    } else {
        p.shape_at(lo)
    }
}

/// The context of the spans of a selection (one span for a caret, whose
/// ends are equal). `paras` gives each span's paragraphs.
pub fn context(doc: &Document, spans: &[(&[Paragraph], u32, u32, u32, u32)]) -> FormatContext {
    let mut shapes: Vec<u32> = Vec::new();
    let mut para_list: Vec<&Paragraph> = Vec::new();
    for &(paras, a, ao, b, bo) in spans {
        for pi in a..=b {
            let Some(p) = paras.get(pi as usize) else {
                continue;
            };
            // A range ending at a paragraph's start leaves that paragraph.
            if pi == b && pi > a && bo == 0 {
                continue;
            }
            para_list.push(p);
            let s = if pi == a { ao } else { 0 };
            let e = if pi == b { bo } else { p.len() };
            for (pos, u) in p.units() {
                if pos >= s && pos < e && !matches!(u, Unit::CharControl(13)) {
                    shapes.push(p.shape_at(pos));
                }
            }
        }
    }
    let first = spans
        .first()
        .and_then(|&(paras, a, ao, _, _)| paras.get(a as usize).map(|p| caret_shape(p, ao)));
    if shapes.is_empty() {
        shapes.extend(first);
    }
    let head = para_list.first().copied();
    let basis = |p: &Paragraph| {
        doc.info
            .char_shape(p.shape_at(0))
            .map_or(1000, |c| c.base_size)
    };
    let chars: Vec<CharPatchJs> = {
        let mut ids = shapes.clone();
        ids.sort_unstable();
        ids.dedup();
        ids.iter().map(|&i| characters(doc, i)).collect()
    };
    let paras: Vec<ParaPatchJs> = para_list
        .iter()
        .map(|p| paragraphs(doc, p.header.para_shape, basis(p)))
        .collect();
    let mut mixed = Vec::new();
    let at = shapes
        .first()
        .map_or_else(|| characters(doc, 0), |&i| characters(doc, i));
    macro_rules! differs {
        ($list:expr, $first:expr, $prefix:literal, $($f:ident: $name:literal),*) => {
            $(if $list.iter().any(|x| x.$f != $first.$f) {
                mixed.push(format!("{}.{}", $prefix, $name));
            })*
        };
    }
    differs!(chars, at, "characters", bold: "bold", italic: "italic", underline: "underline",
        strike: "strike", size: "size", color: "color", font_face: "fontFace",
        font_faces: "fontFaces", char_spacing: "charSpacing");
    let para0 = paras.first().cloned().unwrap_or_default();
    differs!(paras, para0, "paragraphs", align: "align", space_before: "spaceBefore",
        space_after: "spaceAfter", left: "left", right: "right", indent: "indent",
        line_spacing: "lineSpacing");
    let style_id = head.map_or(0, |p| p.header.style as u32);
    if para_list.iter().any(|p| p.header.style as u32 != style_id) {
        mixed.push("styleId".into());
    }
    let list = head.and_then(|p| list_of(doc, p));
    if para_list.iter().any(|p| list_of(doc, p) != list) {
        mixed.push("list".into());
    }
    FormatContext {
        characters: at,
        paragraphs: para0,
        style_id,
        list,
        mixed,
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunFormat {
    pub offset: u32,
    pub characters: CharPatchJs,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParagraphFormat {
    pub index: u32,
    pub style_id: u32,
    pub paragraphs: ParaPatchJs,
    pub list: Option<ListRef>,
    pub runs: Vec<RunFormat>,
}

/// Paragraph `index` with its shape and the character shape of each run,
/// adjacent runs of equal shape merged.
pub fn paragraph_format(doc: &Document, index: u32, p: &Paragraph) -> ParagraphFormat {
    let basis = doc
        .info
        .char_shape(p.shape_at(0))
        .map_or(1000, |c| c.base_size);
    let mut runs: Vec<RunFormat> = Vec::new();
    let mut last = None;
    for r in &p.runs {
        if last == Some(r.shape) {
            continue;
        }
        last = Some(r.shape);
        runs.push(RunFormat {
            offset: r.pos,
            characters: characters(doc, r.shape),
        });
    }
    ParagraphFormat {
        index,
        style_id: p.header.style as u32,
        paragraphs: paragraphs(doc, p.header.para_shape, basis),
        list: list_of(doc, p),
        runs,
    }
}

/// Style `id`, with its shapes as patches.
pub fn style(doc: &Document, id: u32) -> Option<StyleInfo> {
    let s = doc.info.styles.get(id as usize)?;
    let basis = doc
        .info
        .char_shape(s.char_shape as u32)
        .map_or(1000, |c| c.base_size);
    Some(StyleInfo {
        id,
        name: s.local_name.to_string_lossy(),
        english_name: s.english_name.to_string_lossy(),
        kind: if s.attr & 7 == 1 {
            "character"
        } else {
            "paragraph"
        },
        next: s.next as u32,
        para_shape: s.para_shape as u32,
        char_shape: s.char_shape as u32,
        characters: characters(doc, s.char_shape as u32),
        paragraphs: paragraphs(doc, s.para_shape, basis),
    })
}

/// Bullet and numbering definitions, numberings first.
pub fn lists(doc: &Document) -> Vec<ListInfo> {
    let numberings =
        (1..=doc.info.numberings.len() as u32).filter_map(|id| list(doc, "numbering", id));
    let bullets = (1..=doc.info.bullets.len() as u32).filter_map(|id| list(doc, "bullet", id));
    numberings.chain(bullets).collect()
}

/// One bullet or numbering definition.
pub fn list(doc: &Document, kind: &str, id: u32) -> Option<ListInfo> {
    let i = (id as usize).checked_sub(1)?;
    match kind {
        "numbering" => {
            let n = doc.info.numberings.get(i)?;
            let start = n.start.map_or(1, u32::from);
            Some(ListInfo {
                kind: "numbering",
                id,
                bullet: None,
                start: Some(start),
                levels: n
                    .levels
                    .iter()
                    .enumerate()
                    .map(|(k, l)| LevelInfo {
                        format: l.format.to_string_lossy(),
                        number_kind: (l.head.attr >> 5) & 0xF,
                        start: n
                            .level_starts
                            .map_or(start, |s| s.get(k).copied().unwrap_or(start)),
                    })
                    .collect(),
            })
        }
        "bullet" => {
            let b = doc.info.bullets.get(i)?;
            Some(ListInfo {
                kind: "bullet",
                id,
                bullet: Some(String::from_utf16_lossy(&[b.bullet_char])),
                start: None,
                levels: Vec::new(),
            })
        }
        _ => None,
    }
}
