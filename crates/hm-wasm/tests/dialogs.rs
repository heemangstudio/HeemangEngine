//! Dialog requests exercise native history and both save formats.
use hm_wasm::{Doc, Engine};
use serde_json::{Value, json};

fn query(doc: &mut Doc, value: Value) -> Value {
    serde_json::from_str(&doc.query(&value.to_string()).unwrap()).unwrap()
}
fn edit(doc: &mut Doc, ops: Value) -> Value {
    serde_json::from_str(&doc.transact(&json!({"ops":ops}).to_string()).unwrap()).unwrap()
}
fn sources() -> [&'static [u8]; 2] {
    [
        include_bytes!("fixtures/query-equations.hwp"),
        include_bytes!("fixtures/query-equations.hwpx"),
    ]
}
fn at(offset: u32) -> Value {
    json!({"section":0,"para":0,"offset":offset})
}
fn start(doc: &mut Doc) -> u32 {
    let p = query(
        doc,
        json!({"kind":"paragraphs","list":{"section":0},"count":1}),
    );
    // End of paragraph avoids the protected leading section/column controls.
    p[0]["length"].as_u64().unwrap() as u32 - 1
}
fn controls(doc: &mut Doc) -> Value {
    query(doc, json!({"kind":"controls"}))
}
fn assert_locatable(doc: &mut Doc, at: &Value) {
    let list = json!({"section":at["section"],"steps":at["steps"]});
    let lines = query(doc, json!({"kind":"lines","list":list,"para":at["para"]}));
    assert!(
        !lines.as_array().unwrap().is_empty(),
        "empty furniture needs an editable line: {at}"
    );
    let located: Value = serde_json::from_str(
        &doc.locate(&json!({"kind":"caret","position":at}).to_string())
            .unwrap(),
    )
    .unwrap();
    assert!(located["height"].as_i64().unwrap_or(0) > 0, "{located}");
}
fn created(doc: &mut Doc, kind: &str) -> Value {
    controls(doc)
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|c| c["kind"] == kind)
        .unwrap()
        .clone()
}

#[test]
fn section_dialogs_roundtrip_and_undo_as_one_step() {
    for bytes in sources() {
        let engine = Engine::new();
        let mut doc = engine.open(bytes.to_vec()).unwrap();
        let before = query(&mut doc, json!({"kind":"sectionSettings","section":0}));
        let props = query(&mut doc, json!({"kind":"documentProperties"}));
        let initial_state: Value = serde_json::from_str(&doc.state().unwrap()).unwrap();
        let receipt = edit(
            &mut doc,
            json!([
                {"op":"setPageSettings","section":0,"patch":{"width":61000,"height":85000,"marginLeft":5000,"landscape":true,"binding":1}},
                {"op":"setSectionSettings","section":0,"patch":{"tabStop":4200,"pageNumber":7}},
                {"op":"setNoteShape","section":0,"kind":"footnote","patch":{"start":3,"prefix":40,"suffix":41,"spaceBetween":150}},
                {"op":"setNoteShape","section":0,"kind":"endnote","patch":{"start":9,"lineColor":255}},
                {"op":"setDocumentProperties","patch":{"pageStart":7,"footnoteStart":3}}
            ]),
        );
        assert_eq!(
            receipt["undo"].as_u64().unwrap(),
            initial_state["undo"].as_u64().unwrap() + 1
        );
        let after = query(&mut doc, json!({"kind":"sectionSettings","section":0}));
        assert_eq!(after["page"]["width"], 61000);
        assert_eq!(after["page"]["landscape"], true);
        assert_eq!(after["settings"]["tabStop"], 4200);
        assert_eq!(after["noteShapes"]["endnote"]["start"], 9);
        assert_eq!(
            query(&mut doc, json!({"kind":"documentProperties"}))["footnoteStart"],
            3
        );
        let saved = doc.serialize(false).unwrap();
        let mut reopened = engine.open(saved).unwrap();
        assert_eq!(
            query(&mut reopened, json!({"kind":"sectionSettings","section":0})),
            after
        );
        assert_eq!(
            query(&mut reopened, json!({"kind":"documentProperties"}))["footnoteStart"],
            3
        );
        doc.undo().unwrap();
        assert_eq!(
            query(&mut doc, json!({"kind":"sectionSettings","section":0})),
            before
        );
        assert_eq!(query(&mut doc, json!({"kind":"documentProperties"})), props);
        assert_eq!(doc.serialize(false).unwrap(), bytes);
        doc.redo().unwrap();
        assert_eq!(
            query(&mut doc, json!({"kind":"sectionSettings","section":0})),
            after
        );
    }
}

