//! Page furniture and face choices against Hancom Office for Mac's PDF
//! exports of probe documents (`<corpus>/../probes`) and corpus fixtures.
//!
//! Needs `HEEMANG_CORPUS` and `HEEMANG_FONTS` as in `pagination.rs`; skips
//! when either is unset or a document is missing.

mod common;

use hm_fonts::FontSystem;
use hm_layout::{DocLayout, Kind};

#[test]
fn word_tables_keep_final_spacing_and_move_past_unheld_anchors() {
    let Some((corpus, fonts)) = common::setup() else {
        return;
    };
    let Some(doc) = common::read(&corpus.join("ms_word_fresh_page_float_table_offset.hwpx")) else {
        return;
    };
    let layout = hm_layout::layout(&doc, &fonts);
    assert_eq!(layout.pages.len(), 6);
    let header = &layout.para(0, 0).lines[0];
    assert_eq!((header.height, header.baseline), (2942, 2725));
    let first = layout.para(0, 2).table(0).unwrap();
    assert_eq!(first.cells[0].h, 2120);
    let last = &first.cells[49];
    let end = last.lines.last().unwrap();
    let p = &last.paras[end.para as usize];
    let l = &p.lines[end.line as usize];
    assert_eq!(last.content, end.y + l.advance() + p.after);
    let second = layout
        .pages
        .iter()
        .enumerate()
        .flat_map(|(pi, p)| p.tables.iter().map(move |t| (pi, p, t)))
        .find(|(_, _, t)| t.para == 3)
        .unwrap();
    assert_eq!(second.0, 3);
    assert_eq!(second.2.y, second.1.body.y);
}

/// Punctuation an HFT face lacks takes the char shape's HFT symbol face at
/// its full-width cell, as Hancom draws ․ and … in 신명 중명조 (the Korean
/// exam papers) and ～ in 신명 태고딕. Punctuation the symbol face lacks
/// too takes that face's substitute: Hancom's PDFs embed 함초롬바탕 for ｢
/// and ― in 신명 중명조 runs whose symbol face is 한양신명조. Unit squares
/// keep the name's own fallback.
#[test]
fn punctuation_missing_from_an_hft_face_takes_the_symbol_slot_face() {
    use hm_layout::{measure::measure, style::Styles};
    use hm_model::{CharShape, DocInfo, FaceName, HwpString, Paragraph};

    let Some((_, fonts)) = common::setup() else {
        return;
    };
    for (body, symbol) in [
        ("신명 중명조", "한양신명조"),
        ("신명 태고딕", "신명 태그래픽"),
    ] {
        let info = DocInfo {
            faces: std::array::from_fn(|slot| {
                vec![FaceName {
                    name: HwpString::from_text(if slot == 5 { symbol } else { body }),
                    ..Default::default()
                }]
            }),
            char_shapes: vec![CharShape {
                base_size: 1000,
                ratios: [100; 7],
                rel_sizes: [100; 7],
                ..Default::default()
            }],
            ..Default::default()
        };
        let para = Paragraph {
            text: "가․…～㎜｢―\r".encode_utf16().collect(),
            ..Default::default()
        };
        let mut styles = Styles::new(&info, &fonts);
        let m = measure(&para, &mut styles, (false, false), &[]);
        let family = |p: usize| {
            let face = styles.get(m.styles[p]).face.expect("a face");
            fonts.entry(face).families[0].clone()
        };
        for p in 1..4 {
            assert_eq!(family(p), symbol, "{body} position {p}");
            assert_eq!(m.advances[p], 1000, "{body} position {p}");
        }
        assert_ne!(family(4), symbol, "{body} unit square");
        if symbol == "한양신명조" {
            for p in 5..7 {
                let face = styles.get(m.styles[p]).face.expect("a face");
                let ps = fonts.entry(face).postscript.clone();
                assert_eq!(ps.as_deref(), Some("HCRBatang"), "position {p}");
            }
        }
    }
}

#[test]
fn fixed_soft_hyphen_bullets_keep_a_half_em_slot() {
    use hm_layout::{heads::Head, para::ParaEnv, style::Styles};
    use hm_model::{CharShape, DocInfo, FaceName, HwpString, ParaHead, ParaShape, Paragraph};

    let Some((_, fonts)) = common::setup() else {
        return;
    };
    let info = DocInfo {
        faces: std::array::from_fn(|_| {
            vec![FaceName {
                name: HwpString::from_text("휴먼명조"),
                ..Default::default()
            }]
        }),
        char_shapes: vec![CharShape {
            base_size: 1400,
            ..Default::default()
        }],
        para_shapes: vec![ParaShape {
            attr1: 3 << 23,
            ..Default::default()
        }],
        ..Default::default()
    };
    let p = Paragraph {
        text: vec!['A' as u16, hm_model::PARA_END],
        ..Default::default()
    };
    let mut head = Head {
        image: None,
        text: "\u{ad}".into(),
        info: ParaHead {
            text_distance: 50,
            ..Default::default()
        },
    };
    let mut styles = Styles::new(&info, &fonts);
    let env = ParaEnv {
        column_width: 10000,
        default_tab: 4000,
    };
    let fixed = hm_layout::para::format(&p, &info, &mut styles, env, Some(&head));
    assert_eq!(fixed.lines[0].x, 1400);
    head.info.attr |= 4;
    let instance = hm_layout::para::format(&p, &info, &mut styles, env, Some(&head));
    assert!(instance.lines[0].x > fixed.lines[0].x);
}

