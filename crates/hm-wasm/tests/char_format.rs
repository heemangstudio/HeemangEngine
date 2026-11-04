//! Character formatting that Hancom's dialog offers reads, applies and saves as Hancom does.
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

#[test]
fn centered_underline_is_its_own_kind_and_a_strikeout_does_not_add_one() {
    let engine = Engine::new();
    let mut doc = engine
        .open(include_bytes!("fixtures/query-equations.hwpx").to_vec())
        .unwrap();
    let p = q(
        &mut doc,
        json!({"kind":"paragraphs","list":{"section":0},"count":1}),
    );
    let s = p[0]["length"].as_u64().unwrap() - 1;
    tx(
        &mut doc,
        json!([{"op":"insertText","at":at(s),"text":"abc def"}]),
    );
    tx(
        &mut doc,
        json!([
            {"op":"formatChars","from":at(s),"to":at(s+3),"patch":{"underlineKind":"center"}},
            {"op":"formatChars","from":at(s+4),"to":at(s+7),"patch":{"strike":true}}
        ]),
    );
    let kind = |doc: &mut Doc, o: u64| {
        q(doc, json!({"kind":"formatContext","at":at(o),"to":at(o+1)}))["characters"]
            ["underlineKind"]
            .clone()
    };
    assert_eq!(kind(&mut doc, s), "center");
    assert_eq!(kind(&mut doc, s + 4), "none");
    let saved = doc.serialize(false).unwrap();
    let header = String::from_utf8_lossy(&unzip(&saved, "Contents/header.xml")).into_owned();
    assert_eq!(header.matches(r#"<hh:underline type="CENTER""#).count(), 1);
    assert_eq!(header.matches(r#"<hh:strikeout shape="SOLID""#).count(), 1);
    let mut reopened = engine.open(saved).unwrap();
    assert_eq!(kind(&mut reopened, s), "center");
    assert_eq!(kind(&mut reopened, s + 4), "none");
    // Turning the underline off removes a centered one; it no longer belongs to a strikeout.
    tx(
        &mut doc,
        json!([{"op":"formatChars","from":at(s),"to":at(s+3),"patch":{"underline":false}}]),
    );
    assert_eq!(kind(&mut doc, s), "none");
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

fn blank() -> (Doc, u64) {
    let engine = Engine::new();
    let mut doc = engine
        .open(include_bytes!("fixtures/query-equations.hwpx").to_vec())
        .unwrap();
    let p = q(
        &mut doc,
        json!({"kind":"paragraphs","list":{"section":0},"count":1}),
    );
    let s = p[0]["length"].as_u64().unwrap() - 1;
    tx(
        &mut doc,
        json!([{"op":"insertText","at":at(s),"text":"abc def"}]),
    );
    (doc, s)
}

#[test]
fn a_border_or_background_change_keeps_the_other_half() {
    let (mut doc, s) = blank();
    let line = json!({"style":1,"width":1,"color":255});
    tx(
        &mut doc,
        json!([{"op":"formatChars","from":at(s),"to":at(s+3),"patch":{"borderFill":{"left":line,"right":line,"top":line,"bottom":line}}}]),
    );
    tx(
        &mut doc,
        json!([{"op":"formatChars","from":at(s),"to":at(s+3),"patch":{"borderFill":{"fill":{"kind":"solid","background":65280}}}}]),
    );
    let c = q(
        &mut doc,
        json!({"kind":"formatContext","at":at(s),"to":at(s+3)}),
    )["characters"]["borderFill"]
        .clone();
    assert_eq!(c["left"]["color"], 255, "{c}");
    assert_eq!(c["fill"]["background"], 65280, "{c}");
}

#[test]
fn faces_sent_back_unchanged_need_no_registered_family() {
    // No fonts are registered here: the document's own faces stay valid, a new name does not.
    let (mut doc, s) = blank();
    let mut faces =
        q(&mut doc, json!({"kind":"formatContext","at":at(s)}))["characters"]["fontFaces"].clone();
    faces[1] = faces[0].clone();
    let ok = doc.transact(
        &json!({"ops":[{"op":"formatChars","from":at(s),"to":at(s+3),"patch":{"fontFaces":faces}}]})
            .to_string(),
    );
    assert!(ok.is_ok(), "{ok:?}");
    let bad = doc.transact(
        &json!({"ops":[{"op":"formatChars","from":at(s),"to":at(s+3),"patch":{"fontFaces":[null,"No Such Font"]}}]})
            .to_string(),
    );
    assert!(bad.is_err());
}

#[test]
fn font_spacing_mixed_state_and_size_bounds() {
    let (mut doc, s) = blank();
    tx(
        &mut doc,
        json!([{"op":"formatChars","from":at(s),"to":at(s+3),"patch":{"useFontSpace":true,"emboss":true}}]),
    );
    let one = q(
        &mut doc,
        json!({"kind":"formatContext","at":at(s),"to":at(s+3)}),
    );
    assert_eq!(one["characters"]["useFontSpace"], true);
    let both = q(
        &mut doc,
        json!({"kind":"formatContext","at":at(s),"to":at(s+7)}),
    );
    let mixed: Vec<&str> = both["mixed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(mixed.contains(&"characters.useFontSpace"), "{mixed:?}");
    assert!(mixed.contains(&"characters.emboss"), "{mixed:?}");
    let header = String::from_utf8_lossy(&unzip(
        &doc.serialize(false).unwrap(),
        "Contents/header.xml",
    ))
    .into_owned();
    assert!(header.contains(r#"useFontSpace="1""#));
    for size in [0, -100, 409_700] {
        let r = doc.transact(
            &json!({"ops":[{"op":"formatChars","from":at(s),"to":at(s+3),"patch":{"size":size}}]})
                .to_string(),
        );
        assert!(r.is_err(), "size {size}");
    }
}

#[test]
fn formatting_to_the_last_character_takes_the_paragraph_end_mark() {
    // Hancom's selection to a paragraph's end formats its end mark too, which sizes an
    // empty last line and the paragraph head.
    let (mut doc, s) = blank();
    tx(
        &mut doc,
        json!([{"op":"formatChars","from":at(s+4),"to":at(s+7),"patch":{"size":2400}}]),
    );
    let p = q(
        &mut doc,
        json!({"kind":"formatRuns","list":{"section":0},"count":1}),
    )[0]
    .clone();
    let runs = p["runs"].as_array().unwrap();
    let last = runs.last().unwrap();
    assert_eq!(last["offset"], s + 4, "{p}");
    // A range that stops short of the last character leaves the end mark alone.
    tx(
        &mut doc,
        json!([{"op":"formatChars","from":at(s),"to":at(s+2),"patch":{"bold":true}}]),
    );
    let p = q(
        &mut doc,
        json!({"kind":"formatRuns","list":{"section":0},"count":1}),
    )[0]
    .clone();
    assert_eq!(
        p["runs"].as_array().unwrap().last().unwrap()["offset"],
        s + 4
    );
}

/// Hancom (COM capture formatting-20261104/C c2, c4): a format set at a paragraph's end
/// with no selection is stored on the end mark at once, from the shape before the caret,
/// as one undo step; typing there later still takes the shape before the caret.
#[test]
fn a_caret_format_at_a_paragraph_end_is_stored_on_its_end_mark() {
    let (mut doc, s) = blank();
    let end = s + 7;
    // The run covering wire offset `o` of a paragraph (the end mark included).
    let run = |doc: &mut Doc, para: u64, o: u64| {
        let f = q(
            doc,
            json!({"kind":"formatRuns","list":{"section":0},"from":para,"count":1}),
        );
        let runs = f[0]["runs"].as_array().unwrap().clone();
        runs.into_iter()
            .rev()
            .find(|r| r["offset"].as_u64().unwrap() <= o)
            .unwrap()["characters"]
            .clone()
    };
    let shape = |doc: &mut Doc, o: u64| run(doc, 0, o)["color"].clone();
    tx(
        &mut doc,
        json!([{"op":"formatChars","from":at(s),"to":at(s+3),"patch":{"size":2000}}]),
    );
    tx(
        &mut doc,
        json!([{"op":"formatChars","from":at(end),"to":at(end),"patch":{"color":0x00FFFF}}]),
    );
    assert_eq!(shape(&mut doc, end), 0x00FFFF);
    // The end mark took the shape before the caret (size 1000 of "def"), not the first run's.
    assert_eq!(
        run(&mut doc, 0, end)["size"],
        run(&mut doc, 0, end - 1)["size"]
    );
    assert_ne!(run(&mut doc, 0, end)["size"], 2000);
    tx(
        &mut doc,
        json!([{"op":"insertText","at":at(end),"text":"x"}]),
    );
    assert_eq!(
        shape(&mut doc, end),
        0,
        "typing takes the shape before the caret"
    );
    assert_eq!(
        shape(&mut doc, end + 1),
        0x00FFFF,
        "the end mark keeps its format"
    );
    let saved = doc.serialize(false).unwrap();
    let mut reopened = Engine::new().open(saved).unwrap();
    assert_eq!(shape(&mut reopened, end + 1), 0x00FFFF);
    doc.undo().unwrap();
    doc.undo().unwrap();
    assert_eq!(shape(&mut doc, end), 0);
    // An empty paragraph takes it on its only mark, and typing into it then takes it too.
    tx(&mut doc, json!([{"op":"split","at":at(end)}]));
    let next = json!({"section":0,"para":1,"offset":0});
    tx(
        &mut doc,
        json!([{"op":"formatChars","from":next,"to":next,"patch":{"color":0xFF0000}}]),
    );
    tx(&mut doc, json!([{"op":"insertText","at":next,"text":"y"}]));
    assert_eq!(run(&mut doc, 1, 0)["color"], 0xFF0000);
    assert_eq!(run(&mut doc, 1, 1)["color"], 0xFF0000);
}

/// Hancom (COM capture formatting-20261104/C c5): 글자 크게, 장평 넓게 and 자간 좁게 over
/// runs of different shapes step each run's own value.
#[test]
fn size_ratio_and_spacing_steps_change_each_run_from_its_own_value() {
    let (mut doc, s) = blank();
    tx(
        &mut doc,
        json!([
            {"op":"formatChars","from":at(s),"to":at(s+3),"patch":{"size":1000,"ratios":[100,100,100,100,100,100,100],"spacings":[0,0,0,0,0,0,0]}},
            {"op":"formatChars","from":at(s+4),"to":at(s+7),"patch":{"size":2000,"ratios":[80,80,80,80,80,80,80],"spacings":[10,10,10,10,10,10,10]}}
        ]),
    );
    let base = q(
        &mut doc,
        json!({"kind":"formatContext","at":at(s),"to":at(s+1)}),
    )["characters"]
        .clone();
    let n = |v: &Value| v.as_i64().unwrap();
    tx(
        &mut doc,
        json!([{"op":"formatChars","from":at(s),"to":at(s+7),"patch":{"sizeStep":100,"ratioStep":1,"spacingStep":-1}}]),
    );
    let c = |doc: &mut Doc, o: u64| {
        q(doc, json!({"kind":"formatContext","at":at(o),"to":at(o+1)}))["characters"].clone()
    };
    let (a, b) = (c(&mut doc, s), c(&mut doc, s + 4));
    assert_eq!(n(&a["size"]), n(&base["size"]) + 100);
    assert_eq!(b["size"], 2100);
    assert_eq!(n(&a["ratios"][0]), n(&base["ratios"][0]) + 1);
    assert_eq!(b["ratios"][6], 81);
    assert_eq!(n(&a["spacings"][0]), n(&base["spacings"][0]) - 1);
    assert_eq!(b["spacings"][6], 9);
    // Each value stops at its range's end.
    tx(
        &mut doc,
        json!([{"op":"formatChars","from":at(s),"to":at(s+7),"patch":{"ratioStep":150,"spacingStep":-90}}]),
    );
    let a = c(&mut doc, s);
    assert_eq!(a["ratios"][0], 200);
    assert_eq!(a["spacings"][0], -50);
}

/// Hancom (COM capture formatting-20261104/C c8, paragraph 4): a dashed strikeout without an
/// underline saves no underline in HWPX, and its HWP 5 centered line has the strike's shape.
#[test]
fn a_strikeout_shape_moves_its_hidden_centered_line_too() {
    let (mut doc, s) = blank();
    tx(
        &mut doc,
        json!([{"op":"formatChars","from":at(s),"to":at(s+3),"patch":{"strike":true,"strikeShape":3}}]),
    );
    let saved = doc.serialize(false).unwrap();
    let header = String::from_utf8_lossy(&unzip(&saved, "Contents/header.xml")).into_owned();
    assert!(
        !header.contains(r#"<hh:underline type="CENTER""#),
        "{header}"
    );
    let hwp = doc.serialize_as("hwp5", false).unwrap();
    for bytes in [saved, hwp] {
        let mut reopened = Engine::new().open(bytes).unwrap();
        let c = q(
            &mut reopened,
            json!({"kind":"formatContext","at":at(s),"to":at(s+1)}),
        )["characters"]
            .clone();
        assert_eq!(c["strikeShape"], 3, "{c}");
        assert_eq!(c["underlineShape"], 3, "{c}");
        assert_eq!(c["underlineKind"], "none", "{c}");
    }
}

#[test]
fn emboss_and_engrave_exclude_each_other_unless_both_are_sent() {
    // Hancom's toolbar and dialog turn one off when the other goes on; its CharShape
    // parameter set with both on stores both (COM capture d3, lane C2).
    let (mut doc, s) = blank();
    let chars = |doc: &mut Doc| {
        q(doc, json!({"kind":"formatContext","at":at(s),"to":at(s+3)}))["characters"].clone()
    };
    tx(
        &mut doc,
        json!([{"op":"formatChars","from":at(s),"to":at(s+3),"patch":{"engrave":true}}]),
    );
    tx(
        &mut doc,
        json!([{"op":"formatChars","from":at(s),"to":at(s+3),"patch":{"emboss":true}}]),
    );
    let c = chars(&mut doc);
    assert_eq!(
        (c["emboss"].clone(), c["engrave"].clone()),
        (json!(true), json!(false))
    );
    tx(
        &mut doc,
        json!([{"op":"formatChars","from":at(s),"to":at(s+3),"patch":{"emboss":true,"engrave":true}}]),
    );
    let c = chars(&mut doc);
    assert_eq!(
        (c["emboss"].clone(), c["engrave"].clone()),
        (json!(true), json!(true))
    );
    let header = String::from_utf8_lossy(&unzip(
        &doc.serialize(false).unwrap(),
        "Contents/header.xml",
    ))
    .into_owned();
    assert!(header.contains("<hh:emboss/><hh:engrave/>"), "{header}");
}
