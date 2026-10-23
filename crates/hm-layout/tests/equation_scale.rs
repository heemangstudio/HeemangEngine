#[allow(dead_code)]
mod common;

use hm_layout::math::{self, MathItem};
use hm_model::{Equation, HwpString};

#[test]
fn serialized_scale_retains_authored_and_script_sizes() {
    let Some((_, fonts)) = common::setup() else {
        return;
    };
    let measure = |script: &str, size| {
        math::layout(
            &Equation {
                script: HwpString::from_text(script),
                size,
                version: Some(HwpString::from_text("Equation Version 60")),
                font: Some(HwpString::from_text("HYhwpEQ")),
                ..Default::default()
            },
            &fonts,
        )
    };
    for (script, plain) in [
        ("scale 50 rm ABC", "rm ABC"),
        ("SCALE 50 rm ABC", "rm ABC"),
        ("Scale 50 rm ABC", "rm ABC"),
        ("rm scale 50 ABC", "rm ABC"),
        ("scale 50 ABC", "ABC"),
        ("{scale 50 ABC} DEF", "{ABC} DEF"),
        ("scale 50 {ABC over 3}", "{ABC over 3}"),
        ("{scale 90 m over n}", "{m over n}"),
        ("{scale 70 분자 over 분모}", "{분자 over 분모}"),
        ("sqrt{scale 50 rm ABC}", "sqrt{rm ABC}"),
        ("scale 50 sqrt{rm ABC}", "sqrt{rm ABC}"),
        (
            "scale 50 {ABC + {rm DEF} + G^{2}}",
            "{ABC + {rm DEF} + G^{2}}",
        ),
    ] {
        assert_eq!(measure(script, 2200), measure(plain, 2200), "{script}");
    }
    let m = measure("scale 50 {ABC + {rm DEF} + G^{2}}", 2200);
    let sizes: Vec<_> = m
        .items
        .iter()
        .filter_map(|i| match i {
            MathItem::Glyph { size, .. } => Some(*size),
            _ => None,
        })
        .collect();
    assert!(sizes.contains(&1496));
    assert!(sizes.iter().all(|s| matches!(s, 2200 | 1496)));
    for (script, width) in [
        ("scale 50 ABC", 4950),
        ("scale 50 {ABC over 3}", 6050),
        ("{scale 90 m over n}", 2855),
    ] {
        assert_eq!(measure(script, 2200).width, width, "{script}");
    }
    // Changing the authored size must still change measuring and paint size.
    for (size, width) in [(1100, 1956), (2200, 4252)] {
        let m = measure("rm ABC", size);
        assert_eq!(m.width, width);
        assert!(
            m.items
                .iter()
                .all(|i| matches!(i, MathItem::Glyph { size: s, .. } if *s == size as i32))
        );
    }
}
