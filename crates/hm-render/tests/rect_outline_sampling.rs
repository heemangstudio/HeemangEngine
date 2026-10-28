//! Rectangle outlines sit where Hancom's PDF puts them and take the sampled
//! coverage of the rasterizer that draws it. The expected gray values come
//! from MuPDF 1.28.5 drawing the same rectangles as Hancom writes them, in
//! whole 1/600 inch units under a 0.1199951 point scale, stroked black at
//! 200 dpi.

use hm_fonts::FontSystem;
use hm_render::draw::{Outline, Path, PathEl, PathItem};
use hm_render::{Dash, DisplayList, Item};

fn rect(l: i32, t: i32, r: i32, b: i32, width: i32) -> Item {
    Item::Path(Box::new(PathItem {
        path: Path {
            els: vec![
                PathEl::Move(l, t),
                PathEl::Line(r, t),
                PathEl::Line(r, b),
                PathEl::Line(l, b),
                PathEl::Close,
            ],
        },
        fill: vec![],
        outline: Some(Outline {
            width,
            color: 0,
            dash: Dash::Solid,
            round_cap: false,
        }),
        fill_alpha: 0,
    }))
}

#[test]
fn rectangle_outlines_match_the_sampled_pdf_raster() {
    let rects = [
        (1032, 1248, 2796, 3180, 12),
        (3215, 1253, 5520, 2907, 60),
        (1105, 3507, 3311, 5921, 36),
        (3907, 3333, 6020, 6110, 24),
    ];
    let dl = DisplayList {
        width: 6400,
        height: 6400,
        items: rects
            .iter()
            .map(|&(l, t, r, b, w)| rect(l, t, r, b, w))
            .collect(),
    };
    let pixmap = hm_render::raster::render(&dl, &FontSystem::new(), 200.0).unwrap();
    let gray = |x: u32, y: u32| pixmap.pixel(x, y).unwrap().green();
    // (vertical run?, fixed coordinate, first varying coordinate, values)
    let profiles: [(bool, u32, u32, [u8; 5]); 16] = [
        (true, 53, 32, [255, 255, 170, 255, 255]),
        (true, 53, 86, [255, 255, 170, 255, 255]),
        (false, 61, 26, [255, 255, 165, 255, 255]),
        (false, 61, 75, [255, 255, 165, 255, 255]),
        (true, 121, 32, [255, 204, 0, 136, 255]),
        (true, 121, 78, [255, 204, 0, 136, 255]),
        (false, 57, 87, [255, 119, 0, 225, 255]),
        (false, 57, 151, [255, 119, 0, 225, 255]),
        (true, 61, 95, [255, 204, 50, 255, 255]),
        (true, 61, 162, [255, 204, 50, 255, 255]),
        (false, 130, 28, [255, 255, 29, 225, 255]),
        (false, 130, 89, [255, 255, 119, 135, 255]),
        (true, 137, 90, [255, 255, 84, 255, 255]),
        (true, 137, 167, [255, 255, 84, 255, 255]),
        (false, 131, 106, [255, 255, 89, 255, 255]),
        (false, 131, 165, [255, 240, 89, 255, 255]),
    ];
    for (vertical, fixed, start, expected) in profiles {
        let actual: Vec<u8> = (0..5)
            .map(|k| {
                if vertical {
                    gray(fixed, start + k)
                } else {
                    gray(start + k, fixed)
                }
            })
            .collect();
        for (a, e) in actual.iter().zip(expected) {
            assert!(
                a.abs_diff(e) <= 1,
                "profile at {fixed}/{start}: {actual:?} vs {expected:?}"
            );
        }
    }
    // Mitred corners count each subsample once.
    for (x, y, e) in [(28, 34, 192u8), (89, 34, 0), (30, 97, 29), (108, 92, 123)] {
        assert!(
            gray(x, y).abs_diff(e) <= 1,
            "corner {x},{y}: {}",
            gray(x, y)
        );
    }
}
