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
    layout_with_size(sections, 0)
}

/// Lays `sections` out; a nonzero `size` gives char shape 0 that base size.
fn layout_with_size(sections: Vec<Section>, size: i32) -> DocLayout {
    hm_layout::layout(&document(sections, size), &hm_fonts::FontSystem::default())
}

/// A document of `sections`; a nonzero `size` gives char shape 0 that base
/// size.
fn document(sections: Vec<Section>, size: i32) -> Document {
    let mut doc = Document {
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
    if size > 0 {
        doc.info.char_shapes.push(hm_model::CharShape {
            base_size: size,
            ratios: [100; 7],
            rel_sizes: [100; 7],
            ..Default::default()
        });
    }
    doc
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

/// End-of-document endnotes follow the last body line under a full-width
/// separator, but open a page of their own, at its top and without a
/// separator, when the page the body ends on has footnotes (captures
/// `c07d-endnote-pages` s1 and s3, `c06a-fn-format`).
#[test]
fn endnotes_leave_a_page_with_footnotes() {
    let only = layout(vec![section(
        [footnotes(0), endnotes(0)],
        vec![para("Body", vec![note(true, 1, "End")])],
    )]);
    assert_eq!(only.pages.len(), 1);
    let rule = &only.pages[0].rules[0];
    assert_eq!((rule.y1, rule.x2 - rule.x1), (TEXT_END + 850, 20000));
    assert_eq!(
        frames(&only, 0, FrameKind::Endnotes)[0].y,
        TEXT_END + 850 + 567
    );

    let both = layout(vec![section(
        [footnotes(0), endnotes(0)],
        vec![para(
            "Body",
            vec![note(false, 1, "Foot"), note(true, 1, "End")],
        )],
    )]);
    assert_eq!(both.pages.len(), 2);
    assert_eq!(frames(&both, 0, FrameKind::Footnotes).len(), 1);
    assert!(frames(&both, 0, FrameKind::Endnotes).is_empty());
    assert_eq!(frames(&both, 1, FrameKind::Endnotes)[0].y, 0);
    assert!(both.pages[1].rules.is_empty());
}

/// End-of-section endnotes follow the same rule section by section
/// (`c06f-sections/v-en-place1`): the first section's page has a footnote,
/// so its endnote takes the next page; the second section's endnote
/// follows its text.
#[test]
fn end_of_section_endnotes_leave_a_page_with_footnotes() {
    let at_section_end = endnotes(1 << 8);
    let l = layout(vec![
        section(
            [footnotes(0), at_section_end.clone()],
            vec![para(
                "One",
                vec![note(false, 1, "Foot"), note(true, 1, "End one")],
            )],
        ),
        section(
            [footnotes(0), at_section_end],
            vec![para("Two", vec![note(true, 2, "End two")])],
        ),
    ]);
    assert_eq!(l.pages.len(), 3);
    assert_eq!(frames(&l, 1, FrameKind::Endnotes)[0].y, 0);
    assert!(l.pages[1].rules.is_empty());
    assert_eq!(
        frames(&l, 2, FrameKind::Endnotes)[0].y,
        TEXT_END + 850 + 567
    );
}

/// The characters drawn for the reference marks of paragraph 1.
fn marks(l: &DocLayout) -> String {
    let pl = l.para(0, 1);
    pl.measured
        .items
        .iter()
        .filter_map(|(pos, k)| match k {
            hm_layout::Kind::Char(c) if *pos >= 4 => Some(*c),
            _ => None,
        })
        .collect()
}

/// Body marks take the number format of their section's note shape
/// (`c06a-fn-format`, `c06b-en-format`, `c11b-dialog-numfmt`,
/// `c17-more-formats`): the circled forms, the heavenly stems, the four
/// symbols `*, †, ‡, §` (the dialog's label), and a user character repeated
/// once per number.
#[test]
fn note_marks_take_the_shape_number_format() {
    let three = || {
        vec![para(
            "Body",
            vec![
                note(false, 1, "a"),
                note(false, 2, "b"),
                note(false, 3, "c"),
            ],
        )]
    };
    let four = || vec![para("Body", (1..=4).map(|n| note(false, n, "x")).collect())];
    let symbols = layout(vec![section([footnotes(0x80), endnotes(0)], four())]);
    assert_eq!(marks(&symbols), "*)†)‡)§)");
    for (format, want) in [
        (7, "ⓐ)ⓑ)ⓒ)"),
        (9, "㉮)㉯)㉰)"),
        (11, "㉠)㉡)㉢)"),
        (14, "㊀)㊁)㊂)"),
        (15, "갑)을)병)"),
    ] {
        let l = layout(vec![section([footnotes(format), endnotes(0)], three())]);
        assert_eq!(marks(&l), want, "format {format}");
    }
    for (user_char, want) in [('*', "*)**)***)"), ('#', "#)##)###)"), ('+', "+)++)+++)")] {
        let user = NoteShape {
            user_char: user_char as u16,
            ..footnotes(0x81)
        };
        let repeated = layout(vec![section([user, endnotes(0)], three())]);
        assert_eq!(marks(&repeated), want);
    }
    let endnote = layout(vec![section(
        [footnotes(0), endnotes(16)],
        vec![para("Body", vec![note(true, 2, "e")])],
    )]);
    assert_eq!(marks(&endnote), "乙)");
}

/// With "beneath text" (shape bit 13) footnotes follow their column's last
/// line: the separator the gap above below it, the notes the gap below
/// under the separator (`c06e-beneath-super/v-fn-beneath1`). Without it
/// they end at the body's foot.
#[test]
fn beneath_text_footnotes_follow_the_last_line() {
    let body = || vec![para("Body", vec![note(false, 1, "Foot")])];
    let beneath = layout(vec![section([footnotes(1 << 13), endnotes(0)], body())]);
    assert_eq!(beneath.pages[0].rules[0].y1, TEXT_END + 850);
    assert_eq!(
        frames(&beneath, 0, FrameKind::Footnotes)[0].y,
        TEXT_END + 850 + 567
    );
    let foot = layout(vec![section([footnotes(0), endnotes(0)], body())]);
    let f = frames(&foot, 0, FrameKind::Footnotes)[0];
    assert_eq!(f.y + f.h, 20000);
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
    s.paras
        .push(para("Body", def.into_iter().chain([long]).collect()));
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

/// Footnotes numbered anew on each page (shape numbering 2) show 1, 2 on
/// every page, in their marks and their own numbers, whatever number they
/// stored (`c06g-page-restart/v-fn-restart-page`); the section layout
/// lists those numbers for the editor to store.
#[test]
fn footnotes_restart_on_each_page() {
    let numbered = |stored: u32| {
        let mut c = note(false, stored, "Note");
        let ControlKind::Note(n) = &mut c.kind else {
            unreachable!()
        };
        let auto = Control {
            kind: ControlKind::AutoNumber(hm_model::AutoNumber {
                attr: 1,
                number: stored as u16,
                suffix: ')' as u16,
                ..Default::default()
            }),
            src: None,
        };
        let mut text = vec![18, 0, 0, 0, 0, 0, 0, 18];
        text.extend(n.list.paras[0].text.iter().copied());
        n.list.paras[0].text = text;
        n.list.paras[0].controls.insert(0, auto);
        c
    };
    // Paragraphs 1-30, one 1000 HU line each; notes on 2 and 4 (page 1)
    // and on 25 and 27 (page 2), stored as 1-4.
    let mut stored = 0;
    let paras = (1..=30)
        .map(|k| {
            if [2, 4, 25, 27].contains(&k) {
                stored += 1;
                para("Text", vec![numbered(stored)])
            } else {
                para("Text", vec![])
            }
        })
        .collect();
    let l = layout(vec![section([footnotes(2 << 10), endnotes(0)], paras)]);
    let shown: Vec<u32> = l.sections[0].note_numbers.iter().map(|(_, n)| *n).collect();
    assert_eq!(shown, [1, 2, 1, 2]);
    let mark = |para: u32| -> String {
        l.para(0, para)
            .measured
            .items
            .iter()
            .filter_map(|(pos, k)| match k {
                hm_layout::Kind::Char(c) if *pos >= 4 => Some(*c),
                _ => None,
            })
            .collect()
    };
    assert_eq!(
        [mark(2), mark(4), mark(25), mark(27)],
        ["1)", "2)", "1)", "2)"]
    );
    let page2: Vec<String> = l.pages[1]
        .frames
        .iter()
        .filter(|f| f.kind == FrameKind::Footnotes)
        .map(|f| {
            let p = &f.paras[0];
            p.measured
                .items
                .iter()
                .filter_map(|(_, k)| match k {
                    hm_layout::Kind::Char(c) => Some(*c),
                    _ => None,
                })
                .take(2)
                .collect()
        })
        .collect();
    assert_eq!(page2, ["1)", "2)"]);
}

/// A note's own number set as superscript (`supscript="1"`) is drawn like
/// the reference mark: three quarters of the note text's size, raised 21
/// percent of it (`c06a-fn-format/v-fn-superscript`: 6.71 pt over 8.99 pt
/// text, raised about 1.9 pt).
#[test]
fn superscript_note_numbers_take_the_mark_size() {
    let mut c = note(false, 1, " Note");
    let ControlKind::Note(n) = &mut c.kind else {
        unreachable!()
    };
    let p = &mut n.list.paras[0];
    p.controls.insert(
        0,
        Control {
            kind: ControlKind::AutoNumber(hm_model::AutoNumber {
                attr: 1 | 1 << 12,
                number: 1,
                suffix: ')' as u16,
                ..Default::default()
            }),
            src: None,
        },
    );
    p.text.splice(0..0, [18, 0, 0, 0, 0, 0, 0, 18]);
    let l = layout_with_size(
        vec![section(
            [footnotes(0), endnotes(0)],
            vec![para("Body", vec![c])],
        )],
        900,
    );
    let f = l.pages[0]
        .frames
        .iter()
        .find(|f| f.kind == FrameKind::Footnotes)
        .unwrap();
    let pl = &f.paras[0];
    let style = |pos: usize| &l.styles[pl.measured.styles[pos] as usize];
    assert_eq!((style(0).size, style(0).offset), (675, -189));
    assert_eq!((style(9).size, style(9).offset), (900, 0));
}

/// The 주석 모양 dialog's length presets: -1 is 5 cm, -2 2 cm, -3 a third
/// of the column and -4 the column (`c11c-dialog-line/v-fn-linelen-*`; the
/// -3 PDF draws 14177 HU over a 42520 HU column). Other lengths are HU,
/// cut at the column.
#[test]
fn separator_length_presets() {
    let rule = |length: i32| {
        let shape = NoteShape {
            separator_length: length,
            ..footnotes(0)
        };
        let l = layout(vec![section(
            [shape, endnotes(0)],
            vec![para("Body", vec![note(false, 1, "Foot")])],
        )]);
        l.pages[0].rules.first().map(|r| r.x2 - r.x1)
    };
    assert_eq!(rule(-1), Some(14173));
    assert_eq!(rule(-2), Some(5669));
    assert_eq!(rule(-3), Some(20000 / 3));
    assert_eq!(rule(-4), Some(20000));
    assert_eq!(rule(8503), Some(8503));
    assert_eq!(rule(30000), Some(20000));
    assert_eq!(rule(0), None);
}

/// A note of `lines` one-line paragraphs.
fn long_note(endnote: bool, lines: usize) -> Control {
    let mut c = note(endnote, 1, "");
    let ControlKind::Note(n) = &mut c.kind else {
        unreachable!()
    };
    n.list.paras = (0..lines).map(|_| para("Line", vec![])).collect();
    c
}

/// Endnotes longer than the room after the body go on at the top of the
/// next page, without a separator there (`c16a-endnote-overflow`).
#[test]
fn endnotes_continue_on_the_next_page_without_a_separator() {
    let l = layout(vec![section(
        [footnotes(0), endnotes(0)],
        vec![para("Body", vec![long_note(true, 30)])],
    )]);
    assert_eq!(l.pages.len(), 2);
    assert_eq!(l.pages[0].rules.len(), 1);
    assert_eq!(
        frames(&l, 0, FrameKind::Endnotes)[0].y,
        TEXT_END + 850 + 567
    );
    assert_eq!(frames(&l, 1, FrameKind::Endnotes)[0].y, 0);
    assert!(l.pages[1].rules.is_empty());
}

/// In two columns with notes under each column, a long footnote from near
/// the foot of the left column goes on in the right column's note area on
/// the same page, under its own separator, and the right column's text
/// ends above it (`c16b-fn-2col-overflow`).
#[test]
fn a_left_column_footnote_goes_on_under_the_right_column() {
    let mut s = section(
        [footnotes(0), endnotes(0)],
        (1..=50)
            .map(|k| {
                if k == 15 {
                    para("Text", vec![long_note(false, 8)])
                } else {
                    para("Text", vec![])
                }
            })
            .collect(),
    );
    let columns = Control {
        kind: ControlKind::Columns(hm_model::ColumnDef {
            attr: (2 << 2) | (1 << 12),
            ..Default::default()
        }),
        src: None,
    };
    let def = s.paras.remove(0).controls;
    s.paras
        .insert(0, para("", def.into_iter().chain([columns]).collect()));
    let l = layout(vec![s]);
    let notes = frames(&l, 0, FrameKind::Footnotes);
    assert_eq!(notes.len(), 2, "{notes:?}");
    let (left, right) = (notes[0], notes[1]);
    assert_eq!((left.x, right.x), (0, 10000));
    assert_eq!((left.y + left.h, right.y + right.h), (20000, 20000));
    let rules: Vec<_> = l.pages[0].rules.iter().filter(|r| r.y1 == r.y2).collect();
    assert_eq!(rules.len(), 2);
    let right_rule = rules.iter().find(|r| r.x1 == 10000).unwrap().y1;
    let right_text_end = l.pages[0]
        .lines
        .iter()
        .filter(|ln| ln.x == 10000)
        .map(|ln| ln.y + 1000)
        .max()
        .unwrap();
    assert!(
        right_text_end + 850 <= right_rule,
        "{right_text_end} {right_rule}"
    );
    assert!(frames(&l, 1, FrameKind::Footnotes).is_empty());
}

/// A section of `paras` in two columns: `kind` 0 newspaper, 1 balanced.
fn two_columns(shapes: [NoteShape; 2], kind: u16, paras: Vec<Paragraph>) -> Section {
    let mut s = section(shapes, paras);
    let columns = Control {
        kind: ControlKind::Columns(hm_model::ColumnDef {
            attr: kind | (2 << 2) | (1 << 12),
            ..Default::default()
        }),
        src: None,
    };
    let def = s.paras.remove(0).controls;
    s.paras
        .insert(0, para("", def.into_iter().chain([columns]).collect()));
    s
}

/// The footnote frames of every page with their page index.
fn all_footnotes(l: &DocLayout) -> Vec<(usize, &hm_layout::Frame)> {
    l.pages
        .iter()
        .enumerate()
        .flat_map(|(k, p)| p.frames.iter().map(move |f| (k, f)))
        .filter(|(_, f)| f.kind == FrameKind::Footnotes)
        .collect()
}

/// A footnote longer than two pages goes on page after page, each part
/// inside its page's body (Hancom splits a long note by line onto the next
/// page's note area, `c07a-long-note`).
#[test]
fn a_footnote_longer_than_two_pages_splits_on_every_page() {
    for after in [0, 30] {
        let mut paras = vec![para("Body", vec![long_note(false, 70)])];
        paras.extend((0..after).map(|_| para("Text", vec![])));
        let l = layout(vec![section([footnotes(0), endnotes(0)], paras)]);
        let parts = all_footnotes(&l);
        assert!(parts.len() >= 4, "{after}: {} parts", parts.len());
        for (k, f) in &parts {
            assert!(
                f.rect.y >= 0 && f.rect.y + f.rect.h <= 20000,
                "{after}: page {k} {:?}",
                f.rect
            );
        }
        let lines: usize = parts.iter().map(|(_, f)| f.lines.len()).sum();
        assert_eq!(lines, 70, "{after}");
        // Body text never runs into a page's notes.
        for (k, p) in l.pages.iter().enumerate() {
            let top = p.rules.iter().map(|r| r.y1).min().unwrap_or(20000);
            assert!(
                p.lines.iter().all(|ln| ln.y + 1000 <= top),
                "{after}: page {k}"
            );
        }
    }
}

/// Footnotes under all columns keep the room they take on the body's last
/// page when the section is laid out again, also when a note goes on past
/// that page: the left column stops above the notes.
#[test]
fn merged_footnotes_keep_their_room_when_a_note_runs_past_the_body() {
    let paras = (1..=36)
        .map(|k| {
            if k == 30 {
                para("Text", vec![long_note(false, 30)])
            } else {
                para("Text", vec![])
            }
        })
        .collect();
    let l = layout(vec![two_columns(
        [footnotes(1 << 8), endnotes(0)],
        0,
        paras,
    )]);
    for (k, p) in l.pages.iter().enumerate() {
        let Some(top) = p.rules.iter().filter(|r| r.y1 == r.y2).map(|r| r.y1).min() else {
            continue;
        };
        assert!(
            p.lines.iter().all(|ln| ln.y + 1000 + 850 <= top),
            "page {k}: lines {:?} rule {top}",
            p.lines.iter().map(|ln| (ln.x, ln.y)).collect::<Vec<_>>()
        );
    }
}

/// Footnotes that follow the text (beneath text) and go on onto the page
/// where endnotes start sit under those endnotes, not over them.
#[test]
fn beneath_text_footnotes_follow_endnotes_on_their_page() {
    let l = layout(vec![section(
        [footnotes(1 << 13), endnotes(0)],
        vec![para("Body", vec![long_note(false, 30), long_note(true, 5)])],
    )]);
    let page = l
        .pages
        .iter()
        .find(|p| p.frames.iter().any(|f| f.kind == FrameKind::Endnotes))
        .unwrap();
    let ends = page
        .frames
        .iter()
        .filter(|f| f.kind == FrameKind::Endnotes)
        .map(|f| f.rect.y + f.rect.h)
        .max()
        .unwrap();
    for f in page
        .frames
        .iter()
        .filter(|f| f.kind == FrameKind::Footnotes)
    {
        assert!(
            f.rect.y >= ends,
            "{:?} over endnotes ending at {ends}",
            f.rect
        );
    }
}

/// In balanced columns, "beneath text" footnotes follow the column's last
/// line as the balancing left it.
#[test]
fn beneath_text_footnotes_follow_balanced_columns() {
    let paras = (1..=10)
        .map(|k| {
            if k == 2 {
                para("Text", vec![note(false, 1, "Foot")])
            } else {
                para("Text", vec![])
            }
        })
        .collect();
    let l = layout(vec![two_columns(
        [footnotes(1 << 13), endnotes(0)],
        1,
        paras,
    )]);
    let p = &l.pages[0];
    let rule = p.rules.iter().find(|r| r.y1 == r.y2 && r.x1 == 0).unwrap();
    let left_end = p
        .lines
        .iter()
        .filter(|ln| ln.x == 0)
        .map(|ln| ln.y + 1000)
        .max()
        .unwrap();
    assert_eq!(rule.y1, left_end + 850);
}
