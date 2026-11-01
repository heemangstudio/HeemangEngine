//! Our fresh line breaks against the line segments Hancom stored, over the
//! corpus.
//!
//! Needs `HEEMANG_CORPUS` (documents) and `HEEMANG_FONTS` (`:`-separated, `;` on Windows:
//! font files or directories, walked recursively); skips when either is unset.
//! The ratchet holds for Hancom's TTF and HFT fonts with its code tables:
//!
//! ```sh
//! H="/Applications/Hancom Office HWP.app/Contents/Resources/Hnc"
//! HEEMANG_CORPUS=<corpus> \
//! HEEMANG_FONTS="$H/Shared/TTF:$H/Shared/Fonts:$H/Bin/Code" \
//! cargo test --profile parity -p hm-layout --test lineseg -- --nocapture
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
    for part in std::env::split_paths(&spec).filter(|p| !p.as_os_str().is_empty()) {
        let p = part.as_path();
        if p.is_dir() {
            walk(p, &mut files)
        } else {
            files.push(p.to_path_buf())
        }
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
        if matches!(ext.as_str(), "ttf" | "otf" | "ttc" | "hft") {
            if let Ok(bytes) = std::fs::read(&f) {
                fs.add(f.display().to_string(), Arc::new(bytes));
            }
        } else if name.starts_with("u300_")
            && let Ok(bytes) = std::fs::read(&f)
        {
            fs.add_code_table(&bytes);
        } else if name == "FontMap.dat"
            && let Ok(bytes) = std::fs::read(&f)
        {
            fs.add_font_map(&bytes);
        }
    }
    fs
}

#[test]
fn breaks_agree_with_stored_segments() {
    let (Some(corpus), Some(font_spec)) = (
        std::env::var_os("HEEMANG_CORPUS"),
        std::env::var("HEEMANG_FONTS").ok(),
    ) else {
        eprintln!("HEEMANG_CORPUS or HEEMANG_FONTS is not set; skipping");
        return;
    };
    let fonts = fonts(&font_spec);
    let mut files = Vec::new();
    walk(Path::new(&corpus), &mut files);
    files.sort();
    let (mut exact, mut exact_ok, mut all, mut all_ok) = (0u64, 0u64, 0u64, 0u64);
    for f in files {
        let Ok(bytes) = std::fs::read(&f) else {
            continue;
        };
        if !hm_hwp5::is_hwp5(&bytes) {
            continue;
        }
        let Ok(doc) = hm_hwp5::read(SharedBytes::from(bytes)) else {
            continue;
        };
        // Fresh breaks: replayed stored lines would agree by construction.
        let fresh = hm_layout::Options {
            stored_lines: false,
            ..Default::default()
        };
        let layout = hm_layout::layout_with(&doc, &fonts, fresh);
        let a = hm_layout::audit(&doc, &layout, 0);
        exact += a.exact_paras as u64;
        exact_ok += a.exact_breaks_match as u64;
        all += a.paras as u64;
        all_ok += a.paras_breaks_match as u64;
    }
    let rate = |n: u64, d: u64| 100.0 * n as f64 / d.max(1) as f64;
    eprintln!(
        "fresh break agreement: {:.2}% of {exact} paragraphs with exact faces, {:.2}% of {all} overall",
        rate(exact_ok, exact),
        rate(all_ok, all)
    );
    // A ratchet: raise it as agreement improves. The protected bodies of
    // distribution documents (Hancom's own format specifications, about
    // 3,000 paragraphs at 76%) joined the corpus at 97.38%.
    assert!(
        rate(exact_ok, exact) >= 98.7,
        "break agreement fell below 98.7%"
    );
}
