//! Our own stored lines replay into our own layout. A document laid out
//! afresh (as when its file asks for that) and converted with the lines that
//! layout selects reopens with the same pages; an edited document saved and
//! reopened shows what it showed.
//!
//! The corpus variant needs `HEEMANG_CORPUS` and `HEEMANG_FONTS` and says
//! loudly when it skips. It checks the documents named below, each also with
//! every stored line replaced by the empty cache Hancom writes in a file that
//! asks for fresh lines; `HEEMANG_REPLAY_DOCS` adds `:`-separated paths and
//! `HEEMANG_REPLAY_ALL=1` checks every document of the corpus.

use std::path::{Path, PathBuf};

use hm_model::{Document, Format, ImportedLineMembership, LineSeg, Paragraph};
use serde_json::{Value, json};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn query(doc: &mut hm_wasm::Doc, q: Value) -> Value {
    serde_json::from_str(&doc.query(&q.to_string()).unwrap()).unwrap()
}

fn pages(doc: &mut hm_wasm::Doc) -> Vec<String> {
    let n = serde_json::from_str::<Value>(&doc.pages().unwrap())
        .unwrap()
        .as_array()
        .unwrap()
        .len();
    (0..n).map(|i| doc.page_svg(i, 96.0).unwrap()).collect()
}

#[test]
fn empty_paragraphs_typed_across_a_page_end_reopen_where_they_were() {
    // Typed empty paragraphs with space before them run past the page end:
    // the first that does not fit moves to the next page, and its saved line
    // sits at the page top, at the space before, as Hancom stores one.
    for name in ["hancom-notes.hwp", "hancom-notes.hwpx"] {
        let mut doc = hm_wasm::Engine::new().open(fixture(name)).unwrap();
        let p = query(
            &mut doc,
            json!({"kind": "paragraphs", "list": {"section": 0}}),
        );
        let last = p.as_array().unwrap().len() - 1;
        let at = json!({"section": 0, "para": last, "offset": p[last]["caret"][0]});
        doc.transact(
            &json!({"ops": [{"op": "insertText", "at": at, "text": "\n".repeat(90)}]}).to_string(),
        )
        .unwrap();
        let n = query(
            &mut doc,
            json!({"kind": "paragraphs", "list": {"section": 0}}),
        )
        .as_array()
        .unwrap()
        .len();
        let from = json!({"section": 0, "para": last + 1, "offset": 0});
        let to = json!({"section": 0, "para": n - 1, "offset": 0});
        doc.transact(
            &json!({"ops": [{"op": "formatParas", "from": from, "to": to, "patch": {"spaceBefore": 600}}]})
                .to_string(),
        )
        .unwrap();
        let shown = pages(&mut doc);
        assert!(shown.len() > 2, "{name}: {} pages", shown.len());
        let bytes = doc.serialize(false).unwrap();
        let mut back = hm_wasm::Engine::new().open(bytes).unwrap();
        assert_eq!(pages(&mut back), shown, "{name}");
    }
}

