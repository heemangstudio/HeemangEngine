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
        if ["ttf", "otf", "ttc", "hft"].contains(&e.as_str()) {
            fs.add(f.display().to_string(), Arc::new(b));
        } else if e == "dat" {
            fs.add_code_table(&b);
        }
    }
    let mut files = Vec::new();
    walk(
        std::path::Path::new(&std::env::args().nth(1).unwrap()),
        &mut files,
    );
    files.sort();
    for f in files {
        let Ok(b) = std::fs::read(&f) else { continue };
        if !hm_hwp5::is_hwp5(&b) {
            continue;
        }
        let Ok(doc) = hm_hwp5::read(SharedBytes::from(b)) else {
            continue;
        };
        let layout = hm_layout::layout(&doc, &fs);
        // first vpos mismatch where the paragraph's breaks match and all prior paragraphs' breaks match
        let mut vpos = std::collections::HashMap::new();
        for p in &layout.pages {
            for l in &p.lines {
                vpos.insert((p.section, l.para, l.line), l.vpos);
            }
        }
        let mut first: Option<String> = None;
        let (mut n, mut ok) = (0, 0);
        'outer: for (si, s) in doc.sections.iter().enumerate() {
            for (pi, p) in s.paras.iter().enumerate() {
                let pl = &layout.sections[si].paras[pi];
                let stored: Vec<u32> = p.line_segs.iter().map(|s| s.text_start).collect();
                let mine: Vec<u32> = pl.lines.iter().map(|l| l.start).collect();
                if stored != mine {
                    break 'outer;
                }
                for (li, seg) in p.line_segs.iter().enumerate() {
                    n += 1;
                    if vpos.get(&(si as u32, pi as u32, li as u32)) == Some(&seg.vpos) {
                        ok += 1;
                    } else if first.is_none() {
                        first = Some(format!(
                            "s{si} p{pi} l{li} ours {:?} stored {} prev-stored {:?}",
                            vpos.get(&(si as u32, pi as u32, li as u32)),
                            seg.vpos,
                            if li > 0 {
                                Some(p.line_segs[li - 1].vpos)
                            } else {
                                s.paras
                                    .get(pi.wrapping_sub(1))
                                    .and_then(|q| q.line_segs.last())
                                    .map(|q| q.vpos)
                            }
                        ));
                    }
                }
            }
        }
        if let Some(fm) = first {
            println!(
                "{:>4}/{:<4} {} :: {}",
                ok,
                n,
                f.file_name().unwrap().to_string_lossy(),
                fm
            );
        }
    }
}
