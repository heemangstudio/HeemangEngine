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
