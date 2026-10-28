//! A transparent picture paints as a PDF image through its soft mask:
//! color and coverage sampled apart, then blended once into the backdrop.
//! References are native PDF rasterizations of RGB plus SMask images.
use hm_render::draw::ImageItem;
use hm_render::image::Effects;
use hm_render::{DisplayList, Item};

fn picture(data: &[u8], (w, h): (i32, i32), background: Option<u32>) -> tiny_skia::Pixmap {
    let mut dl = DisplayList {
        width: w,
        height: h,
        items: Vec::new(),
    };
    if let Some(v) = background {
        dl.items.push(Item::Rect {
            x: 0,
            y: 0,
            w,
            h,
            color: v * 0x010101,
        });
    }
    dl.items.push(Item::Image(Box::new(ImageItem {
        data: hm_model::SharedBytes::from(data),
        frame: [w, 0, 0, 0, h, 0],
        crop: [0; 4],
        natural: (0, 0),
        effects: Effects::default(),
    })));
    hm_render::raster::render_clear(&dl, &hm_fonts::FontSystem::new(), 72.0).unwrap()
}

/// Every color byte under every coverage byte, on four opaque backdrops
/// and a transparent one, at the picture's own size.
#[test]
fn coverage_moves_the_backdrop_in_256_steps() {
    let source = include_bytes!("fixtures/mask-blend-source.png");
    for (background, native) in [
        (
            Some(0),
            include_bytes!("fixtures/mask-blend-0-native.png").as_slice(),
        ),
        (
            Some(64),
            include_bytes!("fixtures/mask-blend-64-native.png").as_slice(),
        ),
        (
            Some(128),
            include_bytes!("fixtures/mask-blend-128-native.png").as_slice(),
        ),
        (
            Some(255),
            include_bytes!("fixtures/mask-blend-255-native.png").as_slice(),
        ),
        (
            None,
            include_bytes!("fixtures/mask-blend-clear-native.png").as_slice(),
        ),
    ] {
        let actual = picture(source, (25600, 25600), background);
        let expected = tiny_skia::Pixmap::decode_png(native).unwrap();
        assert_eq!(actual.width(), expected.width());
        let wrong_alpha = actual
            .pixels()
            .iter()
            .zip(expected.pixels())
            .filter(|(a, b)| a.alpha() != b.alpha())
            .count();
        assert_eq!(wrong_alpha, 0, "backdrop {background:?}");
        let worst = actual
            .data()
            .iter()
            .zip(expected.data())
            .map(|(&a, &b)| a.abs_diff(b))
            .max()
            .unwrap();
        // Storing the transparent reference as PNG can move a
        // premultiplied channel by one.
        assert!(
            worst <= u8::from(background.is_none()),
            "backdrop {background:?}: off by {worst}"
        );
    }
}
