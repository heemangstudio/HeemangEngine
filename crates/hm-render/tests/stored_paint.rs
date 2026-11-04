//! Borders and fills paint as Hancom Mac prints them (probe PDFs in the
//! formatting-20261104/R notes: dashes, lines, hatches, gradients, images).
use hm_fonts::FontSystem;
use hm_model::{BorderFill, BorderLine, Fill, SolidFill};
use hm_render::paint::{self, Brush, GradientBrush, Sides};
use hm_render::{DisplayList, Item, raster};

/// HU per pixel at the dpi the tests draw at.
const DPI: f32 = 720.0;

fn page() -> DisplayList {
    DisplayList {
        width: 12000,
        height: 9000,
        items: Vec::new(),
    }
}

fn ink(dl: &DisplayList) -> tiny_skia::Pixmap {
    raster::render(dl, &FontSystem::new(), DPI).unwrap()
}

fn dark(pm: &tiny_skia::Pixmap, x: u32, y: u32) -> bool {
    pm.pixel(x, y).is_some_and(|p| p.red() < 128)
}

fn line(kind: u8, width: u8) -> BorderLine {
    BorderLine {
        kind,
        width,
        color: 0,
    }
}

/// Lengths in HU of the alternating ink and gap runs along row `y`.
fn runs(pm: &tiny_skia::Pixmap, y: u32, from: u32, to: u32) -> Vec<u32> {
    let mut out = Vec::new();
    let mut state = dark(pm, from, y);
    let mut len = 0;
    for x in from..to {
        if dark(pm, x, y) == state {
            len += 1;
        } else {
            out.push(len * 10);
            state = !state;
            len = 1;
        }
    }
    out
}

#[test]
fn border_dashes_and_dots_have_hancoms_lengths() {
    // 0.5 mm lines: Hancom prints dashes of 1032 HU with gaps of 612, and
    // dots of 204 with gaps of 300.
    for (kind, on, off) in [(2, 1032, 612), (3, 204, 300)] {
        let mut dl = page();
        paint::border_line(&mut dl, 1000, 3000, 11000, 3000, &line(kind, 7));
        let pm = ink(&dl);
        // The line starts with a dash at x = 1000 HU; the last run is cut.
        let r = runs(&pm, 300, 100, 1100);
        for pair in r[..r.len() - 1].chunks(2) {
            assert!(pair[0].abs_diff(on) <= 20, "kind {kind}: {r:?}");
            if let Some(&gap) = pair.get(1) {
                assert!(gap.abs_diff(off) <= 20, "kind {kind}: {r:?}");
            }
        }
    }
}

#[test]
fn circle_borders_dot_every_twice_the_width() {
    let mut dl = page();
    paint::border_line(&mut dl, 1000, 3000, 11000, 3000, &line(7, 7));
    let pm = ink(&dl);
    let r = runs(&pm, 300, 100, 1100);
    // A 0.5 mm line's dots come every 288 HU.
    for pair in r[1..r.len() - 2].chunks(2) {
        assert!((pair[0] + pair[1]).abs_diff(288) <= 20, "{r:?}");
    }
}

#[test]
fn wave_borders_zigzag_above_the_edge_and_3d_kinds_print_nothing() {
    // Probes lines-hwp5 and dashes (HWP 5 kinds are unambiguous; OWPML spells 13 DOUBLEWAVE).
    let mut dl = page();
    paint::border_line(&mut dl, 1000, 3000, 11000, 3000, &line(12, 7));
    let pm = ink(&dl);
    // A 0.5 mm wave fills the band from 132 HU above the edge to 12 below,
    // and leaves the rest clear.
    let rows: Vec<u32> = (250..320)
        .filter(|&y| (100..1100).any(|x| dark(&pm, x, y)))
        .collect();
    let (top, bottom) = (rows[0] * 10, rows[rows.len() - 1] * 10);
    assert!(top.abs_diff(3000 - 132 - 18) <= 20, "{top}");
    assert!(bottom.abs_diff(3000 + 12 + 18) <= 20, "{bottom}");
    // A double wave repeats it three strokes (108 HU) down and along.
    let mut dl = page();
    paint::border_line(&mut dl, 1000, 3000, 11000, 3000, &line(13, 7));
    let pm = ink(&dl);
    let rows: Vec<u32> = (250..330)
        .filter(|&y| (100..1100).any(|x| dark(&pm, x, y)))
        .collect();
    assert!(
        (rows[rows.len() - 1] * 10).abs_diff(3000 + 12 + 18 + 108) <= 20,
        "{rows:?}"
    );
    for kind in 14..=17 {
        let mut dl = page();
        paint::border_line(&mut dl, 1000, 3000, 11000, 3000, &line(kind, 7));
        let bf = BorderFill {
            sides: [line(kind, 13); 4],
            ..Default::default()
        };
        paint::border_sides(&mut dl, 2000, 2000, 6000, 4000, &bf, Sides::ALL);
        assert!(dl.items.is_empty(), "kind {kind}");
    }
}

