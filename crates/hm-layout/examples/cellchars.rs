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
    fs.add_hft_info(&std::fs::read(h.join("Shared/Fonts/hftinfo.dat")).unwrap());
    for f in ff {
        let e = f
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let b = std::fs::read(&f).unwrap();
        let name = f.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if ["ttf", "otf", "ttc", "hft"].contains(&e.as_str()) {
            fs.add(f.display().to_string(), Arc::new(b));
        } else if name.starts_with("u300_") {
            fs.add_code_table(&b);
        } else if name == "FontMap.dat" {
            fs.add_font_map(&b);
        }
    }
    let a: Vec<String> = std::env::args().collect();
    let doc = hm_hwp5::read(SharedBytes::from(std::fs::read(&a[1]).unwrap())).unwrap();
    let (si, pi): (usize, usize) = (a[2].parse().unwrap(), a[3].parse().unwrap());
    let layout = hm_layout::layout(&doc, &fs);
    let para = &doc.sections[si].paras[pi];
    let pl = layout.para(si as u32, pi as u32);
    let show = |p: &hm_model::Paragraph, m: &hm_layout::ParaLayout| {
        let mut n = 0;
        for (pos, u) in p.units() {
            let hm_model::Unit::Char(c) = u else { continue };
            let st = &layout.styles[m.measured.styles[pos as usize] as usize];
            println!(
                "  {c:?} U+{:04X} adv {} nat {} face {:?} size {} exact {} bold {}",
                c as u32,
                m.measured.advances[pos as usize],
                m.measured.natural[pos as usize],
                st.face
                    .map(|f| fs.entry(f).label.rsplit('/').next().unwrap().to_string()),
                st.size,
                st.exact,
                st.fake_bold
            );
            if c == '※' || c as u32 == 0x387 || c == '‧' {
                let cs = doc.info.char_shape(p.shape_at(pos)).unwrap();
                for sc in [
                    hm_model::Script::Hangul,
                    hm_model::Script::Latin,
                    hm_model::Script::Symbol,
                    hm_model::Script::Other,
                ] {
                    println!(
                        "     slot {sc:?}: {:?}",
                        doc.info
                            .face(sc, cs.face(sc))
                            .map(|f| f.name.to_string_lossy())
                    );
                }
                println!("     script {:?}", hm_layout::script::script_of(c));
            }
            n += 1;
            if n > 400 {
                break;
            }
        }
    };
    show(para, pl);
    for (k, c) in para.controls.iter().enumerate() {
        if let (Some(t), Some(tl)) = (c.as_table(), pl.table(k as u32)) {
            for (ci, cell) in t.cells.iter().enumerate() {
                for (cp, p) in cell.paras.iter().enumerate() {
                    println!("cell {ci} para {cp}");
                    show(p, &tl.cells[ci].paras[cp]);
                }
            }
        }
    }
}
