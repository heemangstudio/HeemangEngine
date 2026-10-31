//! Paragraph edits that follow Hancom's rules: merging, breaking and splitting
//! paragraphs, and their styles and shapes.
use hm_wasm::{Doc, Engine};
use serde_json::{Value, json};

fn query(d: &mut Doc, q: Value) -> Value {
    serde_json::from_str(&d.query(&q.to_string()).unwrap()).unwrap()
}
fn tx(d: &mut Doc, ops: Value) -> Value {
    serde_json::from_str(&d.transact(&json!({"ops":ops}).to_string()).unwrap()).unwrap()
}
/// An empty document: one paragraph holding the section and column definitions.
fn blank() -> Doc {
    Engine::new()
        .create(include_bytes!("fixtures/hancom-table-4x4.hwpx").to_vec())
        .unwrap()
}
fn reopen(d: &mut Doc, format: &str) -> Doc {
    Engine::new()
        .open(d.serialize_as(format, false).unwrap())
        .unwrap()
}
fn at(para: u32, offset: u32) -> Value {
    json!({"section":0,"para":para,"offset":offset})
}
fn paras(d: &mut Doc) -> Vec<Value> {
    query(d, json!({"kind":"paragraphs","list":{"section":0}}))
        .as_array()
        .unwrap()
        .clone()
}
fn texts(d: &mut Doc) -> Vec<String> {
    paras(d)
        .iter()
        .map(|p| p["text"].as_str().unwrap().to_string())
        .collect()
}
/// Where text starts in paragraph `para` and where it ends.
fn ends(d: &mut Doc, para: usize) -> (u32, u32) {
    let p = &paras(d)[para];
    (
        p["caret"][0].as_u64().unwrap() as u32,
        p["length"].as_u64().unwrap() as u32 - 1,
    )
}
fn formats(d: &mut Doc) -> Vec<Value> {
    query(d, json!({"kind":"formatRuns","list":{"section":0}}))
        .as_array()
        .unwrap()
        .clone()
}
/// Alignment, page break and column break of every paragraph.
fn shapes(d: &mut Doc) -> Vec<(String, bool, bool)> {
    formats(d)
        .iter()
        .map(|f| {
            (
                f["paragraphs"]["align"].as_str().unwrap().to_string(),
                f["pageBreak"].as_bool().unwrap(),
                f["columnBreak"].as_bool().unwrap(),
            )
        })
        .collect()
}
fn shape(align: &str, page: bool, column: bool) -> (String, bool, bool) {
    (align.to_string(), page, column)
}

/// Paragraphs from `(text, align)` pairs, the first one after the definitions.
fn build(d: &mut Doc, rows: &[(&str, &str)]) {
    let (lead, _) = ends(d, 0);
    for (i, (text, align)) in rows.iter().enumerate() {
        let i = i as u32;
        let start = if i == 0 { lead } else { 0 };
        if i > 0 {
            let (_, end) = ends(d, i as usize - 1);
            tx(d, json!([{"op":"split","at":at(i - 1, end)}]));
        }
        tx(
            d,
            json!([{"op":"formatParas","from":at(i, start),"to":at(i, start),"patch":{"align":align}}]),
        );
        if !text.is_empty() {
            tx(
                d,
                json!([{"op":"insertText","at":at(i, start),"text":text}]),
            );
        }
    }
}

#[test]
fn merging_into_an_empty_paragraph_keeps_the_lower_paragraph_in_the_upper_ones_place() {
    // Hancom (typing cases backspace-merge-empty-prev, delete-merge-empty-cur,
    // delete-multi-para-whole-first; fixes/typing-01/hancom): an upper paragraph holding no
    // characters gives way to the lower one, which keeps its shape but takes the upper one's
    // breaks.
    let mut d = blank();
    build(
        &mut d,
        &[("AAA", "justify"), ("", "center"), ("BBB", "right")],
    );
    tx(&mut d, json!([{"op":"pageBreak","at":at(1, 0),"on":true}]));
    let before = shapes(&mut d);
    assert_eq!(before[1], shape("center", true, false));
    tx(
        &mut d,
        json!([{"op":"delete","from":at(1, 0),"to":at(2, 0)}]),
    );
    assert_eq!(texts(&mut d), ["AAA", "BBB"]);
    assert_eq!(shapes(&mut d)[1], shape("right", true, false));
    d.undo().unwrap();
    assert_eq!(texts(&mut d), ["AAA", "", "BBB"]);
    assert_eq!(shapes(&mut d), before);

    // The lower paragraph's own break goes with it: the result stands where the upper was.
    tx(&mut d, json!([{"op":"pageBreak","at":at(1, 0),"on":false}]));
    tx(&mut d, json!([{"op":"pageBreak","at":at(2, 0),"on":true}]));
    tx(
        &mut d,
        json!([{"op":"delete","from":at(1, 0),"to":at(2, 1)}]),
    );
    assert_eq!(texts(&mut d), ["AAA", "BB"]);
    assert_eq!(shapes(&mut d)[1], shape("right", false, false));

    // With characters left in the upper paragraph, it keeps its own shape.
    let mut d = blank();
    build(&mut d, &[("AAA", "center"), ("BBB", "right")]);
    let (_, end) = ends(&mut d, 0);
    tx(
        &mut d,
        json!([{"op":"delete","from":at(0, end),"to":at(1, 0)}]),
    );
    assert_eq!(texts(&mut d), ["AAABBB"]);
    assert_eq!(shapes(&mut d), [shape("center", false, false)]);
}

