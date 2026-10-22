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
fn main() {
    let h = std::path::Path::new("/Applications/Hancom Office HWP.app/Contents/Resources/Hnc");
    let mut fs = hm_fonts::FontSystem::new();
    let mut ff = Vec::new();
    walk(&h.join("Shared/TTF"), &mut ff);
    walk(&h.join("Shared/Fonts"), &mut ff);
    walk(&h.join("Bin/Code"), &mut ff);
    ff.sort();
    for f in ff {
        let e = f
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let b = std::fs::read(&f).unwrap();
        let name = f.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if ["ttf", "otf", "ttc", "hft"].contains(&e.as_str()) {
            fs.add(f.display().to_string(), Arc::new(b));
        } else if name.starts_with("u300_") {
            fs.add_code_table(&b);
        } else if name == "FontMap.dat" {
            fs.add_font_map(&b);
        }
    }
    let a: Vec<String> = std::env::args().collect();
    let doc = hm_hwp5::read(SharedBytes::from(std::fs::read(&a[1]).unwrap())).unwrap();
    let chars: Vec<char> = a[2].chars().collect();
    for s in 0..7 {
        for f in &doc.info.faces[s] {
            let n = f.name.to_string_lossy();
            for &c in &chars {
                if let Some(id) = fs.mapped_fallback(&n, c) {
                    println!(
                        "slot {s} {n} {c:?} -> {}",
                        fs.entry(id).label.rsplit('/').next().unwrap()
                    );
                }
            }
        }
    }
}
