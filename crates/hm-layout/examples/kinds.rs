use hm_model::{ControlKind, SectionChild, SharedBytes};
use std::collections::BTreeMap;
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
    let mut n: BTreeMap<String, u32> = BTreeMap::new();
    for f in files {
        let Ok(b) = std::fs::read(&f) else { continue };
        if !hm_hwp5::is_hwp5(&b) {
            continue;
        }
        let Ok(doc) = hm_hwp5::read(SharedBytes::from(b)) else {
            continue;
        };
        for s in &doc.sections {
            for p in &s.paras {
                for c in &p.controls {
                    let k = match &c.kind {
                        ControlKind::Raw(r) => format!("raw {:?}", r.id),
                        ControlKind::Section(sd) => {
                            for ch in &sd.children {
                                if let SectionChild::MasterPage(_) = ch {
                                    *n.entry("master".into()).or_default() += 1;
                                }
                                if let SectionChild::Raw(r) = ch {
                                    *n.entry(format!(
                                        "secd raw tag {:#x} lvl {}",
                                        r.tag, r.level
                                    ))
                                    .or_default() += 1;
                                }
                            }
                            "secd".into()
                        }
                        k => {
                            format!("{:?}", c.id()).to_string()
                                + if matches!(k, ControlKind::Object(_)) {
                                    " obj"
                                } else {
                                    ""
                                }
                        }
                    };
                    *n.entry(k).or_default() += 1;
                }
            }
        }
    }
    for (k, v) in n {
        println!("{v:>6} {k}");
    }
}