#[test]
fn an_emptied_first_paragraph_hands_its_definitions_to_the_lower_one() {
    let mut d = blank();
    build(&mut d, &[("", "center"), ("BBB", "right")]);
    tx(
        &mut d,
        json!([{"op":"formatChars","from":at(1, 0),"to":at(1, 3),"patch":{"bold":true}}]),
    );
    let bold = formats(&mut d)[1]["runs"][0]["charShapeId"].clone();
    let (lead, _) = ends(&mut d, 0);
    tx(
        &mut d,
        json!([{"op":"delete","from":at(0, lead),"to":at(1, 0)}]),
    );
    assert_eq!(texts(&mut d), ["BBB"]);
    assert_eq!(shapes(&mut d), [shape("right", false, false)]);
    // The section and column definitions stay first and take the text's shape, as Hancom
    // saves them.
    assert!(lead > 0);
    assert_eq!(ends(&mut d, 0).0, lead);
    let runs = formats(&mut d)[0]["runs"].clone();
    assert_eq!(runs.as_array().unwrap().len(), 1, "{runs}");
    assert_eq!(runs[0]["charShapeId"], bold);
    for format in ["hwp5", "hwpx", "hml"] {
        let mut r = reopen(&mut d, format);
        assert_eq!(texts(&mut r), ["BBB"], "{format}");
        assert_eq!(shapes(&mut r), [shape("right", false, false)], "{format}");
    }
}

#[test]
fn column_breaks_split_only_inside_a_paragraph_and_clear_like_page_breaks() {
    // Hancom's BreakColumn (page cases col06-column-break, col08-colbreak-mid-empty).
    let mut d = blank();
    build(
        &mut d,
        &[
            ("Col A", "justify"),
            ("Col B", "justify"),
            ("", "justify"),
            ("Col D", "justify"),
        ],
    );
    let flags = |d: &mut Doc| -> Vec<bool> { shapes(d).iter().map(|s| s.2).collect() };
    let column = |d: &mut Doc, para: u32, offset: u32| {
        tx(d, json!([{"op":"columnBreak","at":at(para, offset)}]))["caret"].clone()
    };
    // At the start the paragraph itself starts the column.
    assert_eq!(column(&mut d, 1, 0), at(1, 0));
    assert_eq!(texts(&mut d), ["Col A", "Col B", "", "Col D"]);
    assert_eq!(flags(&mut d), [false, true, false, false]);
    d.undo().unwrap();
    // At the end the next one does, and the caret goes there.
    assert_eq!(column(&mut d, 1, 5), at(2, 0));
    assert_eq!(flags(&mut d), [false, false, true, false]);
    assert_eq!(texts(&mut d).len(), 4);
    d.undo().unwrap();
    // An empty paragraph is marked, not split.
    assert_eq!(column(&mut d, 2, 0), at(2, 0));
    assert_eq!(flags(&mut d), [false, false, true, false]);
    assert_eq!(texts(&mut d).len(), 4);
    d.undo().unwrap();
    // Only the middle of a paragraph, or the end of the last one, splits.
    assert_eq!(column(&mut d, 1, 2), at(2, 0));
    assert_eq!(texts(&mut d), ["Col A", "Co", "l B", "", "Col D"]);
    assert_eq!(flags(&mut d), [false, false, true, false, false]);
    d.undo().unwrap();
    assert_eq!(column(&mut d, 3, 5), at(4, 0));
    assert_eq!(texts(&mut d), ["Col A", "Col B", "", "Col D", ""]);
    assert_eq!(flags(&mut d), [false, false, false, false, true]);
    d.undo().unwrap();

    // A column break replaces a page break and both clear with `on:false`.
    tx(&mut d, json!([{"op":"pageBreak","at":at(1, 0),"on":true}]));
    column(&mut d, 1, 0);
    assert_eq!(shapes(&mut d)[1], shape("justify", false, true));
    for format in ["hwp5", "hwpx", "hml"] {
        let mut r = reopen(&mut d, format);
        assert_eq!(shapes(&mut r)[1], shape("justify", false, true), "{format}");
    }
    tx(
        &mut d,
        json!([{"op":"columnBreak","at":at(1, 0),"on":false}]),
    );
    assert_eq!(shapes(&mut d)[1], shape("justify", false, false));
    for format in ["hwp5", "hwpx", "hml"] {
        let mut r = reopen(&mut d, format);
        assert_eq!(
            shapes(&mut r)[1],
            shape("justify", false, false),
            "{format}"
        );
    }
    d.undo().unwrap();
    tx(&mut d, json!([{"op":"pageBreak","at":at(1, 0),"on":false}]));
    assert_eq!(shapes(&mut d)[1], shape("justify", false, false));
    d.undo().unwrap();
    assert_eq!(shapes(&mut d)[1], shape("justify", false, true));
}

fn style_ids(d: &mut Doc) -> Vec<u64> {
    formats(d)
        .iter()
        .map(|f| f["styleId"].as_u64().unwrap())
        .collect()
}
/// Alignment, left margin, space before and line spacing of every paragraph.
fn spacing(d: &mut Doc) -> Vec<(String, i64, i64, i64)> {
    formats(d)
        .iter()
        .map(|f| {
            let p = &f["paragraphs"];
            (
                p["align"].as_str().unwrap().to_string(),
                p["left"].as_i64().unwrap(),
                p["spaceBefore"].as_i64().unwrap(),
                p["lineSpacing"].as_i64().unwrap(),
            )
        })
        .collect()
}
fn sp(align: &str, left: i64, before: i64, line: i64) -> (String, i64, i64, i64) {
    (align.to_string(), left, before, line)
}
/// Offset, bold and color of each run of paragraph `para`.
fn runs(d: &mut Doc, para: usize) -> Vec<(u64, bool, u64)> {
    formats(d)[para]["runs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["offset"].as_u64().unwrap(),
                r["characters"]["bold"].as_bool().unwrap(),
                r["characters"]["color"].as_u64().unwrap(),
            )
        })
        .collect()
}
fn style_count(d: &mut Doc) -> u64 {
    query(d, json!({"kind":"styles"})).as_array().unwrap().len() as u64
}
fn create_style(d: &mut Doc, style: Value) -> u64 {
    tx(d, json!([{"op":"createStyle","style":style}]));
    style_count(d) - 1
}
fn fmt_para(d: &mut Doc, para: u32, patch: Value) {
    let at = at(para, 0);
    tx(
        d,
        json!([{"op":"formatParas","from":at,"to":at,"patch":patch}]),
    );
}

