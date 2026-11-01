//! Without Hancom's faces, a face name resolves to the operating-system face
//! that stands in for the face Hancom would have drawn.

use std::sync::Arc;

use hm_fonts::{FaceQuery, FontSystem};

/// A TrueType font whose only family name is `family`, drawing `a` and `가`
/// with one square glyph.
fn font(family: &str) -> Vec<u8> {
    let be16 = |v: &mut Vec<u8>, x: u16| v.extend_from_slice(&x.to_be_bytes());
    let be32 = |v: &mut Vec<u8>, x: u32| v.extend_from_slice(&x.to_be_bytes());
    let em = 1000u16;
    let mut glyf = Vec::new();
    for x in [1u16, 0, 0, em / 2, em / 2, 3, 0] {
        be16(&mut glyf, x);
    }
    glyf.extend_from_slice(&[1, 1, 1, 1]);
    for d in [0i16, 500, 0, -500, 0, 0, 500, 0] {
        be16(&mut glyf, d as u16);
    }
    let mut loca = Vec::new();
    for off in [0u16, 0, glyf.len() as u16 / 2] {
        be16(&mut loca, off);
    }
    let mut head = Vec::new();
    be32(&mut head, 0x0001_0000);
    be32(&mut head, 0x0001_0000);
    be32(&mut head, 0);
    be32(&mut head, 0x5F0F_3CF5);
    be16(&mut head, 0);
    be16(&mut head, em);
    head.extend_from_slice(&[0; 16]);
    for x in [0, 0, em / 2, em / 2, 0, 8, 2, 0, 0] {
        be16(&mut head, x);
    }
    let mut hhea = Vec::new();
    be32(&mut hhea, 0x0001_0000);
    for x in [800u16, (-200i16) as u16, 0, em, 0, 0, em / 2, 1, 0, 0] {
        be16(&mut hhea, x);
    }
    hhea.extend_from_slice(&[0; 10]);
    be16(&mut hhea, 2);
    let mut hmtx = Vec::new();
    for x in [em, 0, em, 0] {
        be16(&mut hmtx, x);
    }
    let mut maxp = Vec::new();
    be32(&mut maxp, 0x0001_0000);
    be16(&mut maxp, 2);
    maxp.extend_from_slice(&[0; 26]);
    let codes = [0x61u16, 0xAC00, 0xFFFF];
    let seg = codes.len() as u16;
    let mut sub = Vec::new();
    for x in [4, 16 + 8 * seg, 0, seg * 2, 4, 1, seg * 2 - 4] {
        be16(&mut sub, x);
    }
    codes.iter().for_each(|&c| be16(&mut sub, c));
    be16(&mut sub, 0);
    codes.iter().for_each(|&c| be16(&mut sub, c));
    for &c in &codes {
        be16(&mut sub, if c == 0xFFFF { 1 } else { 1u16.wrapping_sub(c) });
    }
    codes.iter().for_each(|_| be16(&mut sub, 0));
    let mut cmap = Vec::new();
    for x in [0u16, 1, 3, 1] {
        be16(&mut cmap, x);
    }
    be32(&mut cmap, 12);
    cmap.extend_from_slice(&sub);
    let strings: Vec<(u16, Vec<u8>)> = [(1, family), (2, "Regular")]
        .iter()
        .map(|&(id, s)| (id, s.encode_utf16().flat_map(u16::to_be_bytes).collect()))
        .collect();
    let mut name = Vec::new();
    for x in [0, strings.len() as u16, 6 + 12 * strings.len() as u16] {
        be16(&mut name, x);
    }
    let mut offset = 0;
    for (id, s) in &strings {
        for x in [3, 1, 0x409, *id, s.len() as u16, offset] {
            be16(&mut name, x);
        }
        offset += s.len() as u16;
    }
    strings.iter().for_each(|(_, s)| name.extend_from_slice(s));
    let mut post = Vec::new();
    be32(&mut post, 0x0003_0000);
    post.extend_from_slice(&[0; 28]);
    let tables: [(&[u8; 4], Vec<u8>); 9] = [
        (b"cmap", cmap),
        (b"glyf", glyf),
        (b"head", head),
        (b"hhea", hhea),
        (b"hmtx", hmtx),
        (b"loca", loca),
        (b"maxp", maxp),
        (b"name", name),
        (b"post", post),
    ];
    let mut out = Vec::new();
    be32(&mut out, 0x0001_0000);
    for x in [9u16, 128, 3, 16] {
        be16(&mut out, x);
    }
    let mut at = 12 + 16 * tables.len() as u32;
    for (tag, data) in &tables {
        out.extend_from_slice(*tag);
        be32(&mut out, 0);
        be32(&mut out, at);
        be32(&mut out, data.len() as u32);
        at += (data.len() as u32).div_ceil(4) * 4;
    }
    for (_, data) in &tables {
        out.extend_from_slice(data);
        out.resize(out.len().div_ceil(4) * 4, 0);
    }
    out
}