/// In a Word-compatible document, an equation set as a character keeps its
/// line's stored metrics while Hancom sizes it to its stored height: the
/// native PDF puts the line after a 1177 HU equation 1728 HU below its top
/// (stored spacing 404), not the 1679 HU a fresh percent spacing gives.
#[test]
fn word_equation_lines_keep_their_stored_metrics() {
    let Some((corpus, fonts)) = common::setup() else {
        return;
    };
    let Some(doc) = common::read(&corpus.join("ms_word_fresh_page_float_table_offset.hwpx")) else {
        return;
    };
    let layout = hm_layout::layout(&doc, &fonts);
    let table = doc.sections[0].paras[2].controls[0].as_table().unwrap();
    let laid = layout.para(0, 2).table(0).unwrap();
    let mut checked = 0;
    for (cell, lc) in table.cells.iter().zip(&laid.cells) {
        for (para, lp) in cell.paras.iter().zip(&lc.paras) {
            let equation = para.controls.iter().any(|c| {
                c.as_object().is_some_and(|o| o.equation().is_some())
                    && c.object_common().is_some_and(|o| o.treat_as_char())
            });
            if !equation || para.line_segs.len() != 1 {
                continue;
            }
            let seg = &para.line_segs[0];
            let line = &lp.lines[0];
            assert_eq!(
                (line.height, line.baseline, line.spacing),
                (seg.height, seg.baseline, seg.spacing)
            );
            checked += 1;
        }
    }
    assert!(checked >= 3, "{checked}");
}

/// Pair kerning follows char shape bit 30 and the face's `kern` table. The
/// native PDF of `04-latin-kerning-spacing.hwpx` puts the AVATAR glyphs of
/// its 12 pt 함초롬돋움 lines at these page x origins (HU, on Hancom's 12 HU
/// grid): only paragraph 2 sets the bit and closes AV, VA, AT and TA by
/// the table's 65 and 69 units; the plain, tracked (+10, -10) and smaller
/// lines keep their advances.
#[test]
fn kerning_follows_the_char_shape_bit_and_the_kern_table() {
    let Some((corpus, fonts)) = common::setup() else {
        return;
    };
    let Some(doc) = common::read(&corpus.join("rendering-fidelity/04-latin-kerning-spacing.hwpx"))
    else {
        return;
    };
    let native: [(u32, [i32; 8]); 5] = [
        (1, [8508, 9288, 10008, 10788, 11472, 12264, 13080, 13680]),
        (2, [8508, 9216, 9840, 10548, 11148, 11928, 12756, 13356]),
        (3, [8508, 9372, 10152, 11016, 11772, 12636, 13548, 14208]),
        (4, [8508, 9216, 9852, 10560, 11172, 11880, 12612, 13152]),
        (5, [8508, 9132, 9696, 10320, 10872, 11496, 12144, 12624]),
    ];
    let layout = hm_layout::layout(&doc, &fonts);
    let mut placed = Vec::new();
    for (para, xs) in native {
        let pl = layout.pages[0]
            .lines
            .iter()
            .find(|l| l.para == para)
            .unwrap();
        let pal = layout.para(0, para);
        hm_layout::place(pal, &pal.lines[pl.line as usize], &mut placed);
        for (k, x) in xs.into_iter().enumerate() {
            let ours = pl.x + placed[k].x;
            assert!(
                (ours - x).abs() <= 12,
                "paragraph {para} glyph {k}: {ours} vs {x}"
            );
        }
    }
}

