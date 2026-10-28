//! Incremental editing paints what a fresh layout paints: random edits
//! (typing, deletion, joins, splits, character and paragraph formatting,
//! table cells) on corpus documents, each step compared with a document
//! that lays the same content out from nothing. Needs `HEEMANG_CORPUS`
//! and `HEEMANG_FONTS`; `HEEMANG_EQUIVALENCE_DOCS` adds `:`-separated
//! paths and `HEEMANG_EQUIVALENCE_STEPS` sets the steps per document.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::*;

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

fn engine(fonts: &str) -> Engine {
    let e = Engine::new();
    for dir in fonts.split(':') {
        let mut files = Vec::new();
        walk(Path::new(dir), &mut files);
        files.sort();
        for f in files {
            let ext = f
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if matches!(ext.as_str(), "ttf" | "otf" | "ttc") {
                e.add_font(&f.display().to_string(), &std::fs::read(&f).unwrap());
            }
        }
    }
    e
}

/// A small deterministic generator.
struct Rng(u64);

impl Rng {
    fn next(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n.max(1) as u64) as usize
    }
}

/// The same content laid out from nothing: no kept paragraph layouts, page
/// indexes or paths.
fn fresh(e: &Engine, bytes: &[u8], a: &Doc) -> Doc {
    let mut b = e.open(bytes.to_vec()).unwrap();
    let d = b.raw().unwrap();
    d.editor = Editor::new(a.inner.as_ref().unwrap().editor.doc.clone());
    d.laid = false;
    b
}

/// A packet without the epochs in its header.
fn content(mut p: Vec<u8>) -> Vec<u8> {
    p[12..24].fill(0);
    p
}

/// Layout facts painting may not show: line placement, tables, objects,
/// page numbers and frames.
fn facts(d: &Inner) -> Vec<String> {
    d.layout
        .pages
        .iter()
        .map(|p| {
            format!(
                "{} {} {} {:?} {:?} {:?} {:?} {:?} {} {:?} {:?} {:?}",
                p.section,
                p.width,
                p.height,
                p.body,
                p.columns,
                p.lines,
                p.objects,
                p.tables,
                p.number,
                p.rules,
                p.border,
                p.frames
                    .iter()
                    .map(|f| format!(
                        "{:?} {:?} {:?} {:?} {:?}",
                        f.kind, f.rect, f.lines, f.objects, f.tables
                    ))
                    .collect::<Vec<_>>()
            )
        })
        .collect()
}

