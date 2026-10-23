//! Missing character styles use Hancom's plain 10 pt fallback; valid styles
//! keep their explicit shading and measurement.
use hm_fonts::FontSystem;
use hm_layout::{measure, style::Styles};
use hm_model::{CharRun, CharShape, DocInfo, Paragraph};

#[test]
fn missing_styles_do_not_create_black_character_cells() {
    let info = DocInfo {
        char_shapes: vec![CharShape {
            base_size: 2400,
            ratios: [80; 7],
            rel_sizes: [75; 7],
            color: 0x0000_00FF,
            shade_color: 0,
            ..Default::default()
        }],
        ..Default::default()
    };
    let fonts = FontSystem::new();
    let mut styles = Styles::new(&info, &fonts);
    for id in [0, 11, u32::MAX] {
        let para = Paragraph {
            text: "A B\r".encode_utf16().collect(),
            runs: vec![CharRun { pos: 0, shape: id }],
            ..Default::default()
        };
        let measured = measure::measure(&para, &mut styles, (false, false), &[]);
        for &si in &measured.styles[..3] {
            let style = styles.get(si);
            if id == 0 {
                assert_eq!((style.size, style.line_size, style.ratio), (1800, 2400, 80));
                assert_eq!((style.color, style.shade), (0x0000_00FF, Some(0)));
            } else {
                assert_eq!(
                    (style.size, style.line_size, style.ratio),
                    (1000, 1000, 100)
                );
                assert_eq!((style.color, style.shade), (0, None));
            }
        }
        assert_eq!(measured.advances[1], if id == 0 { 720 } else { 500 });
        assert_eq!(measured.natural[1], measured.advances[1]);
    }
}

#[allow(dead_code)]
mod common;

#[test]
fn missing_styles_use_the_default_font_instead_of_document_face_zero() {
    use hm_model::{FaceName, HwpString, Script};
    let Ok(spec) = std::env::var("HEEMANG_FONTS") else {
        eprintln!("HEEMANG_FONTS is unset; skipping font compatibility check");
        return;
    };
    let fonts = common::fonts(&spec);
    let info = DocInfo {
        faces: std::array::from_fn(|_| {
            vec![FaceName {
                name: HwpString::from_text("한컴바탕"),
                ..Default::default()
            }]
        }),
        char_shapes: vec![CharShape {
            base_size: 1000,
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut styles = Styles::new(&info, &fonts);
    for (id, expected) in [
        (0, "Haansoft-Batang"),
        (11, "HCRDotum"),
        (u32::MAX, "HCRDotum"),
    ] {
        let si = styles.style_for(id, Script::Latin, Some('A'));
        let face_id = styles.get(si).face.expect("registered compatibility face");
        assert_eq!(fonts.entry(face_id).postscript.as_deref(), Some(expected));
        let face = styles.face(si).unwrap();
        let glyph = face.glyph('A').unwrap();
        assert!(!face.outline(glyph.glyph).ops.is_empty());
    }
}
