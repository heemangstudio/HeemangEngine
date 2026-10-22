//! Page counts against Hancom Office for Mac's PDF exports of the corpus.
//!
//! Needs `HEEMANG_CORPUS` (documents) and `HEEMANG_FONTS` (`:`-separated
//! font files or directories, walked recursively); skips when either is
//! unset. The expected counts live in `data/hancom_pages.tsv`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use hm_fonts::FontSystem;
use hm_model::SharedBytes;

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
    let mut files = Vec::new();
    for part in spec.split(':').filter(|s| !s.is_empty()) {
        walk(Path::new(part), &mut files);
    }
    files.sort();
    for f in files {
        let ext = f
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let name = f.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let Ok(bytes) = std::fs::read(&f) else {
            continue;
        };
        if matches!(ext.as_str(), "ttf" | "otf" | "ttc" | "hft") {
            fs.add(f.display().to_string(), Arc::new(bytes));
        } else if name.starts_with("u300_") {
            fs.add_code_table(&bytes);
        }
    }
    fs
}

#[test]
fn page_counts_match_hancom() {
    let (Some(corpus), Some(font_spec)) = (
        std::env::var_os("HEEMANG_CORPUS"),
        std::env::var("HEEMANG_FONTS").ok(),
    ) else {
        eprintln!("HEEMANG_CORPUS or HEEMANG_FONTS is not set; skipping");
        return;
    };
    let fonts = fonts(&font_spec);
    let table = include_str!("data/hancom_pages.tsv");
    let (mut checked, mut equal) = (0, 0);
    for line in table.lines().filter(|l| !l.starts_with('#')) {
        let Some((path, pages)) = line.split_once('\t') else {
            continue;
        };
        let pages: usize = pages.trim().parse().unwrap();
        let Ok(bytes) = std::fs::read(Path::new(&corpus).join(path)) else {
            continue;
        };
        let doc = if hm_hwp5::is_hwp5(&bytes) {
            hm_hwp5::read(SharedBytes::from(bytes)).ok()
        } else if hm_hwpx::is_hwpx(&bytes) {
            hm_hwpx::read(SharedBytes::from(bytes)).ok()
        } else {
            None
        };
        let Some(doc) = doc else { continue };
        let ours = hm_layout::layout(&doc, &fonts).pages.len();
        checked += 1;
        if ours == pages {
            equal += 1;
        } else {
            eprintln!("{ours:>4} pages, Hancom {pages:>4}: {path}");
        }
    }
    eprintln!("page counts equal to Hancom: {equal} of {checked}");
    // A ratchet: raise it as pagination improves.
    assert!(
        equal >= RATCHET,
        "{equal} of {checked} page counts match, below {RATCHET}"
    );
}

const RATCHET: usize = 277;
