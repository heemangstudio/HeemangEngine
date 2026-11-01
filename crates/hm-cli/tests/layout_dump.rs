//! `layout-dump` lists only text a page paints: text Hancom leaves out of
//! print and glyphs a clip hides entirely are left out of `drawn`.
//!
//! Needs `HEEMANG_CORPUS` (the fixture samples) and `HEEMANG_FONTS`
//! (`:`-separated, `;` on Windows: font files or directories); skips when either is unset.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The `drawn` texts of each page of `doc`, or `None` when the fixtures or
/// fonts are not configured.
fn drawn(doc: &str) -> Option<Vec<Vec<String>>> {
    let (Some(corpus), Some(fonts)) = (
        std::env::var_os("HEEMANG_CORPUS"),
        std::env::var("HEEMANG_FONTS").ok(),
    ) else {
        eprintln!("HEEMANG_CORPUS or HEEMANG_FONTS is not set; skipping");
        return None;
    };
    let path: PathBuf = Path::new(&corpus).join(doc);
    if !path.exists() {
        eprintln!("{} is missing; skipping", path.display());
        return None;
    }
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_heemang"));
    cmd.arg("layout-dump").arg(&path);
    for f in std::env::split_paths(&fonts).filter(|f| !f.as_os_str().is_empty()) {
        cmd.arg("--font-path").arg(f);
    }
    let out = cmd.output().expect("heemang runs");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let s = String::from_utf8(out.stdout).unwrap();
    // Each page's drawn list follows its `"drawn":[`; its entries start
    // with `{"kind":` (stored lines start with `{"para":`).
    Some(
        s.split("\"drawn\":[")
            .skip(1)
            .map(|page| {
                page.split("{\"kind\":")
                    .skip(1)
                    .filter_map(|entry| {
                        let at = entry.find("\"text\":\"")? + 8;
                        let rest = &entry[at..];
                        let mut end = 0;
                        let bytes = rest.as_bytes();
                        while end < bytes.len() && bytes[end] != b'"' {
                            end += if bytes[end] == b'\\' { 2 } else { 1 };
                        }
                        Some(rest[..end].to_string())
                    })
                    .collect()
            })
            .collect(),
    )
}

#[test]
fn unfilled_click_here_guide_text_is_not_listed() {
    let Some(pages) = drawn("issue1893_clickhere_field_roundtrip.hwpx") else {
        return;
    };
    let all: Vec<&String> = pages.iter().flatten().collect();
    // "소속관서" is the guide text of an unfilled field; the document
    // number is the content of a field that has no guide text.
    assert!(!all.iter().any(|t| t.contains("소속관서")), "{all:?}");
    assert!(all.iter().any(|t| t.contains("0000-000000")), "{all:?}");
}

#[test]
fn text_a_cell_clips_away_is_listed_on_the_page_that_shows_it_only() {
    let Some(pages) = drawn("rowbreak-problem-pages.hwpx") else {
        return;
    };
    // Lines deep inside a text box that a split cell cuts off at a page
    // bottom: the page with the cut paints none of them.
    for line in [
        "수립 단계부터 개방 대상 정의",
        "점검하여 항목별 품질수준을 관리하여야 함)",
        "공공데이터를 별도 테이블(table)로 구성·설계",
    ] {
        let on: Vec<usize> = (0..pages.len())
            .filter(|&p| pages[p].iter().any(|t| t.contains(line)))
            .collect();
        assert_eq!(on.len(), 1, "{line} listed on pages {on:?}");
    }
}
