//! Edits and reads Studio's agent tools need: equations and table caption
//! text. Every edit is a paragraph replacement, so it undoes as one step
//! with the rest of its transaction.
use hm_edit::{ListPath, Op, Pos};
use hm_fonts::FontSystem;
use hm_layout::math::{self, MathFonts, MathLayout};
use hm_model::*;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::types::{Address, EngineError, Result, TableAddress};

fn bad(s: impl std::fmt::Display) -> EngineError {
    EngineError::new("BAD_REQUEST", s)
}

fn address(s: &str) -> EngineError {
    EngineError::new("BAD_ADDRESS", s)
}

/// Longest equation script accepted, in characters.
const MAX_SCRIPT: usize = 10_000;
/// Longest caption text accepted, in characters.
const MAX_CAPTION: usize = 10_000;
/// The version string Hancom writes on the equations it makes.
const EQUATION_VERSION: &str = "Equation Version 60";
/// The face Hancom names on the equations it makes.
const EQUATION_FONT: &str = "HYhwpEQ";

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EquationPatch {
    pub script: Option<String>,
    pub size: Option<u32>,
    pub color: Option<u32>,
    pub baseline: Option<i16>,
    pub font: Option<String>,
}

fn rgb(color: u32) -> Result<u32> {
    if color > 0x00FF_FFFF {
        return Err(bad("colors are COLORREF numbers up to 0xFFFFFF"));
    }
    Ok(color)
}

fn script_ok(s: &str) -> Result<()> {
    if s.trim().is_empty() {
        return Err(bad("an equation needs a script"));
    }
    if s.chars().count() > MAX_SCRIPT {
        return Err(bad(format!(
            "equation scripts are limited to {MAX_SCRIPT} characters"
        )));
    }
    Ok(())
}

fn size_ok(size: u32) -> Result<u32> {
    if !(100..=20000).contains(&size) {
        return Err(bad("equation size must be 100 to 20000 HU"));
    }
    Ok(size)
}

fn baseline_ok(b: i16) -> Result<i16> {
    if !(0..=100).contains(&b) {
        return Err(bad("equation baseline is a percentage from 0 to 100"));
    }
    Ok(b)
}

fn font_ok(f: &str) -> Result<()> {
    if f.trim().is_empty() || f.chars().count() > 255 {
        return Err(bad("equation font names one family"));
    }
    Ok(())
}

/// The object box an equation lays out to: width, height and baseline
/// percent.
fn measure(eq: &Equation, fonts: &FontSystem) -> (i32, i32, i16) {
    let m = math::layout(eq, fonts);
    let height = if m.height() > 0 {
        m.height()
    } else {
        eq.size as i32
    };
    (
        m.width.max(1),
        height,
        m.baseline_percent().clamp(0, 100) as i16,
    )
}

/// Gives an equation object the box its script lays out to; `baseline`
/// keeps a host-chosen baseline instead of the measured one.
fn fit(o: &mut ObjectControl, fonts: &FontSystem, baseline: Option<i16>) {
    let Some(ObjectPart::Equation(eq)) = o
        .parts
        .iter_mut()
        .find(|p| matches!(p, ObjectPart::Equation(_)))
    else {
        return;
    };
    let (w, h, b) = measure(eq, fonts);
    eq.baseline = baseline.unwrap_or(b);
    o.header.common.width = w;
    o.header.common.height = h;
}

/// A new inline equation, sized by its script's layout.
pub fn create_equation(
    fonts: &FontSystem,
    script: &str,
    size: Option<u32>,
    color: Option<u32>,
    font: Option<String>,
    baseline: Option<i16>,
) -> Result<Control> {
    script_ok(script)?;
    let size = size_ok(size.unwrap_or(1000))?;
    let color = rgb(color.unwrap_or(0))?;
    let baseline = baseline.map(baseline_ok).transpose()?;
    if let Some(f) = &font {
        font_ok(f)?;
    }
    let eq = Equation {
        attr: 0,
        script: HwpString::from_text(script),
        size,
        color,
        baseline: 85,
        reserved: Some(0),
        version: Some(HwpString::from_text(EQUATION_VERSION)),
        font: Some(HwpString::from_text(
            font.as_deref().unwrap_or(EQUATION_FONT),
        )),
        tail: SharedBytes::default(),
    };
    // Treated as a character, placed relative to the paragraph, flowing
    // with the text, absolute size, top and bottom wrap, equation
    // numbering: the header Hancom gives a new equation.
    let attr = 1 | (2 << 3) | (3 << 8) | (1 << 13) | (4 << 15) | (2 << 18) | (1 << 21) | (3 << 26);
    let mut o = ObjectControl {
        id: CtrlId::EQUATION,
        header: ObjectHeader {
            common: ObjectCommon {
                attr,
                margins: [56, 56, 0, 0],
                ..Default::default()
            },
            instance_id: Some(0),
            prevent_split: Some(0),
            ..Default::default()
        },
        parts: vec![ObjectPart::Equation(eq)],
    };
    fit(&mut o, fonts, baseline);
    Ok(Control {
        kind: ControlKind::Object(Box::new(o)),
        src: None,
    })
}