#[test]
fn redefining_a_style_restyles_its_paragraphs_and_keeps_their_own_formatting() {
    // Hancom's StyleChangeToCurrentShape (parashape case style-redefine-propagate): every
    // paragraph in the style follows it, except where it sets an attribute itself.
    let mut d = blank();
    let rows = [
        ("zero", "justify"),
        ("one", "justify"),
        ("two", "justify"),
        ("three", "justify"),
    ];
    build(&mut d, &rows);
    let base = json!({"align":"justify","left":0,"spaceBefore":0,"lineSpacing":160,"lineSpacingKind":"percent"});
    let id = create_style(
        &mut d,
        json!({"name":"Body","paragraphs":base,"characters":{"bold":false,"color":0}}),
    );
    let (lead, _) = ends(&mut d, 0);
    let (_, end) = ends(&mut d, 3);
    let apply = json!({"op":"applyStyle","from":at(0, lead),"to":at(3, end),"styleId":id});
    tx(&mut d, json!([apply]));
    fmt_para(&mut d, 1, json!({"align":"right"}));
    fmt_para(&mut d, 2, json!({"left":1000}));
    fmt_para(&mut d, 3, json!({"lineSpacing":250}));
    let bold =
        json!({"op":"formatChars","from":at(0, lead),"to":at(0, lead + 2),"patch":{"bold":true}});
    tx(&mut d, json!([bold]));
    let before = (spacing(&mut d), runs(&mut d, 0));
    let styles = style_count(&mut d);
    let patch = json!({
        "paragraphs":{"align":"center","left":600,"spaceBefore":700,"lineSpacing":200},
        "characters":{"color":0xFF0000}});
    tx(&mut d, json!([{"op":"updateStyle","id":id,"style":patch}]));
    let after = [
        sp("center", 600, 700, 200),
        sp("right", 600, 700, 200),
        sp("center", 1000, 700, 200),
        sp("center", 600, 700, 250),
    ];
    assert_eq!(spacing(&mut d), after);
    assert_eq!(style_ids(&mut d), [id; 4]);
    // The bold run stays bold and takes the style's color like the rest.
    let red = 0xFF0000;
    let (lead, split) = (u64::from(lead), u64::from(lead) + 2);
    assert_eq!(
        runs(&mut d, 0),
        [(0, false, red), (lead, true, red), (split, false, red)]
    );
    assert_eq!(runs(&mut d, 1), [(0, false, red)]);
    for format in ["hwp5", "hwpx"] {
        let mut r = reopen(&mut d, format);
        assert_eq!(spacing(&mut r), after, "{format}");
        assert_eq!(
            runs(&mut r, 0),
            [(0, false, red), (lead, true, red), (split, false, red)],
            "{format}"
        );
    }
    // One step: undo puts the paragraphs back with the old definition.
    d.undo().unwrap();
    assert_eq!((spacing(&mut d), runs(&mut d, 0)), before);
    assert_eq!(style_count(&mut d), styles);
}

#[test]
fn a_style_applied_at_the_caret_keeps_direct_formatting_and_a_selection_replaces_it() {
    // Hancom (parashape case style-apply-caret, _verify v1/v2): with only the caret in the
    // paragraph its own alignment, margins, spacing and bold run stay and the rest follows
    // the style, characters included; with the paragraph selected the style replaces all.
    let mut d = blank();
    build(
        &mut d,
        &[("first text", "justify"), ("second text", "justify")],
    );
    let heading = create_style(
        &mut d,
        json!({"name":"Heading","paragraphs":{"left":1000,"lineSpacing":160},"characters":{"color":0x0000FF}}),
    );
    let (lead, _) = ends(&mut d, 0);
    let direct = json!({"align":"center","left":500,"spaceBefore":400,"lineSpacing":220});
    fmt_para(&mut d, 0, direct.clone());
    fmt_para(&mut d, 1, direct);
    let bold = json!({"bold":true});
    tx(
        &mut d,
        json!([{"op":"formatChars","from":at(0, lead),"to":at(0, lead + 3),"patch":bold},
               {"op":"formatChars","from":at(1, 0),"to":at(1, 3),"patch":bold}]),
    );
    let blue = 0x0000FF;
    let caret = at(0, lead + 4);
    let apply = json!({"op":"applyStyle","from":caret,"to":caret,"styleId":heading});
    tx(&mut d, json!([apply]));
    assert_eq!(style_ids(&mut d)[0], heading);
    assert_eq!(spacing(&mut d)[0], sp("center", 500, 400, 220));
    let (lead, split) = (u64::from(lead), u64::from(lead) + 3);
    assert_eq!(
        runs(&mut d, 0),
        [(0, false, blue), (lead, true, blue), (split, false, blue)]
    );
    // A selection takes the style whole.
    let (_, end) = ends(&mut d, 1);
    let apply = json!({"op":"applyStyle","from":at(1, 0),"to":at(1, end),"styleId":heading});
    tx(&mut d, json!([apply]));
    assert_eq!(spacing(&mut d)[1], sp("justify", 1000, 0, 160));
    assert_eq!(runs(&mut d, 1), [(0, false, blue)]);
    d.undo().unwrap();
    d.undo().unwrap();
    assert_eq!(spacing(&mut d)[0], sp("center", 500, 400, 220));
    assert_eq!(runs(&mut d, 0)[1], (lead, true, 0));
}

