use hm_model::{ControlKind, ShapePart, SharedBytes};

fn paragraph(bytes: &mut Vec<u8>, text_len: u16, text: &[u8]) {
    bytes.push(0);
    bytes.extend(text_len.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend([0; 7]);
    let mut character = [0; 31];
    character[..2].copy_from_slice(&250u16.to_le_bytes());
    character[9..16].fill(100);
    bytes.extend(character);
    let mut shape = [0; 187];
    shape[6..8].copy_from_slice(&100u16.to_le_bytes());
    shape[11] = 1;
    shape[172] = 1;
    bytes.extend(shape);
    bytes.extend([0; 14]);
    bytes.extend(text);
}

fn source(kind: u8, anchor: u8, wrap: u8, border: bool, caption: bool) -> Vec<u8> {
    let mut bytes = hm_hwp3::file::SIGNATURE.to_vec();
    let mut info = [0; 128];
    info[6..8].copy_from_slice(&5000u16.to_le_bytes());
    info[8..10].copy_from_slice(&7000u16.to_le_bytes());
    bytes.extend(info);
    bytes.extend([0; 1008]);
    bytes.extend([0; 16]);
    let mut picture = vec![0; 348];
    picture[8] = anchor;
    picture[9] = wrap;
    picture[14] = u8::from(border);
    picture[16..18].copy_from_slice(&11u16.to_le_bytes());
    for (at, v) in [
        (10, 100u16),
        (12, 200),
        (18, 10),
        (20, 20),
        (22, 30),
        (24, 40),
        (42, 500),
        (44, 750),
    ] {
        picture[at..at + 2].copy_from_slice(&v.to_le_bytes());
    }
    picture[74] = kind;
    let name = b"/unavailable/image.gif";
    picture[83..83 + name.len()].copy_from_slice(name);
    let mut text = 11u16.to_le_bytes().to_vec();
    text.extend([0; 4]);
    text.extend(11u16.to_le_bytes());
    text.extend(picture);
    if caption {
        paragraph(&mut text, 2, &[b'C', 0, 13, 0]);
    }
    text.extend([0; 43]);
    text.extend(13u16.to_le_bytes());
    paragraph(&mut bytes, 5, &text);
    paragraph(&mut bytes, 2, &[b'B', 0, 13, 0]);
    bytes.extend([0; 43]);
    bytes
}

#[test]
fn unavailable_hwp3_pictures_keep_geometry_without_image_ink() {
    let fonts = hm_fonts::FontSystem::new();
    for kind in [0, 2] {
        for anchor in 0..4 {
            for wrap in 0..3 {
                for border in [false, true] {
                    for caption in [false, true] {
                        let doc = hm_hwp3::read(SharedBytes::from(source(
                            kind, anchor, wrap, border, caption,
                        )))
                        .unwrap();
                        let p = &doc.sections[0].paras[0];
                        let object = p
                            .controls
                            .iter()
                            .find_map(|c| match &c.kind {
                                ControlKind::Object(o) => Some(o),
                                _ => None,
                            })
                            .expect("missing image must retain its control");
                        let common = &object.header.common;
                        assert_eq!((common.width, common.height), (2000, 3000));
                        assert_eq!(common.margins, [40, 80, 120, 160]);
                        assert_eq!((common.x_offset, common.y_offset), (400, 800));
                        assert_eq!(common.attr & 1 != 0, anchor == 0);
                        assert_eq!((common.attr >> 21) & 7, [1, 3, 0][wrap as usize]);
                        assert_eq!(object.caption().is_some(), caption);
                        if let Some(c) = object.caption() {
                            assert_eq!(c.paras[0].plain_text(), "C");
                        }
                        let picture = object
                            .shape()
                            .unwrap()
                            .parts
                            .iter()
                            .find_map(|p| match p {
                                ShapePart::Picture(p) => Some(p),
                                _ => None,
                            })
                            .unwrap();
                        assert_eq!(
                            picture.corners,
                            [(0, 0), (2000, 0), (2000, 3000), (0, 3000)]
                        );
                        assert_eq!(picture.image.bin_id, 0);
                        assert!(
                            doc.bins
                                .as_ref()
                                .unwrap()
                                .get(picture.image.bin_id)
                                .is_none()
                        );
                        let layout = hm_layout::layout(&doc, &fonts);
                        if anchor == 0 && !caption {
                            assert_eq!(layout.para(0, 0).lines[0].height, 3280);
                            assert_eq!(
                                layout.pages[0]
                                    .lines
                                    .iter()
                                    .find(|l| l.para == 1)
                                    .unwrap()
                                    .vpos,
                                3280
                            );
                        }
                        let dl = hm_render::build_page(&doc, &layout, &fonts, 0);
                        assert!(
                            !dl.items
                                .iter()
                                .any(|i| matches!(i, hm_render::Item::Image(_)))
                        );
                        assert_eq!(
                            dl.items
                                .iter()
                                .filter(|i| matches!(i, hm_render::Item::Path(_)))
                                .count(),
                            usize::from(border)
                        );
                    }
                }
            }
        }
    }
    for kind in [1, 4, 255] {
        let doc = hm_hwp3::read(SharedBytes::from(source(kind, 0, 0, false, false))).unwrap();
        assert!(!doc.sections[0].paras[0].controls.iter().any(|c| {
            if let ControlKind::Object(o) = &c.kind {
                o.shape()
                    .is_some_and(|s| s.parts.iter().any(|p| matches!(p, ShapePart::Picture(_))))
            } else {
                false
            }
        }));
    }
}
