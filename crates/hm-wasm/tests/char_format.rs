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

/// Hancom 2022 (formatting-20261104/K k3, k4, k5): its 밑줄 button shows a centered underline
/// as on and turns it off, its 취소선 button a 3D strikeout (which prints nothing); off keeps
/// the line's shape and color, on is a solid black line whatever was there.
#[test]
fn line_toggles_count_a_centered_underline_and_a_3d_strikeout_as_on() {
    let (mut doc, s) = blank();
    let red = 0x0000FF;
    let fmt = |doc: &mut Doc, patch: Value| {
        tx(
            doc,
            json!([{"op":"formatChars","from":at(s),"to":at(s+3),"patch":patch}]),
        );
        q(doc, json!({"kind":"formatContext","at":at(s),"to":at(s+1)}))["characters"].clone()
    };
    let c = fmt(
        &mut doc,
        json!({"underlineKind":"center","underlineShape":3,"underlineColor":red}),
    );
    assert_eq!(
        (&c["underline"], &c["underlineKind"]),
        (&json!(true), &json!("center"))
    );
    let c = fmt(&mut doc, json!({"underline":false}));
    assert_eq!(
        (&c["underlineKind"], &c["underlineShape"]),
        (&json!("none"), &json!(3))
    );
    let c = fmt(&mut doc, json!({"underline":true}));
    assert_eq!(
        (
            &c["underlineKind"],
            &c["underlineShape"],
            &c["underlineColor"]
        ),
        (&json!("below"), &json!(0), &json!(0))
    );

    let c = fmt(
        &mut doc,
        json!({"strike":true,"strikeShape":15,"strikeColor":red}),
    );
    assert_eq!(c["strike"], true, "{c}");
    let c = fmt(&mut doc, json!({"strike":false}));
    assert_eq!(
        (&c["strike"], &c["strikeShape"]),
        (&json!(false), &json!(15))
    );
    let c = fmt(&mut doc, json!({"strike":true}));
    assert_eq!(
        (&c["strikeShape"], &c["strikeColor"]),
        (&json!(0), &json!(0)),
        "{c}"
    );
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

/// Hancom's selection from the middle of a paragraph to the start of the next, empty one
/// formats the first paragraph's end mark and leaves the empty paragraph alone (formatting
/// V/probes/p3 a4), unless that is the last paragraph, whose end mark ends the selection
/// (a4b); one crossing an empty paragraph formats it (a3).
#[test]
fn a_range_ending_where_an_empty_paragraph_starts_leaves_it_alone() {
    let mut doc = Engine::new()
        .open(include_bytes!("fixtures/query-equations.hwpx").to_vec())
        .unwrap();
    let para = |doc: &mut Doc, i: u64| {
        q(
            doc,
            json!({"kind":"paragraphs","list":{"section":0},"from":i,"count":1}),
        )[0]
        .clone()
    };
    let end = para(&mut doc, 0)["length"].as_u64().unwrap() - 1;
    let a = |p: u64, o: u64| json!({"section":0,"para":p,"offset":o});
    tx(
        &mut doc,
        json!([{"op":"insertText","at":at(end),"text":"aaaa"},
               {"op":"split","at":at(end + 4)},
               {"op":"split","at":a(1, 0)},
               {"op":"insertText","at":a(2, 0),"text":"cccc"},
               {"op":"split","at":a(2, 4)}]),
    );
    // Paragraphs: 0 "…aaaa", 1 "", 2 "cccc", 3 "".
    tx(
        &mut doc,
        json!([{"op":"formatChars","from":at(end + 2),"to":a(1, 0),"patch":{"color":255}},
               {"op":"formatChars","from":at(end),"to":a(2, 0),"patch":{"bold":true}},
               {"op":"formatChars","from":a(2, 2),"to":a(3, 0),"patch":{"italic":true}}]),
    );
    let caret = |doc: &mut Doc, p: u64, o: u64| {
        q(doc, json!({"kind":"formatContext","at":a(p, o)}))["characters"].clone()
    };
    let p0 = caret(&mut doc, 0, end + 4);
    assert_eq!(
        (p0["color"].clone(), p0["bold"].clone()),
        (json!(255), json!(true))
    );
    let p1 = caret(&mut doc, 1, 0);
    assert_eq!(
        (p1["color"].clone(), p1["bold"].clone()),
        (json!(0), json!(true))
    );
    assert_eq!(caret(&mut doc, 2, 4)["italic"], json!(true));
    assert_eq!(caret(&mut doc, 3, 0)["italic"], json!(true));
}

/// Deleting from inside a paragraph to the end of the next one's text leaves the joined
/// paragraph's end mark in the shape of the character before the deletion; ending inside the
/// next paragraph keeps its end mark (formatting V/probes/p9 c1, c2, c4).
#[test]
fn a_delete_to_the_end_of_a_later_paragraph_gives_its_end_mark_the_shape_before() {
    for (end, want) in [(4u64, 1000), (2, 2000)] {
        let mut doc = Engine::new()
            .open(include_bytes!("fixtures/query-equations.hwpx").to_vec())
            .unwrap();
        let s = q(
            &mut doc,
            json!({"kind":"paragraphs","list":{"section":0},"count":1}),
        )[0]["length"]
            .as_u64()
            .unwrap()
            - 1;
        let a = |p: u64, o: u64| json!({"section":0,"para":p,"offset":o});
        tx(
            &mut doc,
            json!([{"op":"insertText","at":at(s),"text":"aaaa"},
                   {"op":"split","at":at(s + 4)},
                   {"op":"insertText","at":a(1, 0),"text":"bbbb"}]),
        );
        tx(
            &mut doc,
            json!([{"op":"formatChars","from":a(1, 0),"to":a(1, 4),"patch":{"size":2000}},
                   {"op":"formatChars","from":at(s),"to":at(s + 4),"patch":{"size":1000}}]),
        );
        tx(
            &mut doc,
            json!([{"op":"delete","from":at(s + 2),"to":a(1, end)}]),
        );
        let len = q(
            &mut doc,
            json!({"kind":"paragraphs","list":{"section":0},"count":1}),
        )[0]["length"]
            .as_u64()
            .unwrap();
        // The end mark's run: the last run starting at or before it.
        let runs = q(
            &mut doc,
            json!({"kind":"formatRuns","list":{"section":0},"from":0,"count":1}),
        )[0]["runs"]
            .clone();
        let mark = runs
            .as_array()
            .unwrap()
            .iter()
            .rfind(|r| r["offset"].as_u64().unwrap() <= len - 1)
            .unwrap()["characters"]["size"]
            .clone();
        assert_eq!(mark, json!(want), "delete to b{end}");
    }
}

/// A paragraph holding only a table has no last character: a range ending at its end leaves its
/// end mark alone, while text before or after the table takes the end mark along (formatting
/// V2/probes/p13 e1, e2; p15 e6, e7).
#[test]
fn a_range_ending_after_a_table_alone_leaves_the_end_mark() {
    let open = || {
        Engine::new()
            .open(include_bytes!("fixtures/hancom-table-4x4.hwpx").to_vec())
            .unwrap()
    };
    let paras = |doc: &mut Doc| q(doc, json!({"kind":"paragraphs","list":{"section":0}}));
    let host = |doc: &mut Doc| {
        paras(doc)
            .as_array()
            .unwrap()
            .iter()
            .position(|p| {
                p["controls"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|c| c["kind"] == "table")
            })
            .unwrap() as u64
    };
    let end_sub = |doc: &mut Doc, p: u64| {
        let len = paras(doc)[p as usize]["length"].as_u64().unwrap();
        let f = q(
            doc,
            json!({"kind":"formatRuns","list":{"section":0},"from":p,"count":1}),
        )[0]
        .clone();
        let run = f["runs"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["offset"].as_u64().unwrap() < len)
            .last()
            .unwrap()
            .clone();
        run["characters"]["subscript"] == json!(true)
    };
    let a = |p: u64, o: u64| json!({"section":0,"para":p,"offset":o});
    let mut doc = open();
    let t = host(&mut doc);
    let len = paras(&mut doc)[t as usize]["length"].as_u64().unwrap();
    // The table's own paragraph, the table only.
    assert_eq!(paras(&mut doc)[t as usize]["text"], "");
    tx(
        &mut doc,
        json!([{"op":"formatChars","from":a(t, 0),"to":a(t, len - 1),"patch":{"subscript":true}}]),
    );
    let last = paras(&mut doc).as_array().unwrap().len() as u64 == t + 1;
    assert_eq!(
        end_sub(&mut doc, t),
        last,
        "a table alone keeps its end mark unless last"
    );
    // Text before the table: the range reaches a character, the end mark follows.
    let mut doc = open();
    tx(
        &mut doc,
        json!([{"op":"insertText","at":a(t, 0),"text":"ab"}]),
    );
    let len = paras(&mut doc)[t as usize]["length"].as_u64().unwrap();
    tx(
        &mut doc,
        json!([{"op":"formatChars","from":a(t, 0),"to":a(t, len - 1),"patch":{"subscript":true}}]),
    );
    assert!(end_sub(&mut doc, t));
}

/// Deleting all text of a table cell leaves its end mark without character border lines: the
/// first deleted character's shape when it has a border, else the end mark's own unless another
/// deleted character has one. Width, colour and fill stay. Emptying one paragraph of a cell with
/// two keeps the border (formatting V3/probes pY y001-y111, pZ, pV v11).
#[test]
fn deleting_a_cells_whole_text_drops_the_character_border_lines() {
    let line = |color: u32| json!({"left":{"style":3,"width":2,"color":color}});
    let (green, red, blue) = (0x00FF00, 0x0000FF, 0xFF0000);
    // A2 holds "A2"; D1 holds "D1" and "D1-2".
    let at = |cell: u64, para: u64, offset: u64| json!({"section":0,"steps":[{"para":1,"control":0,"index":cell}],"para":para,"offset":offset});
    let end_left = |doc: &mut Doc, cell: u64, para: u64| {
        let list = json!({"section":0,"steps":[{"para":1,"control":0,"index":cell}]});
        let f = q(
            doc,
            json!({"kind":"formatRuns","list":list,"from":para,"count":1}),
        );
        let runs = f[0]["runs"].as_array().unwrap().clone();
        let l = runs.last().unwrap()["characters"]["borderFill"]["left"].clone();
        (l["style"].as_u64().unwrap(), l["color"].as_u64().unwrap())
    };
    let case = |formats: &[(u64, u64, u32)], cell: u64, para: u64, len: u64| {
        let mut doc = Engine::new()
            .open(include_bytes!("fixtures/hancom-table-4x4.hwpx").to_vec())
            .unwrap();
        for &(a, b, color) in formats {
            tx(
                &mut doc,
                json!([{"op":"formatChars","from":at(cell, para, a),"to":at(cell, para, b),"patch":{"borderFill":line(color)}}]),
            );
        }
        tx(
            &mut doc,
            json!([{"op":"delete","from":at(cell, para, 0),"to":at(cell, para, len)}]),
        );
        end_left(&mut doc, cell, para)
    };
    // The first character bordered: its border, lines none.
    assert_eq!(case(&[(0, 1, green)], 4, 0, 2), (0, green as u64));
    assert_eq!(
        case(&[(1, 2, red), (0, 1, green), (2, 2, blue)], 4, 0, 2),
        (0, green as u64)
    );
    // The first plain: the end mark's own border loses its lines unless another deleted one has one.
    assert_eq!(case(&[(2, 2, blue)], 4, 0, 2), (0, blue as u64));
    assert_eq!(
        case(&[(1, 2, red), (2, 2, blue)], 4, 0, 2),
        (3, blue as u64)
    );
    // One paragraph of two emptied: kept.
    assert_eq!(case(&[(0, 4, green)], 3, 1, 4), (3, green as u64));
}

/// Hancom's screens (Windows and Mac) and Mac's print (formatting-20261104/K k1 mac/, k6):
/// a centered underline prints nothing and the strikeout draws its own shape and color; HWP 5
/// shows the same since it reads its centered line as the strikeout.
#[test]
fn a_centered_underline_prints_nothing_beside_its_strikeout() {
    let mut doc = Engine::new()
        .open(include_bytes!("fixtures/last-page-master.hwpx").to_vec())
        .unwrap();
    let s = q(
        &mut doc,
        json!({"kind":"paragraphs","list":{"section":0},"count":1}),
    )[0]["length"]
        .as_u64()
        .unwrap()
        - 1;
    tx(
        &mut doc,
        json!([
            {"op":"insertText","at":at(s),"text":"abc def"},
            {"op":"formatChars","from":at(s),"to":at(s+3),"patch":{"underlineKind":"center","underlineColor":0x0000FF}},
            {"op":"formatChars","from":at(s+4),"to":at(s+7),"patch":{"underlineKind":"center","underlineShape":5,"underlineColor":0x00FF00,"strike":true,"strikeShape":3,"strikeColor":0xFF0000}}
        ]),
    );
    let svg = doc.page_svg(0, 96.0).unwrap();
    assert!(
        !svg.to_lowercase().contains("#ff0000"),
        "red centered underline drawn"
    );
    assert!(
        !svg.to_lowercase().contains("#00ff00"),
        "green centered underline drawn"
    );
    assert!(
        svg.to_lowercase().contains("#0000ff"),
        "blue strikeout missing"
    );
}
