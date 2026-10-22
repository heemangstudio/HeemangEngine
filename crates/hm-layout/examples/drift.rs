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
    let a: Vec<String> = std::env::args().collect();
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
    let doc = hm_hwp5::read(SharedBytes::from(std::fs::read(&a[1]).unwrap())).unwrap();
    let layout = hm_layout::layout(&doc, &fs);
    let show: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(10);
    // Our column/page starts: (section, para, line) of the first line of each column of each page.
    let mut ours = std::collections::BTreeSet::new();
    for p in &layout.pages {
        let mut seen_x = Vec::new();
        for l in &p.lines {
            if !seen_x.contains(&l.x) {
                seen_x.push(l.x);
                ours.insert((p.section, l.para, l.line));
            }
        }
    }
    // Hancom's: stored vpos resets.
    let mut hancom = std::collections::BTreeSet::new();
    for (si, s) in doc.sections.iter().enumerate() {
        let mut prev = i32::MIN;
        for (pi, p) in s.paras.iter().enumerate() {
            for (li, seg) in p.line_segs.iter().enumerate() {
                if seg.vpos < prev || seg.flags & 3 != 0 {
                    hancom.insert((si as u32, pi as u32, li as u32));
                }
                prev = seg.vpos;
            }
        }
    }
    let only_ours: Vec<_> = ours.difference(&hancom).take(show).collect();
    let only_hancom: Vec<_> = hancom.difference(&ours).take(show).collect();
    println!(
        "pages {} ours starts {} hancom starts {}",
        layout.pages.len(),
        ours.len(),
        hancom.len()
    );
    println!("only ours: {only_ours:?}");
    println!("only hancom: {only_hancom:?}");
}