/// A copy of `doc` holding only paragraph `p`, for editing it alone.
fn scratch(doc: &Document, p: Paragraph) -> Document {
    Document {
        meta: doc.meta.clone(),
        info: doc.info.clone(),
        sections: vec![Section {
            paras: vec![p],
            ..Default::default()
        }],
        source: None,
        bins: None,
        diagnostics: Vec::new(),
    }
}

/// Inserts `text` at `offset` of the scratch document's only paragraph.
fn type_text(doc: &mut Document, offset: u32, text: String) -> Result<()> {
    if text.is_empty() {
        return Ok(());
    }
    let at = Pos {
        list: ListPath::default(),
        para: 0,
        offset,
    };
    hm_edit::apply(doc, &Op::InsertText { at, text })
        .map(|_| ())
        .map_err(|e| EngineError::new("BAD_ADDRESS", e))
}

/// Anchors `control` at `offset` of the scratch document's only paragraph,
/// written with control character `ch`. Placeholder text goes in through
/// the editor first, so character runs and marks shift as for typing.
fn anchor(doc: &mut Document, offset: u32, ch: u16, control: Control) -> Result<()> {
    type_text(doc, offset, "\u{E000}".repeat(8))?;
    let p = &mut doc.sections[0].paras[0];
    let index = p
        .units()
        .filter(|(at, u)| *at < offset && matches!(u, Unit::Extended { .. }))
        .count();
    let id = control.id().0;
    let units = [ch, id as u16, (id >> 16) as u16, 0, 0, 0, 0, ch];
    p.text[offset as usize..offset as usize + 8].copy_from_slice(&units);
    p.controls.insert(index, control);
    Ok(())
}

/// `control` inserted at `at` as one paragraph replacement.
pub fn insert(doc: &Document, at: &Address, control: Control) -> Result<Op> {
    let pos = at.pos();
    let p = hm_edit::ops::list(doc, &pos.list)
        .and_then(|l| l.get(pos.para as usize))
        .cloned()
        .ok_or_else(|| address("no such paragraph"))?;
    let mut s = scratch(doc, p);
    anchor(&mut s, pos.offset, 11, control)?;
    let mut p = s.sections.remove(0).paras.remove(0);
    p.src = None;
    p.line_segs.clear();
    Ok(Op::Replace {
        list: pos.list,
        start: pos.para,
        count: 1,
        paras: vec![p],
    })
}

fn host(doc: &Document, at: &TableAddress) -> Result<Paragraph> {
    hm_edit::ops::list(doc, &crate::tables::path(at))
        .and_then(|l| l.get(at.para as usize))
        .filter(|p| (at.control as usize) < p.controls.len())
        .cloned()
        .ok_or_else(|| address("no such control"))
}

fn replace(at: &TableAddress, mut p: Paragraph) -> Op {
    p.src = None;
    p.line_segs.clear();
    p.controls[at.control as usize].src = None;
    Op::Replace {
        list: crate::tables::path(at),
        start: at.para,
        count: 1,
        paras: vec![p],
    }
}

fn equation_of(c: &Control) -> Option<(&ObjectControl, &Equation)> {
    let o = c.as_object().filter(|o| o.id == CtrlId::EQUATION)?;
    Some((o, o.equation()?))
}

pub fn set_equation(
    doc: &Document,
    fonts: &FontSystem,
    at: &TableAddress,
    patch: EquationPatch,
) -> Result<Op> {
    let mut para = host(doc, at)?;
    let ControlKind::Object(o) = &mut para.controls[at.control as usize].kind else {
        return Err(bad("the control is not an equation"));
    };
    if o.id != CtrlId::EQUATION || o.equation().is_none() {
        return Err(bad("the control is not an equation"));
    }
    let baseline = patch.baseline.map(baseline_ok).transpose()?;
    let Some(ObjectPart::Equation(eq)) = o
        .parts
        .iter_mut()
        .find(|p| matches!(p, ObjectPart::Equation(_)))
    else {
        unreachable!()
    };
    if let Some(s) = &patch.script {
        script_ok(s)?;
        eq.script = HwpString::from_text(s);
    }
    if let Some(v) = patch.size {
        eq.size = size_ok(v)?;
    }
    if let Some(v) = patch.color {
        eq.color = rgb(v)?;
    }
    if let Some(f) = &patch.font {
        font_ok(f)?;
        eq.font = Some(HwpString::from_text(f));
    }
    // Only the baseline changes: keep the box, as it lays out the same.
    let remeasure = patch.script.is_some() || patch.size.is_some() || patch.font.is_some();
    if remeasure {
        fit(o, fonts, baseline);
    } else if let Some(b) = baseline {
        eq.baseline = b;
    }
    Ok(replace(at, para))
}

