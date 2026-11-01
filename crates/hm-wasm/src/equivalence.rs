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

/// The controls of a paragraph of `list` as `(kind, index)`.
fn controls(doc: &mut Doc, list: &Value, para: u64) -> Vec<(String, u64)> {
    let Ok(s) =
        doc.query(&json!({"kind":"paragraphs","list":list,"from":para,"count":1}).to_string())
    else {
        return Vec::new();
    };
    let v: Value = serde_json::from_str(&s).unwrap();
    v[0]["controls"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| {
            (
                c["kind"].as_str().unwrap_or("").to_string(),
                c["index"].as_u64().unwrap_or(0),
            )
        })
        .collect()
}

/// A body paragraph without controls and with some text whose first line
/// sits on or after the page `fraction` of the way through the document:
/// its section, index and a caret offset inside its text.
fn late_paragraph(doc: &mut Doc, fraction: f64) -> Option<(u32, u64, u64)> {
    let d = doc.inner().ok()?;
    let n = d.layout.pages.len();
    let from = ((n as f64 * fraction) as usize).min(n.saturating_sub(1));
    let mut seen = Vec::new();
    for page in &d.layout.pages[from..] {
        for l in page.lines.iter().filter(|l| l.line == 0) {
            let p = &d.editor.doc.sections[page.section as usize].paras[l.para as usize];
            if p.controls.is_empty() && p.text.len() > 12 {
                seen.push((page.section, l.para as u64));
            }
        }
        if !seen.is_empty() {
            break;
        }
    }
    let (section, para) = *seen.first()?;
    let (_, lead, end, offsets) = paragraphs(doc, &json!({ "section": section }))
        .into_iter()
        .find(|p| p.0 == para)?;
    let inside: Vec<u64> = offsets
        .into_iter()
        .filter(|&o| o > lead && o < end)
        .collect();
    Some((section, para, *inside.get(inside.len() / 2)?))
}

