//! Currency glyphs and advances measured against Hancom Office for Mac.

mod common;

use hm_layout::{measure, style::Styles};
use hm_model::{CharShape, DocInfo, FaceName, HwpString, Paragraph, Script};

#[test]
fn won_uses_the_symbol_fallback_in_loading_fail() {
    let Some((corpus, fonts)) = common::setup() else {
        return;
    };
    let Some(doc) = common::read(&corpus.join("loading-fail-01.hwp")) else {
        return;
    };
    let layout = hm_layout::layout(&doc, &fonts);
    let measured = &layout.para(0, 17).measured;
    let pos = measured
        .items
        .iter()
        .find(|(_, kind)| *kind == hm_layout::Kind::Char('₩'))
        .expect("the budget's won sign")
        .0 as usize;
    let style = &layout.styles[measured.styles[pos] as usize];
    let id = style.face.expect("a currency face");
    let face = fonts.face(id).unwrap();
    let glyph = face.glyph('₩').unwrap();
    assert!(
        !face.outline(glyph.glyph).ops.is_empty(),
        "the budget's won sign must be visible"
    );
    assert_eq!(fonts.entry(id).postscript.as_deref(), Some("HCRBatang"));
    assert_eq!((glyph.advance, glyph.em), (794, 1000));
    assert_eq!(measured.advances[pos], 1032);
}

#[test]
fn currency_slots_preserve_latin_and_hangul_measurement() {
    let Some((_, fonts)) = common::setup() else {
        return;
    };
    let info = DocInfo {
        faces: std::array::from_fn(|slot| {
            vec![FaceName {
                name: HwpString::from_text(match Script::from_index(slot) {
                    Script::Hangul => "휴먼명조",
                    Script::Latin => "Gulim",
                    _ => "함초롬바탕",
                }),
                ..Default::default()
            }]
        }),
        char_shapes: vec![CharShape {
            base_size: 1300,
            ratios: [100; 7],
            rel_sizes: [100; 7],
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut styles = Styles::new(&info, &fonts);
    for (chars, script, family) in [
        (
            "₠₡₢₣₤₥₦₧₨₩₪₫€₭₮₯₰₱₲₳₴₵￠￡￢￣￤￥￦",
            Script::Symbol,
            "함초롬바탕",
        ),
        ("$¢£¤¥", Script::Latin, "Gulim"),
        ("가ㄱ", Script::Hangul, "휴먼명조"),
    ] {
        for ch in chars.chars() {
            let para = Paragraph {
                text: format!("A{ch}A\r").encode_utf16().collect(),
                ..Default::default()
            };
            let measured = measure::measure(&para, &mut styles, (false, false), &[]);
            let style = styles.get(measured.styles[1]);
            let face = styles.face(measured.styles[1]).unwrap();
            let glyph = face.glyph(ch).unwrap();
            assert_eq!(style.script, script, "{ch}");
            assert!(
                fonts
                    .entry(style.face.unwrap())
                    .families
                    .iter()
                    .any(|f| f == family)
            );
            assert!(!face.outline(glyph.glyph).ops.is_empty(), "{ch}");
            let expected = hm_model::units::mul_div(glyph.advance, 325, glyph.em as i32) * 4;
            assert_eq!(measured.advances[1], expected, "{ch}");
        }
    }
}
