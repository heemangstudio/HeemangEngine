//! Files the host keeps ([`FontSystem::add_source`]) give the catalog,
//! metrics, kerning and outlines the whole file gives, while the font
//! system holds only the metric tables of the faces it loaded.
//!
//! Needs `HEEMANG_FONTS` (`:`-separated, `;` on Windows: directories, walked
//! recursively); skips when it is unset. `HM_KEPT_ALL=1` checks every font
//! file and every glyph instead of a sample.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use hm_fonts::{FaceId, FontSystem, SharedSource};

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

fn fonts() -> Vec<PathBuf> {
    let Ok(spec) = std::env::var("HEEMANG_FONTS") else {
        return Vec::new();
    };
    let mut files = Vec::new();
    for part in std::env::split_paths(&spec).filter(|p| !p.as_os_str().is_empty()) {
        walk(&part, &mut files);
    }
    files.retain(|f| {
        let name = f.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        matches!(ext.as_str(), "ttf" | "otf" | "ttc" | "otc")
    });
    files.sort();
    if std::env::var("HM_KEPT_ALL").is_err() {
        let wanted = [
            "hanbatang.ttf",
            "hbatang.ttf",
            "uni_hsr.ttf",
            "malgun.ttf",
            "times.ttf",
            "helvetica.ttc",
            "applesdgothicneo.ttc",
        ];
        files.retain(|f| {
            let name = f.file_name().and_then(|n| n.to_str()).unwrap_or("");
            wanted.contains(&name.to_ascii_lowercase().as_str())
        });
        files.dedup_by_key(|f| f.file_name().map(|n| n.to_ascii_lowercase()));
    }
    files
}

#[test]
fn kept_files_match_whole_files() {
    let files = fonts();
    if files.is_empty() {
        eprintln!("HEEMANG_FONTS is unset or has none of the probed fonts; skipping");
        return;
    }
    let all = std::env::var("HM_KEPT_ALL").is_ok();
    let (mut glyphs, mut lazy_faces) = (0usize, 0usize);
    for f in &files {
        let label = f.display().to_string();
        let bytes = std::fs::read(f).unwrap();
        let mut whole = FontSystem::new();
        let n = whole.add(label.clone(), Arc::new(bytes.clone()));
        let mut kept = FontSystem::new();
        let src: SharedSource = Arc::new(bytes.clone());
        let added = kept.add_source(label.clone(), src, &[]);
        assert_eq!(added.faces, n, "{label}");
        assert_eq!(whole.entries().len(), kept.entries().len(), "{label}");
        for (a, b) in whole.entries().iter().zip(kept.entries()) {
            assert_eq!(
                (
                    &a.families,
                    &a.local,
                    &a.postscript,
                    a.weight,
                    a.italic,
                    a.index
                ),
                (
                    &b.families,
                    &b.local,
                    &b.postscript,
                    b.weight,
                    b.italic,
                    b.index
                ),
                "{label}"
            );
        }
        for id in (0..n as u32).map(FaceId) {
            let (a, b) = (whole.face(id), kept.face(id));
            assert_eq!(a.is_some(), b.is_some(), "{label}");
            let (Some(a), Some(b)) = (a, b) else { continue };
            assert_eq!(
                (a.units_per_em, a.ascent, a.descent, &a.family),
                (b.units_per_em, b.ascent, b.descent, &b.family),
                "{label}"
            );
            let font = skrifa::FontRef::from_index(&bytes, whole.entry(id).index).unwrap();
            use skrifa::MetadataProvider;
            let count =
                skrifa::raw::TableProvider::maxp(&font).map_or(0, |m| m.num_glyphs() as u32);
            let step = if all { 1 } else { (count / 3000).max(1) };
            let mut ids: Vec<u16> = Vec::new();
            for (k, (c, _)) in font.charmap().mappings().enumerate() {
                if k as u32 % step != 0 {
                    continue;
                }
                let Some(ch) = char::from_u32(c) else {
                    continue;
                };
                let (ga, gb) = (a.glyph(ch), b.glyph(ch));
                assert_eq!(ga, gb, "{label} {ch:?}");
                if let Some(g) = ga {
                    ids.push(g.glyph);
                }
            }
            for g in (0..count.min(u16::MAX as u32 + 1)).step_by(step as usize) {
                ids.push(g as u16);
            }
            for w in ids.windows(2).step_by(7) {
                assert_eq!(a.kerning(w[0], w[1]), b.kerning(w[0], w[1]), "{label}");
            }
            for &g in &ids {
                assert!(b.resident(g));
                assert_eq!(a.outline(g), b.outline(g), "{label} glyph {g}");
                glyphs += 1;
            }
            lazy_faces += 1;
        }
        // TrueType faces hold their metric tables; CFF and variable faces
        // all their own tables once drawn.
        let truetype = skrifa::raw::FileRef::new(&bytes).unwrap().fonts().all(|f| {
            f.is_ok_and(|f| {
                let has = |t: &[u8; 4]| f.table_data(skrifa::raw::types::Tag::new(t)).is_some();
                has(b"glyf") && !has(b"gvar") && !has(b"VARC")
            })
        });
        assert!(
            !truetype || kept.resident_bytes() < bytes.len() / 2 || bytes.len() < 1 << 20,
            "{label}: {} of {}",
            kept.resident_bytes(),
            bytes.len()
        );
    }
    eprintln!(
        "{} files, {lazy_faces} faces, {glyphs} outlines compared",
        files.len()
    );
}