/// Scripted edits on the later pages of long documents, where a layout
/// goes on from a page of the earlier one and meets it again further on:
/// typing, a paragraph split and join, a footnote and its text, keep-with-
/// next and keep-lines chains, a page break, a footer showing the total page
/// count (also after an edit that changes the count), table rows that move a
/// table across a page end, a total page count in body text, and undoing
/// all of it. Each step lays out as a fresh layout does. Returns the steps
/// done, the steps that took paragraphs from the earlier layout and the
/// steps whose pagination met the earlier layout again.
fn late(e: &Engine, name: &str, bytes: &[u8]) -> (usize, usize, usize) {
    let mut a = e.open(bytes.to_vec()).unwrap();
    let _ = packets(&mut a);
    let Some((sec, p, o)) = late_paragraph(&mut a, 0.6) else {
        eprintln!("{name}: no plain paragraph on the later pages; skipped");
        return (0, 0, 0);
    };
    let (near_sec, q, qo) = late_paragraph(&mut a, 0.35).unwrap_or((sec, p, o));
    let body = json!({ "section": sec });
    let at = |para: u64, offset: u64| address(&json!({ "section": sec }), para, offset);
    let near = |para: u64, offset: u64| address(&json!({ "section": near_sec }), para, offset);
    let mut steps = 0;
    let (mut took, mut met) = (0, 0);
    let mut run = |a: &mut Doc, what: &str, ops: Value| -> bool {
        let r = a.transact(&json!({ "ops": ops }).to_string());
        if r.is_err() {
            eprintln!("{name}: {what} failed: {}", r.unwrap_err().message());
            return false;
        }
        steps += 1;
        let (_, meeting, paras) = a.raw().unwrap().editor.layout_taken();
        took += usize::from(paras > 0);
        met += usize::from(meeting > 0);
        let mut b = fresh(e, bytes, a);
        compare(name, steps, what, a, &mut b);
        true
    };
    run(
        &mut a,
        "type",
        json!([{"op":"insertText","at":at(p, o),"text":"가"}]),
    );
    run(
        &mut a,
        "type more",
        json!([{"op":"insertText","at":at(p, o + 1),"text":"나다 "}]),
    );
    let long = "새로운 문장을 넣어 줄을 늘립니다. ".repeat(12);
    run(
        &mut a,
        "add lines",
        json!([{"op":"insertText","at":at(p, o),"text":long}]),
    );
    run(&mut a, "split", json!([{"op":"split","at":at(p, o)}]));
    if let Some((_, lead, _, _)) = paragraphs(&mut a, &body).into_iter().find(|x| x.0 == p + 1) {
        let end = paragraphs(&mut a, &body)
            .into_iter()
            .find(|x| x.0 == p)
            .map_or(o, |x| x.2);
        run(
            &mut a,
            "join",
            json!([{"op":"delete","from":at(p, end),"to":at(p + 1, lead)}]),
        );
    }
    if run(
        &mut a,
        "footnote",
        json!([{"op":"insertNote","at":at(p, o),"kind":"footnote"}]),
    ) {
        if let Some((_, c)) = controls(&mut a, &body, p)
            .into_iter()
            .find(|c| c.0 == "footnote")
        {
            let note = json!({"section": sec, "steps": [{"para": p, "control": c, "index": 0}]});
            let text = "각주 내용을 길게 적습니다. ".repeat(8);
            run(
                &mut a,
                "footnote text",
                json!([{"op":"insertText","at":address(&note, 0, 0),"text":text}]),
            );
        }
    }
    run(
        &mut a,
        "keep chain",
        json!([{"op":"formatParas","from":near(q, qo),"to":near(q + 2, 0),
            "patch":{"keepWithNext": true, "keepLines": true}}]),
    );
    run(
        &mut a,
        "page break",
        json!([{"op":"pageBreak","at":near(q, qo),"on":true}]),
    );
    run(
        &mut a,
        "footer",
        json!([{"op":"createHeaderFooter","section":0,"kind":"footer","side":"both"}]),
    );
    let first = json!({ "section": 0 });
    let footer = (0..3u64).find_map(|para| {
        controls(&mut a, &first, para)
            .into_iter()
            .find(|c| c.0 == "footer")
            .map(
                |(_, c)| json!({"section": 0, "steps": [{"para": para, "control": c, "index": 0}]}),
            )
    });
    if let Some(footer) = footer {
        run(
            &mut a,
            "total in footer",
            json!([{"op":"insertField","at":address(&footer, 0, 0),"kind":"pageCount"}]),
        );
        let more = "쪽 수를 바꾸는 긴 문단입니다. ".repeat(60);
        run(
            &mut a,
            "change page count",
            json!([{"op":"insertText","at":near(q, qo),"text":more}]),
        );
        run(
            &mut a,
            "type after count",
            json!([{"op":"insertText","at":at(p, o),"text":"라"}]),
        );
    }
    // A table on a later page grows across the page end.
    let tables = tables(&a.raw().unwrap().editor.doc);
    if let Some((t, _, _)) = tables.iter().find(|(t, _, _)| {
        t["steps"].as_array().is_some_and(|s| s.is_empty()) && t["para"].as_u64() > Some(q)
    }) {
        run(
            &mut a,
            "table rows",
            json!([{"op":"insertTableRows","table":t,"beforeRow":0,"count":6}]),
        );
    }
    run(
        &mut a,
        "total in body",
        json!([{"op":"insertField","at":at(p, o),"kind":"pageCount"}]),
    );
    run(
        &mut a,
        "type with body total",
        json!([{"op":"insertText","at":at(p, o),"text":"마"}]),
    );
    // Back through every step.
    while a.undo().is_ok_and(|r| r != "null") {
        steps += 1;
        let mut b = fresh(e, bytes, &a);
        compare(name, steps, "undo", &mut a, &mut b);
    }
    (steps, took, met)
}

#[test]
fn layouts_going_on_from_a_later_page_match_fresh_layouts() {
    let (Some(root), Ok(fonts)) = (
        std::env::var_os("HEEMANG_CORPUS"),
        std::env::var("HEEMANG_FONTS"),
    ) else {
        eprintln!("SKIPPED later-page equivalence: HEEMANG_CORPUS or HEEMANG_FONTS is not set");
        return;
    };
    let root = PathBuf::from(root);
    let mut docs: Vec<PathBuf> = match std::env::var("HEEMANG_LATE_DOCS") {
        Ok(list) => list
            .split(':')
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .collect(),
        Err(_) => [
            "2025 행정업무운영 편람(최종).hwp",
            "hwp3-sample-hwpx.hwpx",
            "footnote-01.hwp",
            "issue1937_rowbreak_footnote_overpagination.hwp",
            "issue2063_huge_cellbreak_table.hwp",
            "multi-table-001.hwp",
        ]
        .iter()
        .map(|r| root.join(r))
        .collect(),
    };
    docs.retain(|p| p.exists());
    let e = engine(&fonts);
    let (mut total, mut took_any, mut met_any) = (0, 0, 0);
    for path in &docs {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let (steps, took, met) = late(&e, &name, &std::fs::read(path).unwrap());
        eprintln!(
            "{name}: {steps} later-page steps equal, {took} took paragraphs from the earlier layout, {met} met it again"
        );
        total += steps;
        took_any += took;
        met_any += met;
    }
    eprintln!(
        "later-page equivalence: {} documents, {total} steps equal",
        docs.len()
    );
    assert!(!docs.is_empty(), "no later-page document was found");
    assert!(took_any > 0, "no step took pages from an earlier layout");
    assert!(met_any > 0, "no step met an earlier layout again");
}

