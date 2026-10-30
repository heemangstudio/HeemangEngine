//! Footnotes and endnotes behave as in Hancom Office 2022: inserting,
//! numbering, note shapes, deleting, the clipboard and undo, compared with
//! files Hancom saved (`fixtures/hancom-note*`).
//!
//! `hancom-notes.*` is one Hancom document saved as HWP, HWPX and HWPML:
//! P0 "The quick[fn A] brown[en A] fox jumps over the lazy dog.[fn B]"
//! (text from offset 16), P1 a 2x2 table, P2 "Tail paragraph one.[fn C]",
//! P3 "Tail paragraph two.[en B]". `hancom-note-shapes.*` is the same
//! document after Hancom set every footnote and endnote shape item.
use hm_hwpx::zip::Archive;
use hm_wasm::{Doc, Engine};
use serde_json::{Value, json};

const NOTES: [(&str, &[u8]); 3] = [
    ("hwp5", include_bytes!("fixtures/hancom-notes.hwp")),
    ("hwpx", include_bytes!("fixtures/hancom-notes.hwpx")),
    ("hml", include_bytes!("fixtures/hancom-notes.hml")),
];
const SHAPES: [(&str, &[u8]); 3] = [
    ("hwp5", include_bytes!("fixtures/hancom-note-shapes.hwp")),
    ("hwpx", include_bytes!("fixtures/hancom-note-shapes.hwpx")),
    ("hml", include_bytes!("fixtures/hancom-note-shapes.hml")),
];
const TWO_SECTIONS: &[u8] = include_bytes!("fixtures/hancom-notes-twosec.hwpx");
const FORMATS: [&str; 3] = ["hwp5", "hwpx", "hml"];

fn open(bytes: &[u8]) -> Doc {
    Engine::new().open(bytes.to_vec()).unwrap()
}
fn query(d: &mut Doc, q: Value) -> Value {
    serde_json::from_str(&d.query(&q.to_string()).unwrap()).unwrap()
}
fn tx(d: &mut Doc, ops: Value) -> Value {
    serde_json::from_str(&d.transact(&json!({ "ops": ops }).to_string()).unwrap()).unwrap()
}
fn reopen(d: &mut Doc, format: &str) -> Doc {
    open(&d.serialize_as(format, false).unwrap())
}
fn at(para: u32, offset: u32) -> Value {
    json!({"section":0,"para":para,"offset":offset})
}
/// The document saved as HWPX, section `i`'s XML.
fn section_xml(d: &mut Doc, i: usize) -> String {
    let bytes = d.serialize_as("hwpx", false).unwrap();
    let zip = Archive::open(bytes.into()).unwrap();
    let part = zip
        .read_name(&format!("Contents/section{i}.xml"))
        .unwrap()
        .unwrap();
    String::from_utf8(part.to_vec()).unwrap()
}
fn file_xml(bytes: &[u8], i: usize) -> String {
    let zip = Archive::open(bytes.to_vec().into()).unwrap();
    let part = zip
        .read_name(&format!("Contents/section{i}.xml"))
        .unwrap()
        .unwrap();
    String::from_utf8(part.to_vec()).unwrap()
}
/// Every start tag named `name`, with the instance ids left out.
fn tags(xml: &str, name: &str) -> Vec<String> {
    let open = format!("<{name} ");
    xml.match_indices(&open)
        .map(|(i, _)| {
            let tag = &xml[i..i + xml[i..].find('>').unwrap() + 1];
            match tag.find(" instId=\"") {
                Some(k) => {
                    let end = k + 9 + tag[k + 9..].find('"').unwrap() + 1;
                    format!("{}{}", &tag[..k], &tag[end..])
                }
                None => tag.to_string(),
            }
        })
        .collect()
}
/// The notes' instance ids, sorted.
fn note_ids(xml: &str) -> Vec<String> {
    let mut out: Vec<String> = xml
        .match_indices(" instId=\"")
        .filter(|(i, _)| {
            let start = xml[..*i].rfind('<').unwrap();
            xml[start..].starts_with("<hp:footNote") || xml[start..].starts_with("<hp:endNote")
        })
        .map(|(i, _)| xml[i + 9..i + 9 + xml[i + 9..].find('"').unwrap()].to_string())
        .collect();
    out.sort();
    out
}
/// The note controls, both shapes and the notes' own numbers, as saved.
fn note_markup(xml: &str) -> Vec<String> {
    let shapes = xml
        .find("<hp:footNotePr>")
        .map(|s| xml[s..xml.find("</hp:endNotePr>").unwrap() + 15].to_string());
    let mut out: Vec<String> = shapes.into_iter().collect();
    for name in ["hp:footNote", "hp:endNote"] {
        out.extend(tags(xml, name));
    }
    for (i, _) in xml.match_indices("<hp:autoNum ") {
        out.push(xml[i..i + xml[i..].find("</hp:autoNum>").unwrap()].to_string());
    }
    out
}
/// `(kind, number)` of every note in document order.
fn numbers(d: &mut Doc) -> Vec<(String, u64)> {
    query(d, json!({"kind":"controls"}))
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["kind"] == "footnote" || c["kind"] == "endnote")
        .map(|c| {
            (
                c["kind"].as_str().unwrap().to_string(),
                c["number"].as_u64().unwrap(),
            )
        })
        .collect()
}
fn of(kind: &str, d: &mut Doc) -> Vec<u64> {
    numbers(d)
        .into_iter()
        .filter(|(k, _)| k == kind)
        .map(|(_, n)| n)
        .collect()
}
fn note_records(d: &mut Doc, kind: &str) -> Vec<Value> {
    query(d, json!({"kind":"controls"}))
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["kind"] == kind)
        .cloned()
        .collect()
}
fn texts(d: &mut Doc, list: &Value) -> Vec<String> {
    query(d, json!({"kind":"paragraphs","list":list}))
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["text"].as_str().unwrap().to_string())
        .collect()
}
fn para_end(d: &mut Doc, list: &Value, para: u32) -> u32 {
    let p = query(
        d,
        json!({"kind":"paragraphs","list":list,"from":para,"count":1}),
    );
    p[0]["length"].as_u64().unwrap() as u32 - 1
}
fn caret_in(list: &Value, para: u32, offset: u32) -> Value {
    json!({"section":list["section"],"steps":list["steps"],"para":para,"offset":offset})
}

