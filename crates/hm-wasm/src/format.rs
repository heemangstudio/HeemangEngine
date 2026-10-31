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
    pub list_restart: Option<crate::types::ListRestartSpec>,
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
        spacings: Some(c.spacings.iter().map(|&v| Some(v as i32)).collect()),
        ratios: Some(c.ratios.iter().map(|&v| Some(v as i32)).collect()),
        rel_sizes: Some(
            c.rel_sizes
                .iter()
                .map(|&v| Some(if v == 0 { 100 } else { v as i32 }))
                .collect(),
        ),
        offsets: Some(c.offsets.iter().map(|&v| Some(v as i32)).collect()),
        superscript: Some(c.superscript()),
        subscript: Some(c.subscript()),
        emboss: Some(c.attr & 1 << 13 != 0),
        engrave: Some(c.attr & 1 << 14 != 0),
        kerning: Some(c.attr & 1 << 30 != 0),
        use_font_space: Some(c.attr & 1 << 25 != 0),
        underline_kind: Some(
            match c.underline_kind() {
                1 => "below",
                // Without a strikeout and not the hidden no-strike line.
                2 if c.strike() == 0 && c.underline_shape() != 15 => "center",
                3 => "above",
                _ => "none",
            }
            .into(),
        ),
        underline_shape: Some(c.underline_shape()),
        underline_color: Some(c.underline_color),
        strike_shape: Some((c.attr >> 26) & 15),
        strike_color: c.strike_color,
        outline: Some((c.attr >> 8) & 7),
        shadow: Some((c.attr >> 11) & 3),
        shadow_color: Some(c.shadow_color),
        shadow_offset: Some(c.shadow_offset.iter().map(|&v| v as i32).collect()),
        emphasis: Some((c.attr >> 21) & 15),
        shade_color: Some(c.shade_color),
        border_fill_id: Some(c.border_fill.unwrap_or(0)),
        border_fill: c.border_fill.and_then(|id| id.checked_sub(1)).and_then(|id| doc.info.border_fills.get(id as usize)).map(|b| {
            let line = |l: hm_model::BorderLine| serde_json::json!({"style":l.kind,"width":l.width,"color":l.color});
            let fill = b.fill.as_ref().and_then(|f| f.solid).map(|s| serde_json::json!({"kind":"solid","background":s.background,"patternColor":s.pattern_color,"pattern":s.pattern})).unwrap_or(serde_json::json!({"kind":"none"}));
            serde_json::json!({"left":line(b.sides[0]),"right":line(b.sides[1]),"top":line(b.sides[2]),"bottom":line(b.sides[3]),"fill":fill})
        }),
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
    let mut patch = paragraphs_of(&s, basis);
    let tabs = doc.info.tab_defs.get(s.tab_def as usize);
    patch.tab_stops = tabs.map(|d| {
        d.tabs
            .iter()
            .map(|t| crate::types::TabStopJs {
                position: hm_model::units::unpack_distance(t.position, || basis),
                kind: t.kind,
                fill: t.fill,
            })
            .collect()
    });
    let attr = tabs.map_or(0, |d| d.attr);
    patch.auto_tab_left = Some(attr & 1 != 0);
    patch.auto_tab_right = Some(attr & 2 != 0);
    patch
}

