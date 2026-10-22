use hm_model::SharedBytes;
use std::sync::Arc;
fn walk(d: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for e in std::fs::read_dir(d).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out)
        } else {
            out.push(p)
        }
    }
}
fn fonts(hft: bool) -> hm_fonts::FontSystem {
    let h = std::path::Path::new("/Applications/Hancom Office HWP.app/Contents/Resources/Hnc");
    let mut fs = hm_fonts::FontSystem::new();
    let mut files = Vec::new();
    walk(&h.join("Shared/TTF"), &mut files);
    if hft {
        walk(&h.join("Shared/Fonts"), &mut files);
        walk(&h.join("Bin/Code"), &mut files);
    }
    files.sort();
    for f in files {
        let e = f
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let b = std::fs::read(&f).unwrap();
        if ["ttf", "otf", "ttc", "hft"].contains(&e.as_str()) {
            fs.add(f.display().to_string(), Arc::new(b));
        } else if e == "dat" {
            fs.add_code_table(&b);
        }
    }
    fs
}
fn main() {
    let (a, b) = (fonts(false), fonts(true));
    let mut files = Vec::new();
    walk(
        std::path::Path::new(&std::env::args().nth(1).unwrap()),
        &mut files,
    );
    files.sort();
    let mut rows = Vec::new();
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
        let la = hm_layout::audit(&doc, &hm_layout::layout(&doc, &a), 0);
        let lb = hm_layout::audit(&doc, &hm_layout::layout(&doc, &b), 0);
        rows.push((
            lb.paras_breaks_match as i64 - la.paras_breaks_match as i64,
            la.paras,
            la.paras_breaks_match,
            lb.paras_breaks_match,
            f.display().to_string(),
        ));
    }
    rows.sort();
    for r in rows.iter().take(25) {
        println!(
            "{:>5} paras {:>5} before {:>5} after {:>5} {}",
            r.0, r.1, r.2, r.3, r.4
        );
    }
    println!("...");
    for r in rows.iter().rev().take(10) {
        println!(
            "{:>5} paras {:>5} before {:>5} after {:>5} {}",
            r.0, r.1, r.2, r.3, r.4
        );
    }
}
