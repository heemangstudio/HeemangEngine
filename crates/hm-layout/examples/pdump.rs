use hm_model::SharedBytes;
use std::sync::Arc;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let doc = hm_hwp5::read(SharedBytes::from(std::fs::read(&a[1]).unwrap())).unwrap();
    let para: usize = a[2].parse().unwrap();
    let mut fonts = hm_fonts::FontSystem::new();
    for dir in a[3..].iter() {
        let mut v: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file())
            .collect();
        v.sort();
        for f in v {
            let e = f
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if ["ttf", "otf", "ttc", "hft"].contains(&e.as_str()) {
                fonts.add(
                    f.display().to_string(),
                    Arc::new(std::fs::read(&f).unwrap()),
                );
            }
        }
        if dir.ends_with("Fonts") {
            let code = std::path::Path::new(dir).join("../../Bin/Code");
            for e in std::fs::read_dir(code).unwrap().flatten() {
                fonts.add_code_table(&std::fs::read(e.path()).unwrap());
            }
        }
    }
    let layout = hm_layout::layout(&doc, &fonts);
    let p = &doc.sections[0].paras[para];
    let pl = layout.para(0, para as u32);
    let m = &pl.measured;
    let ps = doc.info.para_shape(p.header.para_shape as u32).unwrap();
    println!(
        "para shape attr1 {:#x} align {:?} ls {:?}",
        ps.attr1,
        ps.align(),
        ps.line_spacing()
    );
    for r in &p.runs {
        let cs = doc.info.char_shape(r.shape).unwrap();
        println!(
            "run at {} cs {} size {} ratios {:?} spacing {:?} rel {:?} attr {:#x}",
            r.pos, r.shape, cs.base_size, cs.ratios, cs.spacings, cs.rel_sizes, cs.attr
        );
    }
    for (li, l) in pl.lines.iter().enumerate() {
        let mut s = String::new();
        let mut sum = 0;
        for &(pos, k) in &m.items[l.first_item as usize..l.end_item as usize] {
            let st = &layout.styles[m.styles[pos as usize] as usize];
            sum += m.advances[pos as usize];
            s.push_str(&format!(
                "{:?}:{}/{}{} ",
                k,
                m.natural[pos as usize],
                m.advances[pos as usize],
                if st.exact { "" } else { "*" }
            ));
        }
        println!(
            "line {li} start {} used {} width {} sum {sum}\n  {s}",
            l.start, l.used, l.width
        );
        if li >= 4 {
            break;
        }
    }
    let st = layout
        .styles
        .iter()
        .filter(|s| s.char_shape == p.runs[0].shape)
        .collect::<Vec<_>>();
    for s in st {
        println!(
            "style script {:?} face {:?} size {} ratio {} spacing {} exact {} fam {:?}",
            s.script,
            s.face,
            s.size,
            s.ratio,
            s.spacing,
            s.exact,
            s.face.map(|f| fonts.entry(f).families[0].clone())
        );
    }
}
