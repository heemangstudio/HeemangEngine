#[allow(dead_code)]
mod common;

use hm_fonts::{FaceId, FontSystem};
use hm_layout::{
    Kind,
    measure::{advance, measure},
    style::Styles,
};
use hm_model::{CharRun, CharShape, DocInfo, FaceName, HwpString, Paragraph};

fn info() -> DocInfo {
    let shape = CharShape {
        base_size: 1000,
        ratios: [100; 7],
        rel_sizes: [100; 7],
        ..Default::default()
    };
    let mut other = shape.clone();
    other.color = 0xff;
    DocInfo {
        faces: std::array::from_fn(|_| {
            vec![FaceName {
                name: HwpString::from_text("Haansoft Dotum"),
                ..Default::default()
            }]
        }),
        char_shapes: vec![shape, other],
        ..Default::default()
    }
}

#[test]
fn mapped_archaic_clusters_take_one_face_advance_without_rewriting_wire_text() {
    let Some((_, fonts)) = common::setup() else {
        return;
    };
    let info = info();
    for (text, mapped) in [
        ("\u{1112}\u{119e}\u{11ab}", '\u{f53a}'),
        ("\u{1100}\u{119e}", '\u{e1a7}'),
        ("\u{1100}\u{11a1}", '\u{e1cd}'),
        ("\u{1102}\u{119e}\u{11ab}", '\u{e285}'),
    ] {
        let para = Paragraph {
            text: format!("{text}글\r").encode_utf16().collect(),
            ..Default::default()
        };
        let original = para.text.clone();
        let mut styles = Styles::new(&info, &fonts);
        let m = measure(&para, &mut styles, (false, false), &[]);
        let style = styles.get(m.styles[0]);
        let face = styles.face(m.styles[0]).unwrap();
        assert!(face.has(mapped));
        let expected = advance(style, Some(face), mapped);
        assert_eq!(m.advances[0], expected);
        assert!(
            m.advances[1..text.encode_utf16().count()]
                .iter()
                .all(|&w| w == 0),
            "{text:?}: {:?}",
            m.advances
        );
        assert_eq!(m.items[0], (0, Kind::Char(text.chars().next().unwrap())));
        assert_eq!(m.items[1].0 as usize, text.chars().count());
        assert_eq!(para.text, original);
    }
}

#[test]
fn missing_mapping_and_style_or_control_boundaries_keep_original_elements() {
    let Some((_, fonts)) = common::setup() else {
        return;
    };
    let info = info();
    let base = Paragraph {
        text: "\u{1112}\u{119e}\u{11ab}\r".encode_utf16().collect(),
        ..Default::default()
    };
    let mut split = base.clone();
    split.runs = vec![CharRun { pos: 0, shape: 0 }, CharRun { pos: 1, shape: 1 }];
    let control = Paragraph {
        text: "\u{1112}\n\u{119e}\u{11ab}\r".encode_utf16().collect(),
        ..Default::default()
    };
    for para in [&split, &control] {
        let mut styles = Styles::new(&info, &fonts);
        let m = measure(para, &mut styles, (false, false), &[]);
        assert_eq!(
            m.items
                .iter()
                .filter(|(_, k)| matches!(k, Kind::Char(_)))
                .count(),
            3
        );
    }
    // A real face without registered HNC composition data keeps its old path.
    let id = FaceId(fonts.family("Haansoft Dotum")[0]);
    let label = &fonts.entry(id).label;
    let mut without_map = FontSystem::new();
    without_map.add(
        label.clone(),
        std::sync::Arc::new(std::fs::read(label).unwrap()),
    );
    let mut styles = Styles::new(&info, &without_map);
    let m = measure(&base, &mut styles, (false, false), &[]);
    assert_eq!(
        m.items
            .iter()
            .filter(|(_, k)| matches!(k, Kind::Char(_)))
            .count(),
        3
    );
}

#[test]
fn a_cluster_wider_than_the_line_keeps_continuation_offsets_inside_one_item() {
    let Some((_, fonts)) = common::setup() else {
        return;
    };
    let info = info();
    let para = Paragraph {
        text: "\u{1112}\u{119e}\u{11ab}글\r".encode_utf16().collect(),
        ..Default::default()
    };
    let mut styles = Styles::new(&info, &fonts);
    let layout = hm_layout::para::format(
        &para,
        &info,
        &mut styles,
        hm_layout::para::ParaEnv {
            column_width: 500,
            default_tab: 4000,
        },
        None,
    );
    assert_eq!(layout.lines[0].end, 3);
    assert!(
        layout
            .lines
            .iter()
            .all(|l| ![1, 2].contains(&l.start) && ![1, 2].contains(&l.end))
    );
    let mut placed = Vec::new();
    hm_layout::place(&layout, &layout.lines[0], &mut placed);
    assert_eq!(placed.iter().map(|p| p.pos).collect::<Vec<_>>(), [0]);
}

#[test]
fn the_selected_face_must_have_the_composite_and_larger_jamo_groups_stay_unchanged() {
    let Some((_, fonts)) = common::setup() else {
        return;
    };
    let baloo = FaceId(
        *fonts
            .family("Baloo")
            .first()
            .expect("native test font set has Baloo"),
    );
    assert!(!fonts.face(baloo).unwrap().has('\u{f53a}'));
    assert!(
        fonts
            .hangul_cluster(baloo, &['\u{1112}', '\u{119e}', '\u{11ab}'])
            .is_none()
    );
    let info = info();
    for text in [
        "\u{1112}\u{119e}\u{11ab}\u{11a8}",
        "\u{1100}\u{1112}\u{119e}\u{11ab}",
        "\u{1112}\u{1161}\u{11ab}",
    ] {
        let para = Paragraph {
            text: format!("{text}\r").encode_utf16().collect(),
            ..Default::default()
        };
        let mut styles = Styles::new(&info, &fonts);
        let m = measure(&para, &mut styles, (false, false), &[]);
        assert!(m.clusters.is_empty());
        assert_eq!(m.items.len(), text.chars().count() + 1);
    }
}

#[test]
fn native_header_keeps_source_bytes_and_original_utf16_after_measurement() {
    let Some((corpus, fonts)) = common::setup() else {
        return;
    };
    let path = corpus.join("hwp-3.0-HWPML.hwp");
    let bytes = std::fs::read(&path).expect("native HWPML fixture");
    let doc = hm_hwp5::read(hm_model::SharedBytes::from(bytes.clone())).unwrap();
    let header = doc.sections[0].paras[37].controls[1]
        .as_header_footer()
        .unwrap();
    let para = &header.list.paras[0];
    let original = para.text.clone();
    assert_eq!(&original[8..11], &[0x1112, 0x119e, 0x11ab]);
    let mut styles = Styles::new(&doc.info, &fonts);
    let m = measure(para, &mut styles, (false, false), &[]);
    assert_eq!(
        m.clusters
            .iter()
            .map(|g| (g.start, g.end))
            .collect::<Vec<_>>(),
        [(8, 11)]
    );
    assert_eq!(m.advances[8..].iter().sum::<i32>(), 9500);
    assert_eq!(para.text, original);
    assert_eq!(hm_hwp5::write(&doc).unwrap(), bytes);
}