#[test]
fn deleting_a_style_moves_its_paragraphs_to_the_replacement_and_repairs_next_styles() {
    // Hancom's StyleDelete with Target and Alternation (parashape case style-delete-used):
    // paragraphs take the replacement, ids after the deleted style shift down, and a style
    // whose next style was the deleted one names itself.
    let mut d = blank();
    build(&mut d, &[("one", "justify"), ("two", "justify")]);
    let first = create_style(&mut d, json!({"name":"First","paragraphs":{"left":1500}}));
    let second = create_style(
        &mut d,
        json!({"name":"Second","next":first,"paragraphs":{"align":"right"}}),
    );
    let (lead, end) = ends(&mut d, 0);
    tx(
        &mut d,
        json!([{"op":"applyStyle","from":at(0, lead),"to":at(0, end),"styleId":first},
               {"op":"applyStyle","from":at(1, 0),"to":at(1, 3),"styleId":second}]),
    );
    let used = json!({"ops":[{"op":"deleteStyle","id":first}]});
    let refused = d.transact(&used.to_string()).unwrap_err();
    assert!(
        format!("{refused:?}").contains("BAD_REQUEST"),
        "{refused:?}"
    );
    tx(
        &mut d,
        json!([{"op":"deleteStyle","id":first,"replacement":0}]),
    );
    assert_eq!(style_ids(&mut d), [0, first]);
    assert_eq!(spacing(&mut d)[0].1, 0);
    assert_eq!(spacing(&mut d)[1].0, "right");
    let renamed = query(&mut d, json!({"kind":"style","id":first}));
    assert_eq!(renamed["name"], "Second");
    assert_eq!(renamed["next"], first);
    d.undo().unwrap();
    assert_eq!(style_ids(&mut d), [first, second]);
    assert_eq!(spacing(&mut d)[0].1, 1500);
    let next = query(&mut d, json!({"kind":"style","id":second}))["next"].clone();
    assert_eq!(next, first);
}

fn code(d: &mut Doc, ops: Value) -> String {
    let e = d.transact(&json!({ "ops": ops }).to_string()).unwrap_err();
    let e = format!("{e:?}");
    ["BAD_REQUEST", "BAD_ADDRESS"]
        .into_iter()
        .find(|c| e.contains(c))
        .unwrap_or(&e)
        .to_string()
}
fn paragraph(d: &mut Doc, para: usize) -> Value {
    formats(d)[para]["paragraphs"].clone()
}

#[test]
fn paragraph_flags_break_units_borders_and_auto_tabs_set_report_and_save() {
    // Hancom writes these from its paragraph dialog (parashape cases flags-break,
    // tabs-auto, border-lines); the engine already laid them out but could not set them.
    let mut d = blank();
    build(&mut d, &[("one", "justify"), ("two", "justify")]);
    let before = formats(&mut d)[0]["paraShapeId"].clone();
    let flags = json!({
        "keepWithNext":true, "keepLines":true, "widowOrphan":true, "pageBreakBefore":true,
        "breakLatinWord":1, "breakNonLatinWord":0, "lineWrap":1, "snapToGrid":false,
        "condense":20, "fontLineHeight":true, "verticalAlign":2, "eAsianEng":true,
        "eAsianNum":false, "borderConnect":true, "borderIgnoreMargin":true,
        "borderOffsets":[100, 200, 300, 400], "autoTabLeft":true
    });
    fmt_para(&mut d, 0, flags.clone());
    assert_ne!(formats(&mut d)[0]["paraShapeId"], before);
    let set = paragraph(&mut d, 0);
    for (key, value) in flags.as_object().unwrap() {
        assert_eq!(&set[key], value, "{key}");
    }
    assert_eq!(paragraph(&mut d, 1)["keepWithNext"], false);
    for format in ["hwp5", "hwpx", "hml"] {
        let mut r = reopen(&mut d, format);
        let read = paragraph(&mut r, 0);
        for (key, value) in flags.as_object().unwrap() {
            assert_eq!(&read[key], value, "{format} {key}");
        }
    }
    // A paragraph keeps its automatic tabs when only its stops change, and the reverse.
    let stops = json!([{"position":4000,"type":0,"fill":0}]);
    fmt_para(&mut d, 0, json!({"tabStops":stops}));
    let p = paragraph(&mut d, 0);
    assert_eq!(
        (p["autoTabLeft"].clone(), p["tabStops"].clone()),
        (json!(true), stops.clone())
    );
    fmt_para(&mut d, 0, json!({"autoTabLeft":false}));
    let p = paragraph(&mut d, 0);
    assert_eq!(
        (p["autoTabLeft"].clone(), p["tabStops"].clone()),
        (json!(false), stops)
    );
    // Clearing the flag takes the page break away again.
    fmt_para(&mut d, 0, json!({"pageBreakBefore":false}));
    assert_eq!(paragraph(&mut d, 0)["pageBreakBefore"], false);
    // What a query reports goes back unchanged.
    let echo = paragraph(&mut d, 0);
    fmt_para(&mut d, 1, echo.clone());
    assert_eq!(paragraph(&mut d, 1), echo);
}

