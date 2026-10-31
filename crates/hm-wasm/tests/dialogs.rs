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

#[test]
fn copied_fields_paste_as_fields_of_their_own_and_cut_ones_leave_their_text() {
    // Hancom pastes a field the copied range holds whole as a new field with the same command,
    // under a new id, also after a cut; a field the range cuts in two and a bookmark, point or
    // block, arrive as text only (clipboard c10, fixes/clipboard-03).
    for bytes in sources() {
        let engine = Engine::new();
        let mut doc = engine.open(bytes.to_vec()).unwrap();
        let offset = start(&mut doc);
        let fields = |doc: &mut Doc| {
            let mut out: Vec<Value> = query(doc, json!({"kind":"fields"}))
                .as_array()
                .unwrap()
                .iter()
                .filter(|r| r["kind"] == "field")
                .cloned()
                .collect();
            out.sort_by_key(|r| r["offset"].as_u64());
            out
        };
        let text = |doc: &mut Doc| {
            let p = query(
                doc,
                json!({"kind":"paragraphs","list":{"section":0},"count":1}),
            );
            (
                p[0]["text"].as_str().unwrap().to_string(),
                p[0]["length"].as_u64().unwrap() as u32 - 1,
            )
        };
        edit(
            &mut doc,
            json!([{"op":"insertText","at":at(offset),"text":"Hello world and more"}]),
        );
        // 'Hello ' [begin] 'world' [end] ' and more', a block bookmark over 'more' and a
        // point bookmark at the end.
        edit(
            &mut doc,
            json!([{"op":"insertField","at":at(offset+6),"to":at(offset+11),"kind":"hyperlink","command":r"https\://example.com/a\?b=1;1;0;0;"}]),
        );
        edit(
            &mut doc,
            json!([{"op":"insertField","at":at(offset+32),"to":at(offset+36),"kind":"bookmark","name":"blk"}]),
        );
        let (_, end) = text(&mut doc);
        edit(
            &mut doc,
            json!([{"op":"insertBookmark","at":at(end),"name":"bm1"}]),
        );
        let (before, end) = text(&mut doc);
        assert_eq!(fields(&mut doc).len(), 2);
        let copied: Value = serde_json::from_str(
            &doc.copy(&json!({"from":at(offset),"to":at(end)}).to_string())
                .unwrap(),
        )
        .unwrap();
        let html = copied["html"].as_str().unwrap();
        assert!(
            html.contains("<a href=\"https://example.com/a?b=1\">"),
            "{html}"
        );
        assert_eq!(html.matches("</a>").count(), 1, "{html}");
        assert_eq!(copied["objects"], 0);
        let pasted: Value =
            serde_json::from_str(&doc.paste(&json!({"at":at(end)}).to_string()).unwrap()).unwrap();
        let (after, last) = text(&mut doc);
        assert!(after.ends_with("Hello world and moreHello world and more"));
        assert_eq!(pasted["caret"]["offset"], last, "{pasted}");
        let check = |doc: &mut Doc| {
            let f = fields(doc);
            let links: Vec<_> = f.iter().filter(|r| r["fieldKind"] == "hyperlink").collect();
            assert_eq!(links.len(), 2, "{f:?}");
            assert_ne!(links[0]["fieldId"], links[1]["fieldId"]);
            for l in &links {
                assert_eq!(l["value"], "world");
                assert_eq!(l["command"], r"https\://example.com/a\?b=1;1;0;0;");
            }
            // The block bookmark and the point bookmark stay single.
            assert_eq!(f.iter().filter(|r| r["fieldKind"] == "bookmark").count(), 1);
            let marks = query(doc, json!({"kind":"fields"}));
            let points = marks
                .as_array()
                .unwrap()
                .iter()
                .filter(|r| r["kind"] == "bookmark");
            assert_eq!(points.count(), 1, "{marks}");
        };
        check(&mut doc);
        for format in ["hwp5", "hwpx"] {
            let mut reopened = engine
                .open(doc.serialize_as(format, false).unwrap())
                .unwrap();
            check(&mut reopened);
        }
        doc.undo().unwrap();
        assert_eq!(text(&mut doc).0, before);
        assert_eq!(fields(&mut doc).len(), 2);
        // A copy holding only the link's start pastes plain text.
        doc.copy(&json!({"from":at(offset),"to":at(offset+16)}).to_string())
            .unwrap();
        doc.paste(&json!({"at":at(end)}).to_string()).unwrap();
        assert!(text(&mut doc).0.ends_with("Hello world and moreHello wo"));
        assert_eq!(fields(&mut doc).len(), 2);
        doc.undo().unwrap();
        // Cut and pasted, the link survives.
        doc.copy(&json!({"from":at(offset+6),"to":at(offset+27)}).to_string())
            .unwrap();
        edit(
            &mut doc,
            json!([{"op":"delete","from":at(offset+6),"to":at(offset+27)}]),
        );
        assert_eq!(fields(&mut doc).len(), 1);
        doc.paste(&json!({"at":at(offset)}).to_string()).unwrap();
        let f = fields(&mut doc);
        assert_eq!(f.len(), 2);
        assert!(
            f.iter()
                .any(|r| r["fieldKind"] == "hyperlink" && r["value"] == "world")
        );
        assert!(text(&mut doc).0.ends_with("worldHello  and more"));
    }
}

