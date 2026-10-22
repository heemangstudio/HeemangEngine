//! Reading and saving checks over the HWPX fixtures (`HEEMANG_CORPUS`)
//! and the probes in the `probes` directory beside it.

use std::path::{Path, PathBuf};

use hm_model::SharedBytes;

fn corpus() -> Option<Vec<PathBuf>> {
    let Some(root) = std::env::var_os("HEEMANG_CORPUS") else {
        eprintln!("HEEMANG_CORPUS is not set; skipping corpus tests");
        return None;
    };
    let mut out = Vec::new();
    collect(Path::new(&root), &mut out);
    // Hand-built probes (track changes, ruby text, overlaid characters)
    // live beside the corpus.
    if let Some(parent) = Path::new(&root).parent() {
        collect(&parent.join("probes"), &mut out);
    }
    out.sort();
    Some(out)
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(&p, out);
        } else if std::fs::read(&p).is_ok_and(|b| hm_hwpx::is_hwpx(&b)) {
            out.push(p);
        }
    }
}

#[test]
fn every_fixture_reads_and_saves_unchanged() {
    let Some(files) = corpus() else { return };
    let mut opened = 0;
    let mut failures = Vec::new();
    for path in &files {
        let bytes = SharedBytes::from(std::fs::read(path).unwrap());
        let doc = match hm_hwpx::read(bytes.clone()) {
            Ok(d) => d,
            Err(e) => {
                failures.push(format!("{}: open: {e}", path.display()));
                continue;
            }
        };
        opened += 1;
        if doc.sections.is_empty() {
            failures.push(format!("{}: no sections", path.display()));
        }
        match hm_hwpx::write(&doc) {
            Ok(saved) if saved[..] == bytes[..] => {}
            Ok(_) => failures.push(format!("{}: no-op save differs", path.display())),
            Err(e) => failures.push(format!("{}: save: {e}", path.display())),
        }
    }
    eprintln!("{opened} of {} HWPX fixtures opened", files.len());
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The model as text with source spans left out: what a save must keep.
fn model_text(doc: &hm_model::Document) -> String {
    let s = format!("{:?}{:?}", doc.info, doc.sections);
    let mut out = String::with_capacity(s.len());
    let mut rest = s.as_str();
    while let Some(i) = rest.find("src: ") {
        out.push_str(&rest[..i]);
        rest = &rest[i + 5..];
        if let Some(r) = rest.strip_prefix("None") {
            rest = r;
        } else if let Some(j) = rest.find("))") {
            rest = &rest[j + 2..];
        }
    }
    out.push_str(rest);
    out
}

#[test]
fn every_fixture_touch_saves_to_an_equal_model() {
    let Some(files) = corpus() else { return };
    let mut failures = Vec::new();
    let mut checked = 0;
    for path in &files {
        let bytes = SharedBytes::from(std::fs::read(path).unwrap());
        let Ok(mut doc) = hm_hwpx::read(bytes) else {
            continue;
        };
        let before = model_text(&doc);
        doc.mark_all_dirty();
        let saved = match hm_hwpx::write(&doc) {
            Ok(s) => s,
            Err(e) => {
                failures.push(format!("{}: save: {e}", path.display()));
                continue;
            }
        };
        match hm_hwpx::read(SharedBytes::from(saved)) {
            Ok(again) if model_text(&again) == before => checked += 1,
            Ok(_) => failures.push(format!("{}: model changed", path.display())),
            Err(e) => failures.push(format!("{}: reopen: {e}", path.display())),
        }
    }
    eprintln!("{checked} HWPX fixtures touch-saved to an equal model");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// BinData ids of pictures anywhere in a paragraph list.
fn picture_ids(paras: &[hm_model::Paragraph], out: &mut Vec<u16>) {
    use hm_model::{ControlKind, ObjectPart, Shape, ShapePart};
    fn shape(s: &Shape, out: &mut Vec<u16>) {
        for p in &s.parts {
            match p {
                ShapePart::Picture(pic) => out.push(pic.image.bin_id),
                ShapePart::Text(t) => picture_ids(&t.paras, out),
                ShapePart::Child(c) => shape(c, out),
                _ => {}
            }
        }
        if let Some(i) = s.style().and_then(|st| st.fill.image) {
            out.push(i.bin_id);
        }
    }
    for p in paras {
        for c in &p.controls {
            match &c.kind {
                ControlKind::Table(t) => {
                    for cell in &t.cells {
                        picture_ids(&cell.paras, out);
                    }
                }
                ControlKind::Object(o) => {
                    for part in &o.parts {
                        match part {
                            ObjectPart::Shape(s) => shape(s, out),
                            ObjectPart::Caption(c) => picture_ids(&c.paras, out),
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

#[test]
fn picture_references_resolve_to_bin_data() {
    let Some(files) = corpus() else { return };
    let (mut pictures, mut failures) = (0, Vec::new());
    for path in &files {
        let bytes = SharedBytes::from(std::fs::read(path).unwrap());
        let Ok(doc) = hm_hwpx::read(bytes) else {
            continue;
        };
        let mut ids = Vec::new();
        for s in &doc.sections {
            picture_ids(&s.paras, &mut ids);
        }
        let bins = doc
            .bins
            .as_ref()
            .expect("HWPX documents carry their BinData");
        for id in ids {
            // Linked pictures and empty references have no bytes to find.
            let linked = (id as usize)
                .checked_sub(1)
                .and_then(|i| doc.info.bin_data.get(i))
                .is_none_or(|e| e.kind() == 0);
            if linked {
                continue;
            }
            pictures += 1;
            if bins.get(id).is_none_or(|b| b.is_empty()) {
                failures.push(format!("{}: picture BinData {id} missing", path.display()));
            }
        }
    }
    eprintln!("{pictures} picture references checked");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
