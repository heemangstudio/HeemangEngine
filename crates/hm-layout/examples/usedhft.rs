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
        }
    }
    let path = std::env::args().nth(1).unwrap();
    let b = std::fs::read(&path).unwrap();
    let doc = if hm_hwp5::is_hwp5(&b) {
        hm_hwp5::read(SharedBytes::from(b)).unwrap()
    } else {
        hm_hwpx::read(SharedBytes::from(b)).unwrap()
    };
    let layout = hm_layout::layout(&doc, &fs);
    let mut counts: std::collections::BTreeMap<String, usize> = Default::default();
    for sec in &layout.sections {
        for pl in &sec.paras {
            for &si in &pl.measured.styles {
                if let Some(f) = layout.styles[si as usize].face {
                    let e = fs.entry(f);
                    *counts
                        .entry(format!(
                            "{} {:?}",
                            e.label.rsplit('/').next().unwrap(),
                            e.families.first()
                        ))
                        .or_default() += 1;
                }
            }
        }
    }
    let mut v: Vec<_> = counts.into_iter().collect();
    v.sort_by_key(|x| std::cmp::Reverse(x.1));
    for (k, n) in v.iter().take(25) {
        println!("{n:>7} {k}");
    }
}
