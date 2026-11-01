//! Generates `data/hancom-metrics.bin`: the layout metrics (names, em,
//! ascent, descent and per-character advances; nothing else) of the faces
//! Hancom ships, read from an installed Hancom Office.
//!
//! ```sh
//! H="/Applications/Hancom Office HWP.app/Contents/Resources/Hnc/Shared"
//! cargo run -p hm-fonts --example metric_tables -- <faces.json> <out.bin> "$H/TTF" "$H/Fonts"
//! ```
//!
//! `faces.json` lists face names (a JSON array of strings). Each name that
//! resolves to a face of the given fonts gets its regular table, and its
//! bold table when Hancom ships a bold face. Characters U+0020 to U+FFFF.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use hm_fonts::metrics::{MetricTable, serialize};
use hm_fonts::{FaceQuery, FontSystem};

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [faces, out, dirs @ ..] = args.as_slice() else {
        eprintln!("usage: metric_tables <faces.json> <out.bin> <font dir>...");
        std::process::exit(1);
    };
    let names: Vec<String> = parse_names(&std::fs::read_to_string(faces).expect("faces file"));
    let mut fonts = FontSystem::new();
    let mut files = Vec::new();
    for d in dirs {
        let d = Path::new(d);
        // Hancom's HFT folder brings its catalog, font map and code tables.
        if let Ok(b) = std::fs::read(d.join("hftinfo.dat")) {
            fonts.add_hft_info(&b);
        }
        let mut code = Vec::new();
        walk(&d.join("../../Bin/Code"), &mut code);
        for c in code {
            if c.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("u300_"))
            {
                fonts.add_code_table(&std::fs::read(&c).unwrap());
            }
        }
        walk(d, &mut files);
    }
    for f in files {
        let ext = f
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if matches!(ext.as_str(), "ttf" | "otf" | "ttc" | "hft") {
            fonts.add(
                f.display().to_string(),
                Arc::new(std::fs::read(&f).unwrap()),
            );
        }
    }
    let mut tables: Vec<MetricTable> = Vec::new();
    let mut seen = Vec::new();
    for name in &names {
        for bold in [false, true] {
            let Some(r) = fonts.resolve(&FaceQuery {
                name,
                bold,
                ..Default::default()
            }) else {
                continue;
            };
            let entry = fonts.entry(r.face);
            if !r.exact || (bold && !entry.bold()) || seen.contains(&r.face) {
                continue;
            }
            seen.push(r.face);
            let Some(face) = fonts.face(r.face) else {
                continue;
            };
            let em = face.units_per_em;
            let mut advances = Vec::new();
            for c in (0x20u32..=0xFFFF).filter_map(char::from_u32) {
                if let Some(g) = face.glyph(c) {
                    let a = (g.advance as f64 * em as f64 / g.em.max(1) as f64).round();
                    advances.push((c as u32, a.clamp(0.0, u16::MAX as f64) as u16));
                }
            }
            let mut names = entry.families.clone();
            names.sort();
            names.dedup();
            eprintln!(
                "{name} bold={bold}: {} ({} chars)",
                names.join(" / "),
                advances.len()
            );
            tables.push(MetricTable::new(
                names,
                bold,
                face.is_hft(),
                em,
                face.ascent,
                face.descent,
                &advances,
            ));
        }
    }
    let bytes = serialize(&tables);
    std::fs::write(out, &bytes).expect("write");
    eprintln!("{} tables, {} bytes", tables.len(), bytes.len());
}

/// A JSON array of strings, without a JSON dependency.
fn parse_names(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '"' {
            continue;
        }
        let mut name = String::new();
        while let Some(c) = chars.next() {
            match c {
                '"' => break,
                '\\' => name.extend(chars.next()),
                c => name.push(c),
            }
        }
        out.push(name);
    }
    out
}
