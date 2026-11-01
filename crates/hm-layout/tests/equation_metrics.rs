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

#[test]
fn root_overline_box_clearance_does_not_grow_nested_math() {
    let Some((_, fonts)) = common::setup() else {
        return;
    };
    let layout = |script, size| {
        math::layout(
            &Equation {
                script: HwpString::from_text(script),
                size,
                font: Some(HwpString::from_text("HYhwpEQ")),
                ..Default::default()
            },
            &fonts,
        )
    };
    for size in [900, 1800] {
        for script in ["rm PQ", "rm PQ'"] {
            let m = layout(script, size);
            assert_eq!((m.height(), m.box_height()), (size as i32, size as i32));
        }
        for script in ["rm bar PQ", "rm bar PQ +bar PF'", "rm bar {F prime F}"] {
            let m = layout(script, size);
            assert_eq!(m.box_height(), (size * 6 / 5) as i32, "{script}");
            assert_eq!(m.box_baseline_percent(), 88, "{script}");
        }
    }
    // The first changed 3-09 formula in the rejected shared-ascent trial.
    let root = layout("=sqrt { rm {bar { AF prime}}^2 -{bar { AF}}}^2 }", 900);
    // Its box stands 0.15 em above the body's, past the vinculum.
    assert_eq!((root.height(), root.box_height()), (1323, 1409));
    let fraction = layout("bar {1 over 2}", 900);
    assert_eq!(fraction.box_height(), fraction.height());
    for (size, height) in [(900, 2430), (1800, 4860)] {
        let m = layout("rm {barOO'} over {barOR}", size);
        assert_eq!(m.box_height(), height, "bar fraction at {size}");
        assert_eq!(m.box_baseline_percent(), 63);
        let plain = layout("1 over 2", size);
        assert_eq!(plain.box_height(), plain.height());
    }
}

#[test]
fn radical_width_uses_its_measuring_cell_and_nominal_margin() {
    let Some((_, fonts)) = common::setup() else {
        return;
    };
    for (script, size, width) in [
        ("sqrt{3}", 900, 1458),
        ("sqrt{3}", 2200, 3636),
        ("sqrt{ABC}", 1100, 3562),
        ("sqrt{ABC}", 2200, 7574),
        ("it sqrt{ABC}", 2200, 7574),
        ("sqrt{it ABC}", 2200, 7574),
        ("root 3 of {ABC}", 2200, 7574),
        ("sqrt{A over B}", 2200, 5449),
        ("{sqrt{A over B}} over it", 2200, 6549),
        ("it over {sqrt{A over B}}", 2200, 6549),
        ("sqrt{sqrt{ABC}}", 2200, 10198),
    ] {
        let eq = Equation {
            script: HwpString::from_text(script),
            size,
            font: Some(HwpString::from_text("HYhwpEQ")),
            version: Some(HwpString::from_text("Equation Version 60")),
            ..Default::default()
        };
        assert_eq!(math::layout(&eq, &fonts).width, width, "{script} at {size}");
    }
}

#[test]
fn versionless_radicals_keep_an_em_and_an_eighth() {
    let Some((_, fonts)) = common::setup() else {
        return;
    };
    // Equations without a script version, as converted from HWP 3, take
    // the sign at its nominal em and an eighth beside the body.
    let layout = |script: &str, size: u32, version: Option<&str>| {
        math::layout(
            &Equation {
                script: HwpString::from_text(script),
                size,
                font: Some(HwpString::from_text("HYhwpEQ")),
                version: version.map(HwpString::from_text),
                ..Default::default()
            },
            &fonts,
        )
        .width
    };
    for (body, size) in [("{Q}", 1000u32), ("{ABC}", 2200)] {
        let script = format!("sqrt {body}");
        let legacy = layout(body, size, None) + (size + size / 8) as i32;
        assert_eq!(layout(&script, size, None), legacy, "{script}");
        assert_eq!(layout(&script, size, Some("")), legacy, "{script}");
        assert!(layout(&script, size, Some("Equation Version 60")) > legacy);
    }
}

