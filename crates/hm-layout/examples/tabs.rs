use hm_model::{ControlKind, SharedBytes};
fn dump(p: &hm_model::Paragraph, info: &hm_model::DocInfo, tag: &str) {
    for (pos, u) in p.units() {
        if u == hm_model::Unit::Inline(9) {
            let t = &p.text[pos as usize..pos as usize + 8];
            let ps = info.para_shape(p.header.para_shape as u32).unwrap();
            let td = info.tab_defs.get(ps.tab_def as usize);
            println!(
                "{tag} tab at {pos}: {:04x?} tabdef {:?}",
                t,
                td.map(|t| (
                    t.attr,
                    t.tabs
                        .iter()
                        .map(|s| (s.position, s.kind, s.fill))
                        .collect::<Vec<_>>()
                ))
            );
        }
    }
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let doc = hm_hwp5::read(SharedBytes::from(std::fs::read(&a[1]).unwrap())).unwrap();
    let want: usize = a[2].parse().unwrap();
    let p = &doc.sections[0].paras[want];
    dump(p, &doc.info, "body");
    for c in &p.controls {
        if let ControlKind::Table(t) = &c.kind {
            for cell in &t.cells {
                for cp in &cell.paras {
                    dump(cp, &doc.info, "cell");
                }
            }
        }
    }
}
