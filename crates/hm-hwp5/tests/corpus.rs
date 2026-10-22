//! Round-trip checks over the fixture corpus (`HEEMANG_CORPUS`).

use std::path::{Path, PathBuf};

use hm_hwp5::{cfb::Cfb, deflate};
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
        } else if std::fs::read(&p).is_ok_and(|b| hm_hwp5::is_hwp5(&b)) {
            out.push(p);
        }
    }
}

fn decompressed(cfb: &Cfb<'_>, path: &str, compressed: bool) -> Option<Vec<u8>> {
    let raw = cfb.read(path)?;
    Some(if compressed {
        deflate::inflate(&raw, 512 << 20).ok()?
    } else {
        raw
    })
}

#[test]
fn every_fixture_reads_and_saves_losslessly() {
    let Some(files) = corpus() else { return };
    let mut opened = 0;
    let mut failures = Vec::new();
    for path in &files {
        let bytes = SharedBytes::from(std::fs::read(path).unwrap());
        let mut doc = match hm_hwp5::read(bytes.clone()) {
            Ok(d) => d,
            Err(hm_hwp5::Error::Encrypted) => continue,
            Err(e) => {
                failures.push(format!("{}: open: {e}", path.display()));
                continue;
            }
        };
        opened += 1;

        // Level 1: a no-op save is byte-identical.
        let saved = hm_hwp5::write(&doc).unwrap();
        if saved != bytes.as_slice() {
            failures.push(format!("{}: no-op save differs", path.display()));
        }

        // Level 2: re-encoding every record reproduces every stream. Records
        // another writer cut short re-encode with defaults for the missing
        // fields, so DocInfo is exempt when it carried such records.
        let damaged_info = doc
            .diagnostics
            .iter()
            .any(|d| d.message.starts_with("DocInfo"));
        doc.mark_all_dirty();
        let touched = match hm_hwp5::write(&doc) {
            Ok(t) => t,
            Err(e) => {
                failures.push(format!("{}: touch save: {e}", path.display()));
                continue;
            }
        };
        let a = Cfb::open(&bytes).unwrap();
        let b = Cfb::open(&touched).unwrap();
        let compressed = doc.meta.compressed();
        for (p, i) in a.paths() {
            if a.entries[*i].kind != hm_hwp5::cfb::STREAM {
                continue;
            }
            if damaged_info && (p == "DocInfo" || p.starts_with("BodyText/")) {
                continue;
            }
            let structural = p == "DocInfo" || p.starts_with("BodyText/");
            let same = if structural {
                decompressed(&a, p, compressed) == decompressed(&b, p, compressed)
            } else {
                a.read(p) == b.read(p)
            };
            if !same {
                failures.push(format!("{}: touch save changed {p}", path.display()));
            }
        }
    }
    eprintln!("{opened} of {} HWP 5 fixtures opened", files.len());
    for f in &failures {
        eprintln!("{f}");
    }
    assert!(failures.is_empty(), "{} failures", failures.len());
}

/// The memo texts of a section, in document order.
fn memos(doc: &hm_model::Document) -> Vec<String> {
    doc.sections[0]
        .paras
        .iter()
        .flat_map(|p| &p.controls)
        .filter_map(|c| c.as_field()?.memo.as_ref())
        .map(|m| {
            m.paras
                .iter()
                .map(|p| String::from_utf16_lossy(&p.text))
                .collect()
        })
        .collect()
}

#[test]
fn an_edited_memo_saves_with_the_others() {
    let Some(root) = std::env::var_os("HEEMANG_CORPUS") else {
        eprintln!("HEEMANG_CORPUS is not set; skipping");
        return;
    };
    let path = Path::new(&root).join("basic/english.hwp");
    let bytes = SharedBytes::from(std::fs::read(path).unwrap());
    let mut doc = hm_hwp5::read(bytes).unwrap();
    let before = memos(&doc);
    assert!(
        before.len() > 1,
        "the fixture's memos are linked to their fields"
    );
    let control = doc.sections[0]
        .paras
        .iter_mut()
        .flat_map(|p| &mut p.controls)
        .find(|c| c.as_field().is_some_and(|f| f.memo.is_some()))
        .unwrap();
    control.src = None;
    let hm_model::ControlKind::Field(f) = &mut control.kind else {
        unreachable!()
    };
    let para = &mut f.memo.as_mut().unwrap().paras[0];
    para.text.insert(0, 'X' as u16);
    para.src = None;
    let saved = hm_hwp5::write(&doc).unwrap();
    let after = memos(&hm_hwp5::read(SharedBytes::from(saved)).unwrap());
    assert_eq!(after.len(), before.len());
    assert_eq!(after[0], format!("X{}", before[0]));
    assert_eq!(after[1..], before[1..]);
}
