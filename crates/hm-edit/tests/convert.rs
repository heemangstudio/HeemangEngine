//! Saving in the other format (`HEEMANG_CORPUS`, `HEEMANG_FONTS`): an HWP 5
//! file written as HWPX and an HWPX file written as HWP 5 read back to the
//! same text and the same layout, line for line.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use hm_fonts::FontSystem;
use hm_model::{Document, Format, SharedBytes};

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

fn fonts() -> Option<FontSystem> {
    let spec = std::env::var("HEEMANG_FONTS").ok()?;
    let mut fs = FontSystem::new();
    for dir in spec.split(':') {
        let mut files = Vec::new();
        walk(Path::new(dir), &mut files);
        for f in files {
            let ext = f.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            if matches!(ext.as_str(), "ttf" | "otf" | "ttc") {
                fs.add(f.display().to_string(), Arc::new(std::fs::read(&f).unwrap()));
            }
        }
    }
    Some(fs)
}

fn read(bytes: Vec<u8>) -> Document {
    let b = SharedBytes::from(bytes);
    if hm_hwpx::is_hwpx(&b) {
        hm_hwpx::read(b).unwrap()
    } else {
        hm_hwp5::read(b).unwrap()
    }
}

/// Every placed body line as `(page, para, line, vpos)`, and the page count.
fn lines(doc: &Document, fonts: &FontSystem) -> (usize, Vec<(usize, u32, u32, i32)>) {
    let layout = hm_layout::layout(doc, fonts);
    let mut v = Vec::new();
    for (k, p) in layout.pages.iter().enumerate() {
        for l in &p.lines {
            v.push((k, l.para, l.line, l.vpos));
        }
    }
    (layout.pages.len(), v)
}

#[test]
fn documents_keep_text_and_layout_in_the_other_format() {
    let (Some(root), Some(fonts)) = (std::env::var_os("HEEMANG_CORPUS"), fonts()) else {
        eprintln!("HEEMANG_CORPUS or HEEMANG_FONTS is not set; skipping");
        return;
    };
    let root = PathBuf::from(root);
    for rel in [
        "2022년 국립국어원 업무계획.hwp",
        "hwpx/2024년 1분기 해외직접투자 보도자료 ff.hwpx",
        "footnote-tbox-01.hwp",
        "hwpx/exam-kor-1p.hwpx",
        "multi-table-001.hwp",
    ] {
        let Ok(bytes) = std::fs::read(root.join(rel)) else {
            continue;
        };
        let original = read(bytes);
        let target = match original.meta.format {
            Format::Hwpx => Format::Hwp5,
            _ => Format::Hwpx,
        };
        let mut doc = original.clone();
        doc.convert_to(target);
        let out = match target {
            Format::Hwpx => hm_hwpx::write_new(&doc).unwrap(),
            _ => hm_hwp5::write_new(&doc).unwrap(),
        };
        let back = read(out);
        assert_eq!(back.meta.format, target, "{rel}");
        let text = |d: &Document| -> Vec<String> {
            d.sections.iter().flat_map(|s| &s.paras).map(|p| p.plain_text()).collect()
        };
        assert_eq!(text(&back), text(&original), "{rel}");
        assert_eq!(back.info.bin_data.len(), original.info.bin_data.len(), "{rel}");
        assert_eq!(lines(&back, &fonts), lines(&original, &fonts), "{rel}");
    }
}