fn catalog(families: &[&str]) -> FontSystem {
    let mut fonts = FontSystem::new();
    for f in families {
        assert_eq!(fonts.add(*f, Arc::new(font(f))), 1, "{f}");
    }
    fonts
}

fn drawn_with(fonts: &FontSystem, name: &str, alternates: &[&str]) -> String {
    let r = fonts
        .resolve(&FaceQuery {
            name,
            alternates: alternates.to_vec(),
            ..Default::default()
        })
        .unwrap();
    assert!(!r.exact, "{name}");
    fonts.entry(r.face).families[0].clone()
}

/// A Mac without Hancom: 함초롬 text takes the 나눔 faces, whose Hangul and
/// space come closest; Hancom's classic faces and the fixed-pitch Windows
/// faces take the Apple faces of their class; HCI Poppy takes Palatino.
#[test]
fn macos_faces_stand_in_for_hancom_faces() {
    let mac = [
        "Apple SD Gothic Neo",
        "AppleGothic",
        "AppleMyungjo",
        "NanumGothic",
        "NanumMyeongjo",
        "Palatino",
        "Times New Roman",
    ];
    let fonts = catalog(&mac);
    assert_eq!(drawn_with(&fonts, "함초롬바탕", &[]), "NanumMyeongjo");
    assert_eq!(drawn_with(&fonts, "HCR Dotum", &[]), "NanumGothic");
    assert_eq!(drawn_with(&fonts, "한컴바탕", &[]), "AppleMyungjo");
    assert_eq!(drawn_with(&fonts, "맑은 고딕", &[]), "NanumGothic");
    assert_eq!(drawn_with(&fonts, "돋움체", &[]), "AppleGothic");
    // Names Hancom draws with one of its faces take that face's stand-in.
    assert_eq!(drawn_with(&fonts, "한양신명조", &[]), "NanumMyeongjo");
    assert_eq!(drawn_with(&fonts, "HCI Poppy", &[]), "Palatino");
    // Names without an entry take the first Apple face of their class.
    assert_eq!(drawn_with(&fonts, "휴먼명조", &[]), "AppleMyungjo");
    assert_eq!(drawn_with(&fonts, "HY헤드라인M", &[]), "AppleGothic");
    // A face Hancom draws with its recorded substitute takes that
    // substitute's stand-in.
    assert_eq!(
        drawn_with(&fonts, "KoPub돋움체 Light", &["한컴바탕"]),
        "AppleMyungjo"
    );
}

/// Windows without Hancom: 함초롬돋움 takes 굴림 rather than 맑은 고딕, 한컴돋움
/// takes 돋움, and the 나눔 faces a user installed come before 맑은 고딕.
#[test]
fn windows_faces_stand_in_for_hancom_faces() {
    let windows = [
        "Malgun Gothic",
        "Batang",
        "Gulim",
        "Dotum",
        "Palatino Linotype",
        "Times New Roman",
    ];
    let fonts = catalog(&windows);
    assert_eq!(drawn_with(&fonts, "함초롬돋움", &[]), "Gulim");
    assert_eq!(drawn_with(&fonts, "함초롬바탕", &[]), "Batang");
    assert_eq!(drawn_with(&fonts, "Haansoft Dotum", &[]), "Dotum");
    assert_eq!(drawn_with(&fonts, "HCI Poppy", &[]), "Palatino Linotype");
    assert_eq!(drawn_with(&fonts, "한양중고딕", &[]), "Malgun Gothic");
    let mut fonts = catalog(&["Malgun Gothic", "Batang"]);
    fonts.add("nanum", Arc::new(font("NanumMyeongjo")));
    assert_eq!(drawn_with(&fonts, "함초롬바탕", &[]), "NanumMyeongjo");
}

/// Hancom's faces, when present, win over every stand-in, and a stand-in
/// does not make a missing face count as available.
#[test]
fn stand_ins_neither_replace_hancom_faces_nor_hide_missing_ones() {
    let fonts = catalog(&["NanumGothic", "HCR Dotum"]);
    assert_eq!(drawn_with(&fonts, "휴먼고딕", &[]), "HCR Dotum");
    let fonts = catalog(&["NanumMyeongjo"]);
    assert!(!fonts.resolves("함초롬바탕"));
    let (candidates, _) = fonts.face_candidates("함초롬바탕", &[]);
    assert_eq!(candidates[0], "함초롬바탕");
    assert!(candidates.iter().any(|c| c == "NanumMyeongjo"));
}
