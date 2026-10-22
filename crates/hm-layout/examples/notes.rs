use hm_model::{ControlKind, SharedBytes};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let doc = hm_hwp5::read(SharedBytes::from(std::fs::read(&a[1]).unwrap())).unwrap();
    for (si, s) in doc.sections.iter().enumerate() {
        for (pi, p) in s.paras.iter().enumerate() {
            for c in &p.controls {
                if let ControlKind::Note(n) = &c.kind {
                    for np in &n.list.paras {
                        let ps = doc.info.para_shape(np.header.para_shape as u32).unwrap();
                        let cs = doc.info.char_shape(np.runs[0].shape).unwrap();
                        println!(
                            "s{si} p{pi} end={} num {} lines {} ls {:?} before {} after {} size {} segs {:?}",
                            n.endnote,
                            n.number,
                            np.line_segs.len(),
                            ps.line_spacing(),
                            ps.space_before,
                            ps.space_after,
                            cs.base_size,
                            np.line_segs
                                .iter()
                                .map(|l| (l.vpos, l.height, l.spacing))
                                .collect::<Vec<_>>()
                        );
                    }
                }
            }
        }
    }
}