#[test]
fn hancom_note_files_keep_every_note_attribute_in_every_format() {
    let expected = note_markup(&file_xml(SHAPES[1].1, 0));
    let hancom_ids = note_ids(&file_xml(SHAPES[1].1, 0));
    assert!(expected[0].contains("type=\"USER_CHAR\""));
    for (source, bytes) in SHAPES {
        let mut d = open(bytes);
        for format in FORMATS {
            let mut r = reopen(&mut d, format);
            let xml = section_xml(&mut r, 0);
            assert_eq!(note_markup(&xml), expected, "{source} -> {format}");
            // HWPML stores no instance ids; like Hancom, reading one makes
            // new ones, unique in the document. Others keep Hancom's.
            let mut ids = note_ids(&xml);
            assert_eq!(ids.len(), 5, "{source} -> {format}");
            if source != "hml" && format != "hml" {
                assert_eq!(ids, hancom_ids, "{source} -> {format}");
            }
            ids.dedup();
            assert_eq!(ids.len(), 5, "{source} -> {format}");
        }
    }
    let mut d = open(SHAPES[1].1);
    let hml = String::from_utf8(d.serialize_as("hml", false).unwrap()).unwrap();
    for part in [
        "<FOOTNOTE><PARALIST",
        "Type=\"Dash\"",
        "Width=\"0.25mm\"",
        "Superscript=\"true\"",
        "BeneathText=\"true\"",
        "Place=\"RightMostColumn\"",
        "Type=\"OnSection\"",
    ] {
        assert!(hml.contains(part), "{part}");
    }
    let mut plain = open(NOTES[1].1);
    let hml = String::from_utf8(plain.serialize_as("hml", false).unwrap()).unwrap();
    assert!(hml.contains("Length=\"5cm\""), "Hancom's default separator");
}

#[test]
fn inserting_notes_matches_hancom_in_body_cells_and_text_boxes() {
    for (source, bytes) in NOTES {
        let mut d = open(bytes);
        let body = json!({"section":0});
        let end = para_end(&mut d, &body, 0);
        let receipt = tx(
            &mut d,
            json!([{"op":"insertNote","at":at(0,end),"kind":"footnote"}]),
        );
        // The caret lands after the new note's number and a space.
        let caret = receipt["caret"].clone();
        assert_eq!(caret["para"], 0, "{source}");
        assert_eq!(caret["offset"], 9, "{source}");
        let list = json!({"section":0,"steps":caret["steps"]});
        assert_eq!(texts(&mut d, &list), [" "], "{source}");
        assert_eq!(of("footnote", &mut d), [1, 2, 3, 4], "{source}");
        tx(
            &mut d,
            json!([{"op":"insertText","at":caret,"text":"new note"}]),
        );
        // A note in table cell A1 counts in document order.
        let table = query(&mut d, json!({"kind":"controls"}))
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["kind"] == "table")
            .unwrap()
            .clone();
        let cell = table["lists"][0].clone();
        let end = para_end(&mut d, &cell, 0);
        let r = tx(
            &mut d,
            json!([{"op":"insertNote","at":caret_in(&cell,0,end),"kind":"footnote"}]),
        );
        assert_eq!(r["caret"]["offset"], 9);
        assert_eq!(of("footnote", &mut d), [1, 2, 3, 4, 5], "{source}");
        // An endnote at the start of P3 comes between en A and en B.
        let r = tx(
            &mut d,
            json!([{"op":"insertNote","at":at(3,0),"kind":"endnote"}]),
        );
        assert_eq!(r["caret"]["offset"], 9);
        assert_eq!(of("endnote", &mut d), [1, 2, 3], "{source}");
        // The new notes use the 각주 and 미주 styles with their shapes, like
        // the ones Hancom made.
        let xml = section_xml(&mut d, 0);
        let all: Vec<&str> = xml
            .split("<hp:footNote ")
            .skip(1)
            .chain(xml.split("<hp:endNote ").skip(1))
            .collect();
        assert_eq!(all.len(), 8, "{source}");
        for n in all {
            let endnote = n.contains("numType=\"ENDNOTE\"");
            let style = if endnote { "15" } else { "14" };
            assert!(
                n.contains(&format!("paraPrIDRef=\"10\" styleIDRef=\"{style}\"")),
                "{source}: {n}"
            );
            assert!(
                n.contains("<hp:run charPrIDRef=\"3\"><hp:ctrl><hp:autoNum "),
                "{source}: {n}"
            );
        }
        for format in FORMATS {
            let mut r = reopen(&mut d, format);
            assert_eq!(of("footnote", &mut r), [1, 2, 3, 4, 5], "{source} {format}");
            assert_eq!(of("endnote", &mut r), [1, 2, 3], "{source} {format}");
            let note = note_records(&mut r, "footnote")[2].clone();
            assert_eq!(texts(&mut r, &note["lists"][0]), [" new note"]);
        }
        // Each insertion is one undo step.
        d.undo().unwrap();
        assert_eq!(of("endnote", &mut d), [1, 2], "{source}");
        d.undo().unwrap();
        assert_eq!(of("footnote", &mut d), [1, 2, 3, 4], "{source}");
    }
}

#[test]
fn notes_in_text_boxes_count_in_document_order() {
    let mut d = open(NOTES[1].1);
    tx(
        &mut d,
        json!([{"op":"createShape","at":at(2,0),"kind":"textbox","width":8000,"height":3000,"text":"box"}]),
    );
    let tb = query(&mut d, json!({"kind":"controls"}))
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["kind"] == "object" && c["target"]["para"] == 2)
        .unwrap()
        .clone();
    let list = tb["lists"][0].clone();
    let r = tx(
        &mut d,
        json!([{"op":"insertNote","at":caret_in(&list,0,3),"kind":"footnote"}]),
    );
    assert_eq!(r["caret"]["offset"], 9);
    // Before fn C in P2, after the notes in P0.
    assert_eq!(of("footnote", &mut d), [1, 2, 3, 4]);
    let boxed = &note_records(&mut d, "footnote")[2];
    assert_eq!(boxed["target"]["steps"], list["steps"]);
    for format in ["hwp5", "hwpx"] {
        let mut r = reopen(&mut d, format);
        assert_eq!(of("footnote", &mut r), [1, 2, 3, 4], "{format}");
    }
}

