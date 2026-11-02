//! A document with FileHeader bit 14 keeps the body Hancom opens in an
//! unprotected `ViewText`; its `BodyText` holds the same text with no stored
//! lines. Hancom cannot open such a file without `ViewText` (windows1).

use hm_hwp5::cfb::{self, Cfb};
use hm_hwp5::{deflate, record};
use hm_model::{Document, SharedBytes};

const PARA_LINE_SEG: u16 = 69;

/// `fixture` with bit 14 set, its section moved to `ViewText` and a copy
/// without line segments in `BodyText`.
fn with_view_text() -> Vec<u8> {
    let path = format!(
        "{}/tests/fixtures/hancom-note-shapes.hwp",
        env!("CARGO_MANIFEST_DIR")
    );
    let data = std::fs::read(path).unwrap();
    let source = Cfb::open(&data).unwrap();
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for (name, i) in source.paths() {
        if source.entries[*i].kind != cfb::STREAM {
            continue;
        }
        let mut bytes = source.read(name).unwrap();
        if name == "FileHeader" {
            bytes[37] |= 0x40;
        }
        if name == "BodyText/Section0" {
            files.push(("ViewText/Section0".into(), bytes.clone()));
            let plain = deflate::inflate(&bytes, 1 << 30).unwrap();
            let (records, _) = record::read_records(&SharedBytes::from(plain));
            let mut out = Vec::new();
            for r in records.iter().filter(|r| r.tag != PARA_LINE_SEG) {
                record::write_record(&mut out, r.tag, r.level, &r.payload);
            }
            bytes = deflate::deflate(&out);
        }
        files.push((name.clone(), bytes));
    }
    cfb::build(&files)
}

fn stored_lines(doc: &Document) -> usize {
    doc.sections[0]
        .paras
        .iter()
        .filter(|p| !p.line_segs.is_empty())
        .count()
}

#[test]
fn the_body_comes_from_view_text_and_conversion_keeps_its_lines() {
    let bytes = with_view_text();
    let doc = hm_hwp5::read(SharedBytes::from(bytes.clone())).unwrap();
    assert!(stored_lines(&doc) > 0);
    assert_eq!(stored_lines(&doc), doc.sections[0].paras.len());

    let mut open = hm_wasm::Engine::new().open(bytes).unwrap();
    let converted = open.serialize_as("hwpx", false).unwrap();
    let hwpx = hm_hwpx::read(SharedBytes::from(converted)).unwrap();
    assert_eq!(stored_lines(&hwpx), doc.sections[0].paras.len());
}

#[test]
fn an_edit_is_saved_where_the_body_was_read() {
    let mut doc = hm_hwp5::read(SharedBytes::from(with_view_text())).unwrap();
    doc.sections[0].paras[0].src = None;
    doc.sections[0].src = None;
    doc.sections[0].paras[0].line_segs[0].height += 100;
    let height = doc.sections[0].paras[0].line_segs[0].height;
    let back = hm_hwp5::read(SharedBytes::from(hm_hwp5::write(&doc).unwrap())).unwrap();
    assert_eq!(back.sections[0].paras[0].line_segs[0].height, height);
}
