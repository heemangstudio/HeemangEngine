use hm_model::{ControlKind, Paragraph, SharedBytes};
fn walk(paras: &[Paragraph], path: &str) {
    for (pi, p) in paras.iter().enumerate() {
        for (ci, c) in p.controls.iter().enumerate() {
            let here = format!("{path}/p{pi}c{ci}");
            match &c.kind {
                ControlKind::HeaderFooter(h) => {
                    let texts: Vec<String> = h
                        .list
                        .paras
                        .iter()
                        .map(|q| {
                            format!(
                                "{:?}",
                                q.controls.iter().map(|x| x.id()).collect::<Vec<_>>()
                            )
                        })
                        .collect();
                    println!(
                        "{here} header footer={} side {:?} {:?}",
                        h.footer,
                        h.side(),
                        texts
                    );
                }
                ControlKind::NewNumber(n) => println!("{here} nwno {:?} {}", n.kind(), n.number),
                ControlKind::Table(t) => {
                    for (k, cell) in t.cells.iter().enumerate() {
                        walk(&cell.paras, &format!("{here}/cell{k}"));
                    }
                }
                _ => {
                    if let Some(sh) = c.as_object().and_then(|o| o.shape())
                        && let Some(t) = sh.text()
                    {
                        walk(&t.paras, &format!("{here}/text"));
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