#[test]
fn a_note_without_the_note_style_takes_the_shapes_at_its_mark() {
    let mut d = Engine::new()
        .create(include_bytes!("fixtures/query-equations.hwpx").to_vec())
        .unwrap();
    let styles = query(&mut d, json!({"kind":"styles"}));
    assert!(
        !styles
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["name"] == "각주")
    );
    let body = json!({"section":0});
    let end = para_end(&mut d, &body, 0);
    let r = tx(
        &mut d,
        json!([{"op":"insertNote","at":at(0,end),"kind":"footnote"}]),
    );
    assert_eq!(r["caret"]["offset"], 9);
    let list = json!({"section":0,"steps":r["caret"]["steps"]});
    assert_eq!(texts(&mut d, &list), [" "]);
    let runs = query(&mut d, json!({"kind":"formatRuns","list":list,"count":1}));
    let host = query(&mut d, json!({"kind":"formatRuns","list":body,"count":1}));
    assert_eq!(runs[0]["paraShapeId"], host[0]["paraShapeId"]);
    // A note cannot hold a note.
    assert!(
        d.transact(
            &json!({"ops":[{"op":"insertNote","at":r["caret"],"kind":"footnote"}]}).to_string()
        )
        .is_err()
    );
}

#[test]
fn note_shapes_rewrite_every_note_as_hancom_does() {
    // Hancom's 'all' settings, applied to the plain document in one step,
    // give Hancom's file attribute by attribute.
    let expected = note_markup(&file_xml(SHAPES[1].1, 0));
    for (source, bytes) in NOTES {
        let mut d = open(bytes);
        let before = note_markup(&section_xml(&mut d, 0));
        tx(
            &mut d,
            json!([
                {"op":"setNoteShape","section":0,"kind":"footnote","patch":{"attr":0x3681,"userChar":42,"prefix":40,"suffix":93,"start":5,"separatorLength":5669,"spaceAbove":1000,"spaceBelow":700,"spaceBetween":400,"lineKind":2,"lineWidth":4,"lineColor":255}},
                {"op":"setNoteShape","section":0,"kind":"endnote","patch":{"attr":0x050d,"start":3,"separatorLength":0}}
            ]),
        );
        assert_eq!(note_markup(&section_xml(&mut d, 0)), expected, "{source}");
        for format in FORMATS {
            let mut r = reopen(&mut d, format);
            assert_eq!(
                note_markup(&section_xml(&mut r, 0)),
                expected,
                "{source} -> {format}"
            );
        }
        d.undo().unwrap();
        assert_eq!(note_markup(&section_xml(&mut d, 0)), before, "{source}");
    }
}

#[test]
fn every_hancom_number_format_is_mirrored_into_the_notes() {
    let names = [
        (0, "DIGIT"),
        (1, "CIRCLED_DIGIT"),
        (2, "ROMAN_CAPITAL"),
        (3, "ROMAN_SMALL"),
        (4, "LATIN_CAPITAL"),
        (5, "LATIN_SMALL"),
        (6, "CIRCLED_LATIN_CAPITAL"),
        (8, "HANGUL_SYLLABLE"),
        (10, "HANGUL_JAMO"),
        (12, "HANGUL_PHONETIC"),
        (13, "IDEOGRAPH"),
        (15, "DECAGON_CIRCLE"),
        (16, "DECAGON_CIRCLE_HANJA"),
        (128, "SYMBOL"),
        (129, "USER_CHAR"),
    ];
    let mut d = open(NOTES[1].1);
    for (code, name) in names {
        tx(
            &mut d,
            json!([{"op":"setNoteShape","section":0,"kind":"footnote","patch":{"attr":code}}]),
        );
        let xml = section_xml(&mut d, 0);
        let flag = if code == 0 {
            String::new()
        } else {
            format!("flag=\"{code}\" ")
        };
        for tag in tags(&xml, "hp:footNote") {
            assert!(
                tag.starts_with(&format!("<hp:footNote {flag}number=")),
                "{tag}"
            );
        }
        // The shape and three notes, and for digits the endnotes as well.
        assert_eq!(
            xml.matches(&format!("<hp:autoNumFormat type=\"{name}\""))
                .count(),
            if code == 0 { 7 } else { 4 },
            "{name}"
        );
        assert!(
            tags(&xml, "hp:endNote")
                .iter()
                .all(|t| !t.contains("flag=")),
            "endnotes keep their shape"
        );
    }
}

