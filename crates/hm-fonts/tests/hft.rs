//! Every HFT file Hancom ships parses, and its faces resolve and measure.
//!
//! Needs `HEEMANG_FONTS` (`:`-separated directories, walked recursively);
//! skips when it is unset or holds no HFT file.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use hm_fonts::hft::HftFile;
use hm_fonts::{Added, FaceId, FaceQuery, FontSystem};

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

#[test]
fn hancom_hft_files_decode_and_measure() {
    let Ok(spec) = std::env::var("HEEMANG_FONTS") else {
        eprintln!("HEEMANG_FONTS is not set; skipping");
        return;
    };
    let mut files = Vec::new();
    for part in spec.split(':').filter(|s| !s.is_empty()) {
        walk(Path::new(part), &mut files);
    }
    files.sort();
    let mut fonts = FontSystem::new();
    let (mut parsed, mut glyphs) = (0, 0);
    for f in &files {
        let ext = f.extension().and_then(|e| e.to_str()).unwrap_or("");
        let name = f.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with("u300_") {
            fonts.add_code_table(&std::fs::read(f).unwrap());
            continue;
        }
        if !ext.eq_ignore_ascii_case("hft") {
            continue;
        }
        let bytes = Arc::new(std::fs::read(f).unwrap());
        let file =
            HftFile::parse(bytes.clone()).unwrap_or_else(|e| panic!("{}: {e:?}", f.display()));
        parsed += 1;
        // Every outline the file declares decodes to a program that draws.
        for code in [0x41u16, 0x8861, 0x4000] {
            if let Some(g) = file.glyph(code) {
                glyphs += 1;
                assert!(file.outline(g).is_some(), "{}: {code:#x}", f.display());
            }
        }
        fonts.add(f.display().to_string(), bytes);
    }
    if parsed == 0 {
        eprintln!("no HFT files under HEEMANG_FONTS; skipping");
        return;
    }
    assert!(glyphs > 0);
    // 신명 중명조 spans an English and a Hangul file; both measure.
    let r = fonts
        .resolve(&FaceQuery {
            name: "신명 중명조",
            ..Default::default()
        })
        .expect("a face");
    assert!(r.exact);
    let face = fonts.face(r.face).unwrap();
    assert!(face.is_hft());
    let ga = face.glyph('가').expect("Hangul glyph");
    assert_eq!((ga.advance, ga.em), (1000, 1000));
    let a = face.glyph('A').expect("Latin glyph");
    assert!(a.advance > 0 && a.advance < 1000);
    assert!(!face.outline(ga.glyph).ops.is_empty());
    // 중간공한 builds its syllables from jamo glyphs: 각 draws more than 가.
    let r = fonts
        .resolve(&FaceQuery {
            name: "중간공한",
            ..Default::default()
        })
        .expect("a face");
    let face = fonts.face(r.face).unwrap();
    let ops = |c| {
        face.outline(face.glyph(c).expect("composed glyph").glyph)
            .ops
            .len()
    };
    assert!(ops('가') > 0 && ops('각') > ops('가'));
}

