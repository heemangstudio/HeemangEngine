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