/// The next paragraph starts at a flowing table's bottom margin also below
/// the table's continued part: Hancom stores `hwpctl_API_v2.4.hwp`'s
/// paragraph 529 at 6648 HU on page 27, the 6082 HU rest of paragraph 528's
/// table and its two 283 HU margins, without 528's spacing after. Below a
/// table kept whole (paragraph 156) Hancom stores paragraph 157 at the
/// table's bottom margin as well.
#[test]
fn spacing_after_does_not_follow_a_continued_table() {
    let Some((corpus, fonts)) = common::setup() else {
        return;
    };
    let Some(doc) = common::read(&corpus.join("hwpctl_API_v2.4.hwp")) else {
        return;
    };
    let layout = hm_layout::layout(&doc, &fonts);
    for para in [529, 157] {
        let line = layout
            .pages
            .iter()
            .flat_map(|p| &p.lines)
            .find(|l| l.para == para && l.line == 0)
            .unwrap();
        let stored = doc.sections[0].paras[para as usize].line_segs[0].vpos;
        assert_eq!(line.vpos, stored, "paragraph {para}");
    }
}

/// A free picture in an inline table wraps the enclosing paragraph; a
/// picture confined to the cell grows that cell instead. The native PDFs
/// put the free picture at body + 10436 and the table below its 32825 HU box.
#[test]
fn free_cell_pictures_wrap_the_enclosing_inline_table() {
    let Some((corpus, fonts)) = common::setup() else {
        return;
    };
    for ext in ["hwp", "hwpx"] {
        let path = corpus.join(format!("ta-pic-001-r-쪽영역안제한no.{ext}"));
        let Some(mut doc) = common::read(&path) else {
            continue;
        };
        let layout = hm_layout::layout(&doc, &fonts);
        assert_eq!(layout.pages.len(), 1);
        let page = &layout.pages[0];
        let line = &page.lines[0];
        let pal = layout.para(0, line.para);
        let (control, table) = doc.sections[0].paras[0]
            .controls
            .iter()
            .enumerate()
            .find_map(|(k, c)| c.as_table().map(|t| (k, t)))
            .unwrap();
        assert_eq!(
            pal.table(control as u32).unwrap().height,
            table.header.common.height
        );
        assert_eq!(line.anchor_y, page.body.y);
        assert_eq!(line.y - line.anchor_y, 10436 + 32825);

        let table = match &mut doc.sections[0].paras[0].controls[control].kind {
            hm_model::ControlKind::Table(t) => t,
            _ => unreachable!(),
        };
        let picture = table.cells[2].paras[0]
            .controls
            .iter_mut()
            .find_map(|c| match &mut c.kind {
                hm_model::ControlKind::Object(o) => Some(o),
                _ => None,
            })
            .unwrap();
        picture.header.common.attr |= 1 << 13;
        let kept = hm_layout::layout(&doc, &fonts);
        let kept_table = kept.para(0, 0).table(control as u32).unwrap();
        assert!(kept_table.cells[2].h > 10436 + 32825);
        assert_eq!(kept.pages[0].lines[0].y, kept.pages[0].body.y);
    }
}

/// The face drawing the first character of body paragraph `para`, by
/// PostScript name (family name for HFT faces).
fn face_of(layout: &DocLayout, fonts: &FontSystem, para: u32) -> String {
    let pl = layout.para(0, para);
    let (pos, _) = pl
        .measured
        .items
        .iter()
        .find(|(_, k)| matches!(k, Kind::Char(c) if !c.is_ascii()))
        .copied()
        .expect("a character");
    let style = &layout.styles[pl.measured.styles[pos as usize] as usize];
    let e = fonts.entry(style.face.expect("a face"));
    e.postscript
        .clone()
        .unwrap_or_else(|| e.families[0].clone())
}

/// Missing faces fall to the face record's substitute only when it names
/// a face by the name Hancom lists it under (its Korean name when it has
/// one), to Hancom's stand-ins for a few names, and otherwise to
/// 함초롬돋움, whatever the name or design class. The expected faces are
/// those Hancom embedded for each probe paragraph.
#[test]
fn missing_faces_fall_back_as_hancom_draws_them() {
    let Some((corpus, fonts)) = common::setup() else {
        return;
    };
    let cases: [(&str, &[(u32, &str)]); 3] = [
        (
            "font-subst.hwpx",
            &[
                (1, "HCRDotum"),
                (2, "HCRDotum"),
                (3, "Haansoft-Batang"),
                (4, "HCRBatang"),
                (5, "HCRDotum"),
                (6, "HCRDotum"),
                (7, "HCRDotum"),
                (10, "HCRDotum"),
                (11, "HCRDotum"),
                (12, "신명 신명조"),
                (13, "HCRDotum"),
                (14, "HCRDotum"),
                (15, "Haansoft-Batang"),
                (16, "HCRDotum"),
                (17, "Gungsuh"),
            ],
        ),
        (
            "font-names.hwpx",
            &[
                (1, "HCRBatang"),
                (3, "Haansoft-Batang"),
                (5, "Gulim"),
                (6, "Gulim"),
                (7, "HCRDotum"),
                (8, "Haansoft-Batang"),
                (10, "BatangChe"),
                (12, "MalgunGothic-Regular"),
                (15, "Haansoft-Dotum"),
                (16, "HCRDotum"),
                (19, "HCRDotum"),
                (23, "GulimChe"),
                (26, "Haansoft-Dotum"),
                (30, "HCRDotum"),
            ],
        ),
        (
            "font-subs3.hwpx",
            &[
                (1, "Gulim"),
                (2, "MalgunGothic-Regular"),
                (3, "HCRDotum"),
                (7, "BatangChe"),
                (8, "Haansoft-Dotum"),
                (13, "HMKMM"),
                (15, "HCRDotum"),
                (16, "HCRDotum"),
                (17, "HCRDotum"),
                (21, "HCRDotum"),
                (22, "GungsuhChe"),
            ],
        ),
    ];
    for (name, want) in cases {
        let Some(doc) = common::read(&corpus.join("../probes/fontsub").join(name)) else {
            continue;
        };
        let layout = hm_layout::layout(&doc, &fonts);
        for &(para, face) in want {
            assert_eq!(
                face_of(&layout, &fonts, para),
                face,
                "{name} paragraph {para}"
            );
        }
    }
}

