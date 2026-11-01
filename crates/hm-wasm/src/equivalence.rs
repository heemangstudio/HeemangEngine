//! Incremental editing paints what a fresh layout paints. Random actions on
//! documents (typing, deletion, joins, splits, character and paragraph
//! formatting, style definitions, table structure and cells, fields,
//! bookmarks, notes, page breaks and settings, IME composition, failing
//! transactions, checkpoints, adoption, undo and redo chains), each step
//! compared with a document that lays the same content out from nothing:
//! page list, layout, page packets, line indexes and cell boxes. A receipt
//! must also name every page whose packet changed.
//!
//! The fixture variant always runs, on the crate's test fixtures, with
//! `HEEMANG_FONTS` when set and no fonts otherwise. The corpus variant
//! needs `HEEMANG_CORPUS` and `HEEMANG_FONTS` and says loudly when it skips;
//! `HEEMANG_EQUIVALENCE_DOCS` adds `:`-separated (`;` on Windows) paths and
//! `HEEMANG_EQUIVALENCE_STEPS` sets its steps per document.

use std::collections::HashMap;
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
    for dir in std::env::split_paths(fonts).filter(|d| !d.as_os_str().is_empty()) {
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

/// Every table cell's address in the model, by where it lives in memory.
fn cell_ids(doc: &hm_model::Document) -> HashMap<usize, String> {
    fn paras(ps: &[hm_model::Paragraph], at: &str, out: &mut HashMap<usize, String>) {
        for (pi, p) in ps.iter().enumerate() {
            for (ci, c) in p.controls.iter().enumerate() {
                if let Some(t) = c.as_table() {
                    for (k, cell) in t.cells.iter().enumerate() {
                        let id = format!("{at}/{pi}.{ci}.{k}");
                        out.insert(cell as *const hm_model::Cell as usize, id.clone());
                        paras(&cell.paras, &id, out);
                    }
                } else {
                    for (k, list) in hm_edit::ops::control_lists(c).into_iter().enumerate() {
                        paras(list, &format!("{at}/{pi}.{ci}:{k}"), out);
                    }
                }
            }
        }
    }
    let mut out = HashMap::new();
    for (si, s) in doc.sections.iter().enumerate() {
        paras(&s.paras, &si.to_string(), &mut out);
    }
    out
}

/// A page's cell boxes with cells named by their place in the model.
fn cells(d: &mut Inner, i: usize) -> Vec<(String, i32, i32, i32, i32)> {
    let ids = cell_ids(&d.editor.doc);
    d.index(i)
        .unwrap()
        .cells
        .iter()
        .map(|c| {
            (
                ids.get(&c.cell).cloned().unwrap_or_default(),
                c.x,
                c.y,
                c.width,
                c.height,
            )
        })
        .collect()
}

fn compare(name: &str, step: usize, what: &str, a: &mut Doc, b: &mut Doc) {
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
        assert_eq!(
            cells(a.inner().unwrap(), i),
            cells(b.inner().unwrap(), i),
            "{name}: step {step} ({what}): page {i} cells"
        );
    }
    // Line indexes found from the paths of the whole document.
    let d = a.inner().unwrap();
    let all = Paths::new(&d.editor.doc);
    for i in 0..n {
        let (boxes, cells) = hm_render::page_index(&d.editor.doc, &d.layout, &d.fonts.borrow(), i);
        let whole = PageIndex::new(boxes, cells, &all);
        let kept = d.index(i).unwrap();
        assert!(
            kept.block() == whole.block() && kept.cells == whole.cells,
            "{name}: step {step} ({what}): page {i} line index"
        );
    }
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

/// Tables anywhere in section bodies and cells, with their grid sizes.
fn tables(doc: &hm_model::Document) -> Vec<(Value, u16, u16)> {
    fn walk(
        paras: &[hm_model::Paragraph],
        section: usize,
        steps: &[Value],
        out: &mut Vec<(Value, u16, u16)>,
    ) {
        for (pi, p) in paras.iter().enumerate() {
            for (ci, c) in p.controls.iter().enumerate() {
                let Some(t) = c.as_table() else { continue };
                out.push((
                    json!({"section": section, "steps": steps, "para": pi, "control": ci}),
                    t.rows,
                    t.cols,
                ));
                if steps.len() < 2 {
                    for (k, cell) in t.cells.iter().enumerate().take(8) {
                        let mut path = steps.to_vec();
                        path.push(json!({"para": pi, "control": ci, "index": k}));
                        walk(&cell.paras, section, &path, out);
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    for (si, s) in doc.sections.iter().enumerate() {
        walk(&s.paras, si, &[], &mut out);
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

/// One thing done to the document under test.
enum Act {
    Transact(Value),
    /// A transaction that must fail and leave everything as it was.
    Fail(Value),
    Undo,
    Redo,
    Compose(Value),
    Commit,
    Cancel,
    Checkpoint,
    Restore,
    /// Takes in the original document with this transaction applied.
    Adopt(Value),
}

impl Act {
    fn label(&self) -> String {
        match self {
            Act::Transact(t) | Act::Fail(t) | Act::Compose(t) | Act::Adopt(t) => t.to_string(),
            Act::Undo => "undo".into(),
            Act::Redo => "redo".into(),
            Act::Commit => "commit".into(),
            Act::Cancel => "cancel".into(),
            Act::Checkpoint => "checkpoint".into(),
            Act::Restore => "restore".into(),
        }
    }
}

/// A random edit at a random place of a random list, as one transaction.
fn random_op(doc: &mut Doc, rng: &mut Rng) -> Option<Value> {
    let model = &doc.raw().ok()?.editor.doc;
    let ls = lists(model);
    let tabs = tables(model);
    let list = &ls[if rng.next(3) == 0 {
        0
    } else {
        rng.next(ls.len())
    }];
    let paras = paragraphs(doc, list);
    if paras.is_empty() {
        return None;
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
    let table = tabs.get(rng.next(tabs.len())).cloned();
    let op = match rng.next(22) {
        0 => {
            let text = [
                "새로운 문장을 넣습니다. ",
                "abc def ",
                "\t",
                "가나다라마바사아자차카타파하 ",
            ][rng.next(4)];
            json!({"op":"insertText","at":at,"text":text})
        }
        1 => json!({"op":"delete","from":at,"to":to}),
        2 => {
            let next = paras.iter().find(|p| p.0 == pi + 1)?;
            json!({"op":"delete","from":address(list, pi, end),"to":address(list, pi + 1, next.1)})
        }
        3 => json!({"op":"split","at":at}),
        4 => {
            let patch = [
                json!({"bold": true}),
                json!({"size": 1400}),
                json!({"italic": true, "color": 255}),
                json!({"charSpacing": -10}),
            ][rng.next(4)]
            .clone();
            json!({"op":"formatChars","from":at,"to":to,"patch":patch})
        }
        5 => {
            let patch = [
                json!({"align": "center"}),
                json!({"spaceBefore": 600}),
                json!({"lineSpacing": 200, "lineSpacingKind": "percent"}),
                json!({"left": 2000, "indent": -1000}),
            ][rng.next(4)]
            .clone();
            json!({"op":"formatParas","from":at,"to":at,"patch":patch})
        }
        // Definitions every paragraph may read: the first style's shapes.
        6 => {
            let patch = [
                json!({"characters": {"size": 1300 + 100 * rng.next(6) as i32}}),
                json!({"paragraphs": {"left": 500 * rng.next(4) as i32}}),
            ][rng.next(2)]
            .clone();
            json!({"op":"updateStyle","id":0,"style":patch})
        }
        7 => {
            let styles: Value =
                serde_json::from_str(&doc.query(r#"{"kind":"styles"}"#).ok()?).ok()?;
            let id = rng.next(styles.as_array()?.len().max(1));
            json!({"op":"applyStyle","from":at,"to":to,"styleId":id})
        }
        8 => json!({"op":"createTable","at":at,"rows":1 + rng.next(3),"columns":1 + rng.next(3)}),
        9 => {
            let (t, rows, _) = table?;
            json!({"op":"insertTableRows","table":t,"beforeRow":rng.next(rows as usize + 1),"count":1})
        }
        10 => {
            let (t, rows, _) = table?;
            json!({"op":"deleteTableRows","table":t,"startRow":rng.next(rows as usize),"count":1})
        }
        11 => {
            let (t, _, cols) = table?;
            json!({"op":"insertTableColumns","table":t,"beforeColumn":rng.next(cols as usize + 1),"count":1})
        }
        12 => {
            let (t, _, cols) = table?;
            json!({"op":"deleteTableColumns","table":t,"startColumn":rng.next(cols as usize),"count":1})
        }
        13 => {
            let (t, rows, cols) = table?;
            let (r, c) = (rng.next(rows as usize), rng.next(cols as usize));
            json!({"op":"mergeTableCells","table":t,"startRow":r,"startColumn":c,
                "endRow":(r + 1).min(rows as usize - 1),"endColumn":(c + 1).min(cols as usize - 1)})
        }
        14 => {
            let (t, rows, cols) = table?;
            json!({"op":"splitTableCell","table":t,"row":rng.next(rows as usize),
                "column":rng.next(cols as usize),"rows":1 + rng.next(2),"columns":2,"equalRowHeight":false})
        }
        15 => {
            let (t, _, _) = table?;
            json!({"op":"resizeTableCells","table":t,
                "cells":[{"index":0,"widthDelta":300 - 600 * rng.next(2) as i32,"heightDelta":200}]})
        }
        16 => {
            let kind = ["clickHere", "pageNumber", "pageCount", "date"][rng.next(4)];
            json!({"op":"insertField","at":at,"kind":kind,"text":"누름틀"})
        }
        17 => json!({"op":"insertBookmark","at":at,"name":format!("b{}", rng.next(1000))}),
        18 => {
            let kind = ["footnote", "endnote"][rng.next(2)];
            json!({"op":"insertNote","at":at,"kind":kind})
        }
        19 => json!({"op":"pageBreak","at":at,"on":rng.next(2) == 0}),
        20 => {
            let patch = [
                json!({"marginLeft": 5000 + 1000 * rng.next(4) as u32}),
                json!({"marginBottom": 3000 + 1000 * rng.next(4) as u32}),
            ][rng.next(2)]
            .clone();
            json!({"op":"setPageSettings","section":0,"patch":patch})
        }
        _ => json!({"op":"insertText","at":at,"text":"한"}),
    };
    Some(json!({ "ops": [op] }))
}

/// A random action, or a burst of them: single-character typing at a
/// moving caret, IME composition, undo and redo chains, checkpoints,
/// adoption or a failing transaction.
fn random_acts(doc: &mut Doc, rng: &mut Rng) -> Vec<Act> {
    match rng.next(14) {
        0..=2 => {
            let model = &doc.raw().unwrap().editor.doc;
            let ls = lists(model);
            let list = ls[if rng.next(3) == 0 {
                0
            } else {
                rng.next(ls.len())
            }]
            .clone();
            let paras = paragraphs(doc, &list);
            let Some((pi, lead, end, _)) = paras.get(rng.next(paras.len())).cloned() else {
                return Vec::new();
            };
            let o = lead + rng.next((end - lead) as usize + 1) as u64;
            (0..3 + rng.next(10))
                .map(|i| {
                    let text = ["가", "a", " ", "한", "1"][(i + rng.next(5)) % 5];
                    Act::Transact(
                        json!({"ops":[{"op":"insertText","at":address(&list, pi, o + i as u64),"text":text}]}),
                    )
                })
                .collect()
        }
        3 => {
            let paras = paragraphs(doc, &json!({"section": 0}));
            let Some((pi, lead, _, _)) = paras.get(rng.next(paras.len())).cloned() else {
                return Vec::new();
            };
            let at = address(&json!({"section": 0}), pi, lead);
            let mut acts = vec![
                Act::Compose(json!({"at": at, "text": "ㅎ"})),
                Act::Compose(json!({"text": "하"})),
                Act::Compose(json!({"text": "한"})),
            ];
            acts.push(if rng.next(2) == 0 {
                Act::Commit
            } else {
                Act::Cancel
            });
            acts
        }
        4 => {
            let back = 1 + rng.next(6);
            let mut acts: Vec<Act> = (0..back).map(|_| Act::Undo).collect();
            acts.extend((0..rng.next(back + 1)).map(|_| Act::Redo));
            acts
        }
        5 => vec![if rng.next(2) == 0 {
            Act::Checkpoint
        } else {
            Act::Restore
        }],
        6 => random_op(doc, rng).map_or_else(Vec::new, |t| vec![Act::Adopt(t)]),
        7 => {
            let Some(mut t) = random_op(doc, rng) else {
                return Vec::new();
            };
            t["ops"].as_array_mut().unwrap().push(
                json!({"op":"insertText","at":{"section":0,"para":1_000_000,"offset":0},"text":"x"}),
            );
            vec![Act::Fail(t)]
        }
        _ => random_op(doc, rng).map_or_else(Vec::new, |t| vec![Act::Transact(t)]),
    }
}

/// Pages of `d` as packets without epochs.
fn packets(d: &mut Doc) -> Vec<Vec<u8>> {
    let n = d.inner().unwrap().layout.pages.len();
    (0..n).map(|i| content(d.page(i).unwrap())).collect()
}

/// Runs `steps` random actions on the document in `bytes`; returns the
/// steps done.
fn check(e: &Engine, name: &str, bytes: &[u8], seed: u64, steps: usize) -> usize {
    let mut a = e.open(bytes.to_vec()).unwrap();
    let mut rng = Rng(seed);
    let mut before = packets(&mut a);
    let mut checkpoint = None;
    let mut step = 0;
    let mut tries = 0;
    while step < steps && tries < steps * 20 {
        tries += 1;
        for act in random_acts(&mut a, &mut rng) {
            let what = act.label();
            let receipt = match &act {
                Act::Transact(t) => a.transact(&t.to_string()),
                Act::Fail(t) => {
                    assert!(
                        a.transact(&t.to_string()).is_err(),
                        "{name}: {what} did not fail"
                    );
                    assert!(packets(&mut a) == before, "{name}: {what} changed pages");
                    Ok("null".into())
                }
                Act::Undo => a.undo(),
                Act::Redo => a.redo(),
                Act::Compose(t) => a.compose(&t.to_string()),
                Act::Commit => a.commit_composition("{}"),
                Act::Cancel => a.cancel_composition(),
                Act::Checkpoint => {
                    if let Ok(id) = a.checkpoint() {
                        checkpoint = Some(id);
                    }
                    continue;
                }
                Act::Restore => match checkpoint {
                    Some(id) => a.restore(id),
                    None => continue,
                },
                Act::Adopt(t) => {
                    let mut other = e.open(bytes.to_vec()).unwrap();
                    let _ = other.transact(&t.to_string());
                    a.adopt(other)
                }
            };
            let Ok(receipt) = receipt else { continue };
            let receipt: Value = serde_json::from_str(&receipt).unwrap();
            step += 1;
            let mut b = fresh(e, bytes, &a);
            compare(name, step, &what, &mut a, &mut b);
            // Pages painted differently than before are among the pages
            // the receipt names.
            let after = packets(&mut a);
            if let Some(changed) = receipt["changedPages"].as_array() {
                let changed: Vec<usize> = changed
                    .iter()
                    .map(|v| v.as_u64().unwrap() as usize)
                    .collect();
                for (i, p) in after.iter().enumerate() {
                    assert!(
                        before.get(i) == Some(p) || changed.contains(&i),
                        "{name}: step {step} ({what}): page {i} changed but not named in {changed:?}"
                    );
                }
            }
            before = after;
        }
    }
    let _ = a.cancel_composition();
    // Back to the start: the original layout.
    while a.undo().is_ok_and(|r| r != "null") {}
    let mut b = fresh(e, bytes, &a);
    compare(name, step + 1, "undo all", &mut a, &mut b);
    step
}

#[test]
fn incremental_editing_paints_like_fresh_layout_on_fixtures() {
    let fonts = std::env::var("HEEMANG_FONTS").unwrap_or_default();
    if fonts.is_empty() {
        eprintln!("equivalence fixtures: HEEMANG_FONTS is not set; laying out without fonts");
    }
    let e = engine(&fonts);
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut docs: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| matches!(p.extension().and_then(|x| x.to_str()), Some("hwp" | "hwpx")))
        .collect();
    docs.sort();
    assert!(!docs.is_empty(), "no fixtures in {}", dir.display());
    let mut total = 0;
    for (i, path) in docs.iter().enumerate() {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let steps = check(
            &e,
            &name,
            &std::fs::read(path).unwrap(),
            0x5DEE_CE66 ^ (i as u64 + 1),
            40,
        );
        total += steps;
    }
    eprintln!(
        "equivalence fixtures: {} documents, {total} steps equal",
        docs.len()
    );
}

/// A paragraph edited the same way in two documents whose definition
/// tables give its paragraph shape different meanings: after one document
/// adopts the other, the layout follows the adopted definitions.
#[test]
fn adoption_does_not_reuse_layouts_of_other_definitions() {
    let e = engine(&std::env::var("HEEMANG_FONTS").unwrap_or_default());
    let bytes = include_bytes!("../tests/fixtures/conversion-linked.hwpx").to_vec();
    let edit = |align: &str| {
        let mut d = e.open(bytes.clone()).unwrap();
        // A paragraph of its own after the first one's section definition.
        let (_, _, end, _) = paragraphs(&mut d, &json!({"section": 0}))[0].clone();
        let body = json!({"section": 0});
        let at = address(&body, 1, 0);
        d.transact(
            &json!({"ops":[{"op":"split","at":address(&body, 0, end)},
                {"op":"insertText","at":at,"text":"같은 글을 같은 자리에 넣는다"},
                {"op":"formatParas","from":at,"to":at,"patch":{"align":align,"left":2400,"right":2400}}]})
            .to_string(),
        )
        .unwrap();
        let pi = 1;
        let runs: Value = serde_json::from_str(
            &d.query(
                &json!({"kind":"formatRuns","list":{"section":0},"from":pi,"count":1}).to_string(),
            )
            .unwrap(),
        )
        .unwrap();
        (d, runs.to_string())
    };
    let (mut a, runs_a) = edit("right");
    let (b, runs_b) = edit("center");
    let shapes = |r: &str| {
        serde_json::from_str::<Value>(r).unwrap()[0]["paraShapeId"]
            .as_u64()
            .unwrap()
    };
    assert_eq!(
        shapes(&runs_a),
        shapes(&runs_b),
        "both documents add the shape under one ID"
    );
    let _ = a.pages().unwrap();
    a.adopt(b).unwrap();
    let mut fresh = fresh(&e, &bytes, &a);
    compare("conversion-linked.hwpx", 1, "adopt", &mut a, &mut fresh);
}

/// Footnote numbers in document order, each with its own automatic
/// number: `(note, automatic)`.
fn note_numbers(bytes: Vec<u8>) -> Vec<(u32, Option<u16>)> {
    let bytes = hm_model::SharedBytes::from(bytes);
    let doc = if hm_hwpx::is_hwpx(&bytes) {
        hm_hwpx::read(bytes).unwrap()
    } else {
        hm_hwp5::read(bytes).unwrap()
    };
    let mut out = Vec::new();
    for s in &doc.sections {
        for p in &s.paras {
            for c in &p.controls {
                if let hm_model::ControlKind::Note(n) = &c.kind {
                    let auto = n.list.paras.first().and_then(|q| {
                        q.controls.iter().find_map(|c| match &c.kind {
                            hm_model::ControlKind::AutoNumber(a) => Some(a.number),
                            _ => None,
                        })
                    });
                    out.push((n.number, auto));
                }
            }
        }
    }
    out
}

/// A note read from the file, renumbered by a note inserted before it,
/// lays out and saves its new number.
#[test]
fn renumbered_notes_lay_out_and_save_their_new_numbers() {
    let e = engine(&std::env::var("HEEMANG_FONTS").unwrap_or_default());
    let body = json!({"section": 0});
    for fixture in ["conversion-linked.hwpx", "query-equations.hwp"] {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(fixture);
        let mut d = e.open(std::fs::read(path).unwrap()).unwrap();
        let (_, _, end, _) = paragraphs(&mut d, &body)[0].clone();
        d.transact(
            &json!({"ops":[{"op":"insertNote","at":address(&body, 0, end),"kind":"footnote"}]})
                .to_string(),
        )
        .unwrap();
        // Read again, the note comes from the file.
        let saved = d.serialize(false).unwrap();
        let mut a = e.open(saved.clone()).unwrap();
        let _ = a.pages().unwrap();
        let (_, lead, _, _) = paragraphs(&mut a, &body)[0].clone();
        a.transact(
            &json!({"ops":[{"op":"insertNote","at":address(&body, 0, lead),"kind":"footnote"}]})
                .to_string(),
        )
        .unwrap();
        let mut b = fresh(&e, &saved, &a);
        compare(fixture, 1, "insertNote", &mut a, &mut b);
        let numbers = note_numbers(a.serialize(false).unwrap());
        assert_eq!(
            numbers.iter().map(|n| n.0).collect::<Vec<_>>(),
            [1, 2],
            "{fixture}: saved note numbers"
        );
        assert!(
            numbers
                .iter()
                .all(|&(n, auto)| auto.is_none_or(|a| a as u32 == n)),
            "{fixture}: saved automatic numbers {numbers:?}"
        );
    }
}

#[test]
fn incremental_editing_paints_like_fresh_layout_on_corpus() {
    let (Some(root), Ok(fonts)) = (
        std::env::var_os("HEEMANG_CORPUS"),
        std::env::var("HEEMANG_FONTS"),
    ) else {
        eprintln!(
            "SKIPPED equivalence corpus: HEEMANG_CORPUS or HEEMANG_FONTS is not set (0 documents checked)"
        );
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
    .collect();
    docs.extend(
        std::env::split_paths(&std::env::var_os("HEEMANG_EQUIVALENCE_DOCS").unwrap_or_default())
            .filter(|p| !p.as_os_str().is_empty()),
    );
    let e = engine(&fonts);
    let (mut checked, mut skipped, mut total) = (0, 0, 0);
    for (i, path) in docs.iter().enumerate() {
        let Ok(bytes) = std::fs::read(path) else {
            eprintln!(
                "SKIPPED equivalence corpus document: {} is missing",
                path.display()
            );
            skipped += 1;
            continue;
        };
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let done = check(
            &e,
            &name,
            &bytes,
            0x9E37_79B9_7F4A_7C15 ^ (i as u64 + 1),
            steps,
        );
        eprintln!("{name}: {done} steps equal");
        checked += 1;
        total += done;
    }
    eprintln!("equivalence corpus: {checked} documents, {total} steps equal, {skipped} skipped");
    assert!(checked > 0, "no corpus document was found");
}
