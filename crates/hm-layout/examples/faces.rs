use hm_model::{Script, SharedBytes};
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let doc = hm_hwp5::read(SharedBytes::from(std::fs::read(&path).unwrap())).unwrap();
    for s in 0..7 {
        for (i, f) in doc.info.faces[s].iter().enumerate() {
            println!(
                "slot {s} id {i}: attr {:#x} {:?} sub {:?} def {:?}",
                f.attr, f.name, f.substitute, f.default_name
            );
        }
    }
    // characters per (script slot, face name)
    let mut counts: std::collections::BTreeMap<(usize, String), std::collections::BTreeSet<char>> =
        Default::default();
    for sec in &doc.sections {
        for p in &sec.paras {
            for (pos, u) in p.units() {
                if let hm_model::Unit::Char(c) = u {
                    let cs = p.shape_at(pos);
                    let Some(shape) = doc.info.char_shape(cs) else {
                        continue;
                    };
                    let script = hm_layout::script::script_of(c).unwrap_or(Script::Hangul);
                    let name = doc
                        .info
                        .face(script, shape.face(script))
                        .map(|f| f.name.to_string_lossy())
                        .unwrap_or_default();
                    counts.entry((script.index(), name)).or_default().insert(c);
                }
            }
        }
    }
    for ((s, n), set) in counts {
        let nonbasic: String = set
            .iter()
            .filter(|c| !c.is_ascii() && !('\u{ac00}'..='\u{d7a3}').contains(c))
            .collect();
        println!("slot {s} {n}: {} chars, other: {nonbasic}", set.len());
    }
}
