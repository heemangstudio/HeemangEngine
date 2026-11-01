//! Compact copies measure every character as the whole file does, draw the
//! characters they were built with identically, and fill in other outlines
//! when the host registers the file again.
//!
//! Needs `HEEMANG_FONTS` (`:`-separated, `;` on Windows: directories, walked recursively);
//! skips when it is unset or holds none of the probed files.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use hm_fonts::{FaceId, FontSystem};

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

/// A CJK font, a collection and a Latin font, when present.
fn probes() -> Vec<PathBuf> {
    let Ok(spec) = std::env::var("HEEMANG_FONTS") else {
        return Vec::new();
    };
    let mut files = Vec::new();
    for part in std::env::split_paths(&spec).filter(|p| !p.as_os_str().is_empty()) {
        walk(&part, &mut files);
    }
    let wanted = [
        "hanbatang.ttf",
        "gulim.ttf",
        "times.ttf",
        "malgun.ttf",
        "helvetica.ttc",
    ];
    files.retain(|f| {
        let name = f.file_name().and_then(|n| n.to_str()).unwrap_or("");
        wanted.contains(&name.to_ascii_lowercase().as_str())
    });
    files.sort();
    files.dedup_by_key(|f| f.file_name().map(|n| n.to_ascii_lowercase()));
    files
}

#[test]
fn compact_copies_match_whole_files_and_refill() {
    let files = probes();
    if files.is_empty() {
        eprintln!("HEEMANG_FONTS is unset or has none of the probed fonts; skipping");
        return;
    }
    let built: Vec<char> = "가나다 ABC xyz 0123 漢字".chars().collect();
    let later: Vec<char> = "鬱뷁Ωq".chars().collect();
    for f in &files {
        let label = f.display().to_string();
        let bytes = std::fs::read(f).unwrap();
        let mut whole = FontSystem::new();
        let n = whole.add(label.clone(), Arc::new(bytes.clone()));
        let mut small = FontSystem::new();
        let added = small.add_compact(label.clone(), bytes.as_slice(), &built);
        assert_eq!(added.faces, n, "{label}");
        assert!(small.resident_bytes() < bytes.len(), "{label}");
        for id in (0..n as u32).map(FaceId) {
            let (a, b) = (whole.face(id).unwrap(), small.face(id).unwrap());
            let (we, se) = (whole.entry(id), small.entry(id));
            assert_eq!(
                (
                    &we.families,
                    &we.local,
                    &we.postscript,
                    we.weight,
                    we.italic
                ),
                (
                    &se.families,
                    &se.local,
                    &se.postscript,
                    se.weight,
                    se.italic
                )
            );
            for &c in built.iter().chain(&later) {
                assert_eq!(a.glyph(c), b.glyph(c), "{label} {c}");
            }
            for &c in &built {
                if let Some(g) = a.glyph(c) {
                    assert!(b.resident(g.glyph));
                    assert_eq!(a.outline(g.glyph), b.outline(g.glyph), "{label} {c}");
                }
            }
        }
        // Outlines outside the copy come empty and ask for the file.
        let mut missing = Vec::new();
        for id in (0..n as u32).map(FaceId) {
            let b = small.face(id).unwrap();
            for &c in &later {
                if let Some(g) = b.glyph(c)
                    && !b.resident(g.glyph)
                {
                    assert!(b.outline(g.glyph).ops.is_empty());
                    missing.push((id, c));
                }
            }
        }
        if missing.is_empty() {
            continue;
        }
        assert_eq!(small.outline_requests(), std::slice::from_ref(&label));
        let again = small.add_compact(label.clone(), bytes.as_slice(), &[]);
        assert!(again.refilled && again.faces == 0 && !again.relayout);
        assert!(small.outline_requests().is_empty());
        for (id, c) in missing {
            let (a, b) = (whole.face(id).unwrap(), small.face(id).unwrap());
            let g = a.glyph(c).unwrap().glyph;
            assert!(b.resident(g));
            assert_eq!(a.outline(g), b.outline(g), "{label} {c}");
        }
    }
}