#[test]
fn fields_inside_a_pasted_table_get_ids_of_their_own() {
    // A field pasted inside a copied table, nested or over a table's cells, is a field of its
    // own like one pasted in the text: it takes a new id and its end follows it, so no two
    // fields of the document share an id after saving and reopening.
    for bytes in sources() {
        let engine = Engine::new();
        let mut doc = engine.open(bytes.to_vec()).unwrap();
        let offset = start(&mut doc);
        edit(
            &mut doc,
            json!([{"op":"createTable","at":at(offset),"rows":1,"columns":2}]),
        );
        let table = created(&mut doc, "table");
        let caret = |list: &Value, offset: u32| json!({"section":list["section"],"steps":list["steps"],"para":0,"offset":offset});
        let (a1, b1) = (table["lists"][0].clone(), table["lists"][1].clone());
        edit(
            &mut doc,
            json!([{"op":"insertText","at":caret(&a1,0),"text":"link"}]),
        );
        edit(
            &mut doc,
            json!([{"op":"insertField","at":caret(&a1,0),"to":caret(&a1,4),"kind":"hyperlink","command":"https://example.com"}]),
        );
        let links = |doc: &mut Doc| {
            let mut ids: Vec<u64> = query(doc, json!({"kind":"fields"}))
                .as_array()
                .unwrap()
                .iter()
                .filter(|r| r["fieldKind"] == "hyperlink")
                .map(|r| {
                    assert_eq!(r["value"], "link", "{r}");
                    r["fieldId"].as_u64().unwrap()
                })
                .collect();
            ids.sort();
            ids.dedup();
            ids.len()
        };
        let all = |doc: &mut Doc| {
            query(doc, json!({"kind":"fields"}))
                .as_array()
                .unwrap()
                .iter()
                .filter(|r| r["fieldKind"] == "hyperlink")
                .count()
        };
        assert_eq!((links(&mut doc), all(&mut doc)), (1, 1));
        let held = table["offset"].as_u64().unwrap() as u32;
        doc.copy(&json!({"from":at(held),"to":at(held + 8)}).to_string())
            .unwrap();
        // Pasted in the text the table nests nothing and its link is a new field.
        let end = start(&mut doc);
        doc.paste(&json!({"at":at(end)}).to_string()).unwrap();
        assert_eq!((links(&mut doc), all(&mut doc)), (2, 2));
        // Pasted with the caret in B1 the copied A1 overwrites B1, link and all.
        doc.paste(&json!({"at":caret(&b1,0)}).to_string()).unwrap();
        assert_eq!((links(&mut doc), all(&mut doc)), (3, 3));
        for format in ["hwp5", "hwpx", "hml"] {
            let mut reopened = engine
                .open(doc.serialize_as(format, false).unwrap())
                .unwrap();
            assert_eq!(
                (links(&mut reopened), all(&mut reopened)),
                (3, 3),
                "{format}"
            );
        }
        doc.undo().unwrap();
        assert_eq!((links(&mut doc), all(&mut doc)), (2, 2));
        doc.redo().unwrap();
        assert_eq!((links(&mut doc), all(&mut doc)), (3, 3));
    }
}
