//! Footnote and endnote placement against Hancom Office 2022's captures
//! (heemang-cleanroom `parity/notes/hancom-note-captures`).

use hm_layout::{DocLayout, FrameKind};
use hm_model::{
    Control, ControlKind, DocInfo, DocMeta, Document, Format, Note, NoteShape, PageDef, ParaList,
    Paragraph, Section, SectionChild, SectionDef,
};

/// The footnote shape Hancom gives a new document, with attribute `attr`.
fn footnotes(attr: u32) -> NoteShape {
    NoteShape {
        attr,
        suffix: ')' as u16,
        start: 1,
        separator_length: -1,
        space_above: 850,
        space_below: 567,
        space_between: 283,
        line_kind: 1,
        line_width: 1,
        ..Default::default()
    }
}

/// The endnote shape Hancom gives a new document, with attribute `attr`:
/// a full-width separator and no gap between notes.
fn endnotes(attr: u32) -> NoteShape {
    NoteShape {
        separator_length: 14692344,
        space_between: 0,
        ..footnotes(attr)
    }
}

fn note(endnote: bool, number: u32, text: &str) -> Control {
    Control {
        kind: ControlKind::Note(Box::new(Note {
            endnote,
            number,
            suffix: ')' as u16,
            list: ParaList {
                paras: vec![para(text, vec![])],
                ..Default::default()
            },
            ..Default::default()
        })),
        src: None,
    }
}

/// A paragraph of `text` followed by `controls`.
fn para(text: &str, controls: Vec<Control>) -> Paragraph {
    let mut units: Vec<u16> = text.encode_utf16().collect();
    for c in &controls {
        let code = match &c.kind {
            ControlKind::Section(_) | ControlKind::Columns(_) => 2,
            ControlKind::Note(_) => 17,
            _ => 11,
        };
        units.extend([code, 0, 0, 0, 0, 0, 0, code]);
    }
    units.push(hm_model::PARA_END);
    Paragraph {
        text: units,
        controls,
        ..Default::default()
    }
}

/// A section on a 20000 HU square page without margins, its note shapes
/// `[footnote, endnote]`, holding `paras` after its definition paragraph.
fn section(shapes: [NoteShape; 2], paras: Vec<Paragraph>) -> Section {
    let def = Control {
        kind: ControlKind::Section(Box::new(SectionDef {
            children: vec![
                SectionChild::Page(PageDef {
                    width: 20000,
                    height: 20000,
                    ..Default::default()
                }),
                SectionChild::NoteShape(shapes[0].clone()),
                SectionChild::NoteShape(shapes[1].clone()),
            ],
            ..Default::default()
        })),
        src: None,
    };
    let mut all = vec![para("", vec![def])];
    all.extend(paras);
    Section {
        paras: all,
        ..Default::default()
    }
}

fn layout(sections: Vec<Section>) -> DocLayout {
    let doc = Document {
        meta: DocMeta {
            format: Format::Hwpx,
            version: Default::default(),
            flags: 0,
        },
        info: DocInfo {
            para_shapes: vec![hm_model::ParaShape {
                line_spacing_legacy: 100,
                ..Default::default()
            }],
            ..Default::default()
        },
        sections,
        source: None,
        bins: None,
        diagnostics: Vec::new(),
    };
    hm_layout::layout(&doc, &hm_fonts::FontSystem::default())
}

/// Body lines are 1000 HU here: paragraph 1 (after the definition
/// paragraph) spans 1000..2000.
const TEXT_END: i32 = 2000;

fn frames(l: &DocLayout, page: usize, kind: FrameKind) -> Vec<hm_layout::Rect> {
    l.pages[page]
        .frames
        .iter()
        .filter(|f| f.kind == kind)
        .map(|f| f.rect)
        .collect()
}

/// A footnote longer than its page splits by line onto the next page's
/// note area (`c07a-long-note`), also when its mark is on the first line
/// of an empty column: the part kept stays inside the body.
#[test]
fn a_footnote_taller_than_the_page_splits_from_an_empty_column() {
    let long = Control {
        kind: ControlKind::Note(Box::new(Note {
            number: 1,
            suffix: ')' as u16,
            list: ParaList {
                paras: (0..30).map(|_| para("Line", vec![])).collect(),
                ..Default::default()
            },
            ..Default::default()
        })),
        src: None,
    };
    let mut s = section([footnotes(0), endnotes(0)], vec![]);
    // The mark goes on the definition paragraph's own line, the page's first.
    let def = s.paras.remove(0).controls;
    s.paras.push(para("Body", def.into_iter().chain([long]).collect()));
    let l = layout(vec![s]);
    assert_eq!(l.pages.len(), 2);
    let first = frames(&l, 0, FrameKind::Footnotes);
    assert_eq!(first.len(), 1);
    assert!(first[0].y >= TEXT_END - 1000, "{:?}", first[0]);
    assert_eq!(first[0].y + first[0].h, 20000);
    let rest = frames(&l, 1, FrameKind::Footnotes);
    assert_eq!(rest.len(), 1);
    assert_eq!(rest[0].y + rest[0].h, 20000);
}