#[test]
fn paragraph_patches_refuse_unknown_keys_and_values_out_of_range() {
    let mut d = blank();
    build(&mut d, &[("one", "justify")]);
    let state = d.state().unwrap();
    let saved = d.serialize(false).unwrap();
    let (lead, _) = ends(&mut d, 0);
    let fmt = |patch: Value| json!([{"op":"formatParas","from":at(0, lead),"to":at(0, lead),"patch":patch}]);
    for patch in [
        json!({"keepWithNex":true}),
        json!({"left":1000000}),
        json!({"left":2147483647}),
        json!({"indent":-2147483648i64}),
        json!({"right":100000}),
        json!({"spaceBefore":2147483647}),
        json!({"lineSpacing":9999,"lineSpacingKind":"percent"}),
        json!({"lineSpacing":2000000,"lineSpacingKind":"fixed"}),
        json!({"lineSpacing":1000000,"lineSpacingKind":"added"}),
        json!({"condense":76}),
        json!({"breakLatinWord":3}),
        json!({"verticalAlign":4}),
        json!({"borderFillId":60000}),
        json!({"borderOffsets":[0, 0, 0, 40000]}),
        json!({"heading":{"kind":"numbering","id":60000}}),
    ] {
        assert_eq!(code(&mut d, fmt(patch.clone())), "BAD_REQUEST", "{patch}");
    }
    let style =
        json!([{"op":"createStyle","style":{"name":"X","paragraphs":{"keepWithNex":true}}}]);
    assert_eq!(code(&mut d, style), "BAD_REQUEST");
    assert_eq!(d.state().unwrap(), state);
    assert_eq!(d.serialize(false).unwrap(), saved);
    // Hancom's own line spacing steps reach 0 percent.
    tx(
        &mut d,
        fmt(json!({"lineSpacing":0,"lineSpacingKind":"percent"})),
    );
    assert_eq!(paragraph(&mut d, 0)["lineSpacing"], 0);
}

#[test]
fn new_numbers_take_a_kind_and_style_shapes_take_a_paragraph_head() {
    // Hancom's 새 번호로 시작 has six counters (page case num02-newnum-types writes
    // numType PICTURE and TABLE); a style's 문단 종류 is its paragraph shape's head.
    let mut d = blank();
    build(
        &mut d,
        &[("one", "justify"), ("two", "justify"), ("three", "justify")],
    );
    let (lead, _) = ends(&mut d, 0);
    for (para, offset) in [(0, lead + 1), (1, 1), (2, 1)] {
        tx(
            &mut d,
            json!([{"op":"insertNote","at":at(para, offset),"kind":"footnote"}]),
        );
    }
    let numbers = |d: &mut Doc| -> Vec<(String, u64)> {
        query(d, json!({"kind":"controls","list":{"section":0}}))
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["kind"] == "footnote" || c["kind"] == "newNumber")
            .map(|c| {
                let kind = c["numberKind"].as_str().unwrap_or("note").to_string();
                (kind, c["number"].as_u64().unwrap())
            })
            .collect()
    };
    let note = |n| ("note".to_string(), n);
    assert_eq!(numbers(&mut d), [note(1), note(2), note(3)]);
    tx(
        &mut d,
        json!([{"op":"insertNewNumber","at":at(1, 0),"number":7,"kind":"footnote"}]),
    );
    let after = [note(1), ("footnote".to_string(), 7), note(7), note(8)];
    assert_eq!(numbers(&mut d), after);
    tx(
        &mut d,
        json!([{"op":"insertNewNumber","at":at(2, 0),"number":4,"kind":"figure"},
               {"op":"insertNewNumber","at":at(2, 0),"number":5,"kind":"table"}]),
    );
    let kinds = |d: &mut Doc| -> Vec<String> {
        numbers(d)
            .into_iter()
            .map(|(k, _)| k)
            .filter(|k| k != "note")
            .collect()
    };
    assert_eq!(kinds(&mut d), ["footnote", "table", "figure"]);
    for format in ["hwp5", "hwpx", "hml"] {
        let mut r = reopen(&mut d, format);
        assert_eq!(kinds(&mut r), ["footnote", "table", "figure"], "{format}");
    }
    assert_eq!(
        code(
            &mut d,
            json!([{"op":"insertNewNumber","at":at(0, lead),"number":1,"kind":"chapter"}])
        ),
        "BAD_REQUEST"
    );

    let heading = json!({"kind":"outline","id":0,"level":2});
    let id = create_style(
        &mut d,
        json!({"name":"Level 3","paragraphs":{"heading":heading}}),
    );
    let style = query(&mut d, json!({"kind":"style","id":id}));
    assert_eq!(style["paragraphs"]["heading"], heading);
    tx(
        &mut d,
        json!([{"op":"applyStyle","from":at(1, 0),"to":at(1, 3),"styleId":id}]),
    );
    let list = json!({"kind":"outline","id":0,"level":2});
    assert_eq!(formats(&mut d)[1]["list"], list);
    for format in ["hwp5", "hwpx"] {
        let mut r = reopen(&mut d, format);
        let style = query(&mut r, json!({"kind":"style","id":id}));
        assert_eq!(style["paragraphs"]["heading"], heading, "{format}");
        assert_eq!(formats(&mut r)[1]["list"], list, "{format}");
    }
}

/// Size and bold of the character shape of each run of paragraph `para`.
fn sizes(d: &mut Doc, para: usize) -> Vec<(u64, i64, bool)> {
    formats(d)[para]["runs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["offset"].as_u64().unwrap(),
                r["characters"]["size"].as_i64().unwrap(),
                r["characters"]["bold"].as_bool().unwrap(),
            )
        })
        .collect()
}

