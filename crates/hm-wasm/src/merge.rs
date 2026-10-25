//! Version merge entry points. The worker protocol is independent of the UI.
use super::*;
use serde_json::Value;
fn value(raw: &str) -> Result<Value> {
    serde_json::from_str(raw).map_err(|e| EngineError::new("BAD_REQUEST", e))
}
#[wasm_bindgen(js_name=structuralMergeAnalyze)]
pub fn structural_merge_analyze(base: &str, current: &str, incoming: &str) -> Result<String> {
    Ok(json(&merge_core::analyze(
        &value(base)?,
        &value(current)?,
        &value(incoming)?,
    )))
}
#[wasm_bindgen(js_name=structuralMergeMaterialize)]
pub fn structural_merge_materialize(analysis: &str, resolutions: &str) -> Result<String> {
    let a = serde_json::from_str(analysis).map_err(|e| EngineError::new("BAD_REQUEST", e))?;
    let r = serde_json::from_str(resolutions).map_err(|e| EngineError::new("BAD_REQUEST", e))?;
    merge_core::materialize(&a, &r)
        .map(|v| json(&v))
        .map_err(|e| EngineError::new("MERGE_UNRESOLVED", e))
}
fn manifests(b: &str, c: &str, i: &str) -> Result<[merge_document::Manifest; 3]> {
    Ok([
        serde_json::from_str(b).map_err(|e| EngineError::new("BAD_REQUEST", e))?,
        serde_json::from_str(c).map_err(|e| EngineError::new("BAD_REQUEST", e))?,
        serde_json::from_str(i).map_err(|e| EngineError::new("BAD_REQUEST", e))?,
    ])
}
fn analyze_document(
    b: &[u8],
    c: &[u8],
    i: &[u8],
    m: [merge_document::Manifest; 3],
    review: bool,
) -> Result<String> {
    let docs = [
        merge_document::open(b)?,
        merge_document::open(c)?,
        merge_document::open(i)?,
    ];
    let p = merge_document::Prepared::new(&docs, &m)?;
    if review {
        let mut value = serde_json::to_value(p.review()?)
            .map_err(|e| EngineError::new("MERGE_VALIDATION_FAILED", e))?;
        for conflict in value["conflicts"].as_array_mut().unwrap() {
            if conflict.get("automatic").is_none() {
                conflict["automatic"] = serde_json::json!(false);
            }
        }
        Ok(json(&value))
    } else {
        Ok(json(&p.analyze()))
    }
}
fn materialize_document(
    b: &[u8],
    c: &[u8],
    i: &[u8],
    m: [merge_document::Manifest; 3],
    resolutions: &str,
    review: bool,
) -> Result<Vec<u8>> {
    let docs = [
        merge_document::open(b)?,
        merge_document::open(c)?,
        merge_document::open(i)?,
    ];
    let p = merge_document::Prepared::new(&docs, &m)?;
    let r = serde_json::from_str(resolutions).map_err(|e| EngineError::new("BAD_REQUEST", e))?;
    let view = if review {
        p.resolve_review(&r)?
    } else {
        merge_core::materialize(&p.analyze(), &r)
            .map_err(|e| EngineError::new("MERGE_UNRESOLVED", e))?
    };
    let doc = p.model(view)?;
    let bytes = merge_document::write(&doc)?;
    let losses = merge_document::roundtrip_losses(&doc, &bytes)?;
    if !losses.is_empty() {
        return Err(EngineError::new(
            "MERGE_VALIDATION_FAILED",
            format!(
                "merged properties did not survive native serialization: {}",
                losses.join(", ")
            ),
        ));
    }
    Ok(bytes)
}
#[wasm_bindgen(js_name=structuralMergeBuildManifest)]
pub fn structural_merge_build_manifest(bytes: &[u8]) -> Result<String> {
    let d = merge_document::open(bytes)?;
    Ok(json(
        &serde_json::json!({"entries":merge_document::manifest_of(&d)?}),
    ))
}
#[wasm_bindgen(js_name=structuralMergeAnalyzeDocument)]
pub fn structural_merge_analyze_document(b: &[u8], c: &[u8], i: &[u8]) -> Result<String> {
    analyze_document(b, c, i, Default::default(), false)
}
#[wasm_bindgen(js_name=structuralMergeAnalyzeDocumentWithManifests)]
pub fn structural_merge_analyze_document_with_manifests(
    b: &[u8],
    c: &[u8],
    i: &[u8],
    bm: &str,
    cm: &str,
    im: &str,
) -> Result<String> {
    analyze_document(b, c, i, manifests(bm, cm, im)?, false)
}
#[wasm_bindgen(js_name=structuralMergeReviewDocument)]
pub fn structural_merge_review_document(
    b: &[u8],
    c: &[u8],
    i: &[u8],
    bm: &str,
    cm: &str,
    im: &str,
) -> Result<String> {
    analyze_document(b, c, i, manifests(bm, cm, im)?, true)
}
#[wasm_bindgen(js_name=structuralMergeMaterializeDocument)]
pub fn structural_merge_materialize_document(
    b: &[u8],
    c: &[u8],
    i: &[u8],
    r: &str,
) -> Result<Vec<u8>> {
    materialize_document(b, c, i, Default::default(), r, false)
}
#[wasm_bindgen(js_name=structuralMergeMaterializeDocumentWithManifests)]
pub fn structural_merge_materialize_document_with_manifests(
    b: &[u8],
    c: &[u8],
    i: &[u8],
    bm: &str,
    cm: &str,
    im: &str,
    r: &str,
) -> Result<Vec<u8>> {
    materialize_document(b, c, i, manifests(bm, cm, im)?, r, false)
}
#[wasm_bindgen(js_name=structuralMergeMaterializeReviewDocument)]
pub fn structural_merge_materialize_review_document(
    b: &[u8],
    c: &[u8],
    i: &[u8],
    bm: &str,
    cm: &str,
    im: &str,
    r: &str,
) -> Result<Vec<u8>> {
    materialize_document(b, c, i, manifests(bm, cm, im)?, r, true)
}
fn common_value(left: &Value, right: &Value) -> Value {
    if left == right {
        return left.clone();
    }
    match (left, right) {
        (Value::Object(l), Value::Object(r)) => Value::Object(
            l.iter()
                .filter_map(|(k, v)| r.get(k).map(|r| (k.clone(), common_value(v, r))))
                .collect(),
        ),
        (Value::Array(l), Value::Array(r)) => {
            if l.iter()
                .chain(r)
                .all(|v| v.get("stableId").is_some() || v.get("id").is_some())
            {
                let key = if l.iter().chain(r).all(|v| v.get("stableId").is_some()) {
                    "stableId"
                } else {
                    "id"
                };
                Value::Array(
                    l.iter()
                        .filter_map(|v| {
                            r.iter().find(|r| r[key] == v[key]).map(|r| {
                                let mut common = common_value(v, r);
                                common[key] = v[key].clone();
                                common
                            })
                        })
                        .collect(),
                )
            } else if l.len() == r.len() {
                Value::Array(l.iter().zip(r).map(|(l, r)| common_value(l, r)).collect())
            } else {
                serde_json::json!([])
            }
        }
        _ => Value::Null,
    }
}
fn virtual_tree(mut bases: Vec<Value>) -> Result<Value> {
    if bases.is_empty() {
        return Err(EngineError::new(
            "BAD_REQUEST",
            "virtual base needs at least one document",
        ));
    }
    bases.sort_by_key(merge_resources::hash);
    let mut out = bases.remove(0);
    for next in bases {
        let neutral = common_value(&out, &next);
        let a = merge_core::analyze(&neutral, &out, &next);
        let r = a
            .conflicts
            .iter()
            .map(|c| (c.id.clone(), merge_core::Resolution::Current))
            .collect();
        out = merge_core::materialize(&a, &r)
            .map_err(|e| EngineError::new("MERGE_VALIDATION_FAILED", e))?;
    }
    Ok(out)
}
#[wasm_bindgen(js_name=structuralMergeVirtualBase)]
pub fn structural_merge_virtual_base(raw: &str) -> Result<String> {
    Ok(json(&virtual_tree(
        serde_json::from_str(raw).map_err(|e| EngineError::new("BAD_REQUEST", e))?,
    )?))
}
#[wasm_bindgen(js_name=structuralMergeVirtualBaseDocument)]
pub fn structural_merge_virtual_base_document(raw: &str, format: &str) -> Result<Vec<u8>> {
    use base64::Engine as _;
    let encoded: Vec<String> =
        serde_json::from_str(raw).map_err(|e| EngineError::new("BAD_REQUEST", e))?;
    let mut bases = encoded
        .iter()
        .map(|v| {
            base64::engine::general_purpose::STANDARD
                .decode(v)
                .map_err(|e| EngineError::new("BAD_REQUEST", e))
        })
        .collect::<Result<Vec<_>>>()?;
    bases.sort_by_key(|v| blake3::hash(v).to_hex().to_string());
    if bases.is_empty() {
        return Err(EngineError::new(
            "BAD_REQUEST",
            "virtual base needs at least one document",
        ));
    }
    let mut out = bases.remove(0);
    for next in bases {
        let left = merge_document::open(&out)?;
        let right = merge_document::open(&next)?;
        let docs = [left.clone(), left, right];
        let p = merge_document::Prepared::new(&docs, &Default::default())?;
        let v = virtual_tree(vec![p.views[1].clone(), p.views[2].clone()])?;
        out = merge_document::write(&p.model(v)?)?;
    }
    let mut doc = merge_document::open(&out)?;
    let desired = if format == "hwp" {
        hm_model::Format::Hwp5
    } else if format == "hwpx" {
        hm_model::Format::Hwpx
    } else {
        return Err(EngineError::new(
            "BAD_REQUEST",
            "invalid virtual base format",
        ));
    };
    if doc.meta.format != desired {
        let container = merge_container::Container::read(&doc)?;
        if container.view.as_object().is_some_and(|m| {
            m.values()
                .any(|v| v["byteLength"].as_u64().unwrap_or(0) > 0)
        }) {
            return Err(EngineError::new(
                "MERGE_VALIDATION_FAILED",
                "virtual base conversion requires resolving opaque container metadata in the target format",
            ));
        }
        doc.convert_to(desired);
        out = match desired {
            hm_model::Format::Hwp5 => hm_hwp5::write_new(&doc)
                .map_err(|e| EngineError::new("MERGE_VALIDATION_FAILED", e))?,
            _ => hm_hwpx::write_new(&doc)
                .map_err(|e| EngineError::new("MERGE_VALIDATION_FAILED", e))?,
        };
    }
    Ok(out)
}
