//! Lines found without painting a page are the lines painting it draws
//! (`HEEMANG_CORPUS`; fonts from `HEEMANG_FONTS` when set).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use hm_fonts::FontSystem;
use hm_model::SharedBytes;

fn fonts() -> FontSystem {
    let mut fs = FontSystem::new();
    fn walk(dir: &Path, fs: &mut FontSystem) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            let ext = p
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if p.is_dir() {
                walk(&p, fs);
            } else if matches!(ext.as_str(), "ttf" | "ttc" | "otf") {
                // Only the families these documents use most, to stay quick.
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if ["HAN", "HBatang", "HDotum", "HY"]
                    .iter()
                    .any(|k| name.starts_with(k))
                {
                    fs.add(
                        p.display().to_string(),
                        Arc::new(std::fs::read(&p).unwrap()),
                    );
                }
            }
        }
    }
    for d in std::env::split_paths(&std::env::var_os("HEEMANG_FONTS").unwrap_or_default()) {
        walk(&d, &mut fs);
    }
    fs
}

#[test]
fn page_lines_match_painted_lines() {
    let Some(root) = std::env::var_os("HEEMANG_CORPUS") else {
        eprintln!("HEEMANG_CORPUS is not set; skipping");
        return;
    };
    let fs = fonts();
    for rel in [
        "2022년 국립국어원 업무계획.hwp",
        "hwpx/2024년 1분기 해외직접투자 보도자료 ff.hwpx",
        "143E433F503322BD33.hwp",
        "footnote-tbox-01.hwp",
        "table-complex.hwp",
    ] {
        let bytes = SharedBytes::from(std::fs::read(PathBuf::from(&root).join(rel)).unwrap());
        let doc = if hm_hwpx::is_hwpx(&bytes) {
            hm_hwpx::read(bytes).unwrap()
        } else {
            hm_hwp5::read(bytes).unwrap()
        };
        let layout = hm_layout::layout(&doc, &fs);
        for i in 0..layout.pages.len().min(12) {
            let (_, painted, painted_cells) = hm_render::build_page_indexed(&doc, &layout, &fs, i);
            let (found, found_cells) = hm_render::page_index(&doc, &layout, &fs, i);
            assert_eq!(painted, found, "{rel} page {i}");
            assert_eq!(painted_cells, found_cells, "{rel} page {i} cells");
            assert_eq!(found_cells, hm_render::page_cells(&doc, &layout, &fs, i));
        }
    }
}