/// A note's reference mark is set at three quarters of its run's size,
/// raised 21 percent of the base size, or 10 percent above a superscript
/// run's raised baseline. `probes/supsub/notemark.hwpx` places marks after
/// plain and superscript runs of 10, 12, 20 and 30 pt.
#[test]
fn note_marks_take_hancoms_size_and_raise() {
    let Some((corpus, fonts)) = common::setup() else {
        return;
    };
    let Some(doc) = common::read(&corpus.join("../probes/supsub/notemark.hwpx")) else {
        return;
    };
    let layout = hm_layout::layout(&doc, &fonts);
    for (k, base) in [1000, 1200, 2000, 3000].into_iter().enumerate() {
        let pl = layout.para(0, k as u32 + 1);
        let marks: Vec<&hm_layout::Style> = pl
            .measured
            .items
            .iter()
            .filter(|(_, kind)| matches!(kind, Kind::Char(')')))
            .map(|(pos, _)| &layout.styles[pl.measured.styles[*pos as usize] as usize])
            .collect();
        assert_eq!(marks.len(), 2, "paragraph {}", k + 1);
        assert_eq!(
            (marks[0].size, marks[0].offset),
            (base * 3 / 4, -base * 21 / 100)
        );
        let sup = base * 16 / 25;
        assert_eq!(
            (marks[1].size, marks[1].offset),
            (sup * 3 / 4, -base * 54 / 100)
        );
    }
}

/// Right-aligned lines end at the margin with their trailing ordinary
/// blanks hanging past it, but no-break and fixed-width blanks count as
/// content, as `probes/blanks/trailing.hwpx` shows.
#[test]
fn trailing_fixed_and_no_break_blanks_are_content() {
    let Some((corpus, fonts)) = common::setup() else {
        return;
    };
    let Some(doc) = common::read(&corpus.join("../probes/blanks/trailing.hwpx")) else {
        return;
    };
    let layout = hm_layout::layout(&doc, &fonts);
    // Right-aligned paragraphs 7-12: no tail, a space, one and two
    // fixed-width blanks (248 HU each), a no-break blank (500 HU) and two
    // spaces. Hancom's glyphs end this far short of the margin.
    let short = [0, 0, 248, 496, 500, 0];
    let mut placed = Vec::new();
    for (k, want) in short.into_iter().enumerate() {
        let pl = layout.para(0, 7 + k as u32);
        let line = &pl.lines[0];
        hm_layout::place(pl, line, &mut placed);
        let end = placed
            .iter()
            .filter(|p| matches!(p.kind, Kind::Char(c) if c != ' '))
            .map(|p| p.x + p.advance)
            .max()
            .unwrap();
        assert_eq!(line.x + line.width - end, want, "paragraph {}", 7 + k);
    }
}

/// Hancom applies a new page number stored under code 21 and ignores one
/// another writer stored under code 18; mel-001 numbers its second page 1.
#[test]
fn new_page_numbers_need_their_page_control_code() {
    let Some((corpus, fonts)) = common::setup() else {
        return;
    };
    for (name, want) in [
        ("multi-table-001.hwp", 2),
        ("multi-table-002.hwp", 1),
        ("mel-001.hwp", 1),
    ] {
        let Some(doc) = common::read(&corpus.join(name)) else {
            continue;
        };
        let layout = hm_layout::layout(&doc, &fonts);
        assert_eq!(layout.pages[1].number, want, "{name}");
    }
}

