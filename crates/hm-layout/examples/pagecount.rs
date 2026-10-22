use hm_model::SharedBytes;
use std::io::BufRead;
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
        if ["ttf", "otf", "ttc", "hft"].contains(&e.as_str()) {
            fs.add(f.display().to_string(), Arc::new(b));
        } else if e == "dat" {
            fs.add_code_table(&b);
        }
    }
    let (mut same, mut total) = (0, 0);
    for line in std::io::stdin().lock().lines() {
        let line = line.unwrap();
        let mut it = line.split('\t');
        let (Some(path), Some(pages), Some(name)) = (it.next(), it.next(), it.next()) else {
            continue;
        };
        let pages: usize = pages.parse().unwrap_or(0);
        let Ok(b) = std::fs::read(path) else { continue };
        let doc = if hm_hwp5::is_hwp5(&b) {
            hm_hwp5::read(SharedBytes::from(b)).ok()
        } else {
            None
        };
        let Some(doc) = doc else { continue };
        let ours = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            hm_layout::layout(&doc, &fs).pages.len()
        }))
        .unwrap_or(0);
        total += 1;
        if ours == pages {
            same += 1;
        } else {
            println!("{ours:>4} vs {pages:>4}  {name}");
        }
    }
    eprintln!("page counts equal: {same} of {total}");
}
