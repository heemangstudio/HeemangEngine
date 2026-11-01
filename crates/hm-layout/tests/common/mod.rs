//! Shared setup for tests against Hancom's output: the corpus and font
//! locations come from `HEEMANG_CORPUS` and `HEEMANG_FONTS`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

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

/// The fonts of `HEEMANG_FONTS` (`:`-separated, `;` on Windows: files or directories, walked
/// recursively), with Hancom's HFT catalog, code tables and font map.
pub fn fonts(spec: &str) -> FontSystem {
    let mut fs = FontSystem::new();
    let mut files = Vec::new();
    for part in std::env::split_paths(&spec).filter(|p| !p.as_os_str().is_empty()) {
        let p = part.as_path();
        if p.is_dir() {
            walk(p, &mut files);
        } else {
            files.push(p.to_path_buf());
        }
    }
    files.sort();
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

/// The corpus directory and fonts, or `None` (with a message) when either
/// variable is unset.
pub fn setup() -> Option<(PathBuf, FontSystem)> {
    let (Some(corpus), Some(spec)) = (
        std::env::var_os("HEEMANG_CORPUS"),
        std::env::var("HEEMANG_FONTS").ok(),
    ) else {
        eprintln!("HEEMANG_CORPUS or HEEMANG_FONTS is not set; skipping");
        return None;
    };
    Some((PathBuf::from(corpus), fonts(&spec)))
}

/// Reads an HWP 5 or HWPX file, or `None` (with a message) when it is
/// missing.
pub fn read(path: &Path) -> Option<Document> {
    let Ok(bytes) = std::fs::read(path) else {
        eprintln!("{} is missing; skipping", path.display());
        return None;
    };
    Some(if hm_hwp5::is_hwp5(&bytes) {
        hm_hwp5::read(SharedBytes::from(bytes)).unwrap()
    } else {
        hm_hwpx::read(SharedBytes::from(bytes)).unwrap()
    })
}