#[test]
fn double_border_bands_close_into_rectangles() {
    // A 3 mm double line: bands 212 wide, 319 either side of the edge.
    let bf = BorderFill {
        sides: [line(8, 13); 4],
        ..Default::default()
    };
    let mut dl = page();
    paint::border_sides(&mut dl, 2000, 2000, 6000, 4000, &bf, Sides::ALL);
    let pm = ink(&dl);
    let at = |x: u32, y: u32| dark(&pm, x / 10, y / 10);
    // The outer vertical band reaches up to the outer top band's edge...
    assert!(at(1681, 1620));
    // ...and the inner one stops at the inner top band.
    assert!(!at(2319, 2150));
    // The inner top band stops at the inner vertical band.
    assert!(!at(2150, 2319));
    assert!(at(1620, 1681));
}

/// Pixels of `brush` over the whole page at 72 dpi (100 HU per pixel).
fn brushed(brush: Brush, (w, h): (i32, i32)) -> tiny_skia::Pixmap {
    let dl = DisplayList {
        width: w,
        height: h,
        items: vec![Item::Fill {
            x: 0,
            y: 0,
            w,
            h,
            brush,
        }],
    };
    raster::render(&dl, &FontSystem::new(), 72.0).unwrap()
}

#[test]
fn hatches_tile_hancoms_bitmaps_from_the_page_bottom() {
    let (w, h) = (4000, 4250);
    for (style, set) in [
        (1u8, (|_: u32, y: u32| y == 3) as fn(u32, u32) -> bool),
        (2, |x, _| x == 4),
        (3, |x, y| x == 7 - y),
        (4, |x, y| x == y),
    ] {
        let pm = brushed(
            Brush::Hatch {
                color: 0,
                style,
                origin: (0, h),
                face: None,
            },
            (w, h),
        );
        // Tile rows count from the page's bottom edge, 50 HU below a pixel row.
        for y in 0..40u32 {
            for x in 0..40u32 {
                let row = (y as i32 * 100 + 50 - h).rem_euclid(800) as u32 / 100;
                assert_eq!(dark(&pm, x, y), set(x % 8, row), "style {style} at {x},{y}");
            }
        }
    }
}

#[test]
fn a_translucent_color_fill_blends_with_the_page() {
    let fill = Fill {
        kind: Fill::SOLID,
        solid: Some(SolidFill {
            background: 0x0000_00FF,
            pattern_color: 0,
            pattern: -1,
        }),
        alpha: vec![128],
        ..Default::default()
    };
    let mut dl = page();
    paint::fill_rect(&mut dl, 1000, 1000, 4000, 4000, &fill);
    let pm = raster::render(&dl, &FontSystem::new(), 72.0).unwrap();
    let p = pm.pixel(30, 30).unwrap();
    assert_eq!(p.red(), 255);
    assert!(
        p.green().abs_diff(128) <= 2 && p.blue().abs_diff(128) <= 2,
        "{p:?}"
    );
}

