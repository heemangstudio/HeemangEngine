//! Version merging preserves typed edits, binary dependencies and native extras.
use hm_model::*;
use hm_wasm::*;
use serde_json::{Value, json};
const SOURCE: &[u8] = include_bytes!("fixtures/query-equations.hwpx");
fn read(b: &[u8]) -> Document {
    if hm_hwp5::is_hwp5(b) {
        hm_hwp5::read(b.into()).unwrap()
    } else {
        hm_hwpx::read(b.into()).unwrap()
    }
}
fn save(d: &Document) -> Vec<u8> {
    match d.meta.format {
        Format::Hwp5 => hm_hwp5::write(d).unwrap(),
        _ => hm_hwpx::write(d).unwrap(),
    }
}
fn base(format: Format) -> Document {
    let mut d = read(SOURCE);
    if format == Format::Hwp5 {
        d.convert_to(format);
        d = hm_hwp5::read(hm_hwp5::write_new(&d).unwrap().into()).unwrap();
    }
    d
}
fn changed(d: &mut Document, prefix: &str, suffix: &str) {
    let p = &mut d.sections[0].paras[0];
    let mut text = prefix.encode_utf16().collect::<Vec<_>>();
    text.extend(&p.text[..p.text.len() - 1]);
    text.extend(suffix.encode_utf16());
    text.push(PARA_END);
    p.text = text;
    p.src = None;
    d.sections[0].src = None;
}
fn review(b: &[u8], c: &[u8], i: &[u8]) -> Value {
    serde_json::from_str(&structural_merge_review_document(b, c, i, "{}", "{}", "{}").unwrap())
        .unwrap()
}
fn materialize(b: &[u8], c: &[u8], i: &[u8], kind: &str) -> Vec<u8> {
    let a = review(b, c, i);
    let choices = a["conflicts"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|v| {
            let choice = json!({"kind":kind});
            let mut pairs = vec![
                (v["id"].as_str().unwrap().to_string(), choice.clone()),
                (
                    v["fingerprint"].as_str().unwrap().to_string(),
                    choice.clone(),
                ),
            ];
            if v.get("position").is_some() {
                pairs.push((
                    format!(
                        "review-pos:{}:{}",
                        v["position"]["section"], v["position"]["paragraph"]
                    ),
                    choice,
                ));
            }
            pairs
        })
        .collect::<serde_json::Map<_, _>>();
    structural_merge_materialize_review_document(
        b,
        c,
        i,
        "{}",
        "{}",
        "{}",
        &json!(choices).to_string(),
    )
    .unwrap()
}
#[test]
fn text_formatting_and_controls_survive_review_and_native_reopen() {
    for format in [Format::Hwp5, Format::Hwpx] {
        let b = base(format);
        let mut c = b.clone();
        let mut i = b.clone();
        changed(&mut c, "Current 😀 ", "");
        changed(&mut i, "", " Incoming 한글");
        let id = c.sections[0].paras[0].runs[0].shape as usize;
        c.info.char_shapes[id].attr |= 2;
        c.info.char_shapes[id].src = None;
        i.info.char_shapes[id].color = 0x123456;
        i.info.char_shapes[id].src = None;
        let (bb, cb, ib) = (save(&b), save(&c), save(&i));
        let a = review(&bb, &cb, &ib);
        assert!(
            a["conflicts"]
                .as_array()
                .unwrap()
                .iter()
                .all(|v| v["automatic"] == true),
            "{a}"
        );
        let bytes = materialize(&bb, &cb, &ib, "incoming");
        let d = read(&bytes);
        let p = &d.sections[0].paras[0];
        assert!(p.plain_text().starts_with("Current 😀 "));
        assert!(p.plain_text().ends_with(" Incoming 한글"));
        assert_eq!(p.controls.len(), b.sections[0].paras[0].controls.len());
        assert!(p.runs.iter().any(|r| {
            let s = &d.info.char_shapes[r.shape as usize];
            s.attr & 2 != 0 && s.color == 0x123456
        }));
    }
}
#[test]
fn paragraph_choices_are_stable_and_unresolved_changes_never_write() {
    let b = base(Format::Hwpx);
    let mut c = b.clone();
    let mut i = b.clone();
    changed(&mut c, "Current", "");
    changed(&mut i, "Incoming", "");
    let (bb, cb, ib) = (save(&b), save(&c), save(&i));
    let a = review(&bb, &cb, &ib);
    assert_eq!(a, review(&bb, &cb, &ib));
    assert!(
        structural_merge_materialize_review_document(&bb, &cb, &ib, "{}", "{}", "{}", "{}")
            .is_err()
    );
    let c = read(&materialize(&bb, &cb, &ib, "current"));
    let i = read(&materialize(&bb, &cb, &ib, "incoming"));
    assert!(c.sections[0].paras[0].plain_text().starts_with("Current"));
    assert!(i.sections[0].paras[0].plain_text().starts_with("Incoming"));
    assert!(!i.sections[0].paras[0].plain_text().contains("Current"));
}
fn zip_member(bytes: &[u8], name: &str, data: &[u8]) -> Vec<u8> {
    use hm_hwpx::zip::*;
    let a = Archive::open(bytes.into()).unwrap();
    let mut m = a
        .entries
        .iter()
        .enumerate()
        .filter(|(_, e)| e.name != name)
        .map(|(n, _)| Member::Copy(n))
        .collect::<Vec<_>>();
    m.push(Member::New {
        name,
        data,
        stored: false,
    });
    write(Some(&a), &m)
}
#[test]
fn unknown_members_and_scripts_are_reviewed_and_preserved() {
    for format in [Format::Hwpx, Format::Hwp5] {
        let d = base(format);
        let b = save(&d);
        let (name, b, c, i) = if format == Format::Hwpx {
            (
                "Scripts/custom.js",
                zip_member(&b, "Scripts/custom.js", b"base"),
                zip_member(&b, "Scripts/custom.js", b"current"),
                zip_member(&b, "Scripts/custom.js", b"incoming"),
            )
        } else {
            let edit = |data: &[u8]| {
                let a = hm_hwp5::cfb::Cfb::open(&b).unwrap();
                let mut files = a
                    .paths()
                    .iter()
                    .filter(|(_, n)| a.entries[*n].kind == 2)
                    .map(|(p, n)| (p.clone(), a.read_entry(*n)))
                    .collect::<Vec<_>>();
                files.push(("Scripts/Custom".into(), data.to_vec()));
                hm_hwp5::cfb::build_preserving(&files, &a)
            };
            (
                "Scripts/Custom",
                edit(b"base"),
                edit(b"current"),
                edit(b"incoming"),
            )
        };
        let bytes = materialize(&b, &c, &i, "incoming");
        let actual = if format == Format::Hwpx {
            hm_hwpx::zip::Archive::open(bytes.clone().into())
                .unwrap()
                .read_name(name)
                .unwrap()
                .unwrap()
                .to_vec()
        } else {
            hm_hwp5::cfb::Cfb::open(&bytes).unwrap().read(name).unwrap()
        };
        assert_eq!(actual, b"incoming");
        assert_eq!(
            read(&bytes).sections[0].paras[0].plain_text(),
            d.sections[0].paras[0].plain_text()
        );
    }
}
#[test]
fn section_insertions_and_deletions_keep_unknown_package_members() {
    let b = base(Format::Hwpx);
    let mut i = b.clone();
    let mut added = b.sections[0].clone();
    added.src = None;
    added.paras[0].text = "Added section".encode_utf16().chain([13]).collect();
    added.paras[0].controls.clear();
    added.paras[0].src = None;
    i.sections.push(added);
    i.info.properties.section_count = 2;
    i.info.properties.src = None;
    i.info.id_mappings.src = None;
    let bb = zip_member(&save(&b), "custom/data.bin", b"keep");
    let ib = zip_member(&save(&i), "custom/data.bin", b"keep");
    let bytes = materialize(&bb, &bb, &ib, "incoming");
    let d = read(&bytes);
    assert_eq!(d.sections.len(), 2);
    assert_eq!(d.sections[1].paras[0].plain_text(), "Added section");
    assert_eq!(
        &*hm_hwpx::zip::Archive::open(bytes.clone().into())
            .unwrap()
            .read_name("custom/data.bin")
            .unwrap()
            .unwrap(),
        b"keep"
    );
    let bytes = materialize(&bytes, &bytes, &bb, "incoming");
    assert_eq!(read(&bytes).sections.len(), 1);
}
#[test]
fn nested_table_edits_merge_without_replacing_the_control() {
    for format in [Format::Hwp5, Format::Hwpx] {
        let b = base(format);
        let mut c = b.clone();
        let mut i = b.clone();
        let table = |d: &mut Document, prefix: &str, suffix: &str| {
            for control in &mut d.sections[0].paras[0].controls {
                if let ControlKind::Table(t) = &mut control.kind {
                    let p = &mut t.cells[0].paras[0];
                    let mut text = prefix.encode_utf16().collect::<Vec<_>>();
                    text.extend(&p.text[..p.text.len() - 1]);
                    text.extend(suffix.encode_utf16());
                    text.push(13);
                    p.text = text;
                    p.src = None;
                    control.src = None;
                    break;
                }
            }
            d.sections[0].paras[0].src = None;
            d.sections[0].src = None;
        };
        table(&mut c, "Cell current ", "");
        table(&mut i, "", " Cell incoming");
        let (bb, cb, ib) = (save(&b), save(&c), save(&i));
        let merged = read(&materialize(&bb, &cb, &ib, "incoming"));
        let t = merged.sections[0].paras[0]
            .controls
            .iter()
            .find_map(Control::as_table)
            .unwrap();
        let p = &t.cells[0].paras[0];
        assert!(p.plain_text().starts_with("Cell current "));
        assert!(p.plain_text().ends_with(" Cell incoming"));
        assert_eq!(p.controls.len(), 2);
    }
}
#[test]
fn foreign_pictures_with_colliding_source_ids_retain_both_payloads() {
    let png = vec![
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 4,
        0, 0, 0, 181, 28, 12, 2, 0, 0, 0, 11, 73, 68, 65, 84, 120, 218, 99, 252, 255, 31, 0, 3, 3,
        2, 0, 239, 191, 107, 153, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ];
    let mut other = png.clone();
    other.extend(b"other");
    for format in ["hwp5", "hwpx"] {
        let e = Engine::new();
        let mut d = e.open(SOURCE.to_vec()).unwrap();
        let b = d.serialize_as(format, false).unwrap();
        let end = read(&b).sections[0].paras[0].len() - 1;
        let mut c = e.open(b.clone()).unwrap();
        let mut i = e.open(b.clone()).unwrap();
        for (d, offset, bytes) in [(&mut c, 0, png.clone()), (&mut i, end, other.clone())] {
            d.transact(&json!({"ops":[{"op":"insertPicture","at":{"section":0,"para":0,"offset":offset},"bytes":bytes,"extension":"png","width":3000,"height":3000}]}).to_string()).unwrap();
        }
        let (cb, mut ib) = (
            c.serialize_as(format, false).unwrap(),
            i.serialize_as(format, false).unwrap(),
        );
        if format == "hwpx" {
            let a = hm_hwpx::zip::Archive::open(ib.clone().into()).unwrap();
            let path = a
                .entries
                .iter()
                .find(|e| e.name.ends_with(".hpf"))
                .unwrap()
                .name
                .clone();
            let xml = String::from_utf8(a.read_name(&path).unwrap().unwrap().to_vec())
                .unwrap()
                .replace("id=\"image1\"", "id=\"image1\" customMergeData=\"kept\"");
            ib = zip_member(&ib, &path, xml.as_bytes());
        }
        let merged = read(&materialize(&b, &cb, &ib, "incoming"));
        let mut payloads = Vec::new();
        for c in &merged.sections[0].paras[0].controls {
            if let Some(o) = c.as_object() {
                for part in &o.parts {
                    if let ObjectPart::Shape(s) = part {
                        for part in &s.parts {
                            if let ShapePart::Picture(p) = part {
                                payloads.push(
                                    merged
                                        .bins
                                        .as_ref()
                                        .unwrap()
                                        .get(p.image.bin_id)
                                        .unwrap()
                                        .to_vec(),
                                );
                            }
                        }
                    }
                }
            }
        }
        assert_eq!(payloads.len(), 2);
        assert!(payloads.contains(&png));
        assert!(payloads.contains(&other));
        if format == "hwpx" {
            assert!(merged.info.bin_data.iter().any(|b| {
                hm_hwpx::rest::pieces(&b.tail)
                    .iter()
                    .any(|p| p.bytes == b"customMergeData=\"kept\"")
            }));
        }
    }
}
#[test]
fn source_header_metadata_is_selected_without_overwriting_current_text() {
    let b = save(&base(Format::Hwpx));
    let a = hm_hwpx::zip::Archive::open(b.clone().into()).unwrap();
    let header = a.read_name("Contents/header.xml").unwrap().unwrap();
    let original = String::from_utf8(header.to_vec()).unwrap();
    let insert = original.rfind("</").unwrap();
    let changed_header = format!(
        "{}<hh:customData value=\"incoming\"/>{}",
        &original[..insert],
        &original[insert..]
    );
    let i = zip_member(&b, "Contents/header.xml", changed_header.as_bytes());
    let mut current = read(&b);
    changed(&mut current, "Current ", "");
    let c = save(&current);
    let bytes = materialize(&b, &c, &i, "incoming");
    assert!(
        read(&bytes).sections[0].paras[0]
            .plain_text()
            .starts_with("Current ")
    );
    let header = hm_hwpx::zip::Archive::open(bytes.into())
        .unwrap()
        .read_name("Contents/header.xml")
        .unwrap()
        .unwrap();
    assert!(String::from_utf8_lossy(&header).contains("value=\"incoming\""));
}
#[test]
fn style_removal_reindexes_typed_references_and_native_record_order() {
    for format in [Format::Hwp5, Format::Hwpx] {
        let mut d = base(format);
        d.info.styles = vec![
            Style {
                local_name: HwpString("A".encode_utf16().collect()),
                ..Default::default()
            },
            Style {
                local_name: HwpString("B".encode_utf16().collect()),
                ..Default::default()
            },
        ];
        d.sections[0].paras[0].header.style = 1;
        d.sections[0].paras[0].src = None;
        d.sections[0].src = None;
        d.info.id_mappings.src = None;
        let b = save(&d);
        let mut c = read(&b);
        changed(&mut c, "Current ", "");
        let mut i = read(&b);
        i.info.styles.remove(0);
        i.info.styles[0].src = None;
        i.info
            .order
            .retain(|s| !matches!(s.item, InfoItem::Style(1)));
        i.info.id_mappings.counts.resize(15, 0);
        i.info.id_mappings.counts[14] = 1;
        i.info.id_mappings.src = None;
        i.sections[0].paras[0].header.style = 0;
        i.sections[0].paras[0].src = None;
        i.sections[0].src = None;
        let result = read(&materialize(&b, &save(&c), &save(&i), "incoming"));
        assert_eq!(result.info.styles.len(), 1);
        assert_eq!(result.info.styles[0].local_name.to_string_lossy(), "B");
        assert_eq!(result.sections[0].paras[0].header.style, 0);
        assert!(
            result.sections[0].paras[0]
                .plain_text()
                .starts_with("Current ")
        );
    }
}
#[test]
fn opaque_records_with_foreign_numeric_dependencies_fail_explicitly() {
    let b = base(Format::Hwp5);
    let mut i = b.clone();
    i.info.raw.push(RawRecord {
        tag: 1000,
        level: 0,
        payload: b"unknown dependency".as_slice().into(),
    });
    i.info.order.push(InfoSlot {
        item: InfoItem::Raw(i.info.raw.len() as u32 - 1),
        level: 0,
    });
    i.info.id_mappings.src = None;
    i.info.char_shapes[0].color = 0xff;
    i.info.char_shapes[0].src = None;
    let (bb, ib) = (save(&b), save(&i));
    let a = review(&bb, &bb, &ib);
    let choices = a["conflicts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["id"].as_str().unwrap().to_string(),
                json!({"kind":"incoming"}),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    let error = structural_merge_materialize_review_document(
        &bb,
        &bb,
        &ib,
        "{}",
        "{}",
        "{}",
        &json!(choices).to_string(),
    )
    .unwrap_err();
    assert!(error.message().contains("opaque records"), "{error:?}");
}
#[test]
fn identical_inserted_paragraphs_are_not_duplicated() {
    let b = base(Format::Hwpx);
    let mut i = b.clone();
    let mut p = i.sections[0].paras[0].clone();
    p.text = "New paragraph".encode_utf16().chain([13]).collect();
    p.controls.clear();
    p.src = None;
    i.sections[0].paras.push(p);
    i.sections[0].src = None;
    let (bb, ib) = (save(&b), save(&i));
    let result = read(&materialize(&bb, &ib, &ib, "incoming"));
    assert_eq!(result.sections[0].paras.len(), i.sections[0].paras.len());
}
#[test]
fn virtual_property_bases_are_order_independent_and_preserve_keyed_insertions() {
    let a = json!({"nodes":[{"id":"shared","text":"same"},{"id":"left","text":"left"}]});
    let b = json!({"nodes":[{"id":"shared","text":"same"},{"id":"right","text":"right"}]});
    let one = structural_merge_virtual_base(&json!([a, b]).to_string()).unwrap();
    let two = structural_merge_virtual_base(&json!([b, a]).to_string()).unwrap();
    assert_eq!(one, two);
    let v: Value = serde_json::from_str(&one).unwrap();
    assert_eq!(v["nodes"].as_array().unwrap().len(), 3);
}
#[test]
fn hwp_verification_compares_typed_content_after_editing() {
    let e = Engine::new();
    let mut d = e
        .open(include_bytes!("fixtures/query-equations.hwp").to_vec())
        .unwrap();
    d.transact(&json!({"ops":[{"op":"insertText","at":{"section":0,"para":0,"offset":0},"text":"Verified 한글😀"}]}).to_string()).unwrap();
    let report: Value = serde_json::from_str(&d.verify_hwp().unwrap()).unwrap();
    assert_eq!(report["recovered"], true);
    assert_eq!(report["structureMatches"], true, "{report}");
}
#[test]
fn opaque_paragraph_attributes_cannot_use_rejected_foreign_namespace_context() {
    let b = save(&base(Format::Hwpx));
    let a = hm_hwpx::zip::Archive::open(b.clone().into()).unwrap();
    let raw = a.read_name("Contents/section0.xml").unwrap().unwrap();
    let x = hm_hwpx::xml::Xml::parse(raw.clone()).unwrap();
    let at = x.root().span().start + x.root().start_tag().len() - 1;
    let mut xml = String::from_utf8(raw.to_vec()).unwrap();
    xml.insert_str(at, " xmlns:v=\"urn:merge-private\"");
    let at = xml.find("<hp:p ").unwrap() + "<hp:p".len();
    xml.insert_str(at, " v:private=\"kept\"");
    let i = zip_member(&b, "Contents/section0.xml", xml.as_bytes());
    let a = review(&b, &b, &i);
    let choices=a["conflicts"].as_array().unwrap().iter().map(|c|(c["id"].as_str().unwrap().to_string(),json!({"kind":if c["path"].as_array().unwrap().iter().any(|p|p=="context:namespaces"){"current"}else{"incoming"}}))).collect::<serde_json::Map<_,_>>();
    let error = structural_merge_materialize_review_document(
        &b,
        &b,
        &i,
        "{}",
        "{}",
        "{}",
        &json!(choices).to_string(),
    )
    .unwrap_err();
    assert!(error.message().contains("namespace bindings"), "{error:?}");
}
#[test]
fn manifest_hashes_follow_text_and_format_changes_of_each_paragraph() {
    let manifest = |d: &Document| -> Vec<Value> {
        let v: Value =
            serde_json::from_str(&structural_merge_build_manifest(&save(d)).unwrap()).unwrap();
        v["entries"].as_array().unwrap().clone()
    };
    let hash = |entries: &[Value], path: &[&str]| -> String {
        entries
            .iter()
            .find(|e| {
                e["path"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|p| p.as_str().unwrap())
                    .eq(path.iter().copied())
            })
            .unwrap()["propertyHash"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let original = base(Format::Hwpx);
    let first = ["sections", "0", "paragraphs", "0"];
    let before = manifest(&original);
    assert!(before.len() > 2, "fixture must have several entries");
    assert_eq!(
        manifest(&original),
        before,
        "equal content must hash equally"
    );
    let mut text = original.clone();
    changed(&mut text, "앞 ", "");
    let after = manifest(&text);
    assert_ne!(hash(&after, &first), hash(&before, &first));
    assert_ne!(hash(&after, &[]), hash(&before, &[]));
    if original.sections[0].paras.len() > 1 {
        let second = ["sections", "0", "paragraphs", "1"];
        assert_eq!(hash(&after, &second), hash(&before, &second));
    }
    let mut format = original.clone();
    let id = format.sections[0].paras[0].runs[0].shape as usize;
    format.info.char_shapes[id].attr ^= 2;
    format.info.char_shapes[id].src = None;
    assert_ne!(
        hash(&manifest(&format), &first),
        hash(&before, &first),
        "a format change must change the paragraph hash"
    );
}
#[test]
fn accepting_an_incoming_picture_while_keeping_current_text_materializes() {
    let png = vec![
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 4,
        0, 0, 0, 181, 28, 12, 2, 0, 0, 0, 11, 73, 68, 65, 84, 120, 218, 99, 252, 255, 31, 0, 3, 3,
        2, 0, 239, 191, 107, 153, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ];
    for format in ["hwp5", "hwpx"] {
        let e = Engine::new();
        let mut d = e.open(SOURCE.to_vec()).unwrap();
        let end = read(SOURCE).sections[0].paras[0].len() - 1;
        d.transact(
            &json!({"ops":[{"op":"split","at":{"section":0,"para":0,"offset":end}}]}).to_string(),
        )
        .unwrap();
        let b = d.serialize_as(format, false).unwrap();
        assert_eq!(read(&b).sections[0].paras.len(), 2);
        let text = |d: &mut Doc, text: &str| {
            d.transact(&json!({"ops":[{"op":"insertText","at":{"section":0,"para":1,"offset":0},"text":text}]}).to_string()).unwrap();
        };
        let mut c = e.open(b.clone()).unwrap();
        text(&mut c, "LOCAL ");
        let mut i = e.open(b.clone()).unwrap();
        text(&mut i, "REMOTE ");
        i.transact(&json!({"ops":[{"op":"insertPicture","at":{"section":0,"para":0,"offset":0},"bytes":png,"extension":"png","width":3000,"height":3000}]}).to_string()).unwrap();
        let (cb, ib) = (
            c.serialize_as(format, false).unwrap(),
            i.serialize_as(format, false).unwrap(),
        );
        let a = review(&b, &cb, &ib);
        let mut choices = serde_json::Map::new();
        for v in a["conflicts"].as_array().unwrap() {
            let paragraph = v["position"]["paragraph"].as_u64();
            let choice = json!({"kind": if paragraph == Some(0) { "incoming" } else { "current" }});
            choices.insert(v["id"].as_str().unwrap().into(), choice.clone());
            choices.insert(v["fingerprint"].as_str().unwrap().into(), choice.clone());
            if let Some(p) = paragraph {
                choices.insert(format!("review-pos:0:{p}"), choice);
            }
        }
        let merged = structural_merge_materialize_review_document(
            &b,
            &cb,
            &ib,
            "{}",
            "{}",
            "{}",
            &Value::Object(choices).to_string(),
        )
        .unwrap();
        let merged = read(&merged);
        assert_eq!(merged.bins.as_ref().unwrap().get(1).unwrap().to_vec(), png);
        let second = String::from_utf16_lossy(&merged.sections[0].paras[1].text);
        assert!(second.starts_with("LOCAL "), "{format}: {second}");
    }
}
