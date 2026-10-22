//! Page counts against Hancom Office for Mac's PDF exports of the corpus.
//!
//! Needs `HEEMANG_CORPUS` (documents) and `HEEMANG_FONTS` (`:`-separated
//! font files or directories, walked recursively); skips when either is
//! unset. The expected counts live in `data/hancom_pages.tsv`. The ratchet
//! holds for Hancom's TTF and HFT fonts with its code tables:
//!
//! ```sh
//! H="/Applications/Hancom Office HWP.app/Contents/Resources/Hnc"
//! HEEMANG_CORPUS=<heemang-cleanroom>/fixtures/samples \
//! HEEMANG_FONTS="$H/Shared/TTF:$H/Shared/Fonts:$H/Bin/Code" \
//! cargo test --profile parity -p hm-layout --test pagination -- --nocapture
//! ```

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
    // Hancom's HFT catalog tells the styles of HFT files apart, so it goes
    // in before them.
    if let Some(info) = files
        .iter()
        .find(|f| f.file_name().is_some_and(|n| n == "hftinfo.dat"))
        && let Ok(bytes) = std::fs::read(info)
    {
        fs.add_hft_info(&bytes);
    }
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
        } else if name == "FontMap.dat" {
            fs.add_font_map(&bytes);
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

const RATCHET: usize = 302;

/// Hancom breaks a line holding an automatic page number with the number it
/// stored, so every page of `probes/page-number-ladder.hwpx` (a ladder of
/// paragraphs ending in the page number, one page each, 101 pages) breaks
/// its paragraphs the same way while showing its own number.
#[test]
fn page_numbers_in_text_keep_the_stored_line_breaks() {
    let (Some(corpus), Some(font_spec)) = (
        std::env::var_os("HEEMANG_CORPUS"),
        std::env::var("HEEMANG_FONTS").ok(),
    ) else {
        eprintln!("HEEMANG_CORPUS or HEEMANG_FONTS is not set; skipping");
        return;
    };
    let path = Path::new(&corpus).join("../probes/page-number-ladder.hwpx");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("{} is missing; skipping", path.display());
        return;
    };
    let doc = hm_hwpx::read(SharedBytes::from(bytes)).unwrap();
    let layout = hm_layout::layout(&doc, &fonts(&font_spec));
    assert_eq!(layout.pages.len(), 101);
    let paras = &doc.sections[0].paras;
    let mut first: Option<Vec<usize>> = None;
    for (k, page) in layout.pages.iter().enumerate() {
        // Lines per paragraph on the page, in order.
        let mut counts: Vec<(u32, usize)> = Vec::new();
        for l in &page.lines {
            match counts.last_mut() {
                Some((p, n)) if *p == l.para => *n += 1,
                _ => counts.push((l.para, 1)),
            }
        }
        let lines: Vec<usize> = counts.iter().map(|c| c.1).collect();
        assert_eq!(lines, *first.get_or_insert(lines.clone()), "page {}", k + 1);
        // The number shown is the page's.
        let pi = counts[0].0 as usize;
        let pl = layout.para(0, pi as u32);
        let slot = paras[pi]
            .units()
            .find_map(|(pos, u)| match u {
                hm_model::Unit::Extended { index, .. } => matches!(
                    paras[pi].controls[index as usize].kind,
                    hm_model::ControlKind::AutoNumber(_)
                )
                .then_some(pos),
                _ => None,
            })
            .unwrap();
        let shown: String = pl
            .measured
            .items
            .iter()
            .filter(|(pos, _)| (slot..slot + 8).contains(pos))
            .filter_map(|(_, kind)| match kind {
                hm_layout::Kind::Char(c) => Some(*c),
                _ => None,
            })
            .collect();
        assert_eq!(shown, (k + 1).to_string());
    }
}
