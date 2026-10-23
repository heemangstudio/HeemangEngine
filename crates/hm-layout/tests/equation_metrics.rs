#[allow(dead_code)]
mod common;

use hm_layout::math;
use hm_model::{Equation, HwpString};

#[test]
fn ordinary_equation_width_changes_after_the_measuring_font_tie() {
    let Some((_, fonts)) = common::setup() else {
        return;
    };
    for (size, width) in [
        (974, 1687),
        (975, 1687),
        (976, 1956),
        (1100, 1956),
        (1124, 1956),
        (1125, 1956),
        (1126, 2294),
        (2174, 3981),
        (2175, 3981),
        (2176, 4252),
    ] {
        let eq = Equation {
            script: HwpString::from_text("rm ABC"),
            size,
            font: Some(HwpString::from_text("HYhwpEQ")),
            ..Default::default()
        };
        assert_eq!(math::layout(&eq, &fonts).width, width, "size {size}");
    }
}
