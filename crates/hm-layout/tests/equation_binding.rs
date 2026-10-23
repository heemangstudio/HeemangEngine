#[allow(dead_code)]
mod common;

use hm_fonts::FontSystem;
use hm_layout::math::{self, MathItem, MathLayout};
use hm_model::{Equation, HwpString};

fn measure(script: &str, fonts: &FontSystem) -> MathLayout {
    math::layout(
        &Equation {
            script: HwpString::from_text(script),
            size: 2200,
            version: Some(HwpString::from_text("Equation Version 60")),
            font: Some(HwpString::from_text("HYhwpEQ")),
            ..Default::default()
        },
        fonts,
    )
}

fn glyphs(m: &MathLayout) -> Vec<(char, i32, i32)> {
    m.items
        .iter()
        .filter_map(|i| match i {
            MathItem::Glyph { ch, x, y, .. } => Some((*ch, *x, *y)),
            _ => None,
        })
        .collect()
}

#[test]
fn a_font_switch_is_an_infix_operand_without_consuming_the_next_letter() {
    let Some((_, fonts)) = common::setup() else {
        return;
    };
    let left = measure("A it over B", &fonts);
    let right = measure("A over it B", &fonts);
    for m in [&left, &right] {
        assert_eq!(m.width, 4400);
        let letters = glyphs(m);
        assert_eq!(letters.len(), 2);
        assert_eq!(letters[0].0, '\u{e000}');
        assert_eq!(letters[1].0, '\u{e001}');
        assert!(letters[0].1 < letters[1].1);
    }
    assert_eq!(left.items, measure("A {{} over B}", &fonts).items);
    assert_eq!(right.items, measure("{A over {}} B", &fonts).items);
    let grouped = measure("A over {it B}", &fonts);
    assert_eq!(grouped.width, 2825);
    assert!(glyphs(&grouped)[1].2 > glyphs(&right)[1].2);
}

#[test]
fn infix_reentry_restores_style_without_losing_ordinary_row_persistence() {
    let Some((_, fonts)) = common::setup() else {
        return;
    };
    assert_eq!(
        measure("A rm over B", &fonts).items,
        measure("A it over B", &fonts).items
    );
    assert_eq!(
        glyphs(&measure("rm A B", &fonts))
            .iter()
            .map(|g| g.0)
            .collect::<Vec<_>>(),
        vec!['A', 'B']
    );
    assert_eq!(
        glyphs(&measure("{rm A} B", &fonts))
            .iter()
            .map(|g| g.0)
            .collect::<Vec<_>>(),
        vec!['A', '\u{e001}']
    );
    assert_eq!(
        measure("A over B it", &fonts).items,
        measure("it A over B", &fonts).items
    );
}