/// A document laid out a few pages at a time from its first pages: the
/// pages each step serves are the complete layout's pages, each step names
/// the pages it changed, and the finished layout equals a fresh one. Also
/// with a footer showing the total page count, and when another request
/// completes the layout early.
fn progressive(e: &Engine, name: &str, bytes: &[u8]) -> usize {
    let mut whole = e.open(bytes.to_vec()).unwrap();
    let full = packets(&mut whole);
    let mut a = e.open(bytes.to_vec()).unwrap();
    let first: Value = serde_json::from_str(&a.layout_first(2).unwrap()).unwrap();
    // Painting the first pages reads their pictures without completing the
    // layout.
    let _ = a.picture_source(0, 0);
    let _ = a.picture_bytes(0, 0);
    assert!(
        first["complete"].as_bool() == Some(true) || a.inner.as_ref().unwrap().partial,
        "{name}: picture reads completed the layout"
    );
    let mut shown: Vec<Vec<u8>> = Vec::new();
    let mut steps = 0;
    let mut progress = first;
    loop {
        steps += 1;
        let n = progress["pageCount"].as_u64().unwrap() as usize;
        let state: Value = serde_json::from_str(&a.state().unwrap()).unwrap();
        assert_eq!(
            state["pageCount"].as_u64(),
            Some(n as u64),
            "{name}: state page count"
        );
        let complete = progress["complete"].as_bool().unwrap();
        assert_eq!(
            state["layoutComplete"].as_bool(),
            Some(complete),
            "{name}: state completeness"
        );
        let changed: Vec<usize> = progress["changedPages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as usize)
            .collect();
        for i in 0..n {
            let p = content(a.page(i).unwrap());
            assert!(
                shown.get(i) == Some(&p) || changed.contains(&i),
                "{name}: step {steps}: page {i} changed but not named in {changed:?}"
            );
            if shown.len() <= i {
                shown.push(p);
            } else {
                shown[i] = p;
            }
        }
        if complete {
            break;
        }
        // Body pages laid out so far are those of the complete layout.
        let d = a.inner.as_ref().unwrap();
        assert!(
            d.partial,
            "{name}: incomplete progress on a complete layout"
        );
        progress = serde_json::from_str(&a.layout_more(3).unwrap()).unwrap();
    }
    assert!(
        shown == full,
        "{name}: progressive pages differ from the complete layout"
    );
    let mut b = fresh(e, bytes, &a);
    compare(name, steps, "progressive", &mut a, &mut b);
    // Completed early by a request that needs the whole layout.
    let mut c = e.open(bytes.to_vec()).unwrap();
    let _ = c.layout_first(1).unwrap();
    let _ = c.query(r#"{"kind":"paragraphs","list":{"section":0},"from":0,"count":1}"#);
    let _ = c.locate(&json!({"kind":"caret","position":{"section":0,"para":0,"offset":0},"affinity":"downstream"}).to_string());
    let mut b = fresh(e, bytes, &c);
    compare(name, steps, "completed by a request", &mut c, &mut b);
    steps
}

#[test]
fn progressive_layouts_serve_the_complete_layout_pages() {
    let (Some(root), Ok(fonts)) = (
        std::env::var_os("HEEMANG_CORPUS"),
        std::env::var("HEEMANG_FONTS"),
    ) else {
        eprintln!("SKIPPED progressive layout: HEEMANG_CORPUS or HEEMANG_FONTS is not set");
        return;
    };
    let root = PathBuf::from(root);
    let e = engine(&fonts);
    let mut docs: Vec<(String, Vec<u8>)> = [
        "2025 행정업무운영 편람(최종).hwp",
        "hwp3-sample-hwpx.hwpx",
        "footnote-01.hwp",
        "issue1937_rowbreak_footnote_overpagination.hwp",
        "issue2063_huge_cellbreak_table.hwp",
    ]
    .iter()
    .filter_map(|r| Some((r.to_string(), std::fs::read(root.join(r)).ok()?)))
    .collect();
    // The handbook with a footer showing the total page count.
    if let Some((_, bytes)) = docs.first().cloned() {
        let mut d = e.open(bytes).unwrap();
        d.transact(
            &json!({"ops":[{"op":"createHeaderFooter","section":0,"kind":"footer","side":"both"}]})
                .to_string(),
        )
        .unwrap();
        let first = json!({ "section": 0 });
        let footer = (0..3u64).find_map(|para| {
            controls(&mut d, &first, para)
                .into_iter()
                .find(|c| c.0 == "footer")
                .map(|(_, c)| json!({"section": 0, "steps": [{"para": para, "control": c, "index": 0}]}))
        });
        if let Some(footer) = footer {
            d.transact(
                &json!({"ops":[{"op":"insertField","at":address(&footer, 0, 0),"kind":"pageCount"}]})
                    .to_string(),
            )
            .unwrap();
            docs.push((
                "handbook with total in footer".into(),
                d.serialize(false).unwrap(),
            ));
        }
    }
    assert!(!docs.is_empty(), "no progressive layout document was found");
    for (name, bytes) in &docs {
        let steps = progressive(&e, name, bytes);
        eprintln!("{name}: {steps} progressive steps equal");
    }
}

/// Typing into a header or footer of the first section, which later
/// sections draw too until they set their own: every section showing it
/// lays out as a fresh layout does. Returns the steps done.
fn furniture_typing(e: &Engine, name: &str, bytes: &[u8]) -> usize {
    let mut a = e.open(bytes.to_vec()).unwrap();
    let _ = packets(&mut a);
    let first = json!({ "section": 0 });
    let mut steps = 0;
    let mut check = |a: &mut Doc, what: &str| {
        steps += 1;
        let mut b = fresh(e, bytes, a);
        compare(name, steps, what, a, &mut b);
    };
    for kind in ["header", "footer"] {
        let found = (0..3u64).find_map(|para| {
            controls(&mut a, &first, para)
                .into_iter()
                .find(|c| c.0 == kind)
                .map(|(_, c)| json!({"section": 0, "steps": [{"para": para, "control": c, "index": 0}]}))
        });
        let list = match found {
            Some(list) => list,
            None => {
                let op = json!({"ops":[{"op":"createHeaderFooter","section":0,"kind":kind,"side":"both"}]});
                if a.transact(&op.to_string()).is_err() {
                    continue;
                }
                check(&mut a, "create");
                let Some(list) = (0..3u64).find_map(|para| {
                    controls(&mut a, &first, para)
                        .into_iter()
                        .find(|c| c.0 == kind)
                        .map(|(_, c)| json!({"section": 0, "steps": [{"para": para, "control": c, "index": 0}]}))
                }) else {
                    continue;
                };
                list
            }
        };
        for text in ["머리글 글자", "가"] {
            let op = json!({"ops":[{"op":"insertText","at":address(&list, 0, 0),"text":text}]});
            if a.transact(&op.to_string()).is_ok() {
                check(&mut a, kind);
            }
        }
    }
    steps
}

#[test]
fn typing_into_furniture_reaches_later_sections() {
    let (Some(root), Ok(fonts)) = (
        std::env::var_os("HEEMANG_CORPUS"),
        std::env::var("HEEMANG_FONTS"),
    ) else {
        eprintln!("SKIPPED furniture typing: HEEMANG_CORPUS or HEEMANG_FONTS is not set");
        return;
    };
    let root = PathBuf::from(root);
    let e = engine(&fonts);
    let mut checked = 0;
    for name in [
        "2025 행정업무운영 편람(최종).hwp",
        "2025 행정업무운영 편람(최종).hwpx",
    ] {
        let Ok(bytes) = std::fs::read(root.join(name)) else {
            continue;
        };
        let steps = furniture_typing(&e, name, &bytes);
        eprintln!("{name}: {steps} furniture steps equal");
        checked += 1;
    }
    assert!(checked > 0, "no furniture typing document was found");
}