/// Paragraph lists to edit: section bodies and the cells of tables in them,
/// nested ones included.
fn lists(doc: &hm_model::Document) -> Vec<Value> {
    fn cells(paras: &[hm_model::Paragraph], section: usize, steps: &[Value], out: &mut Vec<Value>) {
        for (pi, p) in paras.iter().enumerate() {
            for (ci, c) in p.controls.iter().enumerate() {
                let Some(t) = c.as_table() else { continue };
                for (k, cell) in t.cells.iter().enumerate().take(24) {
                    let mut path = steps.to_vec();
                    path.push(json!({"para": pi, "control": ci, "index": k}));
                    out.push(json!({"section": section, "steps": path}));
                    if path.len() < 3 {
                        cells(&cell.paras, section, &path, out);
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    for (si, s) in doc.sections.iter().enumerate() {
        out.push(json!({"section": si}));
        cells(&s.paras, si, &[], &mut out);
    }
    out
}

fn address(list: &Value, para: u64, offset: u64) -> Value {
    let mut a = list.clone();
    a["para"] = json!(para);
    a["offset"] = json!(offset);
    a
}

/// The paragraphs of a list: `(index, caret start, caret end, offsets)`.
fn paragraphs(doc: &mut Doc, list: &Value) -> Vec<(u64, u64, u64, Vec<u64>)> {
    let Ok(s) = doc.query(&json!({"kind":"paragraphs","list":list}).to_string()) else {
        return Vec::new();
    };
    let v: Value = serde_json::from_str(&s).unwrap();
    v.as_array()
        .into_iter()
        .flatten()
        .map(|p| {
            let c = p["caret"].as_array().unwrap();
            let offsets = p["offsets"]
                .as_array()
                .unwrap()
                .iter()
                .map(|o| o.as_u64().unwrap())
                .collect();
            (
                p["index"].as_u64().unwrap(),
                c[0].as_u64().unwrap(),
                c[1].as_u64().unwrap(),
                offsets,
            )
        })
        .collect()
}

/// A random edit, or a burst of single-character typing, as transactions.
fn random_edit(doc: &mut Doc, lists: &[Value], rng: &mut Rng) -> Vec<Value> {
    let list = &lists[if rng.next(3) == 0 {
        0
    } else {
        rng.next(lists.len())
    }];
    let paras = paragraphs(doc, list);
    if paras.is_empty() {
        return Vec::new();
    }
    let (pi, lead, end, offsets) = paras[rng.next(paras.len())].clone();
    let inside: Vec<u64> = offsets
        .iter()
        .copied()
        .filter(|&o| o >= lead && o <= end)
        .collect();
    let o = inside.get(rng.next(inside.len())).copied().unwrap_or(lead);
    let at = address(list, pi, o);
    let to = address(
        list,
        pi,
        inside
            .iter()
            .copied()
            .find(|&x| x > o + rng.next(8) as u64)
            .unwrap_or(end),
    );
    let one = |op: Value| vec![json!({ "ops": [op] })];
    match rng.next(10) {
        // Typing: one transaction per keystroke at a moving caret.
        0..=2 => {
            let n = 3 + rng.next(12);
            (0..n)
                .map(|i| {
                    let text = ["가", "a", " ", "한", "1"][(i + rng.next(5)) % 5];
                    json!({"ops":[{"op":"insertText","at":address(list, pi, o + i as u64),"text":text}]})
                })
                .collect()
        }
        3 => {
            let text = [
                "새로운 문장을 넣습니다. ",
                "abc def ",
                "\t",
                "가나다라마바사아자차카타파하 ",
            ][rng.next(4)];
            one(json!({"op":"insertText","at":at,"text":text}))
        }
        4 => one(json!({"op":"delete","from":at,"to":to})),
        5 => {
            let Some(next) = paras.iter().find(|p| p.0 == pi + 1) else {
                return Vec::new();
            };
            one(
                json!({"op":"delete","from":address(list, pi, end),"to":address(list, pi + 1, next.1)}),
            )
        }
        6 => one(json!({"op":"split","at":at})),
        7 => {
            let patch = [
                json!({"bold": true}),
                json!({"size": 1400}),
                json!({"italic": true, "color": 255}),
                json!({"charSpacing": -10}),
            ][rng.next(4)]
            .clone();
            one(json!({"op":"formatChars","from":at,"to":to,"patch":patch}))
        }
        8 => {
            let patch = [
                json!({"align": "center"}),
                json!({"spaceBefore": 600}),
                json!({"lineSpacing": 200, "lineSpacingKind": "percent"}),
                json!({"left": 2000}),
            ][rng.next(4)]
            .clone();
            one(json!({"op":"formatParas","from":at,"to":at,"patch":patch}))
        }
        _ => {
            // Undo the last step, then redo it.
            vec![json!("undo"), json!("redo")]
        }
    }
}

fn compare(name: &str, step: usize, what: &Value, a: &mut Doc, b: &mut Doc) {
    assert_eq!(
        a.pages().unwrap(),
        b.pages().unwrap(),
        "{name}: step {step} ({what}): page list"
    );
    let (fa, fb) = (facts(a.inner().unwrap()), facts(b.inner().unwrap()));
    for (i, (x, y)) in fa.iter().zip(&fb).enumerate() {
        assert!(
            x == y,
            "{name}: step {step} ({what}): page {i} layout\n inc: {x}\n new: {y}"
        );
    }
    let n = a.inner().unwrap().layout.pages.len();
    for i in 0..n {
        assert!(
            content(a.page(i).unwrap()) == content(b.page(i).unwrap()),
            "{name}: step {step} ({what}): page {i} packet"
        );
    }
    // Line indexes found from the paths of the whole document.
    let d = a.inner().unwrap();
    let all = Paths::new(&d.editor.doc);
    for i in 0..n {
        let (boxes, cells) =
            hm_render::page_index(&d.editor.doc, &d.layout, &d.fonts.borrow(), i);
        let whole = PageIndex::new(boxes, cells, &all).block();
        assert!(
            d.index(i).unwrap().block() == whole,
            "{name}: step {step} ({what}): page {i} line index"
        );
    }
}

fn check(e: &Engine, path: &Path, seed: u64, steps: usize) {
    let name = path.file_name().unwrap().to_string_lossy().to_string();
    let bytes = std::fs::read(path).unwrap();
    let mut a = e.open(bytes.clone()).unwrap();
    let lists = lists(&a.raw().unwrap().editor.doc);
    let mut rng = Rng(seed);
    let mut before: Vec<Vec<u8>> = Vec::new();
    let pages = |d: &mut Doc| -> Vec<Vec<u8>> {
        let n = d.inner().unwrap().layout.pages.len();
        (0..n).map(|i| content(d.page(i).unwrap())).collect()
    };
    let mut step = 0;
    let mut tries = 0;
    while step < steps && tries < steps * 20 {
        tries += 1;
        for t in random_edit(&mut a, &lists, &mut rng) {
            if before.is_empty() {
                before = pages(&mut a);
            }
            let receipt = match t.as_str() {
                Some("undo") => a.undo(),
                Some("redo") => a.redo(),
                _ => a.transact(&t.to_string()),
            };
            let Ok(receipt) = receipt else { continue };
            let receipt: Value = serde_json::from_str(&receipt).unwrap();
            if receipt.is_null() {
                continue;
            }
            step += 1;
            let mut b = fresh(e, &bytes, &a);
            compare(&name, step, &t, &mut a, &mut b);
            // Pages painted differently than before are among the pages
            // the receipt names.
            let after = pages(&mut a);
            if let Some(changed) = receipt["changedPages"].as_array() {
                let changed: Vec<usize> = changed
                    .iter()
                    .map(|v| v.as_u64().unwrap() as usize)
                    .collect();
                for (i, p) in after.iter().enumerate() {
                    assert!(
                        before.get(i) == Some(p) || changed.contains(&i),
                        "{name}: step {step} ({t}): page {i} changed but not named in {changed:?}"
                    );
                }
            }
            before = after;
        }
    }
    // Back to the start: the original layout.
    while a.undo().is_ok_and(|r| r != "null") {}
    let mut b = fresh(e, &bytes, &a);
    compare(&name, step + 1, &json!("undo all"), &mut a, &mut b);
    eprintln!("{name}: {step} steps equal");
}

#[test]
fn incremental_editing_paints_like_fresh_layout() {
    let (Some(root), Ok(fonts)) = (
        std::env::var_os("HEEMANG_CORPUS"),
        std::env::var("HEEMANG_FONTS"),
    ) else {
        eprintln!("HEEMANG_CORPUS or HEEMANG_FONTS is not set; skipping");
        return;
    };
    let root = PathBuf::from(root);
    let steps = std::env::var("HEEMANG_EQUIVALENCE_STEPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(24);
    let mut docs: Vec<PathBuf> = [
        "footnote-01.hwp",
        "2022년 국립국어원 업무계획.hwp",
        "hwpx/2024년 1분기 해외직접투자 보도자료 ff.hwpx",
        "issue1949_giant_cell_nested_tables_perf.hwpx",
        "inner-table-01.hwp",
        "multi-table-001.hwp",
        "issue2063_huge_cellbreak_table.hwp",
        "issue1937_rowbreak_footnote_overpagination.hwp",
    ]
    .iter()
    .map(|r| root.join(r))
    .filter(|p| p.exists())
    .collect();
    docs.extend(
        std::env::var("HEEMANG_EQUIVALENCE_DOCS")
            .unwrap_or_default()
            .split(':')
            .filter(|s| !s.is_empty())
            .map(PathBuf::from),
    );
    let e = engine(&fonts);
    for (i, doc) in docs.iter().enumerate() {
        check(&e, doc, 0x9E37_79B9_7F4A_7C15 ^ (i as u64 + 1), steps);
    }
}