#[test]
fn page_and_section_restarts_store_what_hancom_stores() {
    // Restarting each section from 3, as Hancom saves it.
    let mut d = open(TWO_SECTIONS);
    let mut ops = Vec::new();
    for section in 0..2 {
        for kind in ["footnote", "endnote"] {
            ops.push(json!({"op":"setNoteShape","section":section,"kind":kind,"patch":{"attr":1 << 10,"start":3}}));
        }
    }
    tx(&mut d, json!(ops));
    assert_eq!(of("footnote", &mut d), [3, 3]);
    assert_eq!(of("endnote", &mut d), [3, 3]);
    d.undo().unwrap();
    assert_eq!(of("footnote", &mut d), [1, 2]);
    // A start number without a restart is stored but not used.
    tx(
        &mut d,
        json!([{"op":"setNoteShape","section":0,"kind":"footnote","patch":{"start":5}}]),
    );
    assert_eq!(of("footnote", &mut d), [1, 2]);
    assert!(section_xml(&mut d, 0).contains("<hp:numbering type=\"CONTINUOUS\" newNum=\"5\"/>"));
    // Only the second section restarting its footnotes at 7.
    tx(
        &mut d,
        json!([{"op":"setNoteShape","section":1,"kind":"footnote","patch":{"attr":1 << 10,"start":7}}]),
    );
    assert_eq!(of("footnote", &mut d), [1, 7]);
    assert_eq!(of("endnote", &mut d), [1, 2]);

    // Footnotes per page: the controls carry flag 256, page-local numbers
    // stay as stored. Endnotes cannot restart per page; Hancom saves that
    // shape without a numbering type and keeps counting.
    let mut d = open(NOTES[1].1);
    tx(
        &mut d,
        json!([
            {"op":"setNoteShape","section":0,"kind":"footnote","patch":{"attr":2 << 10}},
            {"op":"setNoteShape","section":0,"kind":"endnote","patch":{"attr":2 << 10}}
        ]),
    );
    let xml = section_xml(&mut d, 0);
    assert_eq!(
        tags(&xml, "hp:footNote")[0],
        "<hp:footNote flag=\"256\" number=\"1\" suffixChar=\"41\">"
    );
    assert!(xml.contains("<hp:numbering type=\"ON_PAGE\" newNum=\"1\"/>"));
    assert!(xml.contains("<hp:numbering newNum=\"1\"/>"));
    assert_eq!(of("endnote", &mut d), [1, 2]);
    let hml = String::from_utf8(d.serialize_as("hml", false).unwrap()).unwrap();
    assert_eq!(hml.matches("Type=\"OnPage\"").count(), 2);
    for format in FORMATS {
        let mut r = reopen(&mut d, format);
        let shape = query(
            &mut r,
            json!({"kind":"noteShape","section":0,"noteKind":"endnote"}),
        );
        assert_eq!(shape["attr"].as_u64().unwrap() >> 10 & 3, 2, "{format}");
        assert_eq!(
            note_records(&mut r, "footnote")[0]["noteFlag"],
            256,
            "{format}"
        );
    }
}

#[test]
fn deleting_numbers_and_marks_renumbers_and_undoes() {
    for (source, bytes) in NOTES {
        let mut d = open(bytes);
        let fn_a = note_records(&mut d, "footnote")[0].clone();
        let list = fn_a["lists"][0].clone();
        // Deleting the note's own number leaves a numberless note.
        tx(
            &mut d,
            json!([{"op":"delete","from":caret_in(&list,0,0),"to":caret_in(&list,0,8)}]),
        );
        assert_eq!(texts(&mut d, &list), [" fn A"], "{source}");
        for format in FORMATS {
            let mut r = reopen(&mut d, format);
            let xml = section_xml(&mut r, 0);
            let first = xml.split("<hp:footNote ").nth(1).unwrap();
            let first = &first[..first.find("</hp:footNote>").unwrap()];
            assert!(!first.contains("<hp:autoNum"), "{source} -> {format}");
            assert_eq!(of("footnote", &mut r), [1, 2, 3], "{source} -> {format}");
        }
        d.undo().unwrap();
        assert!(
            section_xml(&mut d, 0)
                .split("<hp:footNote ")
                .nth(1)
                .unwrap()[..400]
                .contains("<hp:autoNum")
        );
        // A range over fn A's mark removes the note and renumbers.
        tx(
            &mut d,
            json!([{"op":"delete","from":at(0,24),"to":at(0,34)}]),
        );
        assert_eq!(of("footnote", &mut d), [1, 2], "{source}");
        d.undo().unwrap();
        assert_eq!(of("footnote", &mut d), [1, 2, 3], "{source}");
        // So does deleting the mark as a control.
        tx(
            &mut d,
            json!([{"op":"deleteControl","target":fn_a["target"]}]),
        );
        assert_eq!(of("footnote", &mut d), [1, 2], "{source}");
        assert_eq!(of("endnote", &mut d), [1, 2], "{source}");
        d.undo().unwrap();
        assert_eq!(texts(&mut d, &list), [" fn A"]);
        assert_eq!(of("footnote", &mut d), [1, 2, 3], "{source}");
    }
}

#[test]
fn clipboard_carries_notes_like_hancom() {
    for (source, bytes) in NOTES {
        let mut d = open(bytes);
        // "quick" with fn A's mark, pasted at the end of P3.
        d.copy(&json!({"from":at(0,20),"to":at(0,33)}).to_string())
            .unwrap();
        let end = para_end(&mut d, &json!({"section":0}), 3);
        let r: Value =
            serde_json::from_str(&d.paste(&json!({"at":at(3,end)}).to_string()).unwrap()).unwrap();
        assert_eq!(r["caret"]["offset"], end + 5 + 8, "{source}");
        assert_eq!(of("footnote", &mut d), [1, 2, 3, 4], "{source}");
        let copies = note_records(&mut d, "footnote");
        assert_eq!(texts(&mut d, &copies[3]["lists"][0]), [" fn A"], "{source}");
        // Pasting earlier renumbers every note after it.
        d.paste(&json!({"at":at(0,16)}).to_string()).unwrap();
        assert_eq!(of("footnote", &mut d), [1, 2, 3, 4, 5], "{source}");
        let first = note_records(&mut d, "footnote")[0].clone();
        assert_eq!(texts(&mut d, &first["lists"][0]), [" fn A"]);
        // The copies are notes of their own: new instance ids.
        let xml = section_xml(&mut d, 0);
        let foot = tags(&xml, "hp:footNote");
        assert_eq!(foot.len(), 5);
        for format in FORMATS {
            let mut r = reopen(&mut d, format);
            assert_eq!(of("footnote", &mut r), [1, 2, 3, 4, 5], "{source} {format}");
        }
        // One paste, one undo step.
        d.undo().unwrap();
        assert_eq!(of("footnote", &mut d), [1, 2, 3, 4], "{source}");

        // A mark pasted into a note is dropped; its text stays.
        let fn_b = note_records(&mut d, "footnote")[1].clone();
        let list = fn_b["lists"][0].clone();
        let end = para_end(&mut d, &list, 0);
        d.paste(&json!({"at":caret_in(&list,0,end)}).to_string())
            .unwrap();
        assert_eq!(texts(&mut d, &list), [" fn Bquick"], "{source}");
        assert_eq!(of("footnote", &mut d), [1, 2, 3, 4], "{source}");

        // A note's own number pasted into the body is a bare number.
        let fn_a = note_records(&mut d, "footnote")[0]["lists"][0].clone();
        d.copy(&json!({"from":caret_in(&fn_a,0,0),"to":caret_in(&fn_a,0,13)}).to_string())
            .unwrap();
        d.paste(&json!({"at":at(2,0)}).to_string()).unwrap();
        let bare = query(&mut d, json!({"kind":"controls","list":{"section":0}}))
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["kind"] == "autoNumber" && c["target"]["steps"] == json!([]))
            .count();
        assert_eq!(bare, 1, "{source}");
        assert!(texts(&mut d, &json!({"section":0}))[2].starts_with(" fn A"));
        assert_eq!(of("footnote", &mut d), [1, 2, 3, 4], "{source}");
        assert!(
            section_xml(&mut d, 0).contains("<hp:ctrl><hp:autoNum num=\"1\" numType=\"FOOTNOTE\">")
        );

        // Replacing a range with a mark in it (a cut and paste) renumbers.
        d.copy(&json!({"from":at(3,0),"to":at(3,4)}).to_string())
            .unwrap();
        d.paste(&json!({"replace":{"from":at(0,24),"to":at(0,34)}}).to_string())
            .unwrap();
        assert_eq!(of("footnote", &mut d), [1, 2, 3], "{source}");
    }
}