#[test]
fn furniture_creation_nested_editing_removal_and_save() {
    for bytes in sources() {
        let engine = Engine::new();
        let mut doc = engine.open(bytes.to_vec()).unwrap();
        let receipt = edit(
            &mut doc,
            json!([{"op":"createHeaderFooter","section":0,"kind":"header","side":"odd"}]),
        );
        let caret = receipt["caret"].clone();
        assert_eq!(caret["steps"][0]["index"], 0);
        assert_locatable(&mut doc, &caret);
        edit(
            &mut doc,
            json!([{"op":"insertText","at":caret,"text":"문서 머리말"}]),
        );
        let header = created(&mut doc, "header");
        assert_eq!(header["side"], "odd");
        let list = header["lists"][0].clone();
        assert_eq!(
            query(&mut doc, json!({"kind":"paragraphs","list":list}))[0]["text"],
            "문서 머리말"
        );
        let offset = start(&mut doc);
        let receipt = edit(
            &mut doc,
            json!([{"op":"insertNote","at":at(offset),"kind":"footnote"}]),
        );
        // Hancom leaves the caret after the note's number and a space.
        assert_eq!(receipt["caret"]["offset"], 9);
        assert_locatable(&mut doc, &receipt["caret"]);
        edit(
            &mut doc,
            json!([{"op":"insertText","at":receipt["caret"],"text":"각주 내용"}]),
        );
        let note = created(&mut doc, "footnote");
        let list = note["lists"][0].clone();
        assert_eq!(
            query(&mut doc, json!({"kind":"paragraphs","list":list}))[0]["text"],
            " 각주 내용"
        );
        let saved = doc.serialize(false).unwrap();
        let mut reopened = engine.open(saved).unwrap();
        let note = created(&mut reopened, "footnote");
        assert_eq!(
            query(
                &mut reopened,
                json!({"kind":"paragraphs","list":note["lists"][0]})
            )[0]["text"],
            " 각주 내용"
        );
        edit(
            &mut reopened,
            json!([{"op":"deleteControl","target":note["target"]}]),
        );
        assert!(
            !controls(&mut reopened)
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c["target"] == note["target"] && c["kind"] == "footnote")
        );
        reopened.undo().unwrap();
        assert_eq!(created(&mut reopened, "footnote")["number"], note["number"]);
    }
}

#[test]
fn fields_wrap_formatted_text_keep_identity_and_remove_only_delimiters() {
    for bytes in sources() {
        let engine = Engine::new();
        let mut doc = engine.open(bytes.to_vec()).unwrap();
        let offset = start(&mut doc);
        edit(
            &mut doc,
            json!([{"op":"insertText","at":at(offset),"text":"가나다라","format":{"bold":true}}]),
        );
        let before = query(
            &mut doc,
            json!({"kind":"formatRuns","list":{"section":0},"count":1}),
        );
        edit(
            &mut doc,
            json!([{"op":"insertField","at":at(offset),"to":at(offset+4),"kind":"hyperlink","command":"https://example.com","name":"링크"}]),
        );
        let field = created(&mut doc, "field");
        assert_eq!(field["value"], "가나다라");
        assert_eq!(field["end"]["offset"], offset + 12);
        edit(
            &mut doc,
            json!([{"op":"updateField","target":field["target"],"patch":{"value":"새 링크","command":"https://example.org"}}]),
        );
        let updated = created(&mut doc, "field");
        assert_eq!(updated["fieldId"], field["fieldId"]);
        assert_eq!(updated["value"], "새 링크");
        let saved = doc.serialize(false).unwrap();
        let mut reopened = engine.open(saved).unwrap();
        let back = created(&mut reopened, "field");
        assert_eq!(back["value"], "새 링크");
        assert_eq!(back["name"], "링크");
        assert_eq!(back["command"], "https://example.org");
        doc.undo().unwrap();
        assert_eq!(created(&mut doc, "field")["value"], "가나다라");
        edit(
            &mut doc,
            json!([{"op":"deleteControl","target":field["target"]}]),
        );
        assert_eq!(
            query(
                &mut doc,
                json!({"kind":"formatRuns","list":{"section":0},"count":1})
            ),
            before
        );
        let p = query(
            &mut doc,
            json!({"kind":"paragraphs","list":{"section":0},"count":1}),
        );
        assert!(p[0]["text"].as_str().unwrap().ends_with("가나다라"));
    }
}

