//! A character a face lacks takes the face Hancom's substitution chain for
//! its script slot (`fontinfo.dat`) names: 신명 신그래픽 has no · and draws it
//! with 굴림 after 한양그래픽, at the run's size and width ratio (Hancom 2022
//! probe t-fallback; exam-social's header: Gulim's 0.375 em dot).

#[allow(dead_code)]
mod common;

use hm_layout::{measure, style::Styles};
use hm_model::{CharRun, CharShape, DocInfo, FaceName, HwpString, Paragraph};

#[test]
fn latin_dot_missing_from_an_hft_face_takes_the_slot_chain() {
    let Ok(spec) = std::env::var("HEEMANG_FONTS") else {
        eprintln!("HEEMANG_FONTS is unset; skipping glyph fallback check");
        return;
    };
    let fonts = common::fonts(&spec);
    if fonts.family("신명 신그래픽").is_empty() || fonts.family("굴림").is_empty() {
        eprintln!("신명 신그래픽 or 굴림 is not registered; skipping");
        return;
    }
    for ratio in [100, 90] {
        let info = DocInfo {
            faces: std::array::from_fn(|_| {
                vec![FaceName {
                    name: HwpString::from_text("신명 신그래픽"),
                    ..Default::default()
                }]
            }),
            char_shapes: vec![CharShape {
                base_size: 2200,
                ratios: [ratio; 7],
                rel_sizes: [100; 7],
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut styles = Styles::new(&info, &fonts);
        let para = Paragraph {
            text: "가·가\r".encode_utf16().collect(),
            runs: vec![CharRun { pos: 0, shape: 0 }],
            ..Default::default()
        };
        let measured = measure::measure(&para, &mut styles, (false, false), &[]);
        let dot = styles.get(measured.styles[1]);
        let face = dot.face.expect("a face draws the dot");
        assert!(
            fonts
                .entry(face)
                .families
                .iter()
                .any(|f| f == "Gulim" || f == "굴림"),
            "the dot is drawn with {:?}",
            fonts.entry(face).families
        );
        // Gulim's dot is 384/1024 em wide.
        let expected = 2200 * 384 / 1024 * ratio as i32 / 100;
        assert!(
            (measured.advances[1] - expected).abs() <= 2,
            "{} vs {expected}",
            measured.advances[1]
        );
    }
}

/// so-sueop p2 (Hancom PDF): dot leaders in a face without the dot are drawn
/// all the same, from the face that would draw the character in the text.
#[test]
fn a_tab_leader_missing_from_its_face_takes_the_slot_chain() {
    let Ok(spec) = std::env::var("HEEMANG_FONTS") else {
        eprintln!("HEEMANG_FONTS is unset; skipping tab leader check");
        return;
    };
    let fonts = common::fonts(&spec);
    if fonts.family("신명 신그래픽").is_empty() || fonts.family("굴림").is_empty() {
        eprintln!("신명 신그래픽 or 굴림 is not registered; skipping");
        return;
    }
    let info = DocInfo {
        faces: std::array::from_fn(|_| {
            vec![FaceName {
                name: HwpString::from_text("신명 신그래픽"),
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
    let face = |leader: u16| {
        let mut styles = Styles::new(&info, &fonts);
        let para = Paragraph {
            text: vec![9, 4000, 0, leader, 0, 0, 0, 9, 13],
            runs: vec![CharRun { pos: 0, shape: 0 }],
            ..Default::default()
        };
        let measured = measure::measure(&para, &mut styles, (false, false), &[]);
        styles.get(measured.styles[0]).face.expect("a face")
    };
    assert_eq!(
        fonts.face(face(0)).map(|f| f.is_hft()),
        Some(true),
        "no leader, the run's face"
    );
    let dotted = fonts.face(face(3)).expect("a face for the dots");
    assert!(dotted.has('\u{00b7}'));
}

/// exam-social p2, so-sueop (Hancom PDFs): Hancom does not slant HFT glyphs.
// hwp3-sample11 p100 (Hancom PDF): bold Latin in 고딕, an HFT face without
// those letters, advances 549 HU a character at 10 pt like the HFT's own bold.
#[test]
fn a_stand_in_for_an_hft_face_widens_its_bold_as_the_hft_face_does() {
    let Ok(spec) = std::env::var("HEEMANG_FONTS") else {
        eprintln!("HEEMANG_FONTS is unset; skipping glyph fallback check");
        return;
    };
    let fonts = common::fonts(&spec);
    if fonts.family("고딕").is_empty() {
        eprintln!("고딕 is not registered; skipping");
        return;
    }
    let info = DocInfo {
        faces: std::array::from_fn(|_| {
            vec![FaceName {
                name: HwpString::from_text("고딕"),
                ..Default::default()
            }]
        }),
        char_shapes: [0, 2]
            .map(|attr| CharShape {
                base_size: 1000,
                ratios: [100; 7],
                rel_sizes: [100; 7],
                attr,
                ..Default::default()
            })
            .into(),
        ..Default::default()
    };
    let mut styles = Styles::new(&info, &fonts);
    let mut advance = |cs| {
        let style = styles.style_for(cs, hm_model::Script::Latin, Some('u'));
        let face = styles.face(style).map(|f| &**f);
        assert!(face.is_some_and(|f| !f.is_hft()), "the stand-in draws it");
        measure::natural_advance(styles.get(style), face, 'u')
    };
    assert_eq!(advance(0), 500);
    assert_eq!(advance(1), 548);
}

#[test]
fn italic_slants_outline_faces_only() {
    let Ok(spec) = std::env::var("HEEMANG_FONTS") else {
        eprintln!("HEEMANG_FONTS is unset; skipping italic check");
        return;
    };
    let fonts = common::fonts(&spec);
    if fonts.family("신명 중고딕").is_empty() || fonts.family("함초롬바탕").is_empty() {
        eprintln!("신명 중고딕 or 함초롬바탕 is not registered; skipping");
        return;
    }
    for (name, slanted) in [("신명 중고딕", false), ("함초롬바탕", true)] {
        let info = DocInfo {
            faces: std::array::from_fn(|_| {
                vec![FaceName {
                    name: HwpString::from_text(name),
                    ..Default::default()
                }]
            }),
            char_shapes: vec![CharShape {
                base_size: 1000,
                ratios: [100; 7],
                rel_sizes: [100; 7],
                attr: 1,
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut styles = Styles::new(&info, &fonts);
        let para = Paragraph {
            text: "가\r".encode_utf16().collect(),
            runs: vec![CharRun { pos: 0, shape: 0 }],
            ..Default::default()
        };
        let measured = measure::measure(&para, &mut styles, (false, false), &[]);
        assert_eq!(styles.get(measured.styles[0]).italic, slanted, "{name}");
    }
}
