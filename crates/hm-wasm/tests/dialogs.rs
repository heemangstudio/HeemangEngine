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
        assert_eq!(receipt["caret"]["offset"], 8);
        assert_locatable(&mut doc, &receipt["caret"]);
        edit(
            &mut doc,
            json!([{"op":"insertText","at":receipt["caret"],"text":"각주 내용"}]),
        );
        let note = created(&mut doc, "footnote");
        let list = note["lists"][0].clone();
        assert_eq!(
            query(&mut doc, json!({"kind":"paragraphs","list":list}))[0]["text"],
            "각주 내용"
        );
        let saved = doc.serialize(false).unwrap();
        let mut reopened = engine.open(saved).unwrap();
        let note = created(&mut reopened, "footnote");
        assert_eq!(
            query(
                &mut reopened,
                json!({"kind":"paragraphs","list":note["lists"][0]})
            )[0]["text"],
            "각주 내용"
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
