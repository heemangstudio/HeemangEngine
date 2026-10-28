//! The packet encoding stays as the version 1 fixture records it; the JS
//! player's tests decode the same file.

use hm_edit::{ListPath, Step};
use hm_fonts::{FaceId, FontSystem};
use hm_render::draw::{ImageItem, Outline, Path, PathEl, PathItem};
use hm_render::image::Effects;
use hm_render::{Brush, Dash, DisplayList, GlyphRun, GradientBrush, Item, Stroke};
use hm_wasm::index::{Line, PageIndex};
use hm_wasm::packet;

fn run(italic: bool, bold: bool) -> GlyphRun {
    GlyphRun {
        face: FaceId(3),
        size: 1000,
        ratio: 95,
        color: 0x00_1F_4E_79,
        fake_bold: bold,
        italic,
        baseline: 8500,
        glyphs: vec![(36, 7200), (37, 7700), (1200, 8250)],
    }
}

fn sample() -> DisplayList {
    let path = Path {
        els: vec![
            PathEl::Move(1000, 1000),
            PathEl::Line(5000, 1000),
            PathEl::Cubic(6000, 1500, 6000, 3500, 5000, 4000),
            PathEl::Close,
        ],
    };
    let gradient = Brush::Gradient(Box::new(GradientBrush {
        kind: 1,
        angle: 90,
        center_x: 50,
        center_y: 50,
        step: 0,
        step_center: 50,
        stops: vec![(0, 0x0000FF), (100, 0xFF0000)],
    }));
    DisplayList {
        width: 59528,
        height: 84188,
        items: vec![
            Item::Clip {
                x: 0,
                y: 0,
                w: 30000,
                h: 40000,
            },
            Item::Clip {
                x: 500,
                y: 500,
                w: 20000,
                h: 20000,
            },
            Item::Glyphs(run(false, false)),
            Item::Glyphs(run(true, true)),
            Item::Unclip,
            Item::Rect {
                x: 7200,
                y: 9000,
                w: 1500,
                h: 50,
                color: 0,
            },
            Item::Line {
                x1: 7200,
                y1: 9500,
                x2: 20000,
                y2: 9500,
                stroke: Stroke {
                    width: 30,
                    color: 0x0000FF,
                    dash: Dash::DashDot,
                },
            },
            Item::Fill {
                x: 100,
                y: 200,
                w: 300,
                h: 400,
                brush: Brush::Solid(0xC0C0C0),
            },
            Item::Fill {
                x: 1000,
                y: 2000,
                w: 3000,
                h: 4000,
                brush: gradient.clone(),
            },
            Item::Path(Box::new(PathItem {
                path: path.clone(),
                fill: vec![Brush::Solid(0x00FF00)],
                outline: Some(Outline {
                    width: 40,
                    color: 0x000000,
                    dash: Dash::Circle,
                    round_cap: true,
                }),
                fill_alpha: 128,
            })),
            Item::Path(Box::new(PathItem {
                path,
                fill: vec![gradient],
                outline: None,
                fill_alpha: 0,
            })),
            Item::Image(Box::new(ImageItem {
                data: hm_model::SharedBytes::from(vec![0u8; 4]),
                frame: [7200, 0, 10000, 0, 3600, 20000],
                crop: [0; 4],
                effects: Effects::default(),
                natural: (0, 0),
            })),
            Item::Outline {
                run: run(false, false),
                width: 20,
                dash: vec![100, 50],
            },
            Item::Unclip,
        ],
    }
}

fn encoded() -> Vec<u8> {
    let dl = sample();
    let fonts = FontSystem::new();
    let index = PageIndex {
        cells: Vec::new(),
        lines: vec![
            Line {
                list: ListPath::default(),
                para: 2,
                line: 0,
                y: 7000,
                height: 1600,
                baseline: 1360,
                origin: 7200,
                stops: vec![(0, 7200), (1, 7700), (2, 8250), (3, 8800)],
            },
            Line {
                list: ListPath {
                    section: 0,
                    steps: vec![Step {
                        para: 4,
                        control: 1,
                        index: 7,
                    }],
                },
                para: 0,
                line: 1,
                y: 12000,
                height: 1000,
                baseline: 850,
                origin: 15000,
                stops: vec![(8, 15000), (9, 15500)],
            },
        ],
    };
    let header = packet::Header {
        page: 2,
        content_epoch: 5,
        layout_epoch: 7,
        font_generation: 1,
    };
    packet::packet(
        &header,
        &dl,
        &[
            (*b"PANT", packet::paint_block(&dl, &fonts)),
            (*b"LINE", index.block()),
            (*b"GLYF", packet::glyph_block(&dl)),
        ],
    )
}

#[test]
fn packet_matches_version_1_fixture() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/packet-v1.bin");
    let bytes = encoded();
    if std::env::var_os("HM_UPDATE_FIXTURES").is_some() {
        std::fs::write(path, &bytes).unwrap();
    }
    let fixture = std::fs::read(path).expect("fixture; set HM_UPDATE_FIXTURES=1 to write it");
    assert!(
        bytes == fixture,
        "the packet encoding changed; bump packet::VERSION and add a new fixture"
    );
}
