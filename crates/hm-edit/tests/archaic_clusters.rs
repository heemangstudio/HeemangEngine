#[allow(dead_code)]
#[path = "../../hm-layout/tests/common/mod.rs"]
mod common;

use hm_edit::ops::{ListPath, Pos, Step, list};
use hm_edit::{Editor, Op};

#[test]
fn partial_archaic_cluster_edits_undo_and_roundtrip_keep_original_wire_text() {
    let Some((corpus, fonts)) = common::setup() else {
        return;
    };
    let bytes = std::fs::read(corpus.join("hwp-3.0-HWPML.hwp")).expect("native HWPML fixture");
    let doc = hm_hwp5::read(hm_model::SharedBytes::from(bytes.clone())).unwrap();
    let path = ListPath {
        section: 0,
        steps: vec![Step {
            para: 37,
            control: 1,
            index: 0,
        }],
    };
    let original = list(&doc, &path).unwrap()[0].text.clone();
    assert_eq!(&original[8..11], &[0x1112, 0x119e, 0x11ab]);
    let at = |offset| Pos {
        list: path.clone(),
        para: 0,
        offset,
    };
    let mut editor = Editor::new(doc);
    editor.layout(&fonts);
    assert_eq!(hm_hwp5::write(&editor.doc).unwrap(), bytes);
    editor
        .apply(Op::Delete {
            from: at(9),
            to: at(10),
        })
        .unwrap();
    let mut deleted = original.clone();
    deleted.remove(9);
    assert_eq!(list(&editor.doc, &path).unwrap()[0].text, deleted);
    editor.layout(&fonts);
    let saved = hm_hwp5::write(&editor.doc).unwrap();
    let reopened = hm_hwp5::read(hm_model::SharedBytes::from(saved)).unwrap();
    assert_eq!(list(&reopened, &path).unwrap()[0].text, deleted);
    assert!(editor.undo().unwrap());
    editor.layout(&fonts);
    assert_eq!(list(&editor.doc, &path).unwrap()[0].text, original);
    assert_eq!(hm_hwp5::write(&editor.doc).unwrap(), bytes);
    editor
        .apply(Op::InsertText {
            at: at(9),
            text: "X".into(),
        })
        .unwrap();
    let mut inserted = original.clone();
    inserted.insert(9, 'X' as u16);
    assert_eq!(list(&editor.doc, &path).unwrap()[0].text, inserted);
    editor.layout(&fonts);
    assert!(editor.undo().unwrap());
    assert_eq!(list(&editor.doc, &path).unwrap()[0].text, original);
    let converted = hm_hwpx::write_new(&editor.doc).unwrap();
    let reopened = hm_hwpx::read(hm_model::SharedBytes::from(converted)).unwrap();
    assert_eq!(list(&reopened, &path).unwrap()[0].text, original);
}