#[test]
fn horizontal_ellipsis_keeps_native_operator_spacing() {
    let Some((_, fonts)) = common::setup() else {
        return;
    };
    for (script, size, width) in [
        ("CDOTS", 900, 810),
        ("CDOTS", 1000, 945),
        ("cdots cdots", 900, 1620),
        ("cdotscdots`", 900, 1921),
        ("a  _{7} =29,~ CDOTS ", 900, 4265),
        ("f(x)=x  ^{3} +` CDOTS ,", 900, 5630),
        ("n=0,``1,``2,``cdots,``5", 900, 6032),
        (
            "43+44+` CDOTS `+50= {8 TIMES  LEFT ( 43+50 RIGHT )} over {2} =372",
            900,
            14426,
        ),
        (
            "= LEFT ( {1} over {1} - {1} over {2} RIGHT ) + LEFT ( {1} over {2} - {1} over {3} RIGHT ) + CDOTS  + LEFT ( {1} over {10} - {1} over {11} RIGHT )",
            900,
            14248,
        ),
    ] {
        let eq = Equation {
            script: HwpString::from_text(script),
            size,
            font: Some(HwpString::from_text("HYhwpEQ")),
            ..Default::default()
        };
        assert_eq!(math::layout(&eq, &fonts).width, width, "{script} at {size}");
    }
}

#[test]
fn relation_spacing_preserves_ordinary_and_font_switch_contexts() {
    let Some((_, fonts)) = common::setup() else {
        return;
    };
    for (script, width) in [
        ("x GEQ 0", 2065),
        ("2 LEQ x LEQ 4", 3658),
        ("sin`theta= it -2`cos`theta", 5496),
        ("-a=2,~-2b= it -8", 6967),
        ("x= it - {1} over {2}", 3090),
        ("p=240,~q=43", 5452),
    ] {
        let eq = Equation {
            script: HwpString::from_text(script),
            size: 900,
            font: Some(HwpString::from_text("HYhwpEQ")),
            ..Default::default()
        };
        assert_eq!(math::layout(&eq, &fonts).width, width, "{script}");
    }
}

/// A capital's superscript starts 0.15 em further right. An explicit space
/// after it keeps that room, as the arrow after `rm H ^{+} \`` in a
/// chemistry equation of Hancom's PDF shows; a glyph takes it back.
#[test]
fn an_explicit_space_keeps_a_capital_superscripts_room() {
    let fonts = hm_fonts::FontSystem::new();
    let mf = math::MathFonts::new(&fonts, Some("HYhwpEQ"));
    let width = |s| math::layout_script(s, 1100, 0, &mf).width;
    let quarter = 1100 / 8;
    assert_eq!(
        width("rm H ^{+} ` rarrow") - width("rm H ^{+} rarrow"),
        quarter + 165
    );
    assert_eq!(
        width("rm h ^{+} ` rarrow") - width("rm h ^{+} rarrow"),
        quarter
    );
}

/// Hancom's stored boxes (corpus equations of 9 pt): a superscript's box
/// stands a fortieth of an em above its ink, a radical's 0.15 em above its
/// body's box. Primed equations stored tall are sized from these.
#[test]
fn superscripts_and_radicals_size_their_boxes_as_hancom_stores_them() {
    let Some((_, fonts)) = common::setup() else {
        return;
    };
    for (script, height) in [
        ("x ^{3}", 1050),
        ("= sqrt {1+4`t}", 1035),
        ("sqrt {3} ` ^{f(n)}", 1050),
    ] {
        let m = math::layout(
            &Equation {
                script: HwpString::from_text(script),
                size: 900,
                font: Some(HwpString::from_text("HYhwpEQ")),
                ..Default::default()
            },
            &fonts,
        );
        assert_eq!(m.box_height(), height, "{script}");
    }
}
