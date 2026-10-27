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
fn of_is_a_root_separator_only_after_the_index() {
    let Some((_, fonts)) = common::setup() else {
        return;
    };
    let letters = measure("rm bar OF = it c", &fonts);
    assert_eq!(letters.items, measure("rm bar {O F} = it c", &fonts).items);
    assert_eq!(letters.width, measure("rm bar {O F} = it c", &fonts).width);
    for script in ["ROOT3OF5", "SQRT3OF5", "root {3} of {5}"] {
        assert_eq!(
            measure(script, &fonts).items,
            measure("root 3 of 5", &fonts).items
        );
    }
    assert_eq!(
        measure("rm OF", &fonts).items,
        measure("rm {O F}", &fonts).items
    );
    assert_eq!(
        measure("\"OF\"", &fonts).items,
        measure("\"O\" \"F\"", &fonts).items
    );
}

#[test]
fn triangle_keyword_takes_the_native_increment_glyph() {
    let Some((_, fonts)) = common::setup() else {
        return;
    };
    let triangle = measure("rm triangle ABC", &fonts);
    assert_eq!(triangle.items, measure("rm ∆ ABC", &fonts).items);
    assert_ne!(triangle.items, measure("rm △ ABC", &fonts).items);
    assert_ne!(triangle.items, measure("rm Δ ABC", &fonts).items);
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

#[test]
fn limits_keep_their_operand_boundary_after_infix_parsing() {
    let Some((_, fonts)) = common::setup() else {
        return;
    };
    for body in ["{b_n}", "{f(x)}", "{f(x)} over x", "rm {b_n}"] {
        let script = format!("lim _{{n->inf}} {body} =6");
        let grouped = format!("{{lim _{{n->inf}} {body}}} =6");
        let a = measure(&script, &fonts);
        let b = measure(&grouped, &fonts);
        assert_eq!(a.items, b.items, "{script}");
        assert_eq!(a.width, b.width, "{script}");
    }
    // An explicit empty operand ends the application. A quoted name and a
    // scripted group on its own retain ordinary scripted spacing.
    for script in ["lim _{n->inf} {} {b_n} =6", "\"lim\" {b_n} =6", "{b_n} =6"] {
        let a = measure(script, &fonts);
        let grouped = script.replace("{b_n}", "{b_n {}}");
        assert_ne!(a.width, measure(&grouped, &fonts).width, "{script}");
    }
}
