//! Reading and saving checks over the HWPX fixtures (`HEEMANG_CORPUS`).

use std::path::{Path, PathBuf};

use hm_model::SharedBytes;

fn corpus() -> Option<Vec<PathBuf>> {
    let Some(root) = std::env::var_os("HEEMANG_CORPUS") else {
        eprintln!("HEEMANG_CORPUS is not set; skipping corpus tests");
        return None;
    };
    let mut out = Vec::new();
    collect(Path::new(&root), &mut out);
    out.sort();
    Some(out)
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(&p, out);
        } else if std::fs::read(&p).is_ok_and(|b| hm_hwpx::is_hwpx(&b)) {
            out.push(p);
        }
    }
}

#[test]
fn every_fixture_reads_and_saves_unchanged() {
    let Some(files) = corpus() else { return };
    let mut opened = 0;
    let mut failures = Vec::new();
    for path in &files {
        let bytes = SharedBytes::from(std::fs::read(path).unwrap());
        let doc = match hm_hwpx::read(bytes.clone()) {
            Ok(d) => d,
            Err(e) => {
                failures.push(format!("{}: open: {e}", path.display()));
                continue;
            }
        };
        opened += 1;
        if doc.sections.is_empty() {
            failures.push(format!("{}: no sections", path.display()));
        }
        match hm_hwpx::write(&doc) {
            Ok(saved) if saved[..] == bytes[..] => {}
            Ok(_) => failures.push(format!("{}: no-op save differs", path.display())),
            Err(e) => failures.push(format!("{}: save: {e}", path.display())),
        }
    }
    eprintln!("{opened} of {} HWPX fixtures opened", files.len());
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The model as text with source spans left out: what a save must keep.
fn model_text(doc: &hm_model::Document) -> String {
    let s = format!("{:?}{:?}", doc.info, doc.sections);
    let mut out = String::with_capacity(s.len());
    let mut rest = s.as_str();
    while let Some(i) = rest.find("src: ") {
        out.push_str(&rest[..i]);
        rest = &rest[i + 5..];
        if let Some(r) = rest.strip_prefix("None") {
            rest = r;
        } else if let Some(j) = rest.find("))") {
            rest = &rest[j + 2..];
        }
    }
    out.push_str(rest);
    out
}

#[test]
fn every_fixture_touch_saves_to_an_equal_model() {
    let Some(files) = corpus() else { return };
    let mut failures = Vec::new();
    let mut checked = 0;
    for path in &files {
        let bytes = SharedBytes::from(std::fs::read(path).unwrap());
        let Ok(mut doc) = hm_hwpx::read(bytes) else {
            continue;
        };
        let before = model_text(&doc);
        doc.mark_all_dirty();
        let saved = match hm_hwpx::write(&doc) {
            Ok(s) => s,
            Err(e) => {
                failures.push(format!("{}: save: {e}", path.display()));
                continue;
            }
        };
        match hm_hwpx::read(SharedBytes::from(saved)) {
            Ok(again) if model_text(&again) == before => checked += 1,
            Ok(_) => failures.push(format!("{}: model changed", path.display())),
            Err(e) => failures.push(format!("{}: reopen: {e}", path.display())),
        }
    }
    eprintln!("{checked} HWPX fixtures touch-saved to an equal model");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
