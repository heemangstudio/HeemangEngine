//! Paragraph shape patches as Hancom stores them (heemang-cleanroom
//! parity/notes/formatting-20261104/P).
use hm_wasm::{Doc, Engine};
use serde_json::{Value, json};

fn query(d: &mut Doc, q: Value) -> Value {
    serde_json::from_str(&d.query(&q.to_string()).unwrap()).unwrap()
}
fn tx(d: &mut Doc, ops: Value) {
    d.transact(&json!({ "ops": ops }).to_string()).unwrap();
}
fn document() -> Doc {
    let mut d = Engine::new()
        .create(include_bytes!("fixtures/hancom-table-4x4.hwpx").to_vec())
        .unwrap();
    let end = paragraph(&mut d, 0)["caret"][0].clone();
    tx(
        &mut d,
        json!([{"op":"insertText","at":{"section":0,"para":0,"offset":end},"text":"가나다"},
               {"op":"split","at":{"section":0,"para":0,"offset":end.as_u64().unwrap() + 3}}]),
    );
    d
}
fn paragraph(d: &mut Doc, para: u32) -> Value {
    query(
        d,
        json!({"kind":"paragraphs","list":{"section":0},"from":para,"count":1}),
    )[0]
    .clone()
}
/// The whole of paragraph `para` as a range.
fn whole(d: &mut Doc, para: u32) -> (Value, Value) {
    let p = paragraph(d, para);
    let offsets = p["offsets"].as_array().unwrap();
    let a = |o: &Value| json!({"section":0,"para":para,"offset":o});
    (a(&p["caret"][0]), a(offsets.last().unwrap()))
}
fn format(d: &mut Doc, para: u32, patch: Value) {
    let (from, to) = whole(d, para);
    tx(
        d,
        json!([{"op":"formatParas","from":from,"to":to,"patch":patch}]),
    );
}
fn paragraphs(d: &mut Doc, para: u32) -> Value {
    query(
        d,
        json!({"kind":"formatRuns","list":{"section":0},"from":para,"count":1}),
    )[0]["paragraphs"]
        .clone()
}
fn line(style: u32, width: u32, color: u32) -> Value {
    json!({"style":style,"width":width,"color":color})
}

/// Hancom's 문단 테두리/배경 (captures b8-para-border): each patch changes the paragraph's own
/// border fill, so a background added after a left line keeps the line, and two paragraphs given
/// the same border share one border fill.
#[test]
fn paragraph_border_patch_follows_each_paragraphs_own_border_fill() {
    let mut d = document();
    format(
        &mut d,
        0,
        json!({"borderFill":{"left":line(3, 4, 0xFF0000)}}),
    );
    format(
        &mut d,
        0,
        json!({"borderFill":{"fill":{"kind":"solid","background":0x00FFFF,"patternColor":0,"pattern":-1}}}),
    );
    let p = paragraphs(&mut d, 0);
    assert_eq!(p["borderFill"]["left"], line(3, 4, 0xFF0000));
    assert_eq!(p["borderFill"]["right"]["style"], 0);
    assert_eq!(p["borderFill"]["fill"]["background"], 0x00FFFF);

    format(
        &mut d,
        1,
        json!({"borderFill":{"left":line(3, 4, 0xFF0000)}}),
    );
    format(
        &mut d,
        1,
        json!({"borderFill":{"fill":{"kind":"solid","background":0x00FFFF,"patternColor":0,"pattern":-1}}}),
    );
    assert_eq!(
        paragraphs(&mut d, 1)["borderFillId"],
        p["borderFillId"],
        "equal borders share one border fill"
    );

    let mut again = Engine::new()
        .open(d.serialize_as("hwpx", false).unwrap())
        .unwrap();
    assert_eq!(paragraphs(&mut again, 0)["borderFill"], p["borderFill"]);
}

/// Hancom stores a paragraph taken out of its numbering or bullet list with no definition
/// (heading NONE, idRef 0), whether through the toolbar toggle or Backspace at its start
/// (captures b4-lists nb-*-off, nb-bs-start-*).
#[test]
fn a_paragraph_taken_out_of_its_list_names_no_definition() {
    let mut d = document();
    tx(
        &mut d,
        json!([{"op":"ensureList","definition":{"kind":"bullet","bullet":"\u{f06c}"}}]),
    );
    let (from, to) = whole(&mut d, 0);
    tx(
        &mut d,
        json!([{"op":"setList","from":from,"to":to,"list":{"kind":"bullet","id":1,"level":0}}]),
    );
    assert_eq!(paragraphs(&mut d, 0)["heading"]["id"], 1);
    tx(
        &mut d,
        json!([{"op":"setList","from":from,"to":to,"list":null}]),
    );
    assert_eq!(
        paragraphs(&mut d, 0)["heading"],
        json!({"kind":"none","id":0,"level":0})
    );
    assert_eq!(
        paragraphs(&mut d, 0)["borderFillId"],
        paragraphs(&mut d, 1)["borderFillId"]
    );
    assert_eq!(
        query(&mut d, json!({"kind":"formatRuns","list":{"section":0}}))[0]["paraShapeId"],
        query(&mut d, json!({"kind":"formatRuns","list":{"section":0}}))[1]["paraShapeId"],
        "the plain paragraph's shape again"
    );
}