#[test]
fn enter_leaves_the_line_above_in_the_shape_before_the_caret() {
    // Hancom (typing cases enter-start-bigend, enter-start-mixed-p1, enter-boundary-twice,
    // enter-boundary-mixed): the end mark left above takes the shape before the cut, so
    // what is typed there is not the shape of the text that moved down.
    let mut d = blank();
    build(&mut d, &[("zzz", "justify"), ("AAABBB", "justify")]);
    let big = json!({"size":2400,"bold":true});
    tx(
        &mut d,
        json!([{"op":"formatChars","from":at(1, 3),"to":at(1, 6),"patch":big}]),
    );
    let plain = sizes(&mut d, 1)[0];
    assert_eq!(plain.0, 0);
    // At the start: the line above is in the first character's shape.
    tx(&mut d, json!([{"op":"split","at":at(1, 0)}]));
    assert_eq!(sizes(&mut d, 1), [(0, plain.1, plain.2)]);
    tx(
        &mut d,
        json!([{"op":"insertText","at":at(1, 0),"text":"x"}]),
    );
    assert_eq!(sizes(&mut d, 1), [(0, plain.1, plain.2)]);
    d.undo().unwrap();
    d.undo().unwrap();
    // Between the runs, then again at the end of the upper half: no trailing run in the
    // moved shape, and the new line types plain.
    tx(&mut d, json!([{"op":"split","at":at(1, 3)}]));
    assert_eq!(sizes(&mut d, 1), [(0, plain.1, plain.2)]);
    assert_eq!(sizes(&mut d, 2), [(0, 2400, true)]);
    tx(&mut d, json!([{"op":"split","at":at(1, 3)}]));
    tx(
        &mut d,
        json!([{"op":"insertText","at":at(2, 0),"text":"x"}]),
    );
    assert_eq!(sizes(&mut d, 2), [(0, plain.1, plain.2)]);
    // The same for a page break.
    let mut d2 = blank();
    build(&mut d2, &[("AAABBB", "justify")]);
    let (lead, _) = ends(&mut d2, 0);
    tx(
        &mut d2,
        json!([{"op":"formatChars","from":at(0, lead + 3),"to":at(0, lead + 6),"patch":big}]),
    );
    tx(
        &mut d2,
        json!([{"op":"split","at":at(0, lead + 3),"pageBreak":true}]),
    );
    let head = sizes(&mut d2, 0);
    assert!(head.iter().all(|r| r.1 != 2400), "{head:?}");
    assert_eq!(shapes(&mut d2)[1], shape("justify", true, false));
}

#[test]
fn enter_at_the_end_of_a_paragraph_starts_the_next_style() {
    // Hancom (typing case enter-end-next-style, parashape case style-next-enter,
    // fixes/typing-08/hancom/empty-next-style): Enter at the end of a paragraph, empty or
    // not, gives the new one the style's next style with its shapes; at the start or in the
    // middle both halves keep the style.
    let mut d = blank();
    build(&mut d, &[("Heading text", "justify")]);
    let body = create_style(
        &mut d,
        json!({"name":"Body","paragraphs":{"left":1500},"characters":{"size":900}}),
    );
    let heading = create_style(
        &mut d,
        json!({"name":"Heading","next":body,"paragraphs":{"align":"center"},"characters":{"size":1600}}),
    );
    let (lead, end) = ends(&mut d, 0);
    tx(
        &mut d,
        json!([{"op":"applyStyle","from":at(0, lead),"to":at(0, end),"styleId":heading}]),
    );
    for (offset, styles) in [(lead, [heading, heading]), (lead + 3, [heading, heading])] {
        tx(&mut d, json!([{"op":"split","at":at(0, offset)}]));
        assert_eq!(style_ids(&mut d), styles, "{offset}");
        d.undo().unwrap();
    }
    let receipt = tx(&mut d, json!([{"op":"split","at":at(0, end)}]));
    assert_eq!(receipt["caret"], at(1, 0));
    assert_eq!(style_ids(&mut d), [heading, body]);
    assert_eq!(spacing(&mut d)[1].0, "justify");
    assert_eq!(spacing(&mut d)[1].1, 1500);
    assert_eq!(sizes(&mut d, 1), [(0, 900, false)]);
    tx(
        &mut d,
        json!([{"op":"insertText","at":at(1, 0),"text":"x"}]),
    );
    assert_eq!(sizes(&mut d, 1), [(0, 900, false)]);
    // One step with the split.
    d.undo().unwrap();
    d.undo().unwrap();
    assert_eq!(style_ids(&mut d), [heading]);
    // In an empty paragraph too; `nextStyle:false` keeps the style.
    tx(
        &mut d,
        json!([{"op":"split","at":at(0, end),"nextStyle":false}]),
    );
    assert_eq!(style_ids(&mut d), [heading, heading]);
    tx(&mut d, json!([{"op":"split","at":at(1, 0)}]));
    assert_eq!(style_ids(&mut d), [heading, heading, body]);
    // A page break at the end, and a column break after the last paragraph, likewise.
    d.undo().unwrap();
    tx(
        &mut d,
        json!([{"op":"split","at":at(1, 0),"pageBreak":true}]),
    );
    assert_eq!(style_ids(&mut d), [heading, heading, body]);
    assert_eq!(shapes(&mut d)[2], shape("justify", true, false));
    d.undo().unwrap();
    tx(&mut d, json!([{"op":"columnBreak","at":at(1, 0)}]));
    assert_eq!(style_ids(&mut d), [heading, heading]);
    d.undo().unwrap();
    tx(
        &mut d,
        json!([{"op":"insertText","at":at(1, 0),"text":"tail"}]),
    );
    tx(&mut d, json!([{"op":"columnBreak","at":at(1, 4)}]));
    assert_eq!(style_ids(&mut d), [heading, heading, body]);
    assert_eq!(shapes(&mut d)[2], shape("justify", false, true));
}