/// A footnote in a text box goes to the foot of the page like a body one:
/// footnote-tbox-01 shows both its notes.
#[test]
fn footnotes_in_text_boxes_reach_the_page_foot() {
    let Some((corpus, fonts)) = common::setup() else {
        return;
    };
    let Some(doc) = common::read(&corpus.join("hwpx/footnote-tbox-01.hwpx")) else {
        return;
    };
    let layout = hm_layout::layout(&doc, &fonts);
    let notes = layout.pages[0]
        .frames
        .iter()
        .filter(|f| f.kind == hm_layout::FrameKind::Footnotes)
        .count();
    assert_eq!(notes, 2);
}

/// Endnotes start under the body's last line without the spacing below it:
/// the separator there, the first note the shape's gap below it. In this
/// exam (gaps 0 above and 576 below) Hancom's first note line tops at
/// 26463 HU on page 10 (its baseline 27228 less 765).
#[test]
fn endnotes_open_under_the_last_body_line() {
    let Some((corpus, fonts)) = common::setup() else {
        return;
    };
    let path = corpus.join("3-11월_실전_통합_2024-구분선위0미주사이7구분선아래2.hwpx");
    let Some(doc) = common::read(&path) else {
        return;
    };
    let layout = hm_layout::layout(&doc, &fonts);
    let first = layout.pages[9]
        .frames
        .iter()
        .find(|f| f.kind == hm_layout::FrameKind::Endnotes)
        .expect("endnotes on page 10");
    assert!((first.rect.y - 26463).abs() <= 12, "{}", first.rect.y);
}

/// The page number of a document without a page number style is drawn in
/// 함초롬돋움 at 10 pt.
#[test]
fn page_numbers_default_to_the_builtin_style() {
    let Some((corpus, fonts)) = common::setup() else {
        return;
    };
    let path = corpus.join("task2093/1192000_hydrogen_policy_research.hwp");
    let Some(doc) = common::read(&path) else {
        return;
    };
    let layout = hm_layout::layout(&doc, &fonts);
    let mark = layout.pages[3].marks.first().expect("a page number");
    for &(_, si, _) in &mark.glyphs {
        let style = &layout.styles[si as usize];
        assert_eq!(style.size, 1000);
        let face = fonts.entry(style.face.unwrap());
        assert_eq!(face.postscript.as_deref(), Some("HCRDotum"));
    }
}

/// Hancom lays out a paragraph anew from the line holding a table the text
/// wraps around, as it does for drawing objects. In this workbook the [A]
/// bracket is such a table, and Hancom breaks its paragraph's first line
/// before 공중에 where the stored line ran on (wire position 50, not 51).
#[test]
fn paragraphs_beside_square_wrapped_tables_break_anew() {
    let Some((corpus, fonts)) = common::setup() else {
        return;
    };
    let Some(doc) = common::read(&corpus.join("hwpx/[2027] 온새미로 1 본교재.hwpx")) else {
        return;
    };
    let layout = hm_layout::layout(&doc, &fonts);
    let pl = layout.para(3, 102);
    assert_eq!(pl.lines[0].end, 50);
    assert_eq!(layout.pages.len(), 47);
}

/// A flowing table cut across the columns of a page goes on in each
/// column it reaches: on the manual's two-column landscape forms
/// (section 12), the two fragments of paragraph 12's table on its second
/// page sit in the left and the right column.
#[test]
fn table_fragments_follow_their_column() {
    let Some((corpus, fonts)) = common::setup() else {
        return;
    };
    let Some(doc) = common::read(&corpus.join("2025 행정업무운영 편람(최종).hwp")) else {
        return;
    };
    let layout = hm_layout::layout(&doc, &fonts);
    let page = layout
        .pages
        .iter()
        .filter(|p| p.section == 12 && p.tables.iter().any(|t| t.para == 12))
        .nth(1)
        .unwrap();
    let xs: Vec<i32> = page
        .tables
        .iter()
        .filter(|t| t.para == 12)
        .map(|t| t.x)
        .collect();
    assert_eq!(xs.len(), 2);
    assert!(xs[1] - xs[0] > page.body.w / 2, "{xs:?}");
}

#[test]
fn continued_tables_drop_unused_height_that_overflows_the_page() {
    let Some((corpus, fonts)) = common::setup() else {
        return;
    };
    let Some(doc) = common::read(&corpus.join("task2097/1741000_project_application.hwp")) else {
        return;
    };
    let layout = hm_layout::layout(&doc, &fonts);
    assert_eq!(layout.pages.len(), 2);
    let page = &layout.pages[1];
    let part = &page.tables[0];
    let table = doc.sections[page.section as usize].paras[part.para as usize].controls
        [part.control as usize]
        .as_table()
        .unwrap();
    let tl = layout
        .para(page.section, part.para)
        .table(part.control)
        .unwrap();
    assert!(part.top > 0 && part.bottom < tl.height);
    assert_eq!(
        part.y + part.height(),
        page.body.y + page.body.h - i32::from(table.header.common.margins[3])
    );
    assert!(
        tl.cells
            .iter()
            .enumerate()
            .all(|(k, _)| { tl.cell_spans(k).iter().all(|&(_, end)| end <= part.bottom) })
    );
}