#[test]
fn failed_batch_restores_clean_bytes_and_page_hide_zero_removes_control() {
    for bytes in sources() {
        let engine = Engine::new();
        let mut doc = engine.open(bytes.to_vec()).unwrap();
        let offset = start(&mut doc);
        let before = doc.state().unwrap();
        assert!(doc.transact(&json!({"ops":[{"op":"setDocumentProperties","patch":{"pageStart":17}},{"op":"insertNote","at":at(offset),"kind":"endnote"},{"op":"setPageSettings","section":0,"patch":{"width":0}}]}).to_string()).is_err());
        assert_eq!(doc.state().unwrap(), before);
        assert_eq!(doc.serialize(false).unwrap(), bytes);
        edit(
            &mut doc,
            json!([{"op":"setPageHide","at":at(offset),"flags":3}]),
        );
        assert!(
            controls(&mut doc)
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c["flags"] == 3)
        );
        edit(
            &mut doc,
            json!([{"op":"setPageHide","at":at(offset),"flags":0}]),
        );
        assert!(
            !controls(&mut doc)
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c.get("flags").is_some())
        );
        doc.undo().unwrap();
        assert!(
            controls(&mut doc)
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c["flags"] == 3)
        );
    }
}

#[test]
fn columns_bookmarks_and_page_number_controls_roundtrip() {
    for bytes in sources() {
        let engine = Engine::new();
        let mut doc = engine.open(bytes.to_vec()).unwrap();
        let offset = start(&mut doc);
        edit(
            &mut doc,
            json!([
                {"op":"setColumns","at":at(offset),"patch":{"count":2,"equalWidths":true,"gap":900}},
                {"op":"insertNewNumber","at":at(offset),"number":12},
                {"op":"insertBookmark","at":at(offset),"name":"책갈피"}
            ]),
        );
        let bookmark = created(&mut doc, "bookmark");
        edit(
            &mut doc,
            json!([{"op":"renameBookmark","target":bookmark["target"],"name":"새 이름"}]),
        );
        let receipt = edit(
            &mut doc,
            json!([{"op":"createHeaderFooter","section":0,"kind":"footer","side":"both"}]),
        );
        edit(
            &mut doc,
            json!([{"op":"insertField","at":receipt["caret"],"kind":"pageNumber"}]),
        );
        let footer = created(&mut doc, "footer");
        assert!(
            query(
                &mut doc,
                json!({"kind":"controls","list":footer["lists"][0]})
            )
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["fieldKind"] == "pageNumber")
        );
        let saved = doc.serialize(false).unwrap();
        let mut reopened = engine.open(saved).unwrap();
        assert_eq!(
            query(&mut reopened, json!({"kind":"sectionSettings","section":0}))["columns"]["count"],
            2
        );
        assert_eq!(created(&mut reopened, "bookmark")["name"], "새 이름");
        let offset = start(&mut reopened);
        let receipt = edit(&mut reopened, json!([{"op":"columnBreak","at":at(offset)}]));
        assert_eq!(receipt["caret"]["para"], 1);
        assert_eq!(receipt["caret"]["offset"], 0);
    }
}

