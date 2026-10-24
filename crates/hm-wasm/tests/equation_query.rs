//! Equation source reads preserve the document and its wire addresses.

use hm_model::{Document, Paragraph, SharedBytes, Unit};
use serde_json::{Value, json};

fn read(bytes: Vec<u8>) -> Document {
    let bytes = SharedBytes::from(bytes);
    if hm_hwpx::is_hwpx(&bytes) {
        hm_hwpx::read(bytes).unwrap()
    } else {
        hm_hwp5::read(bytes).unwrap()
    }
}

fn check_list(
    doc: &mut hm_wasm::Doc,
    paras: &[Paragraph],
    list: Value,
    scripts: &mut Vec<(Value, String)>,
) {
    let query = |list: &Value| json!({"kind":"paragraphs", "list":list}).to_string();
    let got: Value = serde_json::from_str(&doc.query(&query(&list)).unwrap()).unwrap();
    assert_eq!(got.as_array().unwrap().len(), paras.len());
    for (pi, p) in paras.iter().enumerate() {
        assert_eq!(got[pi]["index"], pi);
        assert_eq!(got[pi]["text"], p.plain_text());
        assert_eq!(got[pi]["length"], p.len());
        let controls = got[pi]["controls"].as_array().unwrap();
        let expected: Vec<_> = p
            .units()
            .filter_map(|(offset, unit)| match unit {
                Unit::Extended { index, .. } => Some((offset, index)),
                _ => None,
            })
            .collect();
        assert_eq!(controls.len(), expected.len());
        for (c, (offset, index)) in controls.iter().zip(expected) {
            let source = &p.controls[index as usize];
            let lists = hm_edit::ops::control_lists(source);
            assert_eq!(c["index"], index);
            assert_eq!(c["offset"], offset);
            assert_eq!(c["lists"], lists.len());
            if let Some(eq) = source.as_object().and_then(|o| o.equation()) {
                assert_eq!(c["kind"], "equation");
                assert_eq!(c["script"], eq.script.to_string_lossy());
                assert_eq!(c.as_object().unwrap().len(), 5);
                scripts.push((list.clone(), c["script"].as_str().unwrap().to_string()));
            } else {
                assert!(c.get("script").is_none());
                assert_eq!(c.as_object().unwrap().len(), 4);
            }
            for (li, nested) in lists.iter().enumerate() {
                let mut address = list.clone();
                address["steps"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"para":pi,"control":index,"index":li}));
                check_list(doc, nested, address, scripts);
            }
        }
    }
    let bounded: Value = serde_json::from_str(
        &doc.query(&json!({"kind":"paragraphs","list":list,"from":0,"count":1}).to_string())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        bounded.as_array().unwrap(),
        &got.as_array().unwrap()[..paras.len().min(1)]
    );
}

fn check(bytes: Vec<u8>) -> Vec<(Value, String)> {
    let source = read(bytes.clone());
    let engine = hm_wasm::Engine::new();
    let mut doc = engine.open(bytes.clone()).unwrap();
    let before = doc.state().unwrap();
    let fonts = doc.font_requests().unwrap();
    let page = doc.page(0).unwrap();
    let mut scripts = Vec::new();
    for (section, s) in source.sections.iter().enumerate() {
        check_list(
            &mut doc,
            &s.paras,
            json!({"section":section,"steps":[]}),
            &mut scripts,
        );
    }
    assert!(!scripts.is_empty());
    assert_eq!(doc.state().unwrap(), before);
    assert_eq!(doc.font_requests().unwrap(), fonts);
    assert_eq!(doc.page(0).unwrap(), page);
    let saved = doc.serialize(false).unwrap();
    assert_eq!(saved, bytes);
    let mut reopened = engine.open(saved).unwrap();
    let mut again = Vec::new();
    for (section, s) in source.sections.iter().enumerate() {
        check_list(
            &mut reopened,
            &s.paras,
            json!({"section":section,"steps":[]}),
            &mut again,
        );
    }
    assert_eq!(scripts, again);
    scripts
}

#[test]
fn authored_scripts_include_empty_and_nested_equations() {
    for bytes in [
        include_bytes!("fixtures/query-equations.hwp").as_slice(),
        include_bytes!("fixtures/query-equations.hwpx").as_slice(),
    ] {
        let scripts = check(bytes.to_vec());
        assert_eq!(
            scripts.iter().map(|(_, s)| s.as_str()).collect::<Vec<_>>(),
            ["\"한글\" `x` \\\n+ y😀", "", "α + β"]
        );
        assert_eq!(
            scripts
                .iter()
                .map(|(l, _)| l["steps"].as_array().unwrap().len())
                .collect::<Vec<_>>(),
            [0, 1, 2]
        );
    }
    let caps: Value = serde_json::from_str(&hm_wasm::capabilities()).unwrap();
    assert_eq!(caps["equationScripts"], true);
    assert_eq!(caps["contract"], 1);
    assert_eq!(caps["packet"], hm_wasm::packet::VERSION);
}

#[test]
fn original_hwp5_and_hwpx_equations_match_the_parsed_source() {
    let Some(root) = std::env::var_os("HEEMANG_CORPUS") else {
        return;
    };
    for rel in [
        "math-001.hwp",
        "hwpx/math-001.hwpx",
        "task1725/text_footnote_tail_overpagination.hwp",
        "task1725/text_footnote_tail_overpagination.hwpx",
    ] {
        let path = std::path::Path::new(&root).join(rel);
        let scripts = check(std::fs::read(path).unwrap());
        if rel.starts_with("task1725/") {
            assert!(
                scripts
                    .iter()
                    .any(|(l, _)| !l["steps"].as_array().unwrap().is_empty())
            );
        }
        eprintln!("{rel}: {} source equations", scripts.len());
    }
}