#[test]
fn missing_outlines_during_layout_ask_for_a_new_layout() {
    let Some(f) = probes().into_iter().next() else {
        eprintln!("HEEMANG_FONTS is unset or has none of the probed fonts; skipping");
        return;
    };
    let bytes = std::fs::read(&f).unwrap();
    let mut fonts = FontSystem::new();
    fonts.add_compact("f", bytes.as_slice(), &[]);
    let face = fonts.face(FaceId(0)).unwrap();
    let g = face.glyph('A').unwrap().glyph;
    fonts.set_layout_running(true);
    assert!(face.outline(g).ops.is_empty());
    fonts.set_layout_running(false);
    let again = fonts.add_compact("f", bytes.as_slice(), &[]);
    assert!(again.refilled && again.relayout);
    assert!(!fonts.face(FaceId(0)).unwrap().outline(g).ops.is_empty());
}

#[test]
fn host_complete_sfnt_sources_match_whole_files_without_refills_or_duplicate_storage() {
    let files = probes();
    if files.is_empty() {
        eprintln!("HEEMANG_FONTS lacks the probed fonts; skipping");
        return;
    }
    for path in files {
        let bytes = std::fs::read(path).unwrap();
        let mut host = FontSystem::new();
        let mut whole = FontSystem::new();
        let added = host.add_host_font("f", bytes.as_slice(), &['A']);
        assert_eq!(added.faces, whole.add("f", Arc::new(bytes.clone())));
        assert_eq!(host.resident_bytes(), bytes.len());
        for id in (0..added.faces as u32).map(FaceId) {
            let (a, b) = (host.face(id).unwrap(), whole.face(id).unwrap());
            for c in "Aq뷁Ω".chars() {
                assert_eq!(a.glyph(c), b.glyph(c));
                if let Some(g) = a.glyph(c) {
                    assert_eq!(a.resident(g.glyph), b.resident(g.glyph));
                    assert_eq!(a.outline(g.glyph), b.outline(g.glyph));
                }
            }
        }
        let again = host.add_host_font("f", bytes.as_slice(), &[]);
        assert!(again.refilled && again.faces == 0 && !again.relayout);
        assert_eq!(host.resident_bytes(), bytes.len());
        assert_eq!(host.entries().len(), whole.entries().len());
        assert!(host.outline_requests().is_empty());
    }
}

#[test]
fn unregistered_files_leave_the_catalog() {
    let Ok(spec) = std::env::var("HEEMANG_FONTS") else {
        eprintln!("HEEMANG_FONTS is not set; skipping");
        return;
    };
    let mut files = Vec::new();
    for part in std::env::split_paths(&spec).filter(|p| !p.as_os_str().is_empty()) {
        walk(&part, &mut files);
    }
    let find = |name: &str| {
        files
            .iter()
            .find(|f| f.file_name().is_some_and(|n| n.eq_ignore_ascii_case(name)))
            .cloned()
    };
    let (Some(ttf), Some(latin), Some(hangul)) =
        (find("times.ttf"), find("ENSMJ.HFT"), find("HGSMJ.HFT"))
    else {
        eprintln!("HEEMANG_FONTS lacks the probed files; skipping");
        return;
    };
    let mut fonts = FontSystem::new();
    let read = |p: &PathBuf| std::fs::read(p).unwrap();
    fonts.add_compact("ttf", read(&ttf).as_slice(), &['A']);
    fonts.add_compact("latin", read(&latin).as_slice(), &[]);
    fonts.add_compact("hangul", read(&hangul).as_slice(), &[]);
    let family = |fonts: &FontSystem| fonts.family("한양신명조").to_vec();
    let hft = FaceId(family(&fonts)[0]);
    let bytes = fonts.resident_bytes();
    assert!(fonts.face(hft).unwrap().has('A') && fonts.face(hft).unwrap().has('가'));

    assert_eq!(fonts.remove("ttf"), 1);
    assert!(fonts.family("Times New Roman").is_empty());
    assert!(fonts.face(FaceId(0)).is_none());
    assert!(fonts.resident_bytes() < bytes);

    // An HFT family keeps the files still registered.
    assert_eq!(fonts.remove("latin"), 0);
    let face = fonts.face(hft).unwrap();
    assert!(!face.has('A') && face.has('가'));
    assert_eq!(fonts.remove("hangul"), 1);
    assert!(family(&fonts).is_empty() && fonts.face(hft).is_none());
}
