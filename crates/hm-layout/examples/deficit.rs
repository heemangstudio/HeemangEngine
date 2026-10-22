use hm_layout::Kind;
use hm_model::SharedBytes;
use std::sync::Arc;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let mut fonts = hm_fonts::FontSystem::new();
    for dir in a[2..].iter() {
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
    for path in a[1].split(',') {
        let doc = hm_hwp5::read(SharedBytes::from(std::fs::read(path).unwrap())).unwrap();
        let layout = hm_layout::layout(&doc, &fonts);
        for (pi, p) in doc.sections[0].paras.iter().enumerate() {
            let pl = layout.para(0, pi as u32);
            let m = &pl.measured;
            if pl.lines.is_empty() || p.line_segs.len() < 2 {
                continue;
            }
            let width = pl.lines[0].seg_width; // approx (no indent handling)
            for w in p.line_segs.windows(2) {
                let (s, e) = (w[0].text_start, w[1].text_start);
                // only lines without indent/head differences: use our line width for line index>0
                let li = p.line_segs.iter().position(|x| x.text_start == s).unwrap();
                if li == 0 {
                    continue;
                }
                let lw = pl.lines.get(1).map_or(width, |l| l.width);
                let items: Vec<_> = m
                    .items
                    .iter()
                    .filter(|(pos, _)| *pos >= s && *pos < e)
                    .collect();
                let next = m.items.iter().find(|(pos, _)| *pos == e);
                let Some(&(npos, nk)) = next else { continue };
                if !matches!(nk, Kind::Char(_)) {
                    continue;
                }
                if let Some(&&(lp, Kind::Space)) = items.last() {
                    // a line ending in a blank: does the blank start a run, and would it fit?
                    if p.runs.iter().any(|r| r.pos == lp) {
                        let x: i32 = items.iter().map(|(pos, _)| m.advances[*pos as usize]).sum();
                        let before = x - m.advances[lp as usize];
                        let xi = items[items.len() - 2].0 as usize;
                        let room = lw - (before - m.advances[xi] + m.natural[xi]);
                        println!(
                            "RUNBLANK {pi} fits_with_blank {} over {} room {} trackdiff {} xchar {:?}",
                            x <= lw,
                            x - lw,
                            room,
                            m.natural[xi] - m.advances[xi],
                            items[items.len() - 2].1
                        );
                        let _ = before;
                    }
                    continue;
                }
                let x: i32 = items.iter().map(|(pos, _)| m.advances[*pos as usize]).sum();
                let fit = x + m.natural[npos as usize];
                let lat: i32 = items
                    .iter()
                    .filter(|(pos, k)| {
                        matches!(k, Kind::Char(_))
                            && layout.styles[m.styles[*pos as usize] as usize].script
                                == hm_model::Script::Latin
                    })
                    .map(|(pos, _)| m.advances[*pos as usize])
                    .sum();
                let (mut nh, mut ns, mut np) = (0, 0, 0);
                let mut punct = String::new();
                for (_, k) in &items {
                    match k {
                        Kind::Char(c) if ('\u{ac00}'..='\u{d7a3}').contains(c) => nh += 1,
                        Kind::Space => ns += 1,
                        Kind::Char(c) => {
                            np += 1;
                            punct.push(*c)
                        }
                        _ => {}
                    }
                }
                if fit <= lw {
                    let t: String = p.text[s as usize..(e as usize + 3).min(p.text.len())]
                        .iter()
                        .map(|&u| char::from_u32(u as u32).unwrap_or(' '))
                        .collect();
                    let runs: Vec<(u32, u32)> = p
                        .runs
                        .iter()
                        .filter(|r| r.pos >= s && r.pos <= e + 1)
                        .map(|r| (r.pos - s, r.shape))
                        .collect();
                    println!("SHORT runs {:?} len {}", runs, e - s);
                    println!(
                        "SHORT {:>5} need>{:>4} lat {lat} f> {:.3} h{nh} s{ns} p{np} {punct} | {t:?}",
                        pi,
                        lw - fit,
                        1.0 + (lw - fit) as f64 / lat.max(1) as f64
                    );
                } else if x > lw {
                    println!(
                        "LONG  {:>5} over {:>4} h{nh} s{ns} p{np} {punct}",
                        pi,
                        x - lw
                    );
                } else {
                    let runs: Vec<(u32, u32)> = p
                        .runs
                        .iter()
                        .filter(|r| r.pos > s && r.pos <= e + 1)
                        .map(|r| (r.pos - s, r.shape))
                        .collect();
                    println!(
                        "OK    {:>5} runs {:?} len {} slack {:>4} margin {:>4} lat {lat} f<= {:.3} h{nh} s{ns} p{np} {punct}",
                        pi,
                        runs,
                        e - s,
                        lw - x,
                        fit - lw,
                        1.0 + (lw - x) as f64 / lat.max(1) as f64
                    );
                }
            }
        }
    }
}