#[test]
fn page_border_lines_and_fill_resolve_to_a_border_fill_that_roundtrips() {
    for bytes in sources() {
        let engine = Engine::new();
        let mut doc = engine.open(bytes.to_vec()).unwrap();
        let line =
            |kind: u8, width: u8, color: u32| json!({"kind":kind,"width":width,"color":color});
        let lines = json!([
            line(1, 3, 0x0000ff),
            line(1, 3, 0x0000ff),
            line(3, 10, 0x00ff00),
            line(8, 0, 0)
        ]);
        edit(
            &mut doc,
            json!([{"op":"setPageBorder","section":0,"side":"both","patch":{
                "attr":1,"offsets":[1417,1417,1417,1417],"lines":lines,
                "fill":{"kind":"solid","color":0x00e0f0ff_u32,"patternColor":0,"pattern":0}}}]),
        );
        let after = query(&mut doc, json!({"kind":"sectionSettings","section":0}));
        let border = &after["pageBorders"][0];
        assert_eq!(border["lines"], lines);
        assert_eq!(border["fill"]["kind"], "solid");
        assert_eq!(border["fill"]["color"], 0x00e0f0ff_u32);
        assert_eq!(border["fill"]["pattern"], -1);
        assert_eq!(border["attr"], 1);
        assert!(border["borderFill"].as_u64().unwrap() > 0);

        // Lines alone keep the fill; the same request reuses the entry.
        let lines2 = json!([line(0, 0, 0), line(0, 0, 0), line(1, 0, 0), line(1, 0, 0)]);
        let set =
            json!([{"op":"setPageBorder","section":0,"side":"both","patch":{"lines":lines2}}]);
        edit(&mut doc, set.clone());
        let second = query(&mut doc, json!({"kind":"sectionSettings","section":0}))["pageBorders"]
            [0]
        .clone();
        assert_eq!(second["lines"], lines2);
        assert_eq!(second["fill"], border["fill"]);
        edit(
            &mut doc,
            json!([{"op":"setPageBorder","section":0,"side":"both","patch":{"fill":{"kind":"none"}}}]),
        );
        edit(
            &mut doc,
            json!([{"op":"setPageBorder","section":0,"side":"both","patch":{"fill":border["fill"]}}]),
        );
        let again = query(&mut doc, json!({"kind":"sectionSettings","section":0}))["pageBorders"]
            [0]
        .clone();
        assert_eq!(
            again["borderFill"], second["borderFill"],
            "equal border fills are shared"
        );

        let saved = doc.serialize(false).unwrap();
        let mut reopened = engine.open(saved).unwrap();
        let reread =
            query(&mut reopened, json!({"kind":"sectionSettings","section":0}))["pageBorders"][0]
                .clone();
        assert_eq!(reread["lines"], lines2);
        assert_eq!(reread["fill"], border["fill"]);
        assert_eq!(reread["offsets"], json!([1417, 1417, 1417, 1417]));

        doc.undo().unwrap();
        doc.undo().unwrap();
        doc.undo().unwrap();
        let undone = query(&mut doc, json!({"kind":"sectionSettings","section":0}))["pageBorders"]
            [0]
        .clone();
        assert_eq!(undone["lines"], lines);
    }
}

#[test]
fn notes_number_in_document_order_through_insert_delete_undo_and_save() {
    fn numbers(doc: &mut Doc) -> Vec<(u64, u64)> {
        controls(doc)
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["kind"] == "footnote")
            .map(|c| (c["offset"].as_u64().unwrap(), c["number"].as_u64().unwrap()))
            .collect()
    }
    fn marks(doc: &mut Doc) -> Vec<u64> {
        controls(doc)
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["kind"] == "autoNumber")
            .map(|c| c["number"].as_u64().unwrap())
            .collect()
    }
    for bytes in sources() {
        let engine = Engine::new();
        let mut doc = engine.open(bytes.to_vec()).unwrap();
        let end = start(&mut doc);
        let lead = query(
            &mut doc,
            json!({"kind":"paragraphs","list":{"section":0},"count":1}),
        )[0]["caret"][0]
            .as_u64()
            .unwrap() as u32;
        edit(
            &mut doc,
            json!([{"op":"insertNote","at":at(end),"kind":"footnote"}]),
        );
        let base = numbers(&mut doc)[0].1;
        // A note before the first one takes its number and pushes it on.
        edit(
            &mut doc,
            json!([{"op":"insertNote","at":at(lead),"kind":"footnote"}]),
        );
        let both = numbers(&mut doc);
        assert_eq!(
            both.iter().map(|n| n.1).collect::<Vec<_>>(),
            [base, base + 1]
        );
        assert!(both[0].0 < both[1].0);
        assert_eq!(
            marks(&mut doc),
            [base, base + 1],
            "each note's own number follows"
        );
        let mut reopened = engine.open(doc.serialize(false).unwrap()).unwrap();
        assert_eq!(
            numbers(&mut reopened)
                .iter()
                .map(|n| n.1)
                .collect::<Vec<_>>(),
            [base, base + 1]
        );
        assert_eq!(marks(&mut reopened), [base, base + 1]);
        // Removing the first note renumbers the rest; undo puts both back.
        let first = controls(&mut reopened)
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["kind"] == "footnote")
            .unwrap()["target"]
            .clone();
        edit(
            &mut reopened,
            json!([{"op":"deleteControl","target":first}]),
        );
        assert_eq!(
            numbers(&mut reopened)
                .iter()
                .map(|n| n.1)
                .collect::<Vec<_>>(),
            [base]
        );
        reopened.undo().unwrap();
        assert_eq!(
            numbers(&mut reopened)
                .iter()
                .map(|n| n.1)
                .collect::<Vec<_>>(),
            [base, base + 1]
        );
        doc.undo().unwrap();
        assert_eq!(
            numbers(&mut doc).iter().map(|n| n.1).collect::<Vec<_>>(),
            [base]
        );
    }
}