/// Hancom's 문단 번호 on a document whose only numbering is the section's outline numbering
/// makes a second, separate definition even though the outline one has the same levels
/// (captures b4-lists nb-number: idRef 2).
#[test]
fn a_paragraph_numbering_is_never_the_outline_numbering() {
    let mut d = document();
    let outline = query(&mut d, json!({"kind":"sectionSettings","section":0}))["settings"]
        ["outlineNumbering"]
        .as_u64()
        .unwrap();
    let count = |d: &mut Doc| {
        query(d, json!({"kind":"lists"}))
            .as_array()
            .unwrap()
            .iter()
            .filter(|l| l["kind"] == "numbering")
            .count()
    };
    let before = count(&mut d);
    let levels: Vec<Value> = [
        ("^1.", 0),
        ("^2.", 8),
        ("^3)", 0),
        ("^4)", 8),
        ("(^5)", 0),
        ("(^6)", 8),
        ("^7", 1),
    ]
    .iter()
    .map(|(f, k)| json!({"format":f,"numberKind":k,"start":1}))
    .collect();
    let ensure =
        json!([{"op":"ensureList","definition":{"kind":"numbering","start":0,"levels":levels}}]);
    tx(&mut d, ensure.clone());
    assert_eq!(count(&mut d), before + 1);
    assert_ne!(before as u64 + 1, outline);
    tx(&mut d, ensure);
    assert_eq!(
        count(&mut d),
        before + 1,
        "the paragraph numbering is reused"
    );
    // With no numbered paragraph before the caret Hancom's 문단 번호 adds a definition even
    // when an equal one exists (formatting V/probes/p4 r1).
    tx(
        &mut d,
        json!([{"op":"ensureList","new":true,"definition":{"kind":"numbering","start":0,"levels":levels}}]),
    );
    assert_eq!(count(&mut d), before + 2);
}

/// Applying, at the caret, the style a paragraph already has resets it to the style as a
/// selection does, also an empty paragraph whose whole range is a caret (formatting V/probes/p9
/// a1-a6; V r4 bokhak seq009). `whole` asks for that; a plain caret apply keeps what the
/// paragraph sets itself.
#[test]
fn a_whole_style_apply_resets_an_empty_paragraph() {
    let mut d = document();
    let at = whole(&mut d, 1).0;
    tx(
        &mut d,
        json!([{"op":"formatChars","from":at,"to":at,"patch":{"bold":true}},
               {"op":"formatParas","from":at,"to":at,"patch":{"left":2000}}]),
    );
    let state = |d: &mut Doc| {
        let f = query(
            d,
            json!({"kind":"formatRuns","list":{"section":0},"from":1,"count":1}),
        )[0]
        .clone();
        (
            f["runs"][0]["characters"]["bold"].clone(),
            f["paragraphs"]["left"].clone(),
        )
    };
    let set = state(&mut d);
    assert_eq!(set.0, json!(true));
    let style =
        |whole: bool| json!([{"op":"applyStyle","from":at,"to":at,"styleId":0,"whole":whole}]);
    tx(&mut d, style(false));
    assert_eq!(state(&mut d), set);
    tx(&mut d, style(true));
    let reset = state(&mut d);
    assert_ne!(reset.0, json!(true));
    assert_ne!(reset.1, set.1);
}

