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
    let mut n = 0;
    for f in files {
        let Ok(b) = std::fs::read(&f) else { continue };
        if !hm_hwp5::is_hwp5(&b) {
            continue;
        }
        let Ok(doc) = hm_hwp5::read(SharedBytes::from(b)) else {
            continue;
        };
        let layout = hm_layout::layout(&doc, &fs);
        for (si, sec) in doc.sections.iter().enumerate() {
            for (pi, p) in sec.paras.iter().enumerate() {
                let pl = &layout.sections[si].paras[pi];
                if p.line_segs.is_empty() {
                    continue;
                }
                if !pl
                    .measured
                    .styles
                    .iter()
                    .all(|&s| layout.styles[s as usize].exact)
                {
                    continue;
                }
                let stored: Vec<u32> = p.line_segs.iter().map(|s| s.text_start).collect();
                let mine: Vec<u32> = pl.lines.iter().map(|l| l.start).collect();
                if stored == mine {
                    continue;
                }
                let k = stored
                    .iter()
                    .zip(&mine)
                    .position(|(a, b)| a != b)
                    .unwrap_or(stored.len().min(mine.len()));
                let (s, o) = (
                    stored.get(k).copied().unwrap_or(9999),
                    mine.get(k).copied().unwrap_or(9999),
                );
                let lo = s.min(o).saturating_sub(4) as usize;
                let hi = (s.max(o) as usize + 3).min(p.text.len());
                let txt: String = p.text[lo..hi]
                    .iter()
                    .enumerate()
                    .map(|(i, &u)| {
                        let pos = (lo + i) as u32;
                        let c = char::from_u32(u as u32)
                            .filter(|c| !c.is_control())
                            .unwrap_or('·');
                        let mark = if pos == s {
                            "[S]"
                        } else if pos == o {
                            "[O]"
                        } else {
                            ""
                        };
                        let run = if p.runs.iter().any(|r| r.pos == pos) {
                            "|"
                        } else {
                            ""
                        };
                        format!("{run}{mark}{c}")
                    })
                    .collect();
                let shape = doc.info.para_shape(p.header.para_shape as u32).unwrap();
                println!(
                    "{} p{pi} line{k} stored {s} ours {o} kbychar {} latin {} {txt}",
                    f.file_name().unwrap().to_string_lossy(),
                    shape.korean_break_by_char(),
                    shape.latin_break()
                );
                n += 1;
            }
        }
    }
    eprintln!("{n} diverging exact paragraphs");
}