/// Hancom Mac's 8 by 8 tile of a translucent hatch over a face (probe F/hatch-alpha): the face
/// at the fill's opacity `o`, the line at `o` over it, and the tile at `o` again. Green face,
/// black horizontal lines, transparency 128: face pixels (192, 255, 192) and line pixels
/// (160, 192, 160) over white.
#[test]
fn a_translucent_hatch_paints_hancoms_tile_at_its_opacity() {
    let fill = Fill {
        kind: Fill::SOLID,
        solid: Some(SolidFill {
            background: 0x0000_FF00,
            pattern_color: 0,
            pattern: 1,
        }),
        alpha: vec![128],
        ..Default::default()
    };
    let mut dl = page();
    paint::fill_rect(&mut dl, 0, 0, 8000, 8000, &fill);
    resolve(&mut dl);
    let pm = raster::render(&dl, &FontSystem::new(), 72.0).unwrap();
    let rgb = |x, y| {
        let p = pm.pixel(x, y).unwrap();
        (p.red(), p.green(), p.blue())
    };
    let near = |(r, g, b): (u8, u8, u8), (wr, wg, wb): (u8, u8, u8)| {
        r.abs_diff(wr) <= 2 && g.abs_diff(wg) <= 2 && b.abs_diff(wb) <= 2
    };
    // Tile rows count from the page bottom (9000 HU); style 1 draws row 3.
    let row = |y: u32| (y as i32 * 100 + 50 - 9000).rem_euclid(800) / 100;
    let line_row = (0..80).find(|&y| row(y) == 3).unwrap();
    assert!(
        near(rgb(10, line_row), (160, 192, 160)),
        "{:?}",
        rgb(10, line_row)
    );
    assert!(
        near(rgb(10, line_row + 1), (192, 255, 192)),
        "{:?}",
        rgb(10, line_row + 1)
    );
}

/// Anchors hatches at the page's bottom left corner, as painting a page does.
fn resolve(dl: &mut DisplayList) {
    let bottom = dl.height;
    for item in &mut dl.items {
        if let Item::Path(p) = item {
            for b in &mut p.fill {
                if let Brush::Hatch { origin, .. } = b {
                    *origin = (0, bottom);
                }
            }
        }
    }
}

/// Hancom Mac with more than two gradient colors (probes F/grad3, grad3b-d): smooth ramps
/// (step above 100) run from the first color to the second, bands (step 1 to 100) and conical
/// fans print black; step 0 prints nothing and step 1 the first color.
#[test]
fn gradients_of_more_colors_and_edge_steps_print_as_hancoms() {
    let fill = |kind: u8, step: i32, colors: &[u32]| Fill {
        kind: Fill::GRADIENT,
        gradient: Some(hm_model::Gradient {
            kind,
            angle: 0,
            step,
            colors: colors.to_vec(),
            ..Default::default()
        }),
        ..Default::default()
    };
    let rgb3 = [0x0000_00FF, 0x0000_FF00, 0x00FF_0000];
    let paint = |f: Fill| {
        let mut dl = page();
        paint::fill_rect(&mut dl, 0, 0, 6000, 3600, &f);
        let pm = raster::render(&dl, &FontSystem::new(), 72.0).unwrap();
        [(30, 1), (30, 34)].map(|(x, y)| {
            let p = pm.pixel(x, y).unwrap();
            (p.red(), p.green(), p.blue())
        })
    };
    // Linear at angle 0 runs top to bottom: red at the top, green (not blue) at the bottom.
    let [top, bottom] = paint(fill(1, 255, &rgb3));
    assert!(
        top.0 > 240 && top.1 < 20 && bottom.1 > 230 && bottom.2 < 20,
        "{top:?} {bottom:?}"
    );
    let [top, bottom] = paint(fill(1, 128, &rgb3));
    assert!(
        top.0 > 240 && bottom.1 > 230 && bottom.2 < 20,
        "{top:?} {bottom:?}"
    );
    for f in [fill(1, 100, &rgb3), fill(2, 5, &rgb3), fill(3, 255, &rgb3)] {
        assert_eq!(paint(f), [(0, 0, 0); 2]);
    }
    assert_eq!(paint(fill(1, 0, &rgb3[..2])), [(255, 255, 255); 2]);
    assert_eq!(paint(fill(1, 1, &rgb3[..2])), [(255, 0, 0); 2]);
}

fn gradient(kind: u8, angle: i32, (cx, cy): (i32, i32)) -> Brush {
    Brush::Gradient(Box::new(GradientBrush {
        kind,
        angle,
        center_x: cx,
        center_y: cy,
        step: 255,
        step_center: 50,
        // Red to blue.
        stops: vec![(0, 0x0000_00FF), (100, 0x00FF_0000)],
    }))
}

/// Red share (0 blue to 1 red) at pixel `(x, y)`.
fn red(pm: &tiny_skia::Pixmap, x: u32, y: u32) -> f32 {
    pm.pixel(x, y).unwrap().red() as f32 / 255.0
}