#[test]
fn enter_leaves_page_break_before_with_the_first_half() {
    // Hancom (parashape case pagebreak-before): Enter in a paragraph that always starts a
    // page keeps that on the first half only.
    let mut d = blank();
    build(&mut d, &[("one", "justify"), ("two words", "justify")]);
    fmt_para(&mut d, 1, json!({"pageBreakBefore":true}));
    tx(&mut d, json!([{"op":"split","at":at(1, 3)}]));
    let flags: Vec<Value> = (0..3)
        .map(|i| paragraph(&mut d, i)["pageBreakBefore"].clone())
        .collect();
    assert_eq!(flags, [json!(false), json!(true), json!(false)]);
    assert_eq!(texts(&mut d), ["one", "two", " words"]);
    d.undo().unwrap();
    assert_eq!(paragraph(&mut d, 1)["pageBreakBefore"], true);

    // Hancom (fixes/_paragraphs_review pbb-enter-start, pbb-pagebreak-mid, pbb-colbreak-mid):
    // Enter at the start leaves the flag with the empty line above, and a page or column
    // break in the middle leaves it with the first half like Enter.
    let flags = |d: &mut Doc| -> Vec<bool> {
        (0..3)
            .map(|i| paragraph(d, i)["pageBreakBefore"].as_bool().unwrap())
            .collect()
    };
    tx(&mut d, json!([{"op":"split","at":at(1, 0)}]));
    assert_eq!(texts(&mut d), ["one", "", "two words"]);
    assert_eq!(flags(&mut d), [false, true, false]);
    d.undo().unwrap();
    tx(
        &mut d,
        json!([{"op":"split","at":at(1, 3),"pageBreak":true}]),
    );
    assert_eq!(flags(&mut d), [false, true, false]);
    assert_eq!(shapes(&mut d)[2], shape("justify", true, false));
    d.undo().unwrap();
    tx(&mut d, json!([{"op":"columnBreak","at":at(1, 3)}]));
    assert_eq!(texts(&mut d), ["one", "two", " words"]);
    assert_eq!(flags(&mut d), [false, true, false]);
    assert_eq!(shapes(&mut d)[2], shape("justify", false, true));
    for format in ["hwp5", "hwpx", "hml"] {
        let mut r = reopen(&mut d, format);
        assert_eq!(flags(&mut r), [false, true, false], "{format}");
    }
    d.undo().unwrap();
    assert_eq!(texts(&mut d), ["one", "two words"]);
    assert_eq!(paragraph(&mut d, 1)["pageBreakBefore"], true);
}

#[test]
fn a_line_spacing_kind_added_to_an_old_hwp_shape_saves_in_its_own_field() {
    // HWP 5 reads a paragraph shape's optional fields by position: a spacing kind written
    // without the field before it read back as "keep on one line".
    let mut d = Engine::new()
        .open(include_bytes!("fixtures/query-equations.hwp").to_vec())
        .unwrap();
    let (lead, _) = ends(&mut d, 0);
    let patch = json!({"lineSpacing":1800,"lineSpacingKind":"fixed"});
    tx(
        &mut d,
        json!([{"op":"formatParas","from":at(0, lead),"to":at(0, lead),"patch":patch}]),
    );
    let set = paragraph(&mut d, 0);
    let mut r = reopen(&mut d, "hwp5");
    let read = paragraph(&mut r, 0);
    for key in ["lineWrap", "lineSpacing", "lineSpacingKind"] {
        assert_eq!(read[key], set[key], "{key}");
    }
    assert_eq!(read["lineWrap"], 0);
}

#[test]
fn deleting_a_style_renumbers_the_ones_after_it_in_saved_files() {
    let mut d = blank();
    build(&mut d, &[("one", "justify")]);
    let last = create_style(&mut d, json!({"name":"Last","paragraphs":{"left":700}}));
    let (lead, end) = ends(&mut d, 0);
    let apply = json!({"op":"applyStyle","from":at(0, lead),"to":at(0, end),"styleId":last});
    tx(&mut d, json!([apply]));
    let names = |d: &mut Doc| -> Vec<(String, u64)> {
        query(d, json!({"kind":"styles"}))
            .as_array()
            .unwrap()
            .iter()
            .map(|s| {
                (
                    s["name"].as_str().unwrap().to_string(),
                    s["next"].as_u64().unwrap(),
                )
            })
            .collect()
    };
    // A style of the file's own table, with read styles after it.
    tx(&mut d, json!([{"op":"deleteStyle","id":1,"replacement":0}]));
    let kept = names(&mut d);
    assert_eq!(style_ids(&mut d), [last - 1]);
    for format in ["hwp5", "hwpx", "hml"] {
        let mut r = reopen(&mut d, format);
        assert_eq!(names(&mut r), kept, "{format}");
        assert_eq!(style_ids(&mut r), [last - 1], "{format}");
        assert_eq!(spacing(&mut r)[0].1, 700, "{format}");
    }
    // HWPX names styles by id, which Hancom resolves: the ids are the positions again.
    let saved = d.serialize_as("hwpx", false).unwrap();
    let header = hm_hwpx::zip::Archive::open(saved.into())
        .unwrap()
        .read_name("Contents/header.xml")
        .unwrap()
        .unwrap()
        .to_vec();
    let header = String::from_utf8_lossy(&header);
    let ids: Vec<usize> = header
        .split("<hh:style ")
        .skip(1)
        .map(|s| {
            let id = s.split("id=\"").nth(1).unwrap();
            id[..id.find('"').unwrap()].parse().unwrap()
        })
        .collect();
    assert_eq!(ids, (0..kept.len()).collect::<Vec<_>>());
}

#[test]
fn enter_after_deleting_the_last_run_leaves_its_shape_to_the_new_line() {
    // Hancom (typing case enter-selection): select bold BBB after plain AAA and press Enter;
    // the line above ends plain and the new one carries the bold end mark.
    let mut d = blank();
    build(&mut d, &[("zzz", "justify"), ("AAABBB", "justify")]);
    let bold = json!({"bold":true});
    tx(
        &mut d,
        json!([{"op":"formatChars","from":at(1, 3),"to":at(1, 6),"patch":bold}]),
    );
    tx(
        &mut d,
        json!([{"op":"delete","from":at(1, 3),"to":at(1, 6)}]),
    );
    tx(&mut d, json!([{"op":"split","at":at(1, 3)}]));
    let line = |d: &mut Doc, para| -> Vec<bool> { sizes(d, para).iter().map(|r| r.2).collect() };
    assert_eq!(line(&mut d, 1), [false]);
    assert_eq!(line(&mut d, 2), [true]);
}