#[test]
fn kept_files_leave_with_their_label() {
    let Some(f) = fonts().into_iter().next() else {
        eprintln!("HEEMANG_FONTS is unset or has none of the probed fonts; skipping");
        return;
    };
    let bytes = std::fs::read(&f).unwrap();
    let mut fonts = FontSystem::new();
    let n = fonts.add_source("f", Arc::new(bytes.clone()), &[]).faces;
    assert!(n > 0);
    // A label registered before stays as it is.
    let again = fonts.add_source("f", Arc::new(bytes), &[]);
    assert!(again.refilled && again.faces == 0);
    assert!(fonts.face(FaceId(0)).is_some());
    assert_eq!(fonts.remove("f"), n);
    assert!(fonts.face(FaceId(0)).is_none());
    assert_eq!(fonts.resident_bytes(), 0);
}

#[test]
fn kept_hft_files_match_whole_files() {
    let Ok(spec) = std::env::var("HEEMANG_FONTS") else {
        eprintln!("HEEMANG_FONTS is unset; skipping");
        return;
    };
    let mut files = Vec::new();
    for part in std::env::split_paths(&spec).filter(|p| !p.as_os_str().is_empty()) {
        walk(&part, &mut files);
    }
    files.sort();
    let name = |f: &Path| {
        f.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
    };
    let all = std::env::var("HM_KEPT_ALL").is_ok();
    let hft: Vec<&PathBuf> = files
        .iter()
        .filter(|f| name(f).ends_with(".hft"))
        .filter(|f| {
            all || ["hmksm.hft", "ensmj.hft", "enjgt.hft", "hmepo.hft"].contains(&name(f).as_str())
        })
        .collect();
    if hft.is_empty() {
        eprintln!("no HFT files under HEEMANG_FONTS; skipping");
        return;
    }
    let (mut whole, mut kept) = (FontSystem::new(), FontSystem::new());
    for f in &files {
        let n = name(f);
        let b = || std::fs::read(f).unwrap();
        if n == "hftinfo.dat" {
            whole.add_hft_info(&b());
            kept.add_hft_info(&b());
        } else if n.starts_with("u300_") && n.ends_with(".dat") {
            whole.add_code_table(&b());
            kept.add_code_table(&b());
        }
    }
    for f in &hft {
        let label = f.display().to_string();
        let bytes = std::fs::read(f).unwrap();
        let a = whole.add(label.clone(), Arc::new(bytes.clone()));
        let b = kept.add_source(label, Arc::new(bytes), &[]);
        assert_eq!(a, b.faces, "{}", f.display());
    }
    assert_eq!(kept.resident_bytes(), 0);
    assert_eq!(whole.entries().len(), kept.entries().len());
    let step = if all { 1 } else { 97 };
    let chars: Vec<char> = (' '..='~')
        .chain(('가'..='힣').step_by(step))
        .chain(('ㄱ'..='ㆎ').step_by(step))
        .chain(('一'..='鿿').step_by(step * 3))
        .chain(('\u{2000}'..='\u{33ff}').step_by(step))
        .chain(('\u{ff00}'..='\u{ffef}').step_by(step))
        .collect();
    let mut drawn = 0;
    for (id, (ea, eb)) in whole.entries().iter().zip(kept.entries()).enumerate() {
        assert_eq!(
            (&ea.families, ea.weight, ea.italic, &ea.label),
            (&eb.families, eb.weight, eb.italic, &eb.label)
        );
        let id = FaceId(id as u32);
        let (a, b) = (whole.face(id).unwrap(), kept.face(id).unwrap());
        assert_eq!(
            (a.units_per_em, a.ascent, a.descent),
            (b.units_per_em, b.ascent, b.descent)
        );
        for &c in &chars {
            let (ga, gb) = (a.glyph(c), b.glyph(c));
            assert_eq!(ga, gb, "{} {c:?}", ea.label);
            if let Some(g) = ga {
                assert_eq!(a.synthetic_bold_extra(&g), b.synthetic_bold_extra(&g));
                assert_eq!(a.outline(g.glyph), b.outline(g.glyph), "{} {c:?}", ea.label);
                drawn += 1;
            }
        }
    }
    eprintln!("{} HFT files, {drawn} outlines compared", hft.len());
}
