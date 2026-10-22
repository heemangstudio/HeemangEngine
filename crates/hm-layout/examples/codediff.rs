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
fn fonts(code: bool) -> hm_fonts::FontSystem {
    let h = std::path::Path::new("/Applications/Hancom Office HWP.app/Contents/Resources/Hnc");
    let mut fs = hm_fonts::FontSystem::new();
    let mut ff = Vec::new();
    walk(&h.join("Shared/TTF"), &mut ff);
    walk(&h.join("Shared/Fonts"), &mut ff);
    if code {
        walk(&h.join("Bin/Code"), &mut ff);
    }
    ff.sort();
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
        }
    }
    fs
}
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let b = std::fs::read(&path).unwrap();
    let doc = if hm_hwp5::is_hwp5(&b) {
        hm_hwp5::read(SharedBytes::from(b)).unwrap()
    } else {
        hm_hwpx::read(SharedBytes::from(b)).unwrap()
    };
    let (fa, fb) = (fonts(false), fonts(true));
    let (a, b) = (hm_layout::layout(&doc, &fa), hm_layout::layout(&doc, &fb));
    println!("pages {} vs {}", a.pages.len(), b.pages.len());
    for (k, (pa, pb)) in a.pages.iter().zip(&b.pages).enumerate() {
        if pa.lines != pb.lines || pa.tables != pb.tables {
            let la: Vec<_> = pa.lines.iter().map(|l| (l.para, l.line, l.vpos)).collect();
            let lb: Vec<_> = pb.lines.iter().map(|l| (l.para, l.line, l.vpos)).collect();
            println!("page {} differs\n plain {la:?}\n code  {lb:?}", k + 1);
            let ta: Vec<_> = pa
                .tables
                .iter()
                .map(|t| (t.para, t.top, t.bottom, t.y))
                .collect();
            let tb: Vec<_> = pb
                .tables
                .iter()
                .map(|t| (t.para, t.top, t.bottom, t.y))
                .collect();
            println!(" tables plain {ta:?}\n tables code  {tb:?}");
            break;
        }
    }
    let mut shown = 0;
    for (si, s) in doc.sections.iter().enumerate() {
        for (pi, p) in s.paras.iter().enumerate() {
            let (la, lb) = (a.para(si as u32, pi as u32), b.para(si as u32, pi as u32));
            let st: Vec<u32> = p.line_segs.iter().map(|s| s.text_start).collect();
            let sa: Vec<u32> = la.lines.iter().map(|l| l.start).collect();
            let sb: Vec<u32> = lb.lines.iter().map(|l| l.start).collect();
            if sa != sb {
                println!("s{si} p{pi} stored {st:?} plain {sa:?} code {sb:?}");
            }
            for (k, c) in p.controls.iter().enumerate() {
                let (Some(t), Some(ta), Some(tb)) =
                    (c.as_table(), la.table(k as u32), lb.table(k as u32))
                else {
                    continue;
                };
                for (ci, cell) in t.cells.iter().enumerate() {
                    for (cp, para) in cell.paras.iter().enumerate() {
                        let xa: Vec<u32> = ta.cells[ci].paras[cp]
                            .lines
                            .iter()
                            .map(|l| l.start)
                            .collect();
                        let xb: Vec<u32> = tb.cells[ci].paras[cp]
                            .lines
                            .iter()
                            .map(|l| l.start)
                            .collect();
                        if xa != xb {
                            let st: Vec<u32> =
                                para.line_segs.iter().map(|s| s.text_start).collect();
                            let text: String = para
                                .units()
                                .filter_map(|(_, u)| match u {
                                    hm_model::Unit::Char(c) => Some(c),
                                    _ => None,
                                })
                                .collect();
                            println!(
                                "s{si} p{pi} c{k} cell {ci} para {cp}: stored {st:?} plain {xa:?} code {xb:?} {text:?}"
                            );
                            let mp = &tb.cells[ci].paras[cp];
                            for (pos, u) in para.units() {
                                let hm_model::Unit::Char(ch) = u else {
                                    continue;
                                };
                                let fa_ = ta.cells[ci].paras[cp].measured.styles[pos as usize];
                                let fb_ = mp.measured.styles[pos as usize];
                                let (x, y) = (
                                    ta.cells[ci].paras[cp].measured.advances[pos as usize],
                                    mp.measured.advances[pos as usize],
                                );
                                if x != y {
                                    let cs = doc.info.char_shape(para.shape_at(pos)).unwrap();
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
                                    println!(
                                        "   {ch:?} U+{:04X} {x} vs {y} {:?} vs {:?}",
                                        ch as u32,
                                        a.styles[fa_ as usize].face.map(|f| fa
                                            .entry(f)
                                            .label
                                            .rsplit('/')
                                            .next()
                                            .unwrap()
                                            .to_string()),
                                        b.styles[fb_ as usize].face.map(|f| fb
                                            .entry(f)
                                            .label
                                            .rsplit('/')
                                            .next()
                                            .unwrap()
                                            .to_string())
                                    );
                                }
                            }
                        }
                    }
                }
            }
            // Glyph advances that differ, with the face each layout used.
            for (pos, u) in p.units() {
                let hm_model::Unit::Char(c) = u else { continue };
                let (x, y) = (
                    la.measured.advances[pos as usize],
                    lb.measured.advances[pos as usize],
                );
                if x != y && shown < 30 {
                    shown += 1;
                    let fa_ = a.styles[la.measured.styles[pos as usize] as usize].face;
                    let fb_ = b.styles[lb.measured.styles[pos as usize] as usize].face;
                    println!(
                        "s{si} p{pi} {c:?} U+{:04X} {x} vs {y} faces {:?} vs {:?}",
                        c as u32,
                        fa_.map(|f| fa.entry(f).label.rsplit('/').next().unwrap().to_string()),
                        fb_.map(|f| fb.entry(f).label.rsplit('/').next().unwrap().to_string())
                    );
                }
            }
        }
    }
}