#[test]
fn typed_blank_runs_in_a_document_composed_afresh_reopen_where_they_were() {
    // The file asks for fresh lines. A typed paragraph whose run of blanks
    // crosses the line end, wherever the run falls, saves and reopens
    // broken as it was shown.
    for lead in 0..8 {
        let mut doc = hm_wasm::Engine::new()
            .open(fixture("conversion-membership.hwpx"))
            .unwrap();
        let text = format!("{}{}{}", "a".repeat(lead), " ".repeat(12), "b".repeat(30));
        let at = json!({"section": 0, "para": 1, "offset": 0});
        doc.transact(
            &json!({"ops": [
                {"op": "insertText", "at": at, "text": text},
                {"op": "formatParas", "from": at, "to": at, "patch": {"left": 300}},
            ]})
            .to_string(),
        )
        .unwrap();
        let lines = json!({"kind": "lines", "list": {"section": 0}, "para": 1});
        let shown = (pages(&mut doc), query(&mut doc, lines.clone()));
        let bytes = doc.serialize(false).unwrap();
        let mut back = hm_wasm::Engine::new().open(bytes).unwrap();
        let reopened = (pages(&mut back), query(&mut back, lines));
        assert_eq!(reopened, shown, "{lead} letters before the blanks");
    }
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

fn fonts(dirs: &str) -> hm_fonts::FontSystem {
    let mut fs = hm_fonts::FontSystem::new();
    for dir in std::env::split_paths(dirs).filter(|d| !d.as_os_str().is_empty()) {
        let mut files = Vec::new();
        walk(&dir, &mut files);
        files.sort();
        for f in files {
            let ext = f
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if matches!(ext.as_str(), "ttf" | "otf" | "ttc") {
                let data: hm_fonts::FontData = std::sync::Arc::new(std::fs::read(&f).unwrap());
                fs.add(&f.display().to_string(), data);
            }
        }
    }
    fs
}

fn read(bytes: &[u8]) -> Option<Document> {
    let b = hm_model::SharedBytes::from(bytes);
    if bytes.starts_with(b"PK") {
        hm_hwpx::read(b).ok()
    } else {
        hm_hwp5::read(b).ok()
    }
}

/// Every stored line of `paras` and the lists in them replaced by Hancom's
/// empty cache entry (no height, no width, bit 31 set).
fn empty_caches(paras: &mut [Paragraph]) {
    for p in paras {
        p.line_segs = vec![LineSeg {
            flags: 0x8006_0000,
            ..Default::default()
        }];
        for c in &mut p.controls {
            if let hm_model::ControlKind::Table(t) = &mut c.kind {
                for cell in &mut t.cells {
                    empty_caches(&mut cell.paras);
                }
            }
        }
    }
}

/// Pages of `doc` laid out afresh that differ from those of its conversion
/// with the lines that layout selects, reopened and replayed; `None` when
/// the conversion refuses the document.
fn replay_differs(doc: &Document, fonts: &hm_fonts::FontSystem) -> Option<Vec<usize>> {
    let mut doc = doc.clone();
    doc.info.imported_line_membership = ImportedLineMembership::Recompose;
    let opts = hm_layout::Options {
        stored_lines: true,
        guides: true,
    };
    let fresh = hm_layout::layout_with(&doc, fonts, opts);
    let target = if doc.meta.format == Format::Hwpx {
        Format::Hwp5
    } else {
        Format::Hwpx
    };
    let out =
        hm_edit::lines::prepare_conversion_with_fonts(&doc, &fresh, target, Some(fonts)).ok()?;
    let bytes = if target == Format::Hwpx {
        hm_hwpx::write_new(&out).ok()?
    } else {
        hm_hwp5::write_new(&out).ok()?
    };
    let mut back = read(&bytes)?;
    if back.info.imported_line_membership == ImportedLineMembership::Recompose {
        back.info.imported_line_membership = ImportedLineMembership::Unknown;
    }
    let replay = hm_layout::layout_with(&back, fonts, opts);
    let page = |d: &Document, l: &hm_layout::DocLayout, i: usize| {
        let built = hm_render::build_page(d, l, fonts, i);
        (
            hm_render::svg::page(&built, fonts, 96.0),
            hm_render::raster::render_png(&built, fonts, 96.0),
        )
    };
    let mut differ = Vec::new();
    for i in 0..fresh.pages.len().max(replay.pages.len()) {
        if i >= fresh.pages.len() || i >= replay.pages.len() {
            differ.push(i);
            continue;
        }
        let (a, b) = (page(&doc, &fresh, i), page(&back, &replay, i));
        // Regrouped glyph runs that rasterize the same are encoding noise.
        if a.0 != b.0 && a.1 != b.1 {
            differ.push(i);
        }
    }
    Some(differ)
}

#[test]
fn fresh_layouts_replay_from_their_own_lines_on_corpus() {
    let (Some(root), Ok(font_dirs)) = (
        std::env::var_os("HEEMANG_CORPUS"),
        std::env::var("HEEMANG_FONTS"),
    ) else {
        eprintln!(
            "SKIPPED replay equivalence corpus: HEEMANG_CORPUS or HEEMANG_FONTS is not set (0 documents checked)"
        );
        return;
    };
    let root = PathBuf::from(root);
    let mut docs: Vec<PathBuf> = if std::env::var_os("HEEMANG_REPLAY_ALL").is_some() {
        let mut all = Vec::new();
        walk(&root, &mut all);
        all.retain(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("hwp") || e.eq_ignore_ascii_case("hwpx"))
        });
        all.sort();
        all
    } else {
        [
            "80168_regulatory_analysis.hwp",
            "86712_regulatory_analysis.hwp",
            "footnote-01.hwp",
            "inner-table-01.hwp",
            "multi-table-001.hwp",
            "issue1937_rowbreak_footnote_overpagination.hwp",
        ]
        .iter()
        .map(|r| root.join(r))
        .collect()
    };
    docs.extend(
        std::env::split_paths(&std::env::var_os("HEEMANG_REPLAY_DOCS").unwrap_or_default())
            .filter(|p| !p.as_os_str().is_empty()),
    );
    let fonts = fonts(&font_dirs);
    let (mut checked, mut failures) = (0, Vec::new());
    for path in &docs {
        let Some(doc) = std::fs::read(path).ok().and_then(|b| read(&b)) else {
            eprintln!(
                "SKIPPED replay equivalence document: {} does not open",
                path.display()
            );
            continue;
        };
        let mut cleared = doc.clone();
        for s in &mut cleared.sections {
            empty_caches(&mut s.paras);
        }
        for (variant, d) in [("stored lines", &doc), ("empty caches", &cleared)] {
            match replay_differs(d, &fonts) {
                Some(differ) if differ.is_empty() => {}
                Some(differ) => {
                    failures.push(format!("{} ({variant}): pages {differ:?}", path.display()))
                }
                None => eprintln!("{} ({variant}): conversion refused", path.display()),
            }
        }
        checked += 1;
    }
    eprintln!("replay equivalence corpus: {checked} documents");
    assert!(checked > 0, "no corpus document was found");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn a_text_box_edit_converts_with_lines_that_cover_it_on_corpus() {
    // Typing in a text box of a long document: the conversion takes every
    // list's lines from a layout that covers what was typed.
    let Some(root) = std::env::var_os("HEEMANG_CORPUS") else {
        eprintln!("SKIPPED text box conversion: HEEMANG_CORPUS is not set");
        return;
    };
    let path = PathBuf::from(root).join("3-11월_실전_통합_2024-구분선위0미주사이7구분선아래2.hwp");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIPPED text box conversion: {} is missing", path.display());
        return;
    };
    let mut doc = hm_wasm::Engine::new().open(bytes).unwrap();
    let _ = pages(&mut doc);
    let at = json!({"section": 0, "para": 22, "offset": 9,
        "steps": [{"control": 0, "index": 0, "para": 404}]});
    doc.transact(&json!({"ops": [{"op": "insertText", "at": at, "text": "xyz"}]}).to_string())
        .unwrap();
    let _ = pages(&mut doc);
    let converted = doc
        .serialize_as("hwpx", false)
        .unwrap_or_else(|e| panic!("conversion refused: {e:?}"));
    let back = read(&converted).unwrap();
    let typed =
        hm_edit::ops::control_lists(&back.sections[0].paras[404].controls[0])[0][22].plain_text();
    assert!(typed.contains("xyz"), "{typed}");
}