#[test]
fn deleting_one_marker_of_a_field_removes_the_whole_field_and_keeps_its_text() {
    // Hancom drops the field whose begin or end a deletion takes and keeps the text it held.
    for bytes in sources() {
        // (range, text left, text after typing X over the range, caret after typing)
        for (cut, expected, typed, caret) in [
            ((17, 29), "Hello wornd more", "Hello worXnd more", 10),
            ((4, 16), "Hellrld and more", "HellXrld and more", 5),
        ] {
            let engine = Engine::new();
            let mut doc = engine.open(bytes.to_vec()).unwrap();
            let offset = start(&mut doc);
            let text = |doc: &mut Doc| {
                let p = query(
                    doc,
                    json!({"kind":"paragraphs","list":{"section":0},"count":1}),
                );
                p[0]["text"].as_str().unwrap().to_string()
            };
            let fields = |doc: &mut Doc| {
                controls(doc)
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|c| c["kind"] == "field")
                    .count()
            };
            edit(
                &mut doc,
                json!([{"op":"insertText","at":at(offset),"text":"Hello world and more"}]),
            );
            edit(
                &mut doc,
                json!([{"op":"insertField","at":at(offset+6),"to":at(offset+11),"kind":"hyperlink","command":"https://example.com"}]),
            );
            let before = text(&mut doc);
            assert_eq!(fields(&mut doc), 1);
            edit(
                &mut doc,
                json!([{"op":"delete","from":at(offset+cut.0),"to":at(offset+cut.1)}]),
            );
            assert!(text(&mut doc).ends_with(expected), "{}", text(&mut doc));
            assert_eq!(fields(&mut doc), 0);
            let saved = doc.serialize(false).unwrap();
            let mut reopened = engine.open(saved).unwrap();
            assert!(text(&mut reopened).ends_with(expected));
            assert_eq!(fields(&mut reopened), 0);
            doc.undo().unwrap();
            assert_eq!(text(&mut doc), before);
            assert_eq!(fields(&mut doc), 1);
            // Typing over the same selection: the insertion address is the one the caller
            // computed before the deletion, and the caret lands after the typed text.
            let receipt = edit(
                &mut doc,
                json!([
                    {"op":"delete","from":at(offset+cut.0),"to":at(offset+cut.1)},
                    {"op":"insertText","at":at(offset+cut.0),"text":"X"}
                ]),
            );
            assert!(text(&mut doc).ends_with(typed), "{}", text(&mut doc));
            assert_eq!(fields(&mut doc), 0);
            assert_eq!(receipt["caret"]["offset"], offset + caret, "{receipt}");
            // Pasting over it does the same.
            doc.undo().unwrap();
            let pasted: Value = serde_json::from_str(
                &doc.paste(
                    &json!({"replace":{"from":at(offset+cut.0),"to":at(offset+cut.1)},"text":"X"})
                        .to_string(),
                )
                .unwrap(),
            )
            .unwrap();
            assert!(text(&mut doc).ends_with(typed), "{}", text(&mut doc));
            assert_eq!(fields(&mut doc), 0);
            assert_eq!(pasted["caret"]["offset"], offset + caret, "{pasted}");
            doc.undo().unwrap();
            assert_eq!(text(&mut doc), before);
        }
    }
}

/// A click-here command with `guide` as its guide text, as Hancom writes it.
fn click_here(guide: &str) -> String {
    let inner = format!(
        "Direction:wstring:{}:{guide} HelpState:wstring:0: ",
        guide.chars().count()
    );
    format!("Clickhere:set:{}:{inner} ", inner.chars().count())
}

