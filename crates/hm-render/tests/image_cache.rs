//! Reopening images after their previous document was dropped must not reuse its pixels.
use hm_render::draw::ImageItem;
use hm_render::image::Effects;
use hm_render::{DisplayList, Item};

#[test]
fn successive_documents_keep_their_own_image_pixels() {
    for value in 0..128u8 {
        let mut bmp = vec![0u8; 58];
        bmp[..2].copy_from_slice(b"BM");
        bmp[2..6].copy_from_slice(&58u32.to_le_bytes());
        bmp[10..14].copy_from_slice(&54u32.to_le_bytes());
        bmp[14..18].copy_from_slice(&40u32.to_le_bytes());
        bmp[18..22].copy_from_slice(&1u32.to_le_bytes());
        bmp[22..26].copy_from_slice(&1u32.to_le_bytes());
        bmp[26..28].copy_from_slice(&1u16.to_le_bytes());
        bmp[28..30].copy_from_slice(&24u16.to_le_bytes());
        bmp[54..57].copy_from_slice(&[37, value, 211]);
        let dl = DisplayList {
            width: 7200,
            height: 7200,
            items: vec![Item::Image(Box::new(ImageItem {
                data: bmp.into(),
                frame: [3600, 0, 0, 0, 3600, 0],
                crop: [0; 4],
                natural: (75, 75),
                effects: Effects::default(),
            }))],
        };
        let page = hm_render::raster::render(&dl, &hm_fonts::FontSystem::new(), 72.0).unwrap();
        let pixel = page.pixel(18, 18).unwrap();
        assert_eq!([pixel.red(), pixel.green(), pixel.blue()], [211, value, 37]);
    }
}
