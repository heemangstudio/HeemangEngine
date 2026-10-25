//! Explicit editor export repackages preview images without changing content.
use hm_hwpx::zip::{Archive, Member, STORED, write};
use hm_wasm::{Doc, Engine};
use serde_json::{Value, json};

fn state(doc: &mut Doc) -> Value {
    serde_json::from_str(&doc.state().unwrap()).unwrap()
}
fn text(doc: &mut Doc) -> Value {
    let paras: Value = serde_json::from_str(
        &doc.query(&json!({"kind":"paragraphs","list":{"section":0}}).to_string())
            .unwrap(),
    )
    .unwrap();
    json!(
        paras
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["text"].clone())
            .collect::<Vec<_>>()
    )
}

#[test]
fn explicit_export_stores_preview_and_preserves_members_state_and_native_source() {
    let source = Archive::open(
        include_bytes!("fixtures/conversion-membership.hwpx")
            .to_vec()
            .into(),
    )
    .unwrap();
    let mut members: Vec<_> = (0..source.entries.len()).map(Member::Copy).collect();
    let preview = vec![0x80; 5126];
    members.extend([
        Member::New {
            name: "Preview/PrvImage.png",
            data: &preview,
            stored: false,
        },
        Member::New {
            name: "Unknown/opaque.dat",
            data: b"uninterpreted extension",
            stored: false,
        },
    ]);
    let bytes = write(Some(&source), &members);
    let original = Archive::open(bytes.clone().into()).unwrap();
    let engine = Engine::new();
    let mut doc = engine.open(bytes.clone()).unwrap();
    let before = state(&mut doc);
    let content = text(&mut doc);
    assert_eq!(doc.serialize(false).unwrap(), bytes);
    assert_eq!(doc.serialize_as("hwpx", false).unwrap(), bytes);
    let output = doc.export_hwpx().unwrap();
    assert_ne!(output, bytes);
    let exported = Archive::open(output.clone().into()).unwrap();
    assert_eq!(exported.entries.len(), original.entries.len());
    for (i, (was, now)) in original.entries.iter().zip(&exported.entries).enumerate() {
        assert_eq!(now.name, was.name);
        assert_eq!(exported.read(i).unwrap(), original.read(i).unwrap());
        if was.name == "Preview/PrvImage.png" {
            assert_eq!(now.method, STORED);
            assert_eq!(now.compressed_size, preview.len() as u64);
        } else {
            assert_eq!(exported.raw(i), original.raw(i));
            assert_eq!(now.method, was.method);
        }
    }
    assert_eq!(state(&mut doc), before);
    assert_eq!(doc.serialize(false).unwrap(), bytes);
    let mut back = engine.open(output.clone()).unwrap();
    assert_eq!(text(&mut back), content);
    assert_eq!(
        back.export_hwpx().unwrap(),
        output,
        "normalized export is byte-idempotent"
    );
    doc.transact(&json!({"ops":[{"op":"insertText","at":{"section":0,"para":1,"offset":0},"text":"exported edit "}]}).to_string()).unwrap();
    let edited = state(&mut doc);
    let output = doc.export_hwpx().unwrap();
    assert_eq!(state(&mut doc), edited);
    let mut back = engine.open(output).unwrap();
    assert_eq!(text(&mut back), text(&mut doc));
    assert_eq!(state(&mut back)["pageCount"], edited["pageCount"]);
}