#[test]
fn edits_inside_a_note_are_single_undo_steps() {
    let mut d = open(NOTES[1].1);
    let body = json!({"section":0});
    let end = para_end(&mut d, &body, 0);
    let r = tx(
        &mut d,
        json!([{"op":"insertNote","at":at(0,end),"kind":"footnote"}]),
    );
    let caret = r["caret"].clone();
    let list = json!({"section":0,"steps":caret["steps"]});
    tx(&mut d, json!([{"op":"insertText","at":caret,"text":"abc"}]));
    // Insert + typing are two steps, as in Hancom.
    d.undo().unwrap();
    assert_eq!(texts(&mut d, &list), [" "]);
    d.undo().unwrap();
    assert_eq!(of("footnote", &mut d), [1, 2, 3]);
    d.redo().unwrap();
    d.redo().unwrap();
    assert_eq!(texts(&mut d, &list), [" abc"]);
    // Replacing a selection that holds the number is one step.
    tx(
        &mut d,
        json!([
            {"op":"delete","from":caret_in(&list,0,0),"to":caret_in(&list,0,12)},
            {"op":"insertText","at":caret_in(&list,0,0),"text":"Z"}
        ]),
    );
    assert_eq!(texts(&mut d, &list), ["Z"]);
    d.undo().unwrap();
    assert_eq!(texts(&mut d, &list), [" abc"]);
    assert_eq!(of("footnote", &mut d), [1, 2, 3, 4]);
}

#[test]
fn footnotes_numbered_per_page_store_their_page_local_numbers() {
    // Hancom's two-page document set to restart footnotes on every page.
    let hancom = [
        "<hp:footNote flag=\"256\" number=\"1\" suffixChar=\"41\">",
        "<hp:footNote flag=\"256\" number=\"2\" suffixChar=\"41\">",
        "<hp:footNote flag=\"256\" number=\"1\" suffixChar=\"41\">",
        "<hp:footNote flag=\"256\" number=\"2\" suffixChar=\"41\">",
    ];
    let own = |xml: &str| -> Vec<String> {
        xml.match_indices("<hp:autoNum num=\"")
            .map(|(i, _)| xml[i..i + xml[i..].find(" numType").unwrap()].to_string())
            .collect()
    };
    let mut d = open(include_bytes!("fixtures/hancom-notes-twopage.hwpx"));
    tx(
        &mut d,
        json!([{"op":"setNoteShape","section":0,"kind":"footnote","patch":{"attr":2 << 10}}]),
    );
    let xml = section_xml(&mut d, 0);
    assert_eq!(tags(&xml, "hp:footNote"), hancom);
    assert_eq!(of("footnote", &mut d), [1, 2, 1, 2]);
    let autos = own(&xml);
    assert_eq!(autos.len(), 6, "{autos:?}");
    for format in FORMATS {
        let mut r = reopen(&mut d, format);
        assert_eq!(of("footnote", &mut r), [1, 2, 1, 2], "{format}");
        assert_eq!(own(&section_xml(&mut r, 0)), autos, "{format}");
    }
    // The page-local numbers belong to the shape change's undo step.
    d.undo().unwrap();
    assert_eq!(of("footnote", &mut d), [1, 2, 3, 4]);
    d.redo().unwrap();
    assert_eq!(of("footnote", &mut d), [1, 2, 1, 2]);
    // A note inserted first on page 1 takes 1; the others on that page
    // follow it, page 2 keeps its own.
    let lead = query(
        &mut d,
        json!({"kind":"paragraphs","list":{"section":0},"count":1}),
    )[0]["caret"][0]
        .as_u64()
        .unwrap() as u32;
    tx(
        &mut d,
        json!([{"op":"insertNote","at":at(0,lead),"kind":"footnote"}]),
    );
    assert_eq!(of("footnote", &mut d), [1, 2, 3, 1, 2]);
    d.undo().unwrap();
    assert_eq!(of("footnote", &mut d), [1, 2, 1, 2]);
}

#[test]
fn new_and_pasted_notes_get_their_own_instance_ids() {
    let ids = |d: &mut Doc| note_ids(&section_xml(d, 0));
    let mut d = open(NOTES[1].1);
    let body = json!({"section":0});
    let end = para_end(&mut d, &body, 0);
    tx(
        &mut d,
        json!([{"op":"insertNote","at":at(0,end),"kind":"footnote"}]),
    );
    d.copy(&json!({"from":at(0,16),"to":at(0,end + 8)}).to_string())
        .unwrap();
    d.paste(&json!({"at":at(3,0)}).to_string()).unwrap();
    let all = ids(&mut d);
    assert_eq!(all.len(), 5 + 1 + 4, "every note has an id");
    let mut unique = all.clone();
    unique.dedup();
    assert_eq!(unique, all, "no two notes share an id");
}

