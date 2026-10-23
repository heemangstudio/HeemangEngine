//! Reading checks over the HWP 3 fixtures (`HEEMANG_CORPUS`).

use hm_model::{ControlKind, Paragraph, SharedBytes};

fn hwp3_files() -> Option<Vec<std::path::PathBuf>> {
    let Some(root) = std::env::var_os("HEEMANG_CORPUS") else {
        eprintln!("HEEMANG_CORPUS is not set; skipping corpus tests");
        return None;
    };
    let mut out = Vec::new();
    let mut stack = vec![std::path::PathBuf::from(root)];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).ok()?.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if std::fs::read(&p).is_ok_and(|b| hm_hwp3::is_hwp3(&b)) {
                out.push(p);
            }
        }
    }
    out.sort();
    Some(out)
}

fn text(ps: &[Paragraph], out: &mut String) {
    for p in ps {
        out.push_str(&p.plain_text());
        for c in &p.controls {
            if let ControlKind::Table(t) = &c.kind {
                for cell in &t.cells {
                    text(&cell.paras, out);
                }
            }
        }
    }
}

#[test]
fn every_fixture_opens_with_text() {
    let Some(files) = hwp3_files() else { return };
    let mut failures = Vec::new();
    for path in &files {
        let bytes = SharedBytes::from(std::fs::read(path).unwrap());
        match hm_hwp3::read(bytes) {
            Ok(doc) => {
                let mut s = String::new();
                text(&doc.sections[0].paras, &mut s);
                let replaced = s.chars().filter(|&c| c == '\u{FFFD}').count();
                if s.trim().is_empty() {
                    failures.push(format!("{}: no text", path.display()));
                } else if replaced * 200 > s.chars().count() {
                    failures.push(format!("{}: {replaced} unknown characters", path.display()));
                }
            }
            Err(e) => failures.push(format!("{}: {e}", path.display())),
        }
    }
    eprintln!("{} HWP 3 fixtures opened", files.len() - failures.len());
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
