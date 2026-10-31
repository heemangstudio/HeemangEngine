//! Bullets, paragraph numbers and their definitions behave as Hancom's do.
use serde_json::{Value, json};

/// An A4 page with ordinary margins.
const TEMPLATE: &[u8] = include_bytes!("fixtures/last-page-master.hwpx");

fn query(d: &mut hm_wasm::Doc, q: Value) -> Value {
    serde_json::from_str(&d.query(&q.to_string()).unwrap()).unwrap()
}
fn tx(d: &mut hm_wasm::Doc, ops: Value) -> Value {
    serde_json::from_str(&d.transact(&json!({ "ops": ops }).to_string()).unwrap()).unwrap()
}
fn lead(d: &mut hm_wasm::Doc, para: usize) -> Value {
    let p = query(
        d,
        json!({"kind":"paragraphs","list":{"section":0},"from":para,"count":1}),
    );
    json!({"section":0,"para":para,"offset":p[0]["caret"][0]})
}
/// A blank document with `lines` as its paragraphs.
fn document(engine: &hm_wasm::Engine, lines: &[&str]) -> hm_wasm::Doc {
    let mut d = engine.create(TEMPLATE.to_vec()).unwrap();
    let at = lead(&mut d, 0);
    tx(
        &mut d,
        json!([{"op":"insertText","at":at,"text":lines.join("\n")}]),
    );
    d
}
fn bullet(d: &mut hm_wasm::Doc, para: usize, ch: &str) {
    tx(
        d,
        json!([{"op":"ensureList","definition":{"kind":"bullet","bullet":ch}}]),
    );
    let id = query(d, json!({"kind":"lists"}))
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["kind"] == "bullet" && l["bullet"] == ch)
        .unwrap()["id"]
        .clone();
    let at = lead(d, para);
    tx(
        d,
        json!([{"op":"setList","from":at,"to":at,"list":{"kind":"bullet","id":id,"level":0}}]),
    );
}

