//! Plain text and Markdown of corpus documents (`HEEMANG_CORPUS`): the
//! same document saved as HWP and HWPX reads the same, and tables come out
//! as rows of cells.

use std::path::PathBuf;

use hm_edit::export::{markdown, plain_text};
use hm_model::{Document, SharedBytes};

fn corpus(rel: &str) -> Option<Document> {
    let Some(root) = std::env::var_os("HEEMANG_CORPUS") else {
        eprintln!("HEEMANG_CORPUS is not set; skipping");
        return None;
    };
    let bytes = std::fs::read(PathBuf::from(root).join(rel)).ok()?;
    let b = SharedBytes::from(bytes);
    Some(if hm_hwpx::is_hwpx(&b) {
        hm_hwpx::read(b).unwrap()
    } else {
        hm_hwp5::read(b).unwrap()
    })
}

#[test]
fn both_formats_read_the_same() {
    let (Some(a), Some(b)) = (
        corpus("footnote-tbox-01.hwp"),
        corpus("hwpx/footnote-tbox-01.hwpx"),
    ) else {
        return;
    };
    assert!(!plain_text(&a).trim().is_empty());
    assert_eq!(plain_text(&a), plain_text(&b));
    assert_eq!(markdown(&a), markdown(&b));
}

#[test]
fn tables_become_rows_of_cells() {
    let Some(doc) = corpus("2022년 국립국어원 업무계획.hwp") else {
        return;
    };
    let text = plain_text(&doc);
    assert!(text.contains("연도\t‘18년\t‘19년\t‘20년\t‘21년\t누적\n"));
    let md = markdown(&doc);
    assert!(md.contains(
        "| 연도 | ‘18년 | ‘19년 | ‘20년 | ‘21년 | 누적 |\n| --- | --- | --- | --- | --- | --- |\n"
    ));
    assert!(md.contains("| 분석 말뭉치\\*\\* | - | 11종 |"));
}