#[test]
fn gradients_follow_hancoms_shapes() {
    let size = (6000, 3600);
    // Linear: mirrored about the line through the center; a turn by 180
    // degrees draws the same.
    for angle in [0, 180] {
        let pm = brushed(gradient(1, angle, (0, 0)), size);
        assert!(
            red(&pm, 30, 0) > 0.97 && red(&pm, 30, 35) < 0.05,
            "angle {angle}"
        );
    }
    let pm = brushed(gradient(1, 0, (50, 50)), size);
    assert!(red(&pm, 30, 18) > 0.97);
    assert!(red(&pm, 30, 0) < 0.05 && red(&pm, 30, 35) < 0.05);
    // At 45 degrees from the top left corner the ramp spans the box's whole
    // extent across the diagonal: the far corner stops short of blue.
    let pm = brushed(gradient(1, 45, (0, 0)), size);
    assert!(
        (red(&pm, 59, 0) - 0.375).abs() < 0.03,
        "{}",
        red(&pm, 59, 0)
    );
    // Radial: circles, the last color at the farthest corner.
    let pm = brushed(gradient(2, 0, (50, 50)), size);
    assert!((red(&pm, 0, 18) - red(&pm, 30, 0)).abs() > 0.2);
    assert!(red(&pm, 0, 0) < 0.05);
    // Conical: the first color in the angle's direction (90 points up).
    let pm = brushed(gradient(3, 90, (50, 50)), size);
    assert!(red(&pm, 30, 2) > 0.9 && red(&pm, 30, 34) < 0.1);
    // Square: squares around the center, turned by the angle, reaching the
    // last color at the farthest edge (3000 HU here) whatever the box's shape.
    let pm = brushed(gradient(4, 0, (50, 50)), size);
    assert!((red(&pm, 15, 18) - 0.5).abs() < 0.05);
    assert!((red(&pm, 30, 9) - 0.7).abs() < 0.05);
    let pm = brushed(gradient(4, 45, (50, 50)), size);
    assert!((red(&pm, 45, 18) - (1.0 - 0.354)).abs() < 0.05);
}

/// A 48 by 32 PNG, red.
fn picture() -> hm_model::SharedBytes {
    let mut pm = tiny_skia::Pixmap::new(48, 32).unwrap();
    pm.fill(tiny_skia::Color::from_rgba8(255, 0, 0, 255));
    hm_model::SharedBytes::from(pm.encode_png().unwrap())
}

fn picture_fill(mode: u8) -> tiny_skia::Pixmap {
    picture_box(mode, (9000, 6000))
}

fn picture_box(mode: u8, (w, h): (i32, i32)) -> tiny_skia::Pixmap {
    let dl = DisplayList {
        width: w,
        height: h,
        items: vec![Item::Fill {
            x: 0,
            y: 0,
            w,
            h,
            brush: Brush::Image {
                bin_id: 1,
                data: Some(picture()),
                mode,
                effect: 0,
                brightness: 0,
                contrast: 0,
            },
        }],
    };
    // 96 dpi: one picture pixel per device pixel; the box is 120 by 80.
    raster::render(&dl, &FontSystem::new(), 96.0).unwrap()
}

#[test]
fn placed_and_tiled_pictures_keep_their_own_size() {
    let painted = |pm: &tiny_skia::Pixmap, x, y| pm.pixel(x, y).unwrap().green() < 128;
    // Centered: 48 by 32 in the middle, nothing around it.
    let pm = picture_fill(6);
    assert!(painted(&pm, 60, 40));
    assert!(painted(&pm, 37, 25) && !painted(&pm, 35, 23));
    assert!(!painted(&pm, 2, 2) && !painted(&pm, 117, 77));
    // One row of tiles along the bottom.
    let pm = picture_fill(2);
    assert!(painted(&pm, 100, 70) && !painted(&pm, 100, 40));
    // One column along the right.
    let pm = picture_fill(4);
    assert!(painted(&pm, 100, 10) && !painted(&pm, 60, 10));
    // A picture taller than its box squeezes to the box's height and keeps
    // its own width (probe images-big): 48 by 20 in a 120 by 20 box.
    let pm = picture_box(6, (9000, 1500));
    assert!(painted(&pm, 60, 1) && painted(&pm, 60, 18));
    assert!(painted(&pm, 37, 10) && !painted(&pm, 34, 10));
}
