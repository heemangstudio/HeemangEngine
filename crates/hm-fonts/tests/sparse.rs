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

#[test]
fn host_complete_sources_draw_unseen_glyphs_and_promote_sparse_faces_without_new_ids() {
    let mut expected = FontSystem::new();
    expected.add("f", Arc::new(SOURCE.to_vec()));
    let full = expected.face(FaceId(0)).unwrap();
    for sparse_first in [false, true] {
        let mut host = FontSystem::new();
        if sparse_first {
            assert_eq!(host.add_host_font("f", SPARSE, &['A']).faces, 1);
            let face = host.face(FaceId(0)).unwrap();
            let glyph = face.glyph('Q').unwrap().glyph;
            assert!(!face.resident(glyph));
            host.set_layout_running(true);
            face.outline(glyph);
            host.set_layout_running(false);
            assert_eq!(host.outline_requests(), ["f"]);
        }
        let added = host.add_host_font("f", SOURCE, &['A']);
        assert_eq!(added.faces, usize::from(!sparse_first));
        assert_eq!(added.refilled, sparse_first);
        assert_eq!(added.relayout, sparse_first);
        assert_eq!(host.entries().len(), 1);
        assert_eq!(host.entry(FaceId(0)).label, "f");
        for ch in ['A', 'Q', 'é', '각'] {
            let face = host.face(FaceId(0)).unwrap();
            let glyph = full.glyph(ch).unwrap().glyph;
            assert!(face.resident(glyph), "{ch}");
            assert_eq!(face.outline(glyph), full.outline(glyph), "{ch}");
        }
        assert!(host.outline_requests().is_empty());
        let kept = host.resident_bytes();
        assert!(host.add_host_font("f", SPARSE, &[]).refilled);
        assert!(host.add_host_font("f", SOURCE, &[]).refilled);
        assert_eq!(host.entries().len(), 1);
        assert_eq!(host.resident_bytes(), kept);
        assert!(host.outline_requests().is_empty());
        assert_eq!(host.remove("f"), 1);
        assert!(host.resident_bytes() < kept);
        assert_eq!(host.add_host_font("f", SOURCE, &[]).faces, 1);
        assert!(host.face(FaceId(0)).is_none());
        assert!(host.face(FaceId(1)).is_some());
    }
}

#[test]
fn promoting_a_sparse_collection_shares_backing_bytes_and_preserves_face_indices() {
    let collection = |font: &[u8]| {
        let mut bytes = b"ttcf\0\x01\0\0\0\0\0\x02\0\0\0\x14\0\0\0\x14".to_vec();
        bytes.extend_from_slice(font);
        for record in 0..u16::from_be_bytes(font[4..6].try_into().unwrap()) as usize {
            let at = 20 + 12 + record * 16 + 8;
            let offset = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) + 20;
            bytes[at..at + 4].copy_from_slice(&offset.to_be_bytes());
        }
        bytes
    };
    let sparse = collection(SPARSE);
    let complete = collection(SOURCE);
    let mut host = FontSystem::new();
    assert_eq!(host.add_host_font("c", sparse.as_slice(), &['A']).faces, 2);
    assert!(host.add_host_font("c", complete.as_slice(), &[]).refilled);
    assert_eq!(host.resident_bytes(), complete.len());
    for index in 0..2 {
        assert_eq!(host.entry(FaceId(index)).index, index);
        let face = host.face(FaceId(index)).unwrap();
        assert!(face.resident(face.glyph('Q').unwrap().glyph));
    }
    assert_eq!(host.remove("c"), 2);
    assert_eq!(host.resident_bytes(), 0);
}