#[test]
fn a_table_with_notes_pasted_into_a_note_keeps_its_text_only() {
    let mut d = open(NOTES[1].1);
    let table = query(&mut d, json!({"kind":"controls"}))
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["kind"] == "table")
        .unwrap()
        .clone();
    let cell = table["lists"][0].clone();
    let end = para_end(&mut d, &cell, 0);
    tx(
        &mut d,
        json!([{"op":"insertNote","at":caret_in(&cell,0,end),"kind":"footnote"}]),
    );
    assert_eq!(of("footnote", &mut d), [1, 2, 3, 4]);
    // Copy P1 (the table) and paste it into fn B's list.
    d.copy(&json!({"from":at(1,0),"to":at(1,8)}).to_string())
        .unwrap();
    let fn_b = note_records(&mut d, "footnote")[1]["lists"][0].clone();
    let end = para_end(&mut d, &fn_b, 0);
    d.paste(&json!({"at":caret_in(&fn_b,0,end)}).to_string())
        .unwrap();
    // The table arrives in the note; the note mark in its cell does not.
    assert_eq!(of("footnote", &mut d), [1, 2, 3, 4]);
    let inside = query(&mut d, json!({"kind":"controls","list":fn_b}));
    assert!(
        inside
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["kind"] == "table")
    );
    assert!(
        !inside
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["kind"] == "footnote")
    );
    for format in ["hwp5", "hwpx"] {
        let mut r = reopen(&mut d, format);
        assert_eq!(of("footnote", &mut r), [1, 2, 3, 4], "{format}");
    }
}

/// The section shapes as Hancom saves them with only these items changed.
fn shapes_xml(
    fn_format: &str,
    user: &str,
    fn_line: (&str, &str),
    en_format: &str,
    en_line: (&str, &str),
) -> String {
    format!(
        "<hp:footNotePr><hp:autoNumFormat type=\"{fn_format}\" userChar=\"{user}\" prefixChar=\"\" suffixChar=\")\" supscript=\"0\"/><hp:noteLine length=\"{}\" type=\"{}\" width=\"0.12 mm\" color=\"#000000\"/><hp:noteSpacing betweenNotes=\"283\" belowLine=\"567\" aboveLine=\"850\"/><hp:numbering type=\"CONTINUOUS\" newNum=\"1\"/><hp:placement place=\"EACH_COLUMN\" beneathText=\"0\"/></hp:footNotePr><hp:endNotePr><hp:autoNumFormat type=\"{en_format}\" userChar=\"\" prefixChar=\"\" suffixChar=\")\" supscript=\"0\"/><hp:noteLine length=\"{}\" type=\"{}\" width=\"0.12 mm\" color=\"#000000\"/><hp:noteSpacing betweenNotes=\"0\" belowLine=\"567\" aboveLine=\"850\"/><hp:numbering type=\"CONTINUOUS\" newNum=\"1\"/><hp:placement place=\"END_OF_DOCUMENT\" beneathText=\"0\"/></hp:endNotePr>",
        fn_line.0, fn_line.1, en_line.0, en_line.1
    )
}

