//! Sparse host files preserve original glyph IDs and explicit coverage.
use hm_fonts::{FaceId, FontSystem};
use std::sync::Arc;
const SOURCE: &[u8] = include_bytes!("fixtures/source.ttf");
const SPARSE: &[u8] = include_bytes!("fixtures/sparse-A.ttf");
const COMPOSITE: &[u8] = include_bytes!("fixtures/sparse-composite.ttf");
fn coverage_offset(bytes: &[u8]) -> usize {
    let n = u16::from_be_bytes(bytes[4..6].try_into().unwrap()) as usize;
    (0..n)
        .find_map(|i| {
            let at = 12 + i * 16;
            (&bytes[at..at + 4] == b"HMGC")
                .then(|| u32::from_be_bytes(bytes[at + 8..at + 12].try_into().unwrap()) as usize)
        })
        .unwrap()
}
#[test]
fn omitted_glyphs_stay_nonresident_and_refill_without_changing_source_identity() {
    let mut full = FontSystem::new();
    full.add("f", Arc::new(SOURCE.to_vec()));
    let mut small = FontSystem::new();
    assert_eq!(
        small.add_compact("f", SPARSE, &['A', 'Q', 'é', '각']).faces,
        1
    );
    let a = full.face(FaceId(0)).unwrap();
    let b = small.face(FaceId(0)).unwrap();
    let g = a.glyph('A').unwrap().glyph;
    assert!(b.resident(g));
    assert_eq!(b.outline(g), a.outline(g));
    for c in ['Q', 'é', '각'] {
        let g = a.glyph(c).unwrap().glyph;
        assert!(!b.resident(g));
        b.outline(g);
    }
    assert_eq!(small.outline_requests(), ["f"]);
    let kept = small.glyph_coverage(&["f".into()]);
    assert_eq!(kept[0].0, "f");
    assert!(kept[0].2.contains(&g));
    let result = small.add_compact("f", SOURCE, &[]);
    assert!(result.refilled);
    assert_eq!(result.faces, 0);
    for c in ['A', 'Q', 'é', '각'] {
        let g = a.glyph(c).unwrap().glyph;
        let b = small.face(FaceId(0)).unwrap();
        assert!(b.resident(g));
        assert_eq!(b.outline(g), a.outline(g));
    }
    assert_eq!(small.entries().len(), 1);
    assert_eq!(small.entry(FaceId(0)).label, "f");
}
#[test]
fn sparse_manifests_validate_composite_dependencies_and_retained_refills() {
    let mut full = FontSystem::new();
    full.add_compact("f", SOURCE, &['A', 'Q', 'é']);
    let face = full.face(FaceId(0)).unwrap();
    let q = face.glyph('Q').unwrap().glyph;
    let expected = face.outline(q);
    full.add_compact("f", SPARSE, &[]);
    assert_eq!(
        full.face(FaceId(0)).unwrap().outline(q),
        expected,
        "a refill may not discard a retained glyph"
    );
    let mut bad = COMPOSITE.to_vec();
    let at = coverage_offset(&bad);
    let child = face.glyph('a').unwrap().glyph as usize;
    bad[at + 8 + child / 8] &= !(1 << (child % 8));
    let mut rejected = FontSystem::new();
    assert_eq!(rejected.add_compact("bad", bad.as_slice(), &['é']).faces, 0);
    assert!(rejected.entries().is_empty());
    bad = SPARSE.to_vec();
    let at = coverage_offset(&bad);
    bad[at] = 0;
    assert_eq!(rejected.add_compact("bad", bad.as_slice(), &['A']).faces, 0);
}