#[test]
fn unfilled_click_here_guides_show_on_screen_not_in_print_and_leave_with_the_field() {
    // Hancom's editing screen draws an unfilled 누름틀's guide in the room it takes; its print
    // leaves the guide out and closes up the line ("Hello  tail"). A guide too long for one page
    // therefore gives the screen more pages than print.
    let guide = "W".repeat(9000);
    // An A4 page with ordinary margins, in both formats.
    let a4 = Engine::new()
        .create(include_bytes!("fixtures/last-page-master.hwpx").to_vec())
        .unwrap()
        .serialize_as("hwp5", false)
        .unwrap();
    for bytes in [&a4[..], include_bytes!("fixtures/last-page-master.hwpx")] {
        let engine = Engine::new();
        let mut doc = engine.open(bytes.to_vec()).unwrap();
        let offset = start(&mut doc);
        let text = |doc: &mut Doc| {
            let p = query(
                doc,
                json!({"kind":"paragraphs","list":{"section":0},"count":1}),
            );
            p[0]["text"].as_str().unwrap().to_string()
        };
        let field = |doc: &mut Doc| {
            let f = query(doc, json!({"kind":"fields"}));
            f.as_array().unwrap().first().cloned()
        };
        let pages = |doc: &mut Doc| {
            let screen: Value = serde_json::from_str(&doc.pages().unwrap()).unwrap();
            (
                screen.as_array().unwrap().len(),
                doc.print_page_count().unwrap(),
            )
        };
        let one_page = pages(&mut doc);
        edit(
            &mut doc,
            json!([{"op":"insertText","at":at(offset),"text":"Hello  tail"}]),
        );
        edit(
            &mut doc,
            json!([{"op":"insertField","at":at(offset+6),"kind":"clickHere","command":click_here(&guide),"text":guide}]),
        );
        let f = field(&mut doc).unwrap();
        assert_eq!(f["dirty"], false);
        assert_eq!(f["value"], guide);
        let (screen, print) = pages(&mut doc);
        assert!(screen > one_page.0, "the guide takes room on screen");
        assert_eq!(print, one_page.1, "print leaves the guide out");
        assert!(doc.print_page_svg(print, 96.0).is_err());
        assert!(doc.pdf(0, usize::MAX, 96.0).is_ok());
        for format in ["hwp5", "hwpx"] {
            let mut back = engine
                .open(doc.serialize_as(format, false).unwrap())
                .unwrap();
            let f = field(&mut back).unwrap();
            assert_eq!(
                (f["dirty"].clone(), f["value"].clone()),
                (json!(false), json!(guide))
            );
            assert_eq!(pages(&mut back), (screen, print), "{format}");
        }
        // Changing what the field is called keeps it unfilled; rewriting the guide with dirty
        // false (누름틀 고치기 of an unfilled field) too.
        edit(
            &mut doc,
            json!([{"op":"updateField","target":f["target"],"patch":{"name":"이름","editable":true}}]),
        );
        assert_eq!(field(&mut doc).unwrap()["dirty"], false);
        edit(
            &mut doc,
            json!([{"op":"updateField","target":f["target"],"patch":{"command":click_here("NEW"),"value":"NEW","dirty":false}}]),
        );
        let f = field(&mut doc).unwrap();
        assert_eq!(
            (f["dirty"].clone(), f["guide"].clone()),
            (json!(false), json!("NEW"))
        );
        // HWPX spells the new guide out next to the command, as Hancom writes it.
        let xml = section_xml(doc.serialize_as("hwpx", false).unwrap());
        assert!(xml.contains(r#"<hp:stringParam name="Direction">NEW</hp:stringParam>"#));
        assert!(!xml.contains("WWWW"));
        // 누름틀 지우기 of an unfilled field takes its guide with it, in one undo step.
        edit(
            &mut doc,
            json!([{"op":"deleteControl","target":f["target"]}]),
        );
        assert!(
            text(&mut doc).ends_with("Hello  tail"),
            "{}",
            text(&mut doc)
        );
        assert!(field(&mut doc).is_none());
        doc.undo().unwrap();
        assert_eq!(field(&mut doc).unwrap()["value"], "NEW");
        // Typing into it marks it filled: its text then shows everywhere and stays when the
        // field goes.
        edit(
            &mut doc,
            json!([{"op":"updateField","target":f["target"],"patch":{"value":"abc"}}]),
        );
        let f = field(&mut doc).unwrap();
        assert_eq!(
            (f["dirty"].clone(), f["value"].clone()),
            (json!(true), json!("abc"))
        );
        edit(
            &mut doc,
            json!([{"op":"deleteControl","target":f["target"]}]),
        );
        assert!(
            text(&mut doc).ends_with("Hello abc tail"),
            "{}",
            text(&mut doc)
        );
    }
}

#[test]
fn new_headers_follow_creation_order_start_on_the_caret_page_and_take_the_header_style() {
    let engine = Engine::new();
    let open = || {
        engine
            .open(include_bytes!("fixtures/hancom-notes-twopage.hwpx").to_vec())
            .unwrap()
    };
    let pages = |doc: &mut Doc| -> Value { serde_json::from_str(&doc.pages().unwrap()).unwrap() };
    let locate = |doc: &mut Doc, page: usize, x: i64, y: i64, body: bool| -> Value {
        let l: Value = serde_json::from_str(
            &doc.locate(&json!({"kind":"point","page":page,"x":x,"y":y,"body":body}).to_string())
                .unwrap(),
        )
        .unwrap();
        l["position"].clone()
    };
    // The header line a page draws, by the list a point in its header area finds.
    let header_on = |doc: &mut Doc, page: usize| -> Option<Value> {
        let p = pages(doc)[page].clone();
        let a = &p["header"];
        let x = a["x"].as_i64().unwrap() + 200;
        let y = a["y"].as_i64().unwrap() + 200;
        let at = locate(doc, page, x, y, false);
        at["steps"]
            .as_array()
            .filter(|s| !s.is_empty())
            .map(|s| s[0].clone())
    };
    let style = |doc: &mut Doc, list: &Value| {
        let runs = query(doc, json!({"kind":"formatRuns","list":list,"count":1}));
        let p = runs.as_array().unwrap()[0].clone();
        (
            p["styleId"].clone(),
            p["paraShapeId"].clone(),
            p["runs"][0]["charShapeId"].clone(),
        )
    };
    let mut doc = open();
    let styles = query(&mut doc, json!({"kind":"styles"}));
    let header_style = styles
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "머리말")
        .unwrap()
        .clone();
    // Hancom: a 양쪽 header, then a 짝수 쪽 one: both kept in that order, page 2 shows the
    // even one.
    for (side, text) in [("both", "A"), ("even", "B")] {
        let r = edit(
            &mut doc,
            json!([{"op":"createHeaderFooter","section":0,"kind":"header","side":side}]),
        );
        edit(
            &mut doc,
            json!([{"op":"insertText","at":r["caret"],"text":text}]),
        );
    }
    let headers = |doc: &mut Doc| -> Vec<Value> {
        controls(doc)
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["kind"] == "header")
            .cloned()
            .collect()
    };
    let made = headers(&mut doc);
    assert_eq!(
        made.iter().map(|h| h["side"].clone()).collect::<Vec<_>>(),
        [json!("both"), json!("even")]
    );
    let even = made[1]["target"].clone();
    for format in ["live", "hwp5", "hwpx"] {
        let mut d = match format {
            "live" => engine.open(doc.serialize(false).unwrap()).unwrap(),
            f => engine.open(doc.serialize_as(f, false).unwrap()).unwrap(),
        };
        let shown = header_on(&mut d, 1).unwrap();
        assert_eq!(
            (shown["para"].clone(), shown["control"].clone()),
            (even["para"].clone(), even["control"].clone()),
            "{format}"
        );
        let list = headers(&mut d)[1]["lists"][0].clone();
        assert_eq!(
            style(&mut d, &list),
            (
                header_style["id"].clone(),
                header_style["paraShape"].clone(),
                header_style["charShape"].clone()
            ),
            "{format}"
        );
    }
    // A header made from the caret on page 2 starts there; page 1 has none.
    let mut doc = open();
    let p = pages(&mut doc)[1].clone();
    let body = &p["body"];
    let caret = locate(
        &mut doc,
        1,
        body["x"].as_i64().unwrap() + 200,
        body["y"].as_i64().unwrap() + 200,
        true,
    );
    let r = edit(
        &mut doc,
        json!([{"op":"createHeaderFooter","section":0,"kind":"header","side":"both","at":caret}]),
    );
    edit(
        &mut doc,
        json!([{"op":"insertText","at":r["caret"],"text":"HDR-P2"}]),
    );
    assert_eq!(headers(&mut doc)[0]["target"]["para"], caret["para"]);
    assert!(header_on(&mut doc, 0).is_none());
    assert!(header_on(&mut doc, 1).is_some());
    // A new footer sits at the bottom of its area, as Hancom aligns it.
    let r = edit(
        &mut doc,
        json!([{"op":"createHeaderFooter","section":0,"kind":"footer","side":"both"}]),
    );
    edit(
        &mut doc,
        json!([{"op":"insertText","at":r["caret"],"text":"F"}]),
    );
    let located: Value = serde_json::from_str(
        &doc.locate(&json!({"kind":"caret","position":r["caret"]}).to_string())
            .unwrap(),
    )
    .unwrap();
    let footer = &pages(&mut doc)[0]["footer"];
    let bottom = footer["y"].as_i64().unwrap() + footer["height"].as_i64().unwrap();
    let caret_bottom = located["y"].as_i64().unwrap() + located["height"].as_i64().unwrap();
    assert!(
        bottom - caret_bottom < footer["height"].as_i64().unwrap() / 2,
        "{located} {footer}"
    );
}

