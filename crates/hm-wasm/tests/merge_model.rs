//! Property views preserve owned data and exclude file-backed source caches.
use hm_model::{Control, ControlKind, CtrlId, HwpString, RawRecord, SharedBytes};
#[test]
fn shared_byte_views_only_include_the_selected_span() {
    let bytes = SharedBytes::from(b"prefix:selected:suffix".as_slice()).slice(7..15);
    let json = serde_json::to_string(&bytes).unwrap();
    let restored: SharedBytes = serde_json::from_str(&json).unwrap();
    assert_eq!(&*restored, b"selected");
    let control = Control {
        kind: ControlKind::Raw(hm_model::RawControl {
            id: CtrlId::from_chars(b"test"),
            header: bytes.clone(),
            children: vec![RawRecord {
                tag: 1000,
                level: 1,
                payload: bytes.clone(),
            }],
            xml: None,
        }),
        src: Some(SharedBytes::from(b"whole source".as_slice())),
    };
    let value = serde_json::to_value(&control).unwrap();
    assert!(value.get("src").is_none());
    let restored: Control = serde_json::from_value(value).unwrap();
    let ControlKind::Raw(raw) = restored.kind else {
        panic!("raw control disappeared")
    };
    assert_eq!(raw.header, bytes);
    assert_eq!(raw.children[0].payload, bytes);
    assert!(restored.src.is_none());
    let text = HwpString(vec![0xd800, 65, 0xdc00]);
    let restored: HwpString = serde_json::from_value(serde_json::to_value(&text).unwrap()).unwrap();
    assert_eq!(restored, text);
}
#[test]
fn source_containers_and_layout_caches_stay_outside_model_properties() {
    let doc = hm_hwpx::read(
        include_bytes!("fixtures/query-equations.hwpx")
            .as_slice()
            .into(),
    )
    .unwrap();
    let properties = serde_json::to_value(&doc).unwrap();
    assert!(properties.get("source").is_none());
    assert!(properties.get("bins").is_none());
    assert!(properties["sections"][0].get("source_index").is_none());
    assert!(
        properties["sections"][0]["paras"][0]
            .get("line_segs")
            .is_none()
    );
    let restored: hm_model::Document = serde_json::from_value(properties).unwrap();
    assert_eq!(
        restored.sections[0].paras[0].text,
        doc.sections[0].paras[0].text
    );
    assert_eq!(
        restored.sections[0].paras[0].controls.len(),
        doc.sections[0].paras[0].controls.len()
    );
}
