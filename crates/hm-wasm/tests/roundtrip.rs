//! Converting to the other format and back: every conversion writes a file,
//! the strict checks of `hm-check` pass, and the reopened document shows the
//! same pages as the one saved in its own format.

use serde_json::{Value, json};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn transact(doc: &mut hm_wasm::Doc, ops: Value) {
    doc.transact(&json!({ "ops": ops }).to_string()).unwrap();
}

fn query(doc: &mut hm_wasm::Doc, q: Value) -> Value {
    serde_json::from_str(&doc.query(&q.to_string()).unwrap()).unwrap()
}

/// Saves as `format` the way Studio's Save As does, checks the file
/// strictly and opens it again.
fn converted(doc: &mut hm_wasm::Doc, format: &str) -> hm_wasm::Doc {
    let bytes = if format == "hwpx" {
        doc.export_hwpx()
    } else {
        doc.serialize_as(format, false)
    }
    .unwrap_or_else(|e| panic!("{format} conversion refused: {e:?}"));
    let report = hm_check::check(&bytes);
    let errors: Vec<String> = report.errors().map(|i| i.to_string()).collect();
    assert!(errors.is_empty(), "{format} save: {errors:#?}");
    hm_wasm::Engine::new().open(bytes).unwrap()
}

/// Every page as SVG.
fn pages(doc: &mut hm_wasm::Doc) -> Vec<String> {
    let n = serde_json::from_str::<Value>(&doc.pages().unwrap())
        .unwrap()
        .as_array()
        .unwrap()
        .len();
    (0..n).map(|i| doc.page_svg(i, 96.0).unwrap()).collect()
}

/// An HWPX fixture with one part's text replaced.
fn patched(name: &str, part: &str, from: &str, to: &str) -> Vec<u8> {
    use hm_hwpx::zip::{Archive, Member, write};
    let archive = Archive::open(fixture(name).into()).unwrap();
    let xml = archive.read_name(part).unwrap().unwrap();
    let xml = String::from_utf8(xml.to_vec()).unwrap();
    assert!(xml.contains(from), "{part} has no {from}");
    let xml = xml.replacen(from, to, 1);
    let members: Vec<Member<'_>> = archive
        .entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            if e.name == part {
                Member::New {
                    name: &e.name,
                    data: xml.as_bytes(),
                    stored: false,
                }
            } else {
                Member::Copy(i)
            }
        })
        .collect();
    write(Some(&archive), &members)
}

#[test]
fn a_master_page_no_page_shows_converts() {
    // An even-page master page in a one-page section.
    let bytes = patched(
        "master-page-both.hwpx",
        "Contents/masterpage0.xml",
        r#"type="BOTH""#,
        r#"type="EVEN""#,
    );
    let mut doc = hm_wasm::Engine::new().open(bytes).unwrap();
    let direct = pages(&mut doc);
    let mut hwp = converted(&mut doc, "hwp5");
    assert_eq!(pages(&mut hwp), direct);
    let mut back = converted(&mut hwp, "hwpx");
    assert_eq!(pages(&mut back), direct);
}

#[test]
fn a_header_shown_at_two_page_widths_converts() {
    let mut doc = hm_wasm::Engine::new()
        .open(fixture("hancom-notes-twosec.hwpx"))
        .unwrap();
    // The second section gets wider margins; a header made in the first
    // runs on into it.
    transact(
        &mut doc,
        json!([
            {"op": "setPageSettings", "section": 1, "patch": {"marginLeft": 9000, "marginRight": 9000}},
            {"op": "createHeaderFooter", "section": 0, "kind": "header", "side": "both"},
        ]),
    );
    let direct = {
        let bytes = doc.serialize(false).unwrap();
        pages(&mut hm_wasm::Engine::new().open(bytes).unwrap())
    };
    let mut hwp = converted(&mut doc, "hwp5");
    assert_eq!(pages(&mut hwp), direct);
}

/// An HWPX fixture with every stored line removed from its sections.
fn without_lines(name: &str) -> Vec<u8> {
    use hm_hwpx::zip::{Archive, Member, write};
    let archive = Archive::open(fixture(name).into()).unwrap();
    let parts: Vec<(usize, String)> = archive
        .entries
        .iter()
        .enumerate()
        .filter(|(_, e)| e.name.starts_with("Contents/section"))
        .map(|(i, e)| {
            let xml = archive.read_name(&e.name).unwrap().unwrap();
            let mut xml = String::from_utf8(xml.to_vec()).unwrap();
            while let Some(a) = xml.find("<hp:linesegarray>") {
                let b = xml[a..].find("</hp:linesegarray>").unwrap() + a;
                xml.replace_range(a..b + "</hp:linesegarray>".len(), "");
            }
            (i, xml)
        })
        .collect();
    let members: Vec<Member<'_>> = archive
        .entries
        .iter()
        .enumerate()
        .map(|(i, e)| match parts.iter().find(|(k, _)| *k == i) {
            Some((_, xml)) => Member::New {
                name: &e.name,
                data: xml.as_bytes(),
                stored: false,
            },
            None => Member::Copy(i),
        })
        .collect();
    write(Some(&archive), &members)
}

/// Every paragraph's stored lines, lists included, in document order.
fn stored_lines(doc: &hm_model::Document) -> Vec<Vec<hm_model::LineSeg>> {
    fn walk(paras: &[hm_model::Paragraph], out: &mut Vec<Vec<hm_model::LineSeg>>) {
        for p in paras {
            out.push(p.line_segs.clone());
            for c in &p.controls {
                for list in hm_edit::ops::control_lists(c) {
                    walk(list, out);
                }
            }
        }
    }
    let mut out = Vec::new();
    for s in &doc.sections {
        walk(&s.paras, &mut out);
    }
    out
}

#[test]
fn unchanged_paragraphs_keep_their_stored_lines_or_their_absence() {
    for bytes in [
        fixture("hancom-notes.hwpx"),
        without_lines("hancom-notes.hwpx"),
    ] {
        let source = hm_hwpx::read(bytes.clone().into()).unwrap();
        let mut doc = hm_wasm::Engine::new().open(bytes).unwrap();
        let direct = pages(&mut doc);
        let hwp = doc.serialize_as("hwp5", false).unwrap();
        // Reopened, a paragraph replays or refits its lines as before.
        let back = hm_hwp5::read(hwp.clone().into()).unwrap();
        assert_eq!(stored_lines(&back), stored_lines(&source));
        let mut hwp = converted(&mut doc, "hwp5");
        assert_eq!(pages(&mut hwp), direct);
        let hwpx = hwp.export_hwpx().unwrap();
        let again = hm_hwpx::read(hwpx.into()).unwrap();
        assert_eq!(stored_lines(&again), stored_lines(&source));
    }
}
