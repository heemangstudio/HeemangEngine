//! Placing a long table that splits over hundreds of pages stays fast.
//!
//! Needs `HEEMANG_CORPUS`; skips when it or the document is missing.

use std::path::Path;
use std::time::{Duration, Instant};

use hm_fonts::FontSystem;
use hm_model::SharedBytes;

const DOC: &str = "task2070/1130000-201900011_D0150004-1-002_2017년기준 시장구조조사.hwp";

#[test]
fn long_split_tables_lay_out_quickly() {
    let Some(corpus) = std::env::var_os("HEEMANG_CORPUS") else {
        eprintln!("HEEMANG_CORPUS is not set; skipping");
        return;
    };
    let Ok(bytes) = std::fs::read(Path::new(&corpus).join(DOC)) else {
        eprintln!("{DOC} is missing; skipping");
        return;
    };
    let doc = hm_hwp5::read(SharedBytes::from(bytes)).expect("document reads");
    let fonts = FontSystem::new();
    let start = Instant::now();
    let layout = hm_layout::layout(&doc, &fonts);
    let took = start.elapsed();
    assert!(layout.pages.len() > 100, "{} pages", layout.pages.len());
    // Cutting by rescanning the table per fragment took minutes here.
    assert!(took < Duration::from_secs(20), "layout took {took:?}");
}