pub fn equation(doc: &Document, at: &TableAddress) -> Result<Value> {
    let p = host(doc, at)?;
    let (o, e) = equation_of(&p.controls[at.control as usize])
        .ok_or_else(|| bad("the control is not an equation"))?;
    Ok(json!({
        "script": e.script.to_string_lossy(),
        "size": e.size,
        "color": e.color & 0x00FF_FFFF,
        "baseline": e.baseline,
        "font": e.font.as_ref().map(|f| f.to_string_lossy()),
        "width": o.header.common.width,
        "height": o.header.common.height,
    }))
}

/// Characters of a script no registered face can draw in an equation.
fn missing_glyphs(script: &str, mf: &MathFonts) -> Vec<String> {
    let mut out = Vec::new();
    if mf.eq.is_none() {
        out.push("no equation font (HyhwpEQ) is registered".to_string());
    }
    let mut seen = std::collections::BTreeSet::new();
    for c in script.chars() {
        // Whitespace and the script's own syntax characters draw nothing.
        if c.is_whitespace() || c.is_control() || "{}^_~`&#\"\\".contains(c) || !seen.insert(c) {
            continue;
        }
        let in_eq = mf.eq.as_ref().is_some_and(|(_, f)| {
            [math::pua(c, true), math::pua(c, false), Some(c)]
                .into_iter()
                .flatten()
                .any(|ch| f.glyph(ch).is_some())
        });
        if !in_eq && !mf.text.iter().any(|(_, f)| f.glyph(c).is_some()) {
            out.push(format!(
                "no registered font has a glyph for {c:?} (U+{:04X})",
                c as u32
            ));
        }
    }
    out
}

/// An equation laid out with the document's fonts, as a standalone SVG at
/// 96 dpi. Reads nothing from and changes nothing in the document.
pub fn preview(
    fonts: &FontSystem,
    script: &str,
    size: u32,
    color: u32,
    font: Option<&str>,
) -> Result<Value> {
    script_ok(script)?;
    let size = size_ok(size)?;
    let color = rgb(color)?;
    if let Some(f) = font {
        font_ok(f)?;
    }
    let mf = MathFonts::new(fonts, Some(font.unwrap_or(EQUATION_FONT)));
    let m: MathLayout = math::layout_script(script, size as i32, color, &mf);
    let (w, h) = (m.width.max(1), m.height().max(1));
    let mut dl = hm_render::DisplayList {
        width: w,
        height: h,
        items: Vec::new(),
    };
    hm_render::objects::paint_math(&mut dl, fonts, &m, 0, m.ascent);
    let px = |hu: i32| hu as f64 * 96.0 / 7200.0;
    Ok(json!({
        "svg": hm_render::svg::image(&dl, fonts, 96.0),
        "widthPx": px(w),
        "heightPx": px(h),
        "baselinePx": px(m.ascent),
        "warnings": missing_glyphs(script, &mf),
    }))
}

// ---- table captions ----

fn table_mut<'a>(p: &'a mut Paragraph, at: &TableAddress) -> Result<&'a mut Table> {
    match &mut p.controls[at.control as usize].kind {
        ControlKind::Table(t) => Ok(t),
        _ => Err(bad("the object is not a table")),
    }
}

fn is_table_number(c: &Control) -> Option<u16> {
    match &c.kind {
        ControlKind::AutoNumber(a) if a.kind() == NumberKind::Table => Some(a.number),
        _ => None,
    }
}