#[test]
fn compact_hft_copies_draw_what_they_keep_and_refill() {
    let Ok(spec) = std::env::var("HEEMANG_FONTS") else {
        eprintln!("HEEMANG_FONTS is not set; skipping");
        return;
    };
    let mut files = Vec::new();
    for part in spec.split(':').filter(|s| !s.is_empty()) {
        walk(Path::new(part), &mut files);
    }
    // A face that composes syllables from jamo and one that stores them.
    for name in ["KHANM.HFT", "HMKSM.HFT"] {
        let Some(f) = files
            .iter()
            .find(|f| f.file_name().is_some_and(|n| n.eq_ignore_ascii_case(name)))
        else {
            eprintln!("{name} not under HEEMANG_FONTS; skipping it");
            continue;
        };
        let label = f.display().to_string();
        let bytes = std::fs::read(f).unwrap();
        let built: Vec<char> = "가각한글 ABC".chars().collect();
        let later: Vec<char> = "뷁값됐".chars().collect();
        let mut whole = FontSystem::new();
        whole.add(label.clone(), Arc::new(bytes.clone()));
        let mut small = FontSystem::new();
        let added = small.add_compact(label.clone(), bytes.as_slice(), &built);
        assert_eq!(added.faces, 1);
        assert!(small.resident_bytes() * 4 < bytes.len(), "{label}");
        let id = hm_fonts::FaceId(0);
        let (a, b) = (whole.face(id).unwrap(), small.face(id).unwrap());
        for &c in built.iter().chain(&later) {
            assert_eq!(a.glyph(c), b.glyph(c), "{label} {c}");
        }
        for &c in built.iter().filter(|&&c| a.glyph(c).is_some()) {
            let g = b.glyph(c).unwrap().glyph;
            assert!(b.resident(g));
            assert_eq!(a.outline(g), b.outline(g), "{label} {c}");
        }
        // Composed syllables share jamo, so a later one may be whole already.
        let mut missing = 0;
        for &c in &later {
            let g = b.glyph(c).expect("Hangul glyph").glyph;
            if !b.resident(g) {
                assert!(b.outline(g).ops.is_empty(), "{label} {c}");
                missing += 1;
            }
        }
        assert!(missing > 0, "{label}");
        assert_eq!(small.outline_requests(), std::slice::from_ref(&label));
        let again = small.add_compact(label.clone(), bytes.as_slice(), &[]);
        assert!(again.refilled && again.faces == 0);
        assert!(small.outline_requests().is_empty());
        let b = small.face(id).unwrap();
        for &c in built
            .iter()
            .chain(&later)
            .filter(|&&c| a.glyph(c).is_some())
        {
            let g = b.glyph(c).unwrap().glyph;
            assert_eq!(a.outline(g), b.outline(g), "{label} {c} after refill");
        }
    }
}