/// A TrueType font named `family` whose `chars` all draw one square glyph.
fn tiny_font(family: &str, units_per_em: u16, chars: &[char]) -> Vec<u8> {
    let be16 = |v: &mut Vec<u8>, x: u16| v.extend_from_slice(&x.to_be_bytes());
    let be32 = |v: &mut Vec<u8>, x: u32| v.extend_from_slice(&x.to_be_bytes());
    let em = units_per_em;
    // Glyph 1: one square contour, four on-curve points.
    let mut glyf = Vec::new();
    for x in [1u16, 0, 0, em / 2, em / 2] {
        be16(&mut glyf, x);
    }
    for x in [3u16, 0] {
        be16(&mut glyf, x);
    }
    glyf.extend_from_slice(&[1, 1, 1, 1]);
    for d in [0i16, em as i16 / 2, 0, -(em as i16 / 2)] {
        be16(&mut glyf, d as u16);
    }
    for d in [0i16, 0, em as i16 / 2, 0] {
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
    for x in [
        em * 4 / 5,
        (-(em as i16) / 5) as u16,
        0,
        em,
        0,
        0,
        em / 2,
        1,
        0,
        0,
    ] {
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
    let mut codes: Vec<u16> = chars.iter().map(|&c| c as u16).collect();
    codes.sort_unstable();
    codes.push(0xFFFF);
    let seg = codes.len() as u16;
    let mut sub = Vec::new();
    be16(&mut sub, 4);
    be16(&mut sub, 16 + 8 * seg);
    be16(&mut sub, 0);
    be16(&mut sub, seg * 2);
    let pow = 1u16 << (15 - seg.leading_zeros());
    for x in [pow * 2, pow.trailing_zeros() as u16, seg * 2 - pow * 2] {
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
    let strings: Vec<(u16, Vec<u8>)> = [(1, family), (2, "Regular"), (4, family), (6, "Tiny")]
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

/// The scales of the glyphs a page draws.
fn glyph_scales(svg: &str) -> Vec<f64> {
    svg.split(r#"transform="matrix("#)
        .skip(1)
        .filter_map(|s| s.split(' ').next()?.parse().ok())
        .collect()
}

/// Hancom draws its bullets, private-use codes of the symbol fonts
/// (U+F06C for its black circle), with Wingdings whatever the paragraph's
/// face, even a face that has the code itself.
#[test]
fn symbol_font_bullets_draw_with_wingdings_and_ask_for_it() {
    let engine = hm_wasm::Engine::new();
    // A text face that has the bullet's code too, like 한컴바탕.
    engine.add_font(
        "text",
        &tiny_font("Text", 1000, &['b', 'e', 't', 'a', '\u{f06c}']),
    );
    let mut d = document(&engine, &["beta"]);
    bullet(&mut d, 0, "\u{f06c}");
    let svg = d.page_svg(0, 96.0).unwrap();
    assert!(glyph_scales(&svg).iter().all(|&s| s == 1.0));
    let requests: Value = serde_json::from_str(&d.font_requests().unwrap()).unwrap();
    assert!(
        requests["fallbacks"]
            .as_array()
            .unwrap()
            .contains(&json!("Wingdings"))
    );
    engine.add_font("wingdings", &tiny_font("Wingdings", 2048, &['\u{f06c}']));
    let svg = d.page_svg(0, 96.0).unwrap();
    let scales = glyph_scales(&svg);
    assert_eq!(
        scales.iter().filter(|&&s| s != 1.0).count(),
        1,
        "{scales:?}"
    );
    assert!(scales.iter().any(|&s| (s - 1000.0 / 2048.0).abs() < 0.01));
    let requests: Value = serde_json::from_str(&d.font_requests().unwrap()).unwrap();
    assert!(
        !requests["fallbacks"]
            .as_array()
            .unwrap()
            .contains(&json!("Wingdings"))
    );
}

/// The left edge of paragraph `para`'s first line.
fn text_left(d: &mut hm_wasm::Doc, para: usize) -> i64 {
    query(d, json!({"kind":"lines","list":{"section":0},"para":para}))[0]["left"]
        .as_i64()
        .unwrap()
}

/// Definitions the engine creates get the head Hancom's 글머리표 and 문단
/// 번호 create: the text starts half the character size after the head and
/// the head draws in the paragraph's character shape. Saved files say so
/// and reopen with the same heads.
#[test]
fn created_definitions_leave_hancoms_gap_after_the_head() {
    let engine = hm_wasm::Engine::new();
    engine.add_font(
        "text",
        &tiny_font("Text", 1000, &['b', 'n', 'p', '●', '1', '.']),
    );
    let mut d = document(&engine, &["b", "n", "p"]);
    bullet(&mut d, 0, "●");
    tx(
        &mut d,
        json!([{"op":"ensureList","definition":{"kind":"numbering","levels":[{"format":"^1."}]}}]),
    );
    let id = query(&mut d, json!({"kind":"lists"}))
        .as_array()
        .unwrap()
        .iter()
        .filter(|l| l["kind"] == "numbering")
        .last()
        .unwrap()["id"]
        .clone();
    let at = lead(&mut d, 1);
    tx(
        &mut d,
        json!([{"op":"setList","from":at,"to":at,"list":{"kind":"numbering","id":id,"level":0}}]),
    );
    let plain = text_left(&mut d, 2);
    // ● takes one em (1000 HU at 10 pt), "1." its two glyphs; then 500 HU.
    assert_eq!(text_left(&mut d, 0) - plain, 1500);
    assert_eq!(text_left(&mut d, 1) - plain, 2500);
    let hwpx = d.serialize_as("hwpx", false).unwrap();
    let header = String::from_utf8_lossy(&unzip(&hwpx, "Contents/header.xml")).into_owned();
    assert!(header.contains(
        r#"<hh:paraHead level="0" align="LEFT" useInstWidth="0" autoIndent="1" widthAdjust="0" textOffsetType="PERCENT" textOffset="50" numFormat="DIGIT" charPrIDRef="4294967295"/>"#
    ), "{header}");
    assert!(header.contains(
        r#"level="1" align="LEFT" useInstWidth="1" autoIndent="1" widthAdjust="0" textOffsetType="PERCENT" textOffset="50" numFormat="DIGIT" charPrIDRef="4294967295">^1.</hh:paraHead>"#
    ), "{header}");
    for format in ["hwpx", "hwp5"] {
        let saved = d.serialize_as(format, false).unwrap();
        let mut reopened = engine.open(saved).unwrap();
        for (para, gap) in [(0, 1500), (1, 2500)] {
            assert_eq!(text_left(&mut reopened, para) - plain, gap, "{format}");
        }
    }
}

/// One member of a saved HWPX package.
fn unzip(zip: &[u8], name: &str) -> Vec<u8> {
    hm_hwpx::zip::Archive::open(zip.to_vec().into())
        .unwrap()
        .read_name(name)
        .unwrap()
        .unwrap()
        .to_vec()
}
