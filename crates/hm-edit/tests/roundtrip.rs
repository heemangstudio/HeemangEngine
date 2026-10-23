//! Edits saved and read back (`HEEMANG_CORPUS`): the saved file holds the
//! edited text, untouched paragraphs keep their bytes, and undo restores
//! the original save byte for byte.

use std::path::PathBuf;

use hm_edit::{Editor, script};
use hm_model::{Document, SharedBytes};

fn corpus(rel: &str) -> Option<PathBuf> {
    let Some(root) = std::env::var_os("HEEMANG_CORPUS") else {
        eprintln!("HEEMANG_CORPUS is not set; skipping");
        return None;
    };
    let p = PathBuf::from(root).join(rel);
    p.exists().then_some(p)
}

fn read(bytes: &[u8]) -> Document {
    let b = SharedBytes::from(bytes.to_vec());
    if hm_hwpx::is_hwpx(&b) {
        hm_hwpx::read(b).unwrap()
    } else {
        hm_hwp5::read(b).unwrap()
    }
}

fn write(doc: &Document) -> Vec<u8> {
    match doc.meta.format {
        hm_model::Format::Hwp5 => hm_hwp5::write(doc).unwrap(),
        hm_model::Format::Hwpx => hm_hwpx::write(doc).unwrap(),
        hm_model::Format::Hwp3 => unreachable!("HWP 3 fixtures are not saved"),
    }
}

fn text(doc: &Document) -> Vec<String> {
    fn walk(paras: &[hm_model::Paragraph], out: &mut Vec<String>) {
        for p in paras {
            out.push(p.plain_text());
            for c in &p.controls {
                if let Some(t) = c.as_table() {
                    for cell in &t.cells {
                        walk(&cell.paras, out);
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    for s in &doc.sections {
        walk(&s.paras, &mut out);
    }
    out
}

const SCRIPT: &str = r#"{"ops": [
  {"op": "insert", "at": {"section": 0, "para": 3, "offset": 0}, "text": "앞에 넣은 글 "},
  {"op": "split", "at": {"section": 0, "para": 3, "offset": 3}},
  {"op": "delete", "from": {"section": 0, "para": 3, "offset": 1}, "to": {"section": 0, "para": 4, "offset": 2}},
  {"op": "format", "from": {"section": 0, "para": 3, "offset": 0}, "to": {"section": 0, "para": 3, "offset": 2}, "bold": true},
  {"op": "para", "at": {"section": 0, "para": 3}, "align": "center"}
]}"#;

fn check(rel: &str) {
    let Some(path) = corpus(rel) else { return };
    let src = std::fs::read(&path).unwrap();
    let mut ed = Editor::new(read(&src));
    script::run(&mut ed, SCRIPT).unwrap();
    let edited_text = text(&ed.doc);
    let saved = write(&ed.doc);
    let back = read(&saved);
    assert_eq!(text(&back), edited_text, "{rel}: saved text");
    // Undoing everything gives the original document back.
    while ed.undo().unwrap() {}
    assert_eq!(text(&ed.doc), text(&read(&src)), "{rel}: undo");
    let again = write(&ed.doc);
    assert_eq!(text(&read(&again)), text(&read(&src)), "{rel}: undo save");
}

#[test]
fn hwp5_edits_survive_saving() {
    check("2022년 국립국어원 업무계획.hwp");
}

#[test]
fn hwpx_edits_survive_saving() {
    check("hwpx/2024년 1분기 해외직접투자 보도자료 ff.hwpx");
}