/// Hancom for Mac sizes anew a primed equation that Hancom for Windows
/// stored with the prime raised above the line: `f'(x)=6x^2-18x+a` in the
/// endnote of body paragraph 44 is stored 1163 HU tall, and Hancom's PDF
/// sets the next line about 140 HU higher than that height allows.
#[test]
fn primed_equations_stored_tall_are_sized_anew() {
    let Some((corpus, fonts)) = common::setup() else {
        return;
    };
    let path = corpus.join("3-11월_실전_통합_2024-구분선위9미주사이8구분선아래7.hwpx");
    let Some(doc) = common::read(&path) else {
        return;
    };
    let layout = hm_layout::layout(&doc, &fonts);
    let note = hm_layout::ListRef::Control {
        section: 0,
        para: 44,
        control: 0,
    };
    let frame = layout
        .pages
        .iter()
        .flat_map(|p| &p.frames)
        .find(|f| f.list == note)
        .expect("the endnote");
    let line = &frame.paras[2].lines[0];
    assert!(line.height < 1100, "{}", line.height);

    let template = doc.sections[0].paras[44].controls[0]
        .as_note()
        .unwrap()
        .list
        .paras[2]
        .controls
        .iter()
        .find(|c| c.as_object().is_some_and(|o| o.equation().is_some()))
        .unwrap();
    for (script, height, width, cached_baseline, expected, baseline) in [
        ("rm D'", 1125, 10000, 78, 900, 774),
        ("rm D prime", 1125, 10000, 78, 900, 774),
        ("rm D′", 1125, 10000, 78, 900, 774),
        ("\"prime\"", 1125, 10000, 78, 1125, 878),
        ("rm D", 1125, 10000, 78, 1125, 878),
        ("rm bar PQ", 900, 1000, 86, 1080, 950),
        ("rm {barOO'} over {barOR}", 2655, 11502, 67, 2430, 1531),
    ] {
        let mut c = template.clone();
        let hm_model::ControlKind::Object(o) = &mut c.kind else {
            unreachable!()
        };
        o.header.common.width = width;
        o.header.common.height = height;
        for part in &mut o.parts {
            if let hm_model::ObjectPart::Equation(eq) = part {
                eq.script = hm_model::HwpString::from_text(script);
                eq.size = 900;
                eq.baseline = cached_baseline;
            }
        }
        let mut styles = hm_layout::style::Styles::new(&doc.info, &fonts);
        let b = hm_layout::objects::measure_object(&c, &mut styles)
            .unwrap()
            .0
            .unwrap();
        assert_eq!((b.height, b.baseline), (expected, baseline), "{script}");
    }
}