/// Hancom's 문단 번호 definition has ten levels: 8 is `^8` in circled Hangul, 9 and 10 have no
/// text, and `checkable` is the low bit of each number format (b4-lists nb-number, P2
/// d3-styles nb-number-hwp: the same levels in its HWPX and HWP 5 saves). Saved and reopened
/// in either format the created definition keeps them.
#[test]
fn a_created_numbering_has_hancoms_ten_levels() {
    let mut d = document();
    let levels: Vec<Value> = [
        ("^1.", 0),
        ("^2.", 8),
        ("^3)", 0),
        ("^4)", 8),
        ("(^5)", 0),
        ("(^6)", 8),
        ("^7", 1),
    ]
    .iter()
    .map(|(f, k)| json!({"format":f,"numberKind":k,"start":1}))
    .collect();
    tx(
        &mut d,
        json!([{"op":"ensureList","definition":{"kind":"numbering","start":0,"levels":levels}}]),
    );
    let created = |lists: Value| {
        lists
            .as_array()
            .unwrap()
            .iter()
            .filter(|l| l["kind"] == "numbering")
            .last()
            .unwrap()
            .clone()
    };
    let want: Vec<(String, u64, u64)> = [("^8", 9), ("", 0), ("", 0)]
        .iter()
        .map(|(f, k)| (f.to_string(), *k, 1))
        .collect();
    let tail = |l: &Value| -> Vec<(String, u64, u64)> {
        let levels = l["levels"].as_array().unwrap();
        assert_eq!(levels.len(), 10);
        levels[7..]
            .iter()
            .map(|v| {
                (
                    v["format"].as_str().unwrap().to_string(),
                    v["numberKind"].as_u64().unwrap(),
                    v["start"].as_u64().unwrap(),
                )
            })
            .collect()
    };
    assert_eq!(tail(&created(query(&mut d, json!({"kind":"lists"})))), want);
    let hwpx = d.serialize_as("hwpx", false).unwrap();
    let header = String::from_utf8_lossy(
        &hm_hwpx::zip::Archive::open(hwpx.clone().into())
            .unwrap()
            .read_name("Contents/header.xml")
            .unwrap()
            .unwrap(),
    )
    .into_owned();
    let numbering = header.rsplit("<hh:numbering ").next().unwrap();
    let checkable: Vec<&str> = numbering
        .split("checkable=\"")
        .skip(1)
        .map(|s| &s[..1])
        .collect();
    assert_eq!(
        checkable,
        ["0", "0", "0", "0", "0", "0", "1", "1", "0", "0"]
    );
    for bytes in [hwpx, d.serialize_as("hwp5", false).unwrap()] {
        let mut reopened = Engine::new().open(bytes).unwrap();
        assert_eq!(
            tail(&created(query(&mut reopened, json!({"kind":"lists"})))),
            want
        );
    }
}

/// A style applied at the caret follows the paragraph head field by field (formatting
/// V/probes/p4 s1, s2, s5): a bulleted paragraph given 개요 2 keeps its kind and definition and
/// takes the style's level; an outline paragraph already at its own level keeps that too.
#[test]
fn a_caret_style_follows_the_head_kind_definition_and_level_one_by_one() {
    let mut d = document();
    tx(
        &mut d,
        json!([{"op":"ensureList","definition":{"kind":"bullet","bullet":"\u{f06c}"}}]),
    );
    let (from, to) = whole(&mut d, 0);
    tx(
        &mut d,
        json!([{"op":"setList","from":from,"to":to,"list":{"kind":"bullet","id":1,"level":0}}]),
    );
    let (from1, to1) = whole(&mut d, 1);
    tx(
        &mut d,
        json!([{"op":"setList","from":from1,"to":to1,"list":{"kind":"outline","id":0,"level":2}}]),
    );
    let styles = query(&mut d, json!({"kind":"styles"}));
    let outline2 = styles
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "개요 2")
        .unwrap()["id"]
        .clone();
    for at in [from, from1] {
        tx(
            &mut d,
            json!([{"op":"applyStyle","from":at,"to":at,"styleId":outline2}]),
        );
    }
    let head = |d: &mut Doc, p: u32| {
        let h = paragraphs(d, p)["heading"].clone();
        (
            h["kind"].as_str().unwrap().to_string(),
            h["id"].as_u64().unwrap(),
            h["level"].as_u64().unwrap(),
        )
    };
    assert_eq!(head(&mut d, 0), ("bullet".to_string(), 1, 1));
    assert_eq!(head(&mut d, 1), ("outline".to_string(), 0, 2));
}

/// Enter at the end, middle or start of a paragraph that keeps its lines together, keeps
/// with the next one and starts a page leaves those three with the first half; the second
/// keeps widow and orphan control and the margins (formatting V/probes/p4 e1-e3).
#[test]
fn enter_leaves_the_keeps_with_the_first_half() {
    for k in [3u64, 1, 0] {
        let mut d = document();
        format(
            &mut d,
            0,
            json!({"keepLines":true,"keepWithNext":true,"pageBreakBefore":true,"widowOrphan":true,"left":1000}),
        );
        let lead = paragraph(&mut d, 0)["caret"][0].as_u64().unwrap();
        tx(
            &mut d,
            json!([{"op":"split","at":{"section":0,"para":0,"offset":lead + k}}]),
        );
        let flags = |d: &mut Doc, p: u32| {
            let s = paragraphs(d, p);
            [
                &s["keepLines"],
                &s["keepWithNext"],
                &s["pageBreakBefore"],
                &s["widowOrphan"],
                &s["left"],
            ]
            .map(|v| v.clone())
        };
        assert_eq!(
            flags(&mut d, 0),
            [
                json!(true),
                json!(true),
                json!(true),
                json!(true),
                json!(1000)
            ]
        );
        assert_eq!(
            flags(&mut d, 1),
            [
                json!(false),
                json!(false),
                json!(false),
                json!(true),
                json!(1000)
            ]
        );
    }
}
