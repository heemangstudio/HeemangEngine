use hm_model::{ControlKind, NumberKind, SharedBytes};
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
        let mut n = 0;
        for s in &doc.sections {
            for p in &s.paras {
                for c in &p.controls {
                    if let ControlKind::AutoNumber(a) = &c.kind
                        && a.kind() == NumberKind::Page
                    {
                        n += 1;
                    }
                }
            }
        }
        if n > 0 {
            println!("{n:>4} {}", f.display());
        }
    }
}
