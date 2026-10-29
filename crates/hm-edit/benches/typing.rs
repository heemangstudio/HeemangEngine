//! Typing latency: 1,000 single-character inserts, each followed by an
//! incremental layout, in the administration handbook's body (also after
//! edits to 50 other paragraphs) and in a 5,000-paragraph table cell. Needs `HEEMANG_CORPUS` and `HEEMANG_FONTS`.
//!
//! ```sh
//! H="/Applications/Hancom Office HWP.app/Contents/Resources/Hnc"
//! HEEMANG_CORPUS=<corpus> HEEMANG_FONTS="$H/Shared/TTF" \
//! cargo bench --profile parity -p hm-edit --bench typing
//! ```

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use hm_edit::{Editor, ListPath, Op, Pos, Step};
use hm_fonts::FontSystem;
use hm_model::{Document, SharedBytes};

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

fn fonts(spec: &str) -> FontSystem {
    let mut fs = FontSystem::new();
    for dir in spec.split(':') {
        let mut files = Vec::new();
        walk(Path::new(dir), &mut files);
        for f in files {
            let ext = f
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if matches!(ext.as_str(), "ttf" | "otf" | "ttc") {
                fs.add(
                    f.display().to_string(),
                    Arc::new(std::fs::read(&f).unwrap()),
                );
            }
        }
    }
    fs
}

fn open(path: &Path) -> Document {
    let b = SharedBytes::from(std::fs::read(path).unwrap());
    if hm_hwpx::is_hwpx(&b) {
        hm_hwpx::read(b).unwrap()
    } else {
        hm_hwp5::read(b).unwrap()
    }
}

/// Types `n` characters at `at`, one layout after each; returns
/// milliseconds per keystroke (edit plus layout), sorted.
fn type_into(ed: &mut Editor, fs: &FontSystem, at: Pos, n: usize) -> Vec<f64> {
    ed.layout(fs);
    let mut times = Vec::with_capacity(n);
    for i in 0..n {
        let t = Instant::now();
        let op = Op::InsertText {
            at: Pos {
                offset: at.offset + i as u32,
                ..at.clone()
            },
            text: "가".into(),
        };
        ed.apply(op).unwrap();
        let l = ed.layout(fs);
        std::hint::black_box(l.pages.len());
        times.push(t.elapsed().as_secs_f64() * 1e3);
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    times
}

fn report(name: &str, t: &[f64]) {
    let q = |p: f64| t[((t.len() as f64 * p) as usize).min(t.len() - 1)];
    println!(
        "{name}: {} keys, p50 {:.2} ms, p95 {:.2} ms, max {:.2} ms",
        t.len(),
        q(0.5),
        q(0.95),
        t[t.len() - 1]
    );
}

fn main() {
    let (Some(root), Ok(spec)) = (
        std::env::var_os("HEEMANG_CORPUS"),
        std::env::var("HEEMANG_FONTS"),
    ) else {
        eprintln!("HEEMANG_CORPUS or HEEMANG_FONTS is not set; skipping");
        return;
    };
    let root = PathBuf::from(root);
    let fs = fonts(&spec);

    // Body: the first plain paragraph of the handbook with some text.
    let doc = open(&root.join("2025 행정업무운영 편람(최종).hwp"));
    let (sec, para) = doc
        .sections
        .iter()
        .enumerate()
        .flat_map(|(si, s)| s.paras.iter().enumerate().map(move |(pi, p)| (si, pi, p)))
        .find(|(_, _, p)| p.controls.is_empty() && p.text.len() > 60)
        .map(|(s, p, _)| (s as u32, p as u32))
        .unwrap();
    let mut ed = Editor::new(doc);
    let at = Pos {
        list: ListPath {
            section: sec,
            steps: Vec::new(),
        },
        para,
        offset: 4,
    };
    report("handbook body", &type_into(&mut ed, &fs, at, 1000));

    // Session: the same body paragraph after one character went into each of
    // 50 other paragraphs, as typing elsewhere in the document leaves them.
    let doc = open(&root.join("2025 행정업무운영 편람(최종).hwp"));
    let others: Vec<(u32, u32)> = doc
        .sections
        .iter()
        .enumerate()
        .flat_map(|(si, s)| s.paras.iter().enumerate().map(move |(pi, p)| (si, pi, p)))
        .filter(|(si, pi, p)| {
            p.controls.is_empty() && p.text.len() > 20 && (*si as u32, *pi as u32) != (sec, para)
        })
        .take(50)
        .map(|(s, p, _)| (s as u32, p as u32))
        .collect();
    let mut ed = Editor::new(doc);
    ed.layout(&fs);
    for (s, p) in others {
        let at = Pos {
            list: ListPath {
                section: s,
                steps: Vec::new(),
            },
            para: p,
            offset: 4,
        };
        ed.apply(Op::InsertText {
            at,
            text: "가".into(),
        })
        .unwrap();
        ed.layout(&fs);
    }
    let at = Pos {
        list: ListPath {
            section: sec,
            steps: Vec::new(),
        },
        para,
        offset: 4,
    };
    report(
        "handbook body after editing 50 paragraphs",
        &type_into(&mut ed, &fs, at, 1000),
    );

    // Cell: the giant-cell fixture's biggest cell grown to 5,000 paragraphs.
    let mut doc = open(&root.join("issue1949_giant_cell_nested_tables_perf.hwpx"));
    let mut best = (0, 0, 0, 0, 0);
    for (si, s) in doc.sections.iter().enumerate() {
        for (pi, p) in s.paras.iter().enumerate() {
            for (ci, c) in p.controls.iter().enumerate() {
                if let Some(t) = c.as_table() {
                    for (k, cell) in t.cells.iter().enumerate() {
                        if cell.paras.len() > best.0 {
                            best = (cell.paras.len(), si, pi, ci, k);
                        }
                    }
                }
            }
        }
    }
    let (_, si, pi, ci, k) = best;
    let host = &mut doc.sections[si].paras[pi];
    host.controls[ci].src = None;
    if let hm_model::ControlKind::Table(t) = &mut host.controls[ci].kind {
        let cell = &mut t.cells[k];
        let plain: Vec<_> = cell
            .paras
            .iter()
            .filter(|p| p.controls.is_empty())
            .cloned()
            .collect();
        let mut i = 0;
        while cell.paras.len() < 5000 {
            cell.paras.push(plain[i % plain.len()].clone());
            i += 1;
        }
    }
    let cell = &doc.sections[si].paras[pi].controls[ci]
        .as_table()
        .unwrap()
        .cells[k];
    let target = (cell.paras.len() / 2..cell.paras.len())
        .find(|&i| cell.paras[i].controls.is_empty() && cell.paras[i].text.len() > 20)
        .unwrap() as u32;
    let mut ed = Editor::new(doc);
    let at = Pos {
        list: ListPath {
            section: si as u32,
            steps: vec![Step {
                para: pi as u32,
                control: ci as u32,
                index: k as u32,
            }],
        },
        para: target,
        offset: 4,
    };
    report("5,000-paragraph cell", &type_into(&mut ed, &fs, at, 1000));
}