#[test]
fn host_hft_sources_draw_unseen_characters_and_release_without_duplicate_banks() {
    let Ok(spec) = std::env::var("HEEMANG_FONTS") else {
        eprintln!("HEEMANG_FONTS is not set; skipping");
        return;
    };
    let mut files = Vec::new();
    for part in spec.split(':').filter(|s| !s.is_empty()) {
        walk(Path::new(part), &mut files);
    }
    let read = |name: &str| {
        files
            .iter()
            .find(|f| f.file_name().is_some_and(|n| n.eq_ignore_ascii_case(name)))
            .map(|p| std::fs::read(p).unwrap())
    };
    let (Some(hangul), Some(latin)) = (read("TESSMHG.HFT"), read("TESSMEN.HFT")) else {
        eprintln!("TESSMHG.HFT or TESSMEN.HFT not under HEEMANG_FONTS; skipping");
        return;
    };
    let mut host = FontSystem::new();
    let mut whole = FontSystem::new();
    for path in &files {
        if path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("u300_"))
        {
            let bytes = std::fs::read(path).unwrap();
            host.add_code_table(&bytes);
            whole.add_code_table(&bytes);
        }
    }
    whole.add("hangul", Arc::new(hangul.clone()));
    assert_eq!(
        host.add_host_font("hangul", hangul.as_slice(), &['가'])
            .faces,
        1
    );
    let id = FaceId(0);
    let family = host.entry(id).families.clone();
    let source = Arc::downgrade(&host.hft_files()[0].1);
    let resident = host.resident_bytes();
    let reference = whole.face(id).unwrap();
    let face = host.face(id).unwrap();
    // None of these was in the initial document's requested character set.
    // Include compatibility jamo and syllables outside the observed IME trace.
    let mut tested = [0, 0];
    for (kind, chars) in ["ㅆㅉㅘㅢㄲㅏㅔ", "깎뛴쯤땅흠즙"].iter().enumerate() {
        for c in chars.chars() {
            let Some(g) = reference.glyph(c) else {
                continue;
            };
            tested[kind] += 1;
            assert_eq!(face.glyph(c), Some(g), "{c}");
            assert!(face.resident(g.glyph), "{c}");
            let outline = face.outline(g.glyph);
            assert!(!outline.ops.is_empty(), "{c}");
            assert_eq!(outline, reference.outline(g.glyph), "{c}");
        }
    }
    assert!(tested.iter().all(|&n| n >= 2), "{tested:?}");
    assert_eq!(resident, hangul.len());
    assert!(host.outline_requests().is_empty());
    // Promoting an existing compact source preserves its face id and
    // requests the relayout owed to an outline missed during layout.
    let mut promoted = FontSystem::new();
    promoted.add_compact("hangul", hangul.as_slice(), &['가']);
    let compact = promoted.face(id).unwrap();
    let later = "깎뛴쯤땅흠즙"
        .chars()
        .find(|&c| compact.glyph(c).is_some_and(|g| !compact.resident(g.glyph)))
        .expect("an unseen compact glyph");
    promoted.set_layout_running(true);
    assert!(
        compact
            .outline(compact.glyph(later).unwrap().glyph)
            .ops
            .is_empty()
    );
    promoted.set_layout_running(false);
    drop(compact);
    let upgraded = promoted.add_host_font("hangul", hangul.as_slice(), &[]);
    assert!(upgraded.refilled && upgraded.relayout && upgraded.faces == 0);
    let upgraded = promoted.face(id).unwrap();
    let (a, b) = (
        upgraded.glyph(later).unwrap(),
        reference.glyph(later).unwrap(),
    );
    assert_eq!((a.advance, a.em), (b.advance, b.em));
    assert_eq!(
        upgraded.outline(upgraded.glyph(later).unwrap().glyph),
        reference.outline(reference.glyph(later).unwrap().glyph)
    );
    assert!(promoted.outline_requests().is_empty());
    let original = face.glyph('가').unwrap();
    let original_outline = face.outline(original.glyph);
    drop(face);
    // A cached Hangul face must see a subsequently registered Latin bank.
    whole.add("latin", Arc::new(latin.clone()));
    assert_eq!(host.add_host_font("latin", latin.as_slice(), &[]).faces, 0);
    let face = host.face(id).unwrap();
    let reference = whole.face(id).unwrap();
    let a = face.glyph('A').expect("Latin bank glyph");
    let ga = face.glyph('가').unwrap();
    assert_eq!((ga.advance, ga.em), (original.advance, original.em));
    assert_eq!(face.outline(ga.glyph), original_outline);
    assert_eq!(
        face.outline(a.glyph),
        reference.outline(reference.glyph('A').unwrap().glyph)
    );
    assert!(!face.outline(a.glyph).ops.is_empty());
    assert_eq!(host.family(&family[0]), &[id.0]);
    assert_eq!(host.entry(id).label, "hangul");
    assert_eq!(host.hft_files().len(), 2);
    for (label, bytes) in [("hangul", &hangul), ("latin", &latin)] {
        assert_eq!(
            host.add_host_font(label, bytes.as_slice(), &[]),
            Added {
                refilled: true,
                ..Added::default()
            }
        );
    }
    assert_eq!(host.hft_files().len(), 2);
    assert_eq!(host.resident_bytes(), resident + latin.len());
    assert!(Arc::ptr_eq(
        &source.upgrade().unwrap(),
        &host.hft_files()[0].1
    ));
    drop(face);
    assert_eq!(host.remove("latin"), 0);
    assert!(host.face(id).unwrap().has('가'));
    assert!(!host.face(id).unwrap().has('A'));
    assert_eq!(host.remove("hangul"), 1);
    assert!(host.face(id).is_none());
    assert!(host.family(&family[0]).is_empty());
    assert_eq!(host.resident_bytes(), 0);
    assert!(source.upgrade().is_none());
}