/// Table numbers drawn before the table at `target`, in document order.
fn numbers_before(doc: &Document, target: &TableAddress) -> u32 {
    fn walk(
        paras: &[Paragraph],
        path: &ListPath,
        target: &(ListPath, u32, u32),
        n: &mut u32,
    ) -> bool {
        for (pi, p) in paras.iter().enumerate() {
            for (ci, c) in p.controls.iter().enumerate() {
                if *path == target.0 && pi as u32 == target.1 && ci as u32 == target.2 {
                    return true;
                }
                if is_table_number(c).is_some() {
                    *n += 1;
                }
                for (li, list) in hm_edit::ops::control_lists(c).into_iter().enumerate() {
                    let mut child = path.clone();
                    child.steps.push(hm_edit::Step {
                        para: pi as u32,
                        control: ci as u32,
                        index: li as u32,
                    });
                    if walk(list, &child, target, n) {
                        return true;
                    }
                }
            }
        }
        false
    }
    let target = (crate::tables::path(target), target.para, target.control);
    let mut n = 0;
    for (si, s) in doc.sections.iter().enumerate() {
        let path = ListPath {
            section: si as u32,
            ..Default::default()
        };
        if walk(&s.paras, &path, &target, &mut n) {
            break;
        }
    }
    n
}

/// The paragraph a new caption starts from: Hancom's caption style when
/// the document has one, the table's paragraph otherwise.
fn caption_template(doc: &Document, host: &Paragraph) -> Paragraph {
    let style = doc.info.styles.iter().position(|s| {
        s.local_name.to_string_lossy() == "캡션" || s.english_name.to_string_lossy() == "Caption"
    });
    match style {
        Some(i) if i <= u8::MAX as usize => {
            let s = &doc.info.styles[i];
            let mut p = hm_edit::ops::empty_paragraph(&Paragraph::default(), s.char_shape as u32);
            p.header.para_shape = s.para_shape;
            p.header.style = i as u8;
            p
        }
        _ => {
            let mut p = hm_edit::ops::empty_paragraph(host, host.shape_at(0));
            p.header.break_kind = 0;
            p
        }
    }
}

/// Paragraphs holding `text` (split at `\n`), the first starting with the
/// label and automatic table number when `number` is set.
fn caption_paras(
    doc: &Document,
    template: &Paragraph,
    text: &str,
    number: Option<u16>,
) -> Result<Vec<Paragraph>> {
    let mut first = hm_edit::ops::empty_paragraph(template, template.shape_at(0));
    first.header.break_kind = 0;
    let mut doc = scratch(doc, first);
    let mut body = text.to_string();
    if let Some(n) = number {
        // Hancom writes the label, then the number as an automatic-number
        // control, then the caption text.
        type_text(&mut doc, 0, "표 ".to_string())?;
        let number = Control {
            kind: ControlKind::AutoNumber(AutoNumber {
                attr: 4,
                number: n,
                ..Default::default()
            }),
            src: None,
        };
        anchor(&mut doc, 2, 18, number)?;
        if !body.is_empty() && !body.starts_with([' ', '\n']) {
            body.insert(0, ' ');
        }
        type_text(&mut doc, 10, body)?;
    } else {
        type_text(&mut doc, 0, body)?;
    }
    Ok(doc.sections.remove(0).paras)
}

pub fn set_table_caption(
    doc: &Document,
    at: &TableAddress,
    text: &str,
    with_number: bool,
) -> Result<Op> {
    if text.chars().count() > MAX_CAPTION {
        return Err(bad(format!(
            "caption text is limited to {MAX_CAPTION} characters"
        )));
    }
    let mut para = host(doc, at)?;
    let t = para.controls[at.control as usize]
        .as_table()
        .ok_or_else(|| bad("the object is not a table"))?;
    let number = if with_number {
        let kept = t
            .caption
            .iter()
            .flat_map(|c| &c.paras)
            .flat_map(|p| &p.controls)
            .find_map(is_table_number);
        Some(kept.unwrap_or_else(|| {
            let start = u32::from(doc.info.properties.table_start.max(1));
            (start + numbers_before(doc, at)).min(u16::MAX as u32) as u16
        }))
    } else {
        None
    };
    let template = match t.caption.as_ref().and_then(|c| c.paras.first()) {
        Some(p) => p.clone(),
        None => caption_template(doc, &para),
    };
    let paras = caption_paras(doc, &template, text, number)?;
    let t = table_mut(&mut para, at)?;
    let width = t.header.common.width;
    let caption = t.caption.get_or_insert_with(|| Caption {
        // Below the table, Hancom's default side, gap and side width.
        attr: Some(3),
        width: Some(8504),
        gap: Some(850),
        max_width: Some(width),
        ..Default::default()
    });
    caption.count = paras.len() as u32;
    caption.paras = paras;
    // Hancom marks a captioned object in its header and numbers it as a
    // table.
    let attr = &mut t.header.common.attr;
    *attr |= 1 << 29;
    if (*attr >> 26) & 7 == 0 {
        *attr |= 2 << 26;
    }
    Ok(replace(at, para))
}

/// The caret after an object inserted at `at`.
pub fn after(at: &Address) -> Pos {
    Pos {
        offset: at.offset + 8,
        ..at.pos()
    }
}