#[test]
fn circled_formats_line_kinds_length_presets_and_user_characters_match_hancom() {
    // Hancom's saves of each setting (`e1-more-files/v-a` … `v-f`): the
    // shapes, the first footnote and endnote controls, and HWPML names.
    struct Case {
        fn_patch: Value,
        en_patch: Value,
        shapes: String,
        controls: [&'static str; 2],
        hml: &'static [&'static str],
    }
    let cases = [
        Case {
            fn_patch: json!({"attr":7,"lineKind":4,"separatorLength":-2}),
            en_patch: json!({"attr":9,"separatorLength":-2}),
            shapes: shapes_xml(
                "CIRCLED_LATIN_SMALL",
                "",
                ("-2", "DASH_DOT"),
                "CIRCLED_HANGUL_SYLLABLE",
                ("-2", "SOLID"),
            ),
            controls: [
                "<hp:footNote flag=\"7\" number=\"1\" suffixChar=\"41\">",
                "<hp:endNote flag=\"9\" number=\"1\" suffixChar=\"41\">",
            ],
            hml: &[
                "Type=\"CircledLatinSmall\"",
                "Type=\"CircledHangulSyllable\"",
                "<NOTELINE Length=\"2cm\" Type=\"DashDot\" Width=\"0.12mm\"/>",
                "<NOTELINE Length=\"2cm\" Type=\"Solid\" Width=\"0.12mm\"/>",
            ],
        },
        Case {
            fn_patch: json!({"attr":11,"lineKind":5,"separatorLength":-3}),
            en_patch: json!({"attr":14,"lineKind":6,"separatorLength":-3}),
            shapes: shapes_xml(
                "CIRCLED_HANGUL_JAMO",
                "",
                ("-3", "DASH_DOT_DOT"),
                "CIRCLED_IDEOGRAPH",
                ("-3", "LONG_DASH"),
            ),
            controls: [
                "<hp:footNote flag=\"11\" number=\"1\" suffixChar=\"41\">",
                "<hp:endNote flag=\"14\" number=\"1\" suffixChar=\"41\">",
            ],
            hml: &[
                "Type=\"CircledHangulJamo\"",
                "Type=\"CircledIdeograph\"",
                "<NOTELINE Length=\"Column/3\" Type=\"DashDotDot\" Width=\"0.12mm\"/>",
                "<NOTELINE Length=\"Column/3\" Type=\"LongDash\" Width=\"0.12mm\"/>",
            ],
        },
        Case {
            fn_patch: json!({"attr":14,"lineKind":6,"separatorLength":-4}),
            en_patch: json!({"attr":7,"separatorLength":-4}),
            shapes: shapes_xml(
                "CIRCLED_IDEOGRAPH",
                "",
                ("-4", "LONG_DASH"),
                "CIRCLED_LATIN_SMALL",
                ("-4", "SOLID"),
            ),
            controls: [
                "<hp:footNote flag=\"14\" number=\"1\" suffixChar=\"41\">",
                "<hp:endNote flag=\"7\" number=\"1\" suffixChar=\"41\">",
            ],
            hml: &[
                "<NOTELINE Length=\"Column\" Type=\"LongDash\" Width=\"0.12mm\"/>",
                "<NOTELINE Length=\"Column\" Type=\"Solid\" Width=\"0.12mm\"/>",
            ],
        },
        Case {
            fn_patch: json!({"attr":129,"userChar":35,"lineKind":7}),
            en_patch: json!({}),
            shapes: shapes_xml(
                "USER_CHAR",
                "#",
                ("-1", "CIRCLE"),
                "DIGIT",
                ("14692344", "SOLID"),
            ),
            controls: [
                "<hp:footNote flag=\"129\" number=\"1\" userChar=\"35\" suffixChar=\"41\">",
                "<hp:endNote number=\"1\" suffixChar=\"41\">",
            ],
            hml: &[
                "UserChar=\"#\"",
                "<NOTELINE Length=\"5cm\" Type=\"Circle\" Width=\"0.12mm\"/>",
            ],
        },
        Case {
            fn_patch: json!({"attr":129,"userChar":43,"lineKind":9}),
            en_patch: json!({"attr":11}),
            shapes: shapes_xml(
                "USER_CHAR",
                "+",
                ("-1", "SLIM_THICK"),
                "CIRCLED_HANGUL_JAMO",
                ("14692344", "SOLID"),
            ),
            controls: [
                "<hp:footNote flag=\"129\" number=\"1\" userChar=\"43\" suffixChar=\"41\">",
                "<hp:endNote flag=\"11\" number=\"1\" suffixChar=\"41\">",
            ],
            hml: &[
                "UserChar=\"+\"",
                "<NOTELINE Length=\"5cm\" Type=\"SlimThick\" Width=\"0.12mm\"/>",
            ],
        },
        Case {
            fn_patch: json!({"lineKind":10}),
            en_patch: json!({"lineKind":11}),
            shapes: shapes_xml(
                "DIGIT",
                "",
                ("-1", "THICK_SLIM"),
                "DIGIT",
                ("14692344", "SLIM_THICK_SLIM"),
            ),
            controls: [
                "<hp:footNote number=\"1\" suffixChar=\"41\">",
                "<hp:endNote number=\"1\" suffixChar=\"41\">",
            ],
            hml: &[
                "<NOTELINE Length=\"5cm\" Type=\"ThickSlim\" Width=\"0.12mm\"/>",
                "<NOTELINE Length=\"14692344\" Type=\"SlimThickSlim\" Width=\"0.12mm\"/>",
            ],
        },
    ];
    for (k, case) in cases.iter().enumerate() {
        let mut d = open(NOTES[1].1);
        tx(
            &mut d,
            json!([
                {"op":"setNoteShape","section":0,"kind":"footnote","patch":case.fn_patch},
                {"op":"setNoteShape","section":0,"kind":"endnote","patch":case.en_patch}
            ]),
        );
        let markup = note_markup(&section_xml(&mut d, 0));
        assert_eq!(markup[0], case.shapes, "case {k}");
        for c in case.controls {
            assert!(markup.iter().any(|m| m == c), "case {k}: {c}");
        }
        let hml = String::from_utf8(d.serialize_as("hml", false).unwrap()).unwrap();
        for part in case.hml {
            assert!(hml.contains(part), "case {k}: {part}");
        }
        for format in FORMATS {
            let mut r = reopen(&mut d, format);
            assert_eq!(
                note_markup(&section_xml(&mut r, 0)),
                markup,
                "case {k} {format}"
            );
        }
    }
}

#[test]
fn a_shape_change_is_one_undo_step_across_sections() {
    // Hancom applies the dialog to the whole document; Studio sends one
    // op per section and kind in one transaction.
    let mut d = open(TWO_SECTIONS);
    let before = [section_xml(&mut d, 0), section_xml(&mut d, 1)].map(|x| note_markup(&x));
    let mut ops = Vec::new();
    for section in 0..2 {
        ops.push(json!({"op":"setNoteShape","section":section,"kind":"footnote","patch":{"attr":2 | 1 << 10,"start":4,"separatorLength":-2}}));
        ops.push(json!({"op":"setNoteShape","section":section,"kind":"endnote","patch":{"attr":13,"lineKind":4}}));
    }
    tx(&mut d, json!(ops));
    let after = [section_xml(&mut d, 0), section_xml(&mut d, 1)].map(|x| note_markup(&x));
    assert_eq!(of("footnote", &mut d), [4, 4]);
    assert_eq!(of("endnote", &mut d), [1, 2]);
    for x in &after {
        assert!(
            x.iter()
                .filter(|t| t.starts_with("<hp:footNote "))
                .all(|t| t.contains("flag=\"2\""))
        );
        assert!(
            x.iter()
                .filter(|t| t.starts_with("<hp:endNote "))
                .all(|t| t.contains("flag=\"13\""))
        );
    }
    d.undo().unwrap();
    assert_eq!(
        [section_xml(&mut d, 0), section_xml(&mut d, 1)].map(|x| note_markup(&x)),
        before
    );
    d.redo().unwrap();
    assert_eq!(
        [section_xml(&mut d, 0), section_xml(&mut d, 1)].map(|x| note_markup(&x)),
        after
    );
}

#[test]
fn undoing_a_shape_change_restores_every_note_as_hancom_does() {
    // In Hancom, number format 2 with separator length 5669 is one undo step.
    let mut d = open(NOTES[1].1);
    let before = note_markup(&section_xml(&mut d, 0));
    tx(
        &mut d,
        json!([{"op":"setNoteShape","section":0,"kind":"footnote","patch":{"attr":2,"separatorLength":5669}}]),
    );
    let after = note_markup(&section_xml(&mut d, 0));
    let hancom = |m: &[String]| -> (usize, usize, usize) {
        (
            m.iter()
                .filter(|t| t.starts_with("<hp:footNote flag=\"2\" "))
                .count(),
            m.iter()
                .filter(|t| t.contains("<hp:autoNumFormat type=\"ROMAN_CAPITAL\""))
                .count(),
            m.iter()
                .filter(|t| t.contains("<hp:noteLine length=\"5669\" type=\"SOLID\""))
                .count(),
        )
    };
    assert_eq!(hancom(&after), (3, 4, 1), "s-a1: three notes and the shape");
    d.undo().unwrap();
    assert_eq!(note_markup(&section_xml(&mut d, 0)), before, "s-ua1");
    // Nothing earlier to undo.
    d.undo().unwrap();
    assert_eq!(note_markup(&section_xml(&mut d, 0)), before);
    d.redo().unwrap();
    assert_eq!(note_markup(&section_xml(&mut d, 0)), after, "s-ra1");
}

