use hm_model::SharedBytes;
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
        for (si, s) in doc.sections.iter().enumerate() {
            if !s.definition().is_some_and(|d| d.hide_empty_lines()) {
                continue;
            }
            let paras = &s.paras;
            for i in 1..paras.len().saturating_sub(1) {
                let (p, q) = (&paras[i], &paras[i + 1]);
                let (Some(a), Some(b)) = (p.line_segs.first(), q.line_segs.first()) else {
                    continue;
                };
                let prev_last = paras[i - 1].line_segs.last().map_or(0, |l| l.vpos);
                if p.is_empty() && a.vpos < prev_last {
                    println!(
                        "{} s{si} p{i}: empty at top vpos {} h {} next vpos {} h {}",
                        f.file_name().unwrap().to_string_lossy(),
                        a.vpos,
                        a.height,
                        b.vpos,
                        b.height
                    );
                }
            }
        }
    }
}