/// Windows' taller geometric glyphs leave stale equation boxes. Hancom
/// for Mac draws the native fallback square and triangle at text height.
#[test]
fn fallback_shapes_drop_stale_height_without_flattening_tall_math() {
    use hm_layout::{math, objects::measure_object, style::Styles};
    use hm_model::{
        Control, ControlKind, CtrlId, DocInfo, Equation, HwpString, ObjectCommon, ObjectControl,
        ObjectHeader, ObjectPart,
    };

    let Some((_, fonts)) = common::setup() else {
        return;
    };
    let info = DocInfo::default();
    let equation = |script: &str| Equation {
        script: HwpString::from_text(script),
        size: 900,
        baseline: 78,
        version: Some(HwpString::from_text("Equation Version 60")),
        font: Some(HwpString::from_text("HYhwpEQ")),
        ..Default::default()
    };
    let measured = |eq: Equation, height| {
        let c = Control {
            kind: ControlKind::Object(Box::new(ObjectControl {
                id: CtrlId::EQUATION,
                header: ObjectHeader {
                    common: ObjectCommon {
                        attr: 1,
                        width: 6741,
                        height,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                parts: vec![ObjectPart::Equation(eq)],
            })),
            src: None,
        };
        let mut styles = Styles::new(&info, &fonts);
        measure_object(&c, &mut styles).unwrap().0.unwrap()
    };
    let square = equation("= □rm CQRT - triangle{rmCST}");
    let layout = math::layout(&square, &fonts);
    assert_eq!(layout.height(), 900);
    assert!(
        layout
            .items
            .iter()
            .any(|i| matches!(i, math::MathItem::Glyph { ch: '□', .. }))
    );
    let b = measured(square.clone(), 1125);
    assert_eq!((b.height, b.baseline), (900, 774));
    assert_eq!(measured(square.clone(), i32::MAX).height, 900);
    assert_eq!(measured(square, 950).height, 950);
    assert_eq!(measured(equation("= rm CQRT"), 1125).height, 1125);
    let fraction = equation("□ over x");
    let tall = math::layout(&fraction, &fonts).height();
    assert!(tall > 900);
    assert_eq!(
        measured(fraction, tall + tall / 20).height,
        tall + tall / 20
    );

    for size in [900, 1800] {
        let mut triangle = equation("rm triangle ABC");
        triangle.size = size;
        let layout = math::layout(&triangle, &fonts);
        assert!(layout.items.iter().any(|item| {
            matches!(item, math::MathItem::Glyph { ch: '∆', face, .. }
                if math::TEXT_FAMILIES.iter().any(|name| fonts.family(name).contains(&face.0)))
        }));
        let b = measured(triangle.clone(), size as i32 * 5 / 4);
        assert_eq!(
            (b.height, b.baseline),
            (size as i32, size as i32 * 86 / 100)
        );
        let near = size as i32 + size as i32 / 20;
        assert_eq!(measured(triangle, near).height, near);
    }
    for script in ["triangle over x", "root {3} of {triangle}"] {
        let eq = equation(script);
        let tall = math::layout(&eq, &fonts).height();
        assert!(tall > 900);
        assert_eq!(measured(eq, tall * 5 / 4).height, tall);
    }
    for script in ["rm △ ABC", "rm Δ ABC", "\"triangle\""] {
        assert_eq!(measured(equation(script), 1125).height, 1125, "{script}");
    }
}

/// Without a footer area Hancom sets a bottom page number halfway down the
/// bottom margin: in hwp3-sample5 (bottom margin 4252, footer 0) the digits
/// of page 2 end at 81900 HU, not on the body's bottom edge at 79936.
#[test]
fn bottom_page_numbers_without_a_footer_sit_in_the_margin() {
    let Some((corpus, fonts)) = common::setup() else {
        return;
    };
    let Some(doc) = common::read(&corpus.join("hwp3-sample5-hwp5.hwp")) else {
        return;
    };
    let layout = hm_layout::layout(&doc, &fonts);
    let mark = layout.pages[1].marks.first().expect("a page number");
    assert!((mark.baseline - 81900).abs() <= 120, "{}", mark.baseline);
}

/// A char shape that uses the font's space (attribute bit 25) measures its
/// blanks with the Latin slot's face even between Hangul: in issue2559's
/// appendix (휴먼명조 with AmeriGarmnd BT) Hancom's blank after 도 is about
/// 264 HU, a quarter em, not the Hangul face's half em.
#[test]
fn font_spaces_take_the_latin_face() {
    let Some((corpus, fonts)) = common::setup() else {
        return;
    };
    let path = corpus.join("issue2559/1341000_research_report_footnotes.hwp");
    let Some(doc) = common::read(&path) else {
        return;
    };
    let layout = hm_layout::layout(&doc, &fonts);
    let pl = layout.para(0, 795);
    let mut placed = Vec::new();
    hm_layout::place(pl, &pl.lines[5], &mut placed);
    let blank = placed
        .iter()
        .find(|p| p.kind == Kind::Space)
        .expect("a blank");
    assert!((blank.advance - 264).abs() <= 40, "{}", blank.advance);
}

/// An empty paragraph that does not fit the page foot before another empty
/// one moves on when Hancom stored it starting the next page: task1725's
/// paragraph 896 opens page 42, and 897 follows a line below it.
#[test]
fn empty_paragraphs_stored_on_the_next_page_move_there() {
    let Some((corpus, fonts)) = common::setup() else {
        return;
    };
    let path = corpus.join("task1725/text_footnote_tail_overpagination.hwp");
    let Some(doc) = common::read(&path) else {
        return;
    };
    let layout = hm_layout::layout(&doc, &fonts);
    let first = layout.pages[41].lines.first().expect("a line");
    assert_eq!(first.para, 896);
}

/// The empty paragraph before the chapter's page break occupies Mac page 36
/// even though only its footer draws. The chapter then starts on page 37.
#[test]
fn empty_paragraphs_stored_before_explicit_breaks_keep_the_blank_page() {
    let Some((corpus, fonts)) = common::setup() else {
        return;
    };
    let Some(doc) = common::read(&corpus.join("task2097/75544_pii_bunseok.hwpx")) else {
        return;
    };
    let layout = hm_layout::layout(&doc, &fonts);
    for (page, para) in [(35, 316), (36, 317)] {
        let first = layout.pages[page].lines.first().expect("a body line");
        assert_eq!(first.para, para);
        assert_eq!((first.y, first.vpos), (5670, 0));
    }
    assert_eq!(layout.pages[35].lines.len(), 1);
    assert_eq!(layout.para(0, 316).lines[0].height, 1500);
}

#[test]
fn italic_faces_are_not_sheared_twice() {
    let Some((_, fonts)) = common::setup() else {
        return;
    };
    for (family, native_italic) in [("한양견명조", true), ("함초롬바탕", false)] {
        let info = hm_model::DocInfo {
            faces: std::array::from_fn(|_| {
                vec![hm_model::FaceName {
                    name: hm_model::HwpString::from_text(family),
                    ..Default::default()
                }]
            }),
            char_shapes: vec![hm_model::CharShape {
                attr: 1,
                base_size: 3700,
                ratios: [95; 7],
                rel_sizes: [100; 7],
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut styles = hm_layout::style::Styles::new(&info, &fonts);
        let id = styles.style_for(0, hm_model::Script::Latin, Some('1'));
        let style = styles.get(id);
        let face = fonts.entry(style.face.expect("resolved digit face"));
        assert_eq!(face.italic, native_italic, "{family}");
        assert_eq!(style.italic, !native_italic, "synthetic shear for {family}");
        assert!(
            info.char_shapes[0].italic(),
            "retain the document's formatting"
        );
    }
}

#[test]
fn nested_overlay_form_continues_before_following_chapters() {
    let Some((corpus, fonts)) = common::setup() else {
        return;
    };
    let Some(doc) = common::read(&corpus.join("task2097/75544_pii_bunseok.hwpx")) else {
        return;
    };
    let layout = hm_layout::layout(&doc, &fonts);
    assert_eq!(layout.pages.len(), 66);
    let fragments: Vec<_> = layout
        .pages
        .iter()
        .enumerate()
        .flat_map(|(i, page)| {
            page.tables
                .iter()
                .filter(|t| t.para == 527)
                .map(move |_| i + 1)
        })
        .collect();
    assert_eq!(fragments, [59, 60]);
    for (para, physical) in [(531, 61), (552, 62)] {
        let first = layout.pages[physical - 1].lines.first().unwrap();
        assert_eq!((first.para, first.line, first.y), (para, 0, 5670));
    }
}

/// An inline equation whose width is kept relative to the column spans the
/// column: in the market structure survey, body paragraph 399 holds the HHI
/// equation (right caption, 26 blanks after it), and Hancom centres it,
/// keeps two blanks on its line and wraps the rest onto a second line.
#[test]
fn column_relative_equations_span_the_column() {
    let Some((corpus, fonts)) = common::setup() else {
        return;
    };
    let path = corpus.join("task2070/1130000-201900011_D0150004-1-002_2017년기준 시장구조조사.hwp");
    let Some(doc) = common::read(&path) else {
        return;
    };
    let layout = hm_layout::layout(&doc, &fonts);
    let pl = layout.para(0, 399);
    let starts: Vec<u32> = pl.lines.iter().map(|l| l.start).collect();
    assert_eq!(starts, [0, 10]);
    let column = pl.column_width;
    let b = pl.measured.boxes[0];
    assert_eq!(b.width, column);
    let (_, inner) = &pl.measured.objects[0];
    let eq = inner.equation.as_ref().unwrap().width;
    assert_eq!(inner.shift, (column - 112 - eq) / 2);
}

/// An equation stored over a tenth narrower than its script lays out was
/// boxed by another Hancom, and Hancom sizes it anew: in the September 2022
/// exam the similarity equation on body paragraph 272's second line is
/// stored 1125 HU tall, and Hancom's PDF sets the line 900 high. Equations
/// holding Hancom's private characters keep their box: the endnote's
/// `TRIANGLE rmO_1 CH_1 <U+F0040> TRIANGLE rm ACO` lines stay 1190 high, as
/// in the PDF.
#[test]
fn equations_stored_narrower_than_their_script_are_sized_anew() {
    let Some((corpus, fonts)) = common::setup() else {
        return;
    };
    let Some(doc) = common::read(&corpus.join("3-09월_교육_통합_2022.hwpx")) else {
        return;
    };
    let layout = hm_layout::layout(&doc, &fonts);
    assert_eq!(layout.para(0, 272).lines[1].height, 900);
    let note = hm_layout::ListRef::Control {
        section: 0,
        para: 429,
        control: 0,
    };
    let tall = layout
        .pages
        .iter()
        .flat_map(|p| &p.frames)
        .filter(|f| f.list == note)
        .flat_map(|f| &f.paras)
        .flat_map(|p| &p.lines)
        .filter(|l| l.height == 1190)
        .count();
    assert!(tall >= 2, "{tall}");
}