fn paragraphs_of(s: &ParaShape, basis: i32) -> ParaPatchJs {
    let align = ["justify", "left", "right", "center", "distribute", "divide"]
        .get(((s.attr1 >> 2) & 7) as usize)
        .unwrap_or(&"justify");
    let (kind, spacing) = s.line_spacing();
    let a2 = s.attr2.unwrap_or(0);
    ParaPatchJs {
        tab_stops: None,
        align: Some(align.to_string()),
        space_before: Some(s.before(basis)),
        space_after: Some(s.after(basis)),
        left: Some(s.left(basis)),
        right: Some(s.right(basis)),
        indent: Some(s.first_indent(basis)),
        line_spacing: Some(spacing as u32),
        line_spacing_kind: Some(
            match kind {
                LineSpacingKind::Percent => "percent",
                LineSpacingKind::Fixed => "fixed",
                LineSpacingKind::Added => "added",
                LineSpacingKind::AtLeast => "atLeast",
            }
            .into(),
        ),
        keep_with_next: Some(s.keep_with_next()),
        keep_lines: Some(s.keep_lines()),
        widow_orphan: Some(s.widow_orphan()),
        page_break_before: Some(s.page_break_before()),
        break_latin_word: Some(s.latin_break()),
        break_non_latin_word: Some((s.attr1 >> 7) & 1),
        snap_to_grid: Some(s.snap_to_grid()),
        condense: Some(s.min_space()),
        font_line_height: Some(s.attr1 & 1 << 22 != 0),
        vertical_align: Some(s.vertical_align()),
        line_wrap: Some(a2 & 3),
        e_asian_eng: Some(a2 & 1 << 4 != 0),
        e_asian_num: Some(a2 & 1 << 5 != 0),
        border_fill_id: Some(s.border_fill),
        border_offsets: Some(s.border_offsets.map(i32::from)),
        border_connect: Some(s.connect_border()),
        border_ignore_margin: Some(s.border_ignores_margin()),
        auto_tab_left: None,
        auto_tab_right: None,
        heading: Some(crate::types::HeadingJs {
            kind: ["none", "outline", "numbering", "bullet"][s.head_kind() as usize].into(),
            id: s.numbering,
            level: s.level(),
        }),
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
        font_faces: "fontFaces", char_spacing: "charSpacing", spacings: "spacings", ratios: "ratios",
        rel_sizes: "relSizes", offsets: "offsets", superscript: "superscript",
        subscript: "subscript", emboss: "emboss", engrave: "engrave", kerning: "kerning",
        use_font_space: "useFontSpace", underline_kind: "underlineKind",
        underline_shape: "underlineShape", underline_color: "underlineColor",
        strike_shape: "strikeShape", strike_color: "strikeColor", outline: "outline",
        shadow: "shadow", shadow_color: "shadowColor", shadow_offset: "shadowOffset",
        emphasis: "emphasis", shade_color: "shadeColor", border_fill_id: "borderFillId");
    let para0 = paras.first().cloned().unwrap_or_default();
    differs!(paras, para0, "paragraphs", align: "align", space_before: "spaceBefore",
        space_after: "spaceAfter", left: "left", right: "right", indent: "indent",
        line_spacing: "lineSpacing", line_spacing_kind: "lineSpacingKind",
        tab_stops: "tabStops", keep_with_next: "keepWithNext", keep_lines: "keepLines",
        widow_orphan: "widowOrphan", page_break_before: "pageBreakBefore",
        break_latin_word: "breakLatinWord", break_non_latin_word: "breakNonLatinWord",
        snap_to_grid: "snapToGrid", condense: "condense", font_line_height: "fontLineHeight",
        vertical_align: "verticalAlign", line_wrap: "lineWrap", e_asian_eng: "eAsianEng",
        e_asian_num: "eAsianNum", border_fill_id: "borderFillId", border_offsets: "borderOffsets",
        border_connect: "borderConnect", border_ignore_margin: "borderIgnoreMargin",
        auto_tab_left: "autoTabLeft", auto_tab_right: "autoTabRight", heading: "heading");
    let style_id = head.map_or(0, |p| p.header.style as u32);
    if para_list.iter().any(|p| p.header.style as u32 != style_id) {
        mixed.push("styleId".into());
    }
    let list = head.and_then(|p| list_of(doc, p));
    if para_list.iter().any(|p| list_of(doc, p) != list) {
        mixed.push("list".into());
    }
    if para_list
        .iter()
        .any(|p| p.list_restart != head.and_then(|p| p.list_restart))
    {
        mixed.push("listRestart".into());
    }
    FormatContext {
        characters: at,
        paragraphs: para0,
        style_id,
        list,
        list_restart: head.and_then(restart_of),
        mixed,
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunFormat {
    pub offset: u32,
    pub char_shape_id: u32,
    pub characters: CharPatchJs,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParagraphFormat {
    pub index: u32,
    pub para_shape_id: u16,
    pub style_id: u32,
    pub paragraphs: ParaPatchJs,
    pub list: Option<ListRef>,
    pub list_restart: Option<crate::types::ListRestartSpec>,
    pub runs: Vec<RunFormat>,
    /// The paragraph starts a new page.
    pub page_break: bool,
    /// The paragraph starts a new column.
    pub column_break: bool,
}

fn restart_of(p: &Paragraph) -> Option<crate::types::ListRestartSpec> {
    p.list_restart.map(|r| match r {
        hm_model::ListRestart::Continue => crate::types::ListRestartSpec::Continue,
        hm_model::ListRestart::Start(start) => crate::types::ListRestartSpec::Restart { start },
    })
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
            char_shape_id: r.shape,
            characters: characters(doc, r.shape),
        });
    }
    ParagraphFormat {
        index,
        para_shape_id: p.header.para_shape,
        style_id: p.header.style as u32,
        paragraphs: paragraphs(doc, p.header.para_shape, basis),
        list: list_of(doc, p),
        list_restart: restart_of(p),
        runs,
        page_break: p.header.break_kind & hm_model::break_kind::PAGE != 0,
        column_break: p.header.break_kind & hm_model::break_kind::COLUMN != 0,
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