/// The lists of the first table (its cells, then its caption).
fn table_lists(d: &mut Doc) -> (Value, Vec<Value>) {
    let t = query(d, json!({"kind":"controls"}))
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["kind"] == "table")
        .unwrap()
        .clone();
    let lists = t["lists"].as_array().unwrap().clone();
    (t["target"].clone(), lists)
}

#[test]
fn every_edit_that_removes_notes_renumbers_them() {
    // Typed text pasted over a range holding fn A's mark.
    let mut d = open(NOTES[1].1);
    d.paste(&json!({"replace":{"from":at(0,24),"to":at(0,34)},"text":"X"}).to_string())
        .unwrap();
    assert_eq!(of("footnote", &mut d), [1, 2]);
    assert_eq!(of("footnote", &mut reopen(&mut d, "hwpx")), [1, 2]);
    d.undo().unwrap();
    assert_eq!(of("footnote", &mut d), [1, 2, 3]);

    // Deleting the table row holding a note.
    let mut d = open(NOTES[1].1);
    let (table, cells) = table_lists(&mut d);
    let end = para_end(&mut d, &cells[0], 0);
    tx(
        &mut d,
        json!([{"op":"insertNote","at":caret_in(&cells[0],0,end),"kind":"footnote"}]),
    );
    assert_eq!(of("footnote", &mut d), [1, 2, 3, 4]);
    tx(
        &mut d,
        json!([{"op":"deleteTableRows","table":table,"startRow":0,"count":1}]),
    );
    assert_eq!(of("footnote", &mut d), [1, 2, 3]);
    d.undo().unwrap();
    assert_eq!(of("footnote", &mut d), [1, 2, 3, 4]);

    // Replacing a caption that holds a note.
    let mut d = open(NOTES[1].1);
    let (table, _) = table_lists(&mut d);
    tx(
        &mut d,
        json!([{"op":"setTableCaption","table":table,"text":"cap"}]),
    );
    let (_, lists) = table_lists(&mut d);
    let caption = lists.last().unwrap().clone();
    let end = para_end(&mut d, &caption, 0);
    tx(
        &mut d,
        json!([{"op":"insertNote","at":caret_in(&caption,0,end),"kind":"footnote"}]),
    );
    assert_eq!(of("footnote", &mut d), [1, 2, 3, 4]);
    tx(
        &mut d,
        json!([{"op":"setTableCaption","table":table,"text":"new"}]),
    );
    assert_eq!(of("footnote", &mut d), [1, 2, 3]);
}

#[test]
fn notes_go_nowhere_in_headers_or_footers() {
    let mut d = open(NOTES[1].1);
    for kind in ["header", "footer"] {
        let r = tx(
            &mut d,
            json!([{"op":"createHeaderFooter","section":0,"kind":kind,"side":"both"}]),
        );
        for note in ["footnote", "endnote"] {
            let refused = d.transact(
                &json!({"ops":[{"op":"insertNote","at":r["caret"],"kind":note}]}).to_string(),
            );
            assert!(refused.is_err(), "{kind} {note}");
        }
    }
    assert_eq!(of("footnote", &mut d), [1, 2, 3]);
}

#[test]
fn document_start_numbers_renumber_in_their_own_step() {
    let mut d = open(NOTES[1].1);
    tx(
        &mut d,
        json!([{"op":"setDocumentProperties","patch":{"footnoteStart":5,"endnoteStart":3}}]),
    );
    assert_eq!(of("footnote", &mut d), [5, 6, 7]);
    assert_eq!(of("endnote", &mut d), [3, 4]);
    d.undo().unwrap();
    assert_eq!(of("footnote", &mut d), [1, 2, 3]);
}

#[test]
fn typing_in_a_per_page_section_touches_only_changed_page_numbers() {
    use hm_hwpx::zip::{Archive, Member, write};
    // Footnotes numbered per page whose controls lack the mirrored flag,
    // as older files store them.
    let mut d = open(include_bytes!("fixtures/hancom-notes-twopage.hwpx"));
    tx(
        &mut d,
        json!([{"op":"setNoteShape","section":0,"kind":"footnote","patch":{"attr":2 << 10}}]),
    );
    let bytes = d.serialize_as("hwpx", false).unwrap();
    let zip = Archive::open(bytes.into()).unwrap();
    let section = String::from_utf8(
        zip.read_name("Contents/section0.xml")
            .unwrap()
            .unwrap()
            .to_vec(),
    )
    .unwrap()
    .replace("flag=\"256\" ", "");
    let members: Vec<Member<'_>> = (0..zip.entries.len())
        .map(|i| {
            if zip.entries[i].name == "Contents/section0.xml" {
                Member::New {
                    name: "Contents/section0.xml",
                    data: section.as_bytes(),
                    stored: false,
                }
            } else {
                Member::Copy(i)
            }
        })
        .collect();
    let mut old = open(&write(Some(&zip), &members));
    assert_eq!(of("footnote", &mut old), [1, 2, 1, 2]);
    let last = query(&mut old, json!({"kind":"paragraphs","list":{"section":0}}))
        .as_array()
        .unwrap()
        .len() as u32
        - 1;
    tx(
        &mut old,
        json!([{"op":"insertText","at":at(last,0),"text":"x"}]),
    );
    let xml = section_xml(&mut old, 0);
    assert!(
        !xml.contains("flag=\"256\""),
        "untouched notes keep their stored flags"
    );
    assert_eq!(of("footnote", &mut old), [1, 2, 1, 2]);
}
