//! Reading checks over the HWP 3 fixtures (`HEEMANG_CORPUS`).

use hm_model::{ControlKind, Paragraph, SharedBytes};

fn hwp3_files() -> Option<Vec<std::path::PathBuf>> {
    let Some(root) = std::env::var_os("HEEMANG_CORPUS") else {
        eprintln!("HEEMANG_CORPUS is not set; skipping corpus tests");
        return None;
    };
    let mut out = Vec::new();
    let mut stack = vec![std::path::PathBuf::from(root)];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).ok()?.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if std::fs::read(&p).is_ok_and(|b| hm_hwp3::is_hwp3(&b)) {
                out.push(p);
            }
        }
    }
    out.sort();
    Some(out)
}

fn text(ps: &[Paragraph], out: &mut String) {
    for p in ps {
        out.push_str(&p.plain_text());
        for c in &p.controls {
            if let ControlKind::Table(t) = &c.kind {
                for cell in &t.cells {
                    text(&cell.paras, out);
                }
            }
        }
    }
}

#[test]
fn every_fixture_opens_with_text() {
    let Some(files) = hwp3_files() else { return };
    let mut failures = Vec::new();
    for path in &files {
        let bytes = SharedBytes::from(std::fs::read(path).unwrap());
        match hm_hwp3::read(bytes) {
            Ok(doc) => {
                let mut s = String::new();
                text(&doc.sections[0].paras, &mut s);
                let replaced = s.chars().filter(|&c| c == '\u{FFFD}').count();
                if s.trim().is_empty() {
                    failures.push(format!("{}: no text", path.display()));
                } else if replaced * 200 > s.chars().count() {
                    failures.push(format!("{}: {replaced} unknown characters", path.display()));
                }
            }
            Err(e) => failures.push(format!("{}: {e}", path.display())),
        }
    }
    eprintln!("{} HWP 3 fixtures opened", files.len() - failures.len());
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn imported_exclusion_spans_match_native_without_using_aligned_text_origins() {
    let Some(root) = std::env::var_os("HEEMANG_CORPUS") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    for (source_name, native_name, paragraphs) in [
        (
            "hwp3-sample5.hwp",
            "hwp3-sample5-hwp5.hwp",
            &[74, 75, 76, 175, 443, 605][..],
        ),
        (
            "hwp3-sample4.hwp",
            "hwp3-sample4-hwp5.hwp",
            &[27, 29, 1040][..],
        ),
        ("SO-SUEOP.hwp", "SO-SUEOP.hwpx", &[5, 13, 17][..]),
    ] {
        let source = hm_hwp3::read(SharedBytes::from(
            std::fs::read(root.join(source_name)).unwrap(),
        ))
        .unwrap();
        let bytes = SharedBytes::from(std::fs::read(root.join(native_name)).unwrap());
        let native = if hm_hwp5::is_hwp5(&bytes) {
            hm_hwp5::read(bytes).unwrap()
        } else {
            hm_hwpx::read(bytes).unwrap()
        };
        for &index in paragraphs {
            let p = &source.sections[0].paras[index];
            let n = &native.sections[0].paras[index];
            assert_eq!(p.text, n.text, "{source_name} paragraph {index}");
            assert_eq!(p.line_segs, n.line_segs, "{source_name} paragraph {index}");
            assert!(p.src.is_some());
        }
    }
}

#[test]
fn imported_stored_right_spans_match_native_object_boundaries() {
    let Some(root) = std::env::var_os("HEEMANG_CORPUS") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    for (name, paragraphs) in [
        ("hwp3-sample19", &[1, 2, 3, 4, 5, 6, 7][..]),
        ("hwp3-sample4", &[333, 334, 335, 336, 1030][..]),
    ] {
        let source = hm_hwp3::read(SharedBytes::from(
            std::fs::read(root.join(format!("{name}.hwp"))).unwrap(),
        ))
        .unwrap();
        let native = hm_hwp5::read(SharedBytes::from(
            std::fs::read(root.join(format!("{name}-hwp5.hwp"))).unwrap(),
        ))
        .unwrap();
        for &index in paragraphs {
            let p = &source.sections[0].paras[index];
            let n = &native.sections[0].paras[index];
            assert_eq!(p.text, n.text, "{name} paragraph {index}");
            assert_eq!(p.line_segs, n.line_segs, "{name} paragraph {index}");
        }
    }
}

#[test]
fn imported_spacing_matches_native_across_object_placement_and_page_ends() {
    let Some(root) = std::env::var_os("HEEMANG_CORPUS") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    for (name, paragraphs) in [
        ("hwp3-sample", &[77, 91, 194][..]),
        ("hwp3-sample11", &[105, 151, 358, 3369][..]),
    ] {
        let source = hm_hwp3::read(SharedBytes::from(
            std::fs::read(root.join(format!("{name}.hwp"))).unwrap(),
        ))
        .unwrap();
        let native = hm_hwp5::read(SharedBytes::from(
            std::fs::read(root.join(format!("{name}-hwp5.hwp"))).unwrap(),
        ))
        .unwrap();
        for &index in paragraphs {
            let p = &source.sections[0].paras[index];
            let n = &native.sections[0].paras[index];
            assert_eq!(p.plain_text(), n.plain_text(), "{name} paragraph {index}");
            assert_eq!(p.line_segs, n.line_segs, "{name} paragraph {index}");
        }
    }
}

#[test]
fn imported_floating_object_placement_matches_native_conversion() {
    let Some(root) = std::env::var_os("HEEMANG_CORPUS") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    let source = hm_hwp3::read(SharedBytes::from(
        std::fs::read(root.join("hwp3-sample.hwp")).unwrap(),
    ))
    .unwrap();
    let native = hm_hwp5::read(SharedBytes::from(
        std::fs::read(root.join("hwp3-sample-hwp5.hwp")).unwrap(),
    ))
    .unwrap();
    for index in [41, 76, 78, 97] {
        let common = |doc: &hm_model::Document| {
            let mut common = doc.sections[0].paras[index].controls[0]
                .object_common()
                .unwrap();
            // Compare placement independently of numbering and draw order.
            common.attr &= (1 << 26) - 1;
            common.z_order = 0;
            common
        };
        assert_eq!(common(&source), common(&native), "paragraph {index}");
    }
}

#[test]
fn imported_spacing_ignores_styles_on_noncontributing_note_controls() {
    let Some(root) = std::env::var_os("HEEMANG_CORPUS") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    let source = hm_hwp3::read(SharedBytes::from(
        std::fs::read(root.join("SO-SUEOP.hwp")).unwrap(),
    ))
    .unwrap();
    let native = hm_hwpx::read(SharedBytes::from(
        std::fs::read(root.join("SO-SUEOP.hwpx")).unwrap(),
    ))
    .unwrap();
    for index in [231, 233, 266, 542, 544, 546] {
        let note = |d: &hm_model::Document| match &d.sections[0].paras[index].controls[0].kind {
            ControlKind::Note(n) => n.list.paras[0].line_segs.clone(),
            _ => panic!("expected a note"),
        };
        let a = note(&source);
        let b = note(&native);
        assert_eq!(a.len(), b.len());
        assert_eq!(
            a.iter().map(|l| l.spacing).collect::<Vec<_>>(),
            b.iter().map(|l| l.spacing).collect::<Vec<_>>()
        );
        if let ([a0, a1, ..], [b0, b1, ..]) = (a.as_slice(), b.as_slice()) {
            assert_eq!(a1.vpos - a0.vpos, b1.vpos - b0.vpos);
        }
    }
}

#[test]
fn imported_table_grids_match_native_where_cells_leave_gaps() {
    let Some(root) = std::env::var_os("HEEMANG_CORPUS") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    let source = hm_hwp3::read(SharedBytes::from(
        std::fs::read(root.join("hwp3-sample16.hwp")).unwrap(),
    ))
    .unwrap();
    let native = hm_hwp5::read(SharedBytes::from(
        std::fs::read(root.join("hwp3-sample16-hwp5-2010.hwp")).unwrap(),
    ))
    .unwrap();
    // One-unit gaps between neighbours (807-898) and unaligned starts that
    // stay columns of their own (876).
    for index in [807, 817, 821, 876, 894, 898] {
        let table = |d: &hm_model::Document| match &d.sections[0].paras[index].controls[0].kind {
            ControlKind::Table(t) => {
                let cells: Vec<_> = t
                    .cells
                    .iter()
                    .map(|c| (c.row, c.col, c.row_span, c.col_span, c.width))
                    .collect();
                (t.rows, t.cols, cells)
            }
            _ => panic!("expected a table at paragraph {index}"),
        };
        assert_eq!(table(&source), table(&native), "paragraph {index}");
    }
}

fn groups_with_geometry(ps: &[Paragraph], out: &mut usize) {
    fn shape(s: &hm_model::Shape, out: &mut usize) {
        let group = s.component.kind == hm_model::CtrlId::from_chars(b"$con");
        for part in &s.parts {
            match part {
                hm_model::ShapePart::Child(c) => shape(c, out),
                hm_model::ShapePart::Text(t) => groups_with_geometry(&t.paras, out),
                _ if group => *out += 1,
                _ => {}
            }
        }
    }
    for p in ps {
        for c in &p.controls {
            match &c.kind {
                ControlKind::Object(o) => {
                    if let Some(s) = o.shape() {
                        shape(s, out);
                    }
                }
                ControlKind::Table(t) => {
                    for cell in &t.cells {
                        groups_with_geometry(&cell.paras, out);
                    }
                }
                ControlKind::HeaderFooter(h) => groups_with_geometry(&h.list.paras, out),
                ControlKind::Note(n) => groups_with_geometry(&n.list.paras, out),
                _ => {}
            }
        }
    }
}

#[test]
fn drawing_groups_hold_only_their_members() {
    // sample11: a group saved to HWP with a curve record of its own beside
    // its members made Hancom refuse the whole file.
    let Some(files) = hwp3_files() else { return };
    let mut failures = Vec::new();
    for path in &files {
        let bytes = SharedBytes::from(std::fs::read(path).unwrap());
        let Ok(doc) = hm_hwp3::read(bytes) else {
            continue;
        };
        let mut n = 0;
        for s in &doc.sections {
            groups_with_geometry(&s.paras, &mut n);
        }
        if n > 0 {
            failures.push(format!("{}: {n} groups with geometry", path.display()));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
