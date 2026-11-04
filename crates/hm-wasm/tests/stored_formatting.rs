//! Stored formatting paints as Hancom Mac prints it.
use hm_wasm::{Doc, Engine};
use serde_json::{Value, json};

fn q(doc: &mut Doc, value: Value) -> Value {
    serde_json::from_str(&doc.query(&value.to_string()).unwrap()).unwrap()
}
fn tx(doc: &mut Doc, ops: Value) -> Value {
    serde_json::from_str(&doc.transact(&json!({"ops":ops}).to_string()).unwrap()).unwrap()
}
fn at(offset: u64) -> Value {
    json!({"section":0,"para":0,"offset":offset})
}

/// A blank A4 document holding `text`, drawn with a font whose glyphs are
/// squares half the size wide; and where the text starts.
fn document(text: &str) -> (Doc, u64) {
    let engine = Engine::new();
    let chars: Vec<char> = text.chars().collect();
    engine.add_font("text", &font_with_cmap("Text", 1000, &chars, 1));
    let mut doc = engine
        .create(include_bytes!("fixtures/last-page-master.hwpx").to_vec())
        .unwrap();
    let p = q(
        &mut doc,
        json!({"kind":"paragraphs","list":{"section":0},"count":1}),
    );
    let s = p[0]["caret"][0].as_u64().unwrap();
    tx(
        &mut doc,
        json!([{"op":"insertText","at":at(s),"text":text}]),
    );
    (doc, s)
}

/// The rectangles of the SVG painted `fill` as `(x, y, w, h)` in HU.
fn rects(svg: &str, fill: &str) -> Vec<[f64; 4]> {
    svg.split("<rect")
        .skip(1)
        .filter(|r| {
            r.split('>')
                .next()
                .unwrap_or("")
                .contains(&format!("fill=\"{fill}\""))
        })
        .map(|r| {
            let attr = |name: &str| -> f64 {
                let key = format!(" {name}=\"");
                r.split(&key)
                    .nth(1)
                    .and_then(|v| v.split('"').next())
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(f64::NAN)
            };
            [attr("x"), attr("y"), attr("width"), attr("height")]
        })
        .collect()
}

/// Character border fills paint over the shading, one box over a run of
/// the same border fill however its sizes vary, as tall as its largest
/// character (Hancom Mac PDF, probe char-fill-cell and char-border).
#[test]
fn a_character_border_fill_boxes_its_run_over_the_shading() {
    let (mut doc, s) = document("abcdef");
    // Yellow (COLORREF 0x00FFFF) behind all six, cyan shade, the last three twice the size.
    tx(
        &mut doc,
        json!([
            {"op":"formatChars","from":at(s),"to":at(s+6),"patch":{"borderFill":{"fill":{"kind":"solid","background":65535}},"shadeColor":16776960}},
            {"op":"formatChars","from":at(s+3),"to":at(s+6),"patch":{"size":2000}}
        ]),
    );
    let svg = doc.page_svg(0, 96.0).unwrap();
    let yellow = rects(&svg, "#ffff00");
    assert_eq!(yellow.len(), 1, "{yellow:?}");
    let cyan = rects(&svg, "#00ffff");
    assert!(!cyan.is_empty());
    // Painted after the shading.
    assert!(svg.find("#ffff00").unwrap() > svg.rfind("#00ffff").unwrap());
    // As tall as the larger characters (page units are HU).
    let [_, _, w, h] = yellow[0];
    assert!(w > 0.0 && h == 2000.0, "{yellow:?}");
}

/// A font whose Windows `cmap` has encoding `encoding` (1 Unicode, 0 symbol).
fn font_with_cmap(family: &str, units_per_em: u16, chars: &[char], encoding: u16) -> Vec<u8> {
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
    for x in [0u16, 1, 3, encoding] {
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

/// A paragraph background inside a table cell paints under the cell's text, as in the body
/// (Hancom Mac PDF, formatting V/render/min/parabg: A1's paragraph and the body paragraph after
/// the table both yellow).
#[test]
fn a_paragraph_background_in_a_cell_paints() {
    let mut doc = Engine::new()
        .open(include_bytes!("fixtures/hancom-table-4x4.hwpx").to_vec())
        .unwrap();
    let paras = q(&mut doc, json!({"kind":"paragraphs","list":{"section":0}}));
    let (para, control) = paras
        .as_array()
        .unwrap()
        .iter()
        .find_map(|p| {
            p["controls"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["kind"] == "table")
                .map(|c| (p["index"].clone(), c["index"].clone()))
        })
        .unwrap();
    let cell = json!({"section":0,"steps":[{"para":para,"control":control,"index":0}]});
    let a1 = q(&mut doc, json!({"kind":"paragraphs","list":cell}))[0].clone();
    let end = a1["offsets"].as_array().unwrap().last().unwrap().clone();
    let at = |o: &Value| json!({"section":0,"steps":[{"para":para,"control":control,"index":0}],"para":0,"offset":o});
    tx(
        &mut doc,
        json!([{"op":"formatParas","from":at(&a1["offsets"][0]),"to":at(&end),
                "patch":{"borderFill":{"fill":{"kind":"solid","background":65535,"patternColor":0,"pattern":-1}}}}]),
    );
    let svg = doc.page_svg(0, 96.0).unwrap();
    assert!(
        !rects(&svg, "#ffff00").is_empty(),
        "no yellow box in the cell"
    );
}
