//! Hancom 2022's paragraph levels 8 to 10 (0-based 7 to 9). The fixture is Hancom's own save of
//! numbered, bullet and outline paragraphs at those levels (COM capture f1-enums `levels`):
//! HWPX keeps them in a `hp:switch` case, HWP 5 in the record's last field, HWPML as `Level`.
use serde_json::{Value, json};

const HANCOM: &[u8] = include_bytes!("fixtures/hancom-levels-8-10.hwpx");

fn levels(d: &mut hm_wasm::Doc) -> Vec<String> {
    (0..10)
        .map(|para| {
            let p: Value = serde_json::from_str(
                &d.query(
                    &json!({"kind":"paragraphs","list":{"section":0},"from":para,"count":1})
                        .to_string(),
                )
                .unwrap(),
            )
            .unwrap();
            let at = json!({"section":0,"para":para,"offset":p[0]["caret"][0]});
            let q: Value = serde_json::from_str(
                &d.query(&json!({"kind":"formatContext","at":at}).to_string())
                    .unwrap(),
            )
            .unwrap();
            let l = &q["list"];
            format!("{} {}", l["kind"].as_str().unwrap_or("-"), l["level"])
        })
        .collect()
}

/// The number heads of the body paragraphs, as layout resolves them.
fn heads(doc: &hm_model::Document) -> Vec<String> {
    let mut heads = hm_layout::heads::Heads::default();
    doc.sections[0]
        .paras
        .iter()
        .map(|p| {
            let shape = &doc.info.para_shapes[p.header.para_shape as usize];
            heads
                .next(shape, &doc.info, 1)
                .filter(|_| shape.head_kind() != 3)
                .map_or(String::new(), |h| h.text)
        })
        .collect()
}

#[test]
fn levels_8_to_10_survive_every_format_and_number_as_hancom_does() {
    let want = [
        "numbering 7",
        "numbering 8",
        "numbering 9",
        "bullet 7",
        "bullet 8",
        "bullet 9",
        "outline 7",
        "outline 8",
        "outline 9",
        "numbering 6",
    ];
    // Hancom's print: level 8 is ㉮ (its `^8` in 원문자 가나다), levels 9 and 10 have no
    // number text in its new document's definitions.
    let printed = ["㉮", "", "", "", "", "", "㉮", "", "", "②"];
    let engine = hm_wasm::Engine::new();
    let mut d = engine.open(HANCOM.to_vec()).unwrap();
    assert_eq!(levels(&mut d), want);
    assert_eq!(
        heads(&hm_hwpx::read(HANCOM.to_vec().into()).unwrap()),
        printed
    );
    for format in ["hwpx", "hwp5", "hml"] {
        let saved = if format == "hwpx" {
            // Every definition re-encoded, as after editing them.
            let mut doc = hm_hwpx::read(HANCOM.to_vec().into()).unwrap();
            doc.info.mark_all_dirty();
            hm_hwpx::write(&doc).unwrap()
        } else {
            d.serialize_as(format, false).unwrap()
        };
        let mut reopened = engine.open(saved.clone()).unwrap();
        assert_eq!(levels(&mut reopened), want, "{format}");
        let doc = match format {
            "hwpx" => hm_hwpx::read(saved.into()).unwrap(),
            "hwp5" => hm_hwp5::read(saved.into()).unwrap(),
            _ => hm_hml::read(saved.into()).unwrap(),
        };
        assert_eq!(heads(&doc), printed, "{format}");
        let shape = |para: usize| {
            &doc.info.para_shapes[doc.sections[0].paras[para].header.para_shape as usize]
        };
        // HWP 5 keeps attr1's three level bits at 6, as Hancom writes them, and ends each
        // record with the full level, also for levels Hancom reads from attr1 alone.
        assert_eq!((shape(1).attr1 >> 25) & 7, 6, "{format}");
        if format == "hwp5" {
            for (para, level) in [(1, 8u32), (6, 7), (9, 6)] {
                let record = shape(para).src.as_ref().unwrap();
                assert_eq!(record[record.len() - 4..], level.to_le_bytes(), "{para}");
            }
        }
    }
}

#[test]
fn a_list_can_be_set_at_level_10() {
    let engine = hm_wasm::Engine::new();
    let mut d = engine.open(HANCOM.to_vec()).unwrap();
    let at = json!({"section":0,"para":9,"offset":0});
    d.transact(
        &json!({"ops":[{"op":"setList","from":at,"to":at,"list":{"kind":"numbering","id":2,"level":9}}]})
            .to_string(),
    )
    .unwrap();
    assert_eq!(levels(&mut d)[9], "numbering 9");
    let hwp = d.serialize_as("hwp5", false).unwrap();
    assert_eq!(levels(&mut engine.open(hwp).unwrap())[9], "numbering 9");
}
