#[allow(dead_code)]
mod common;

use hm_layout::math::{self, MathItem};
use hm_model::{Equation, HwpString};

#[test]
fn combination_indices_stay_on_opposite_sides_for_both_hancom_spellings() {
    let Some((_, fonts)) = common::setup() else {
        return;
    };
    let measure = |script: &str| {
        math::layout(
            &Equation {
                script: HwpString::from_text(script),
                size: 1100,
                font: Some(HwpString::from_text("HYhwpEQ")),
                ..Default::default()
            },
            &fonts,
        )
    };
    let mut layouts = Vec::new();
    for script in ["rm C LSUB {3} _{1} TIMES 3", "rm {C_1} LSUB {3} TIMES 3"] {
        let layout = measure(script);
        let glyphs: Vec<_> = layout
            .items
            .iter()
            .filter_map(|item| match item {
                MathItem::Glyph { ch, x, y, size, .. } => Some((*ch, *x, *y, *size)),
                _ => None,
            })
            .collect();
        let base = glyphs.iter().find(|g| g.0 == 'C').unwrap();
        let left = glyphs
            .iter()
            .find(|g| g.0 == math::pua('3', false).unwrap() && g.3 < base.3)
            .unwrap();
        let right = glyphs
            .iter()
            .find(|g| g.0 == math::pua('1', false).unwrap())
            .unwrap();
        let times = glyphs
            .iter()
            .find(|g| g.0 == math::pua('×', false).unwrap_or('×'))
            .unwrap();
        assert!(
            left.1 < base.1 && base.1 < right.1 && right.1 < times.1,
            "{script}: {glyphs:?}"
        );
        assert!(left.2 > base.2 && right.2 > base.2, "{script}: {glyphs:?}");
        layouts.push(layout);
    }
    assert_eq!(layouts[0], layouts[1]);
}
