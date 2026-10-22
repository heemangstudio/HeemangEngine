use hm_model::{ControlKind, Paragraph, SharedBytes};
fn walk(paras: &[Paragraph], path: &str) {
    for (pi, p) in paras.iter().enumerate() {
        for (ci, c) in p.controls.iter().enumerate() {
            let here = format!("{path}/p{pi}c{ci}");
            match &c.kind {
                ControlKind::AutoNumber(a) => println!(
                    "{here} atno {:?} number {} attr {:#x}",
                    a.kind(),
                    a.number,
                    a.attr
                ),
                ControlKind::PageHide(h) => println!("{here} pghd attr {:#x}", h.attr),
                ControlKind::NewNumber(n) => println!("{here} nwno {:?} {}", n.kind(), n.number),
                ControlKind::PageNumber(n) => println!("{here} pgnp attr {:#x}", n.attr),
                ControlKind::Table(t) => {
                    for (k, cell) in t.cells.iter().enumerate() {
                        walk(&cell.paras, &format!("{here}/cell{k}"));
                    }
                }
                _ => {
                    if let Some(sh) = c.as_object().and_then(|o| o.shape()) {
                        fn sh_walk(sh: &hm_model::Shape, path: &str) {
                            if let Some(t) = sh.text() {
                                walk(&t.paras, &format!("{path}/text"));
                            }
                            for (k, ch) in sh.children().enumerate() {
                                sh_walk(ch, &format!("{path}/ch{k}"));
                            }
                        }
                        sh_walk(sh, &here);
                    }
                }
            }
        }
    }
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let doc = hm_hwp5::read(SharedBytes::from(std::fs::read(&a[1]).unwrap())).unwrap();
    for (si, s) in doc.sections.iter().enumerate() {
        walk(&s.paras, &format!("s{si}"));
    }
}