/// Section 0's markup in a saved HWPX package.
fn section_xml(bytes: Vec<u8>) -> String {
    let zip = hm_hwpx::zip::Archive::open(hm_model::SharedBytes::from(bytes)).unwrap();
    let xml = zip.read_name("Contents/section0.xml").unwrap().unwrap();
    String::from_utf8(xml.to_vec()).unwrap()
}

#[test]
fn formula_fields_store_hancom_commands_and_keep_their_last_result() {
    // Hancom's 세로 합계 in B4: a FORMULA field, command `=SUM(ABOVE)??%g,;;60`, Prop 8, around
    // the result 60, with Formula, ResultFormat and LastResult spelled out in HWPX.
    let engine = Engine::new();
    let mut doc = engine
        .open(include_bytes!("fixtures/hancom-table-4x4.hwpx").to_vec())
        .unwrap();
    let table = controls(&mut doc)
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["kind"] == "table")
        .unwrap()["target"]
        .clone();
    let cell = json!({"section":0,"steps":[{"para":table["para"],"control":table["control"],"index":13}],"para":0,"offset":0});
    edit(
        &mut doc,
        json!([{"op":"insertField","at":cell,"kind":"formula","command":"=SUM(ABOVE)","text":"60"}]),
    );
    let field = |doc: &mut Doc| query(doc, json!({"kind":"fields"}))[0].clone();
    let f = field(&mut doc);
    assert_eq!(
        (&f["fieldKind"], &f["command"], &f["value"], &f["dirty"]),
        (
            &json!("formula"),
            &json!("=SUM(ABOVE)??%g,;;60"),
            &json!("60"),
            &json!(false)
        )
    );
    assert_eq!(
        (&f["formula"], &f["resultFormat"]),
        (&json!("=SUM(ABOVE)"), &json!("%g,"))
    );
    let xml = section_xml(doc.serialize_as("hwpx", false).unwrap());
    for part in [
        r#"type="FORMULA""#,
        r#"<hp:integerParam name="Prop">8</hp:integerParam>"#,
        r#"<hp:stringParam name="Formula">=SUM(ABOVE)</hp:stringParam>"#,
        r#"<hp:stringParam name="ResultFormat">%g,</hp:stringParam>"#,
        r#"<hp:stringParam name="LastResult">60</hp:stringParam>"#,
    ] {
        assert!(xml.contains(part), "{part}");
    }
    // Recalculating rewrites the stored result with the shown one, also after a reopen.
    for format in ["live", "hwp5", "hwpx"] {
        let mut d = match format {
            "live" => engine.open(doc.serialize(false).unwrap()).unwrap(),
            f => engine.open(doc.serialize_as(f, false).unwrap()).unwrap(),
        };
        let f = field(&mut d);
        assert_eq!(f["command"], "=SUM(ABOVE)??%g,;;60", "{format}");
        edit(
            &mut d,
            json!([{"op":"updateField","target":f["target"],"patch":{"value":"70","dirty":false}}]),
        );
        let f = field(&mut d);
        assert_eq!(
            (&f["command"], &f["value"], &f["dirty"]),
            (&json!("=SUM(ABOVE)??%g,;;70"), &json!("70"), &json!(false)),
            "{format}"
        );
        let xml = section_xml(d.serialize_as("hwpx", false).unwrap());
        assert!(xml.contains(r#"<hp:stringParam name="LastResult">70</hp:stringParam>"#));
        assert!(!xml.contains(">60<"), "{format}");
        d.undo().unwrap();
        assert_eq!(field(&mut d)["value"], "60");
    }
    assert!(
        doc.transact(
            &json!({"ops":[{"op":"insertField","at":cell,"kind":"formula","text":"1"}]})
                .to_string()
        )
        .is_err()
    );
}