/// The list address of the first cell of the table in paragraph `para` of the body.
fn first_cell(d: &mut Doc, para: u32) -> Value {
    let p = query(d, json!({"kind":"paragraphs","list":{"section":0}}))[para as usize].clone();
    let control = p["controls"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["kind"] == "table")
        .unwrap()["index"]
        .clone();
    json!({"section":0,"steps":[{"para":para,"control":control,"index":0}]})
}
fn cell_format(d: &mut Doc, cell: &Value) -> Value {
    query(d, json!({"kind":"formatRuns","list":cell}))[0].clone()
}

#[test]
fn style_edits_reach_paragraphs_in_table_cells() {
    // Updating and deleting a style restyle every paragraph in it, nested lists included.
    let mut d = blank();
    build(&mut d, &[("body", "justify"), ("", "justify")]);
    tx(
        &mut d,
        json!([{"op":"createTable","at":at(1, 0),"rows":1,"columns":1}]),
    );
    let cell = first_cell(&mut d, 1);
    let inside = |offset: u32| {
        let mut a = cell.clone();
        a["para"] = json!(0);
        a["offset"] = json!(offset);
        a
    };
    tx(
        &mut d,
        json!([{"op":"insertText","at":inside(0),"text":"cell"}]),
    );
    let quote = create_style(&mut d, json!({"name":"Quote","paragraphs":{"left":800}}));
    tx(
        &mut d,
        json!([{"op":"applyStyle","from":inside(0),"to":inside(4),"styleId":quote}]),
    );
    tx(
        &mut d,
        json!([{"op":"formatChars","from":inside(0),"to":inside(2),"patch":{"italic":true}}]),
    );
    tx(
        &mut d,
        json!([{"op":"updateStyle","id":quote,"style":{"paragraphs":{"left":1200,"align":"center"},"characters":{"color":0x00FF00}}}]),
    );
    let f = cell_format(&mut d, &cell);
    assert_eq!(f["styleId"], quote);
    assert_eq!(
        (
            f["paragraphs"]["left"].clone(),
            f["paragraphs"]["align"].clone()
        ),
        (json!(1200), json!("center"))
    );
    let runs = |f: &Value| -> Vec<(bool, u64)> {
        f["runs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                (
                    r["characters"]["italic"].as_bool().unwrap(),
                    r["characters"]["color"].as_u64().unwrap(),
                )
            })
            .collect()
    };
    assert_eq!(runs(&f), [(true, 0x00FF00), (false, 0x00FF00)]);
    // A replacement that is not a paragraph style is refused and changes nothing.
    let saved = d.serialize(false).unwrap();
    let character = create_style(&mut d, json!({"name":"Mark","kind":"character"}));
    let state = d.state().unwrap();
    for replacement in [character, quote, 999] {
        assert_eq!(
            code(
                &mut d,
                json!([{"op":"deleteStyle","id":quote,"replacement":replacement}])
            ),
            "BAD_REQUEST",
            "{replacement}"
        );
    }
    assert_eq!(d.state().unwrap(), state);
    d.undo().unwrap();
    assert_eq!(d.serialize(false).unwrap(), saved);
    // Deleting it moves the cell paragraph to the replacement, keeping its own italic run.
    tx(
        &mut d,
        json!([{"op":"deleteStyle","id":quote,"replacement":0}]),
    );
    let f = cell_format(&mut d, &cell);
    assert_eq!(f["styleId"], 0);
    assert_eq!(
        (
            f["paragraphs"]["left"].clone(),
            f["paragraphs"]["align"].clone()
        ),
        (json!(0), json!("justify"))
    );
    assert_eq!(runs(&f), [(true, 0), (false, 0)]);
    for format in ["hwp5", "hwpx", "hml"] {
        let mut r = reopen(&mut d, format);
        let f = cell_format(&mut r, &cell);
        assert_eq!(f["styleId"], 0, "{format}");
        assert_eq!(runs(&f), [(true, 0), (false, 0)], "{format}");
    }
}

#[test]
fn a_style_change_keeps_a_runs_own_superscript_or_subscript_whole() {
    // Superscript and subscript are one choice: a run that set one keeps it alone when the
    // style moves to the other.
    let mut d = blank();
    build(&mut d, &[("H2O water", "justify")]);
    let id = create_style(&mut d, json!({"name":"Plain"}));
    let (lead, end) = ends(&mut d, 0);
    tx(
        &mut d,
        json!([{"op":"applyStyle","from":at(0, lead),"to":at(0, end),"styleId":id},
               {"op":"formatChars","from":at(0, lead + 1),"to":at(0, lead + 2),"patch":{"subscript":true}}]),
    );
    tx(
        &mut d,
        json!([{"op":"updateStyle","id":id,"style":{"characters":{"superscript":true}}}]),
    );
    let scripts: Vec<(bool, bool)> = formats(&mut d)[0]["runs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["characters"]["superscript"].as_bool().unwrap(),
                r["characters"]["subscript"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(scripts, [(true, false), (false, true), (true, false)]);
}

#[test]
fn a_style_at_the_caret_that_changes_nothing_adds_no_step() {
    let mut d = blank();
    build(&mut d, &[("one", "justify")]);
    let (lead, _) = ends(&mut d, 0);
    let state = d.state().unwrap();
    let caret = at(0, lead + 1);
    tx(
        &mut d,
        json!([{"op":"applyStyle","from":caret,"to":caret,"styleId":0}]),
    );
    assert_eq!(d.state().unwrap(), state);
}
