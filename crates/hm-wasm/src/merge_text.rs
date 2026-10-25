//! Wire text becomes identity-bearing units before property merging. Formats
//! travel with their units; offsets are regenerated only after resolution.
use super::{
    merge_resources::hash,
    types::{EngineError, Result},
};
use hm_model::{control_width, is_extended_control};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
fn bad(s: &str) -> EngineError {
    EngineError::new("MERGE_VALIDATION_FAILED", s)
}
fn paragraph(v: &Value) -> bool {
    v.get("header").is_some()
        && v.get("text").is_some()
        && v.get("runs").is_some()
        && v.get("controls").is_some()
}
fn tokens(p: &Value) -> Result<Vec<Value>> {
    let text = p["text"]
        .as_array()
        .ok_or_else(|| bad("paragraph text is not an array"))?;
    let runs = p["runs"]
        .as_array()
        .ok_or_else(|| bad("paragraph formats are not an array"))?;
    let mut out = Vec::new();
    let mut pos = 0;
    let mut control = 0;
    let kind = |c: &Value| {
        let k = c["kind"]
            .as_object()
            .and_then(|m| m.keys().next())
            .cloned()
            .unwrap_or_default();
        format!("{k}:{}", c["kind"][&k].get("id").unwrap_or(&Value::Null))
    };
    let mut counts = BTreeMap::new();
    for c in p["controls"].as_array().into_iter().flatten() {
        *counts.entry(kind(c)).or_insert(0usize) += 1;
    }
    while pos < text.len() {
        let u = text[pos]
            .as_u64()
            .ok_or_else(|| bad("invalid UTF-16 unit"))? as u16;
        let width = if u < 32 {
            control_width(u) as usize
        } else if (0xd800..0xdc00).contains(&u)
            && text
                .get(pos + 1)
                .and_then(Value::as_u64)
                .is_some_and(|v| (0xdc00..0xe000).contains(&v))
        {
            2
        } else {
            1
        };
        if pos + width > text.len() {
            return Err(bad("truncated inline control"));
        }
        let shape = runs
            .iter()
            .rev()
            .find(|r| r["pos"].as_u64().is_some_and(|n| n <= pos as u64))
            .or_else(|| runs.first())
            .map(|r| r["shape"].clone())
            .unwrap_or(json!(0));
        let mut t = json!({"units":&text[pos..pos+width],"format":shape});
        if is_extended_control(u) {
            t["control"] = p["controls"]
                .get(control)
                .cloned()
                .ok_or_else(|| bad("missing inline control"))?;
            let k = kind(&t["control"]);
            if counts.get(&k) == Some(&1) {
                t["controlKey"] = json!(format!("singleton:{k}"));
            }
            control += 1;
        }
        out.push(t);
        pos += width;
    }
    if control != p["controls"].as_array().map_or(0, Vec::len) {
        return Err(bad("unanchored inline control"));
    }
    Ok(out)
}
fn instance(v: &Value) -> Option<u64> {
    match v {
        Value::Object(m) => m
            .get("instance_id")
            .and_then(Value::as_u64)
            .filter(|v| *v != 0)
            .or_else(|| m.values().find_map(instance)),
        Value::Array(a) => a.iter().find_map(instance),
        _ => None,
    }
}
fn key(t: &Value) -> String {
    if let Some(k) = t["controlKey"].as_str() {
        return k.to_string();
    }
    if let Some(c) = t.get("control") {
        let kind = c["kind"]
            .as_object()
            .and_then(|m| m.keys().next())
            .cloned()
            .unwrap_or_default();
        return format!(
            "control:{kind}:{}",
            instance(c).map_or_else(|| hash(c), |id| id.to_string())
        );
    }
    t["units"].to_string()
}
fn matches(b: &[Value], o: &[Value]) -> Option<Vec<(usize, usize)>> {
    let bk = b.iter().map(key).collect::<Vec<_>>();
    let ok = o.iter().map(key).collect::<Vec<_>>();
    if b.len().saturating_mul(o.len()) > 4_000_000 {
        return None;
    }
    let width = o.len() + 1;
    let mut lengths = vec![0u32; (b.len() + 1) * width];
    for x in (0..b.len()).rev() {
        for y in (0..o.len()).rev() {
            lengths[x * width + y] = if bk[x] == ok[y] {
                1 + lengths[(x + 1) * width + y + 1]
            } else {
                lengths[(x + 1) * width + y].max(lengths[x * width + y + 1])
            };
        }
    }
    let (mut x, mut y) = (0, 0);
    let mut out = Vec::new();
    while x < b.len() && y < o.len() {
        if bk[x] == ok[y] {
            out.push((x, y));
            x += 1;
            y += 1;
        } else if lengths[(x + 1) * width + y] >= lengths[x * width + y + 1] {
            x += 1;
        } else {
            y += 1;
        }
    }
    Some(out)
}
fn annotate(
    p: &mut Value,
    mut ts: Vec<Value>,
    mapping: &[(usize, usize)],
    base_len: usize,
) -> Result<()> {
    let mut content = Vec::new();
    let mut prior = 0;
    let mut left = "start".to_string();
    for &(bi, oi) in mapping.iter().chain(std::iter::once(&(base_len, ts.len()))) {
        if prior < oi {
            let right = format!("unit:{bi}");
            let id = format!("gap:{left}:{right}");
            let mut inserted: Vec<Value> = ts[prior..oi].iter_mut().map(std::mem::take).collect();
            for (n, t) in inserted.iter_mut().enumerate() {
                t["stableId"] = json!(format!("{id}:{n}"));
            }
            content.push(json!({"stableId":id,"insertions":inserted}));
        }
        if oi < ts.len() {
            ts[oi]["stableId"] = json!(format!("unit:{bi}"));
            content.push(std::mem::take(&mut ts[oi]));
            left = format!("unit:{bi}");
        }
        prior = oi + 1;
    }
    // Position marks refer to a unit and an offset within it, including marks
    // between UTF-16 surrogates. A deleted anchor must be resolved explicitly.
    let flat = flatten(&content);
    let mut pos = 0u64;
    let mut anchors = BTreeMap::new();
    for t in flat {
        let id = t["stableId"].as_str().unwrap();
        let n = t["units"].as_array().unwrap().len() as u64;
        for k in 0..n {
            anchors.insert(pos + k, json!({"unit":id,"offset":k}));
        }
        pos += n;
    }
    anchors.insert(pos, json!({"end":true}));
    for (name, fields) in [
        ("range_tags", &["start", "end"][..]),
        ("tracks", &["pos"][..]),
    ] {
        if let Some(a) = p[name].as_array_mut() {
            for mark in a {
                for field in fields {
                    let n = mark[*field]
                        .as_u64()
                        .ok_or_else(|| bad("invalid annotation position"))?;
                    mark[*field] = anchors
                        .get(&n)
                        .cloned()
                        .ok_or_else(|| bad("annotation exceeds paragraph"))?;
                }
            }
        }
    }
    let m = p.as_object_mut().unwrap();
    for k in ["text", "runs", "controls"] {
        m.remove(k);
    }
    m.insert("content".into(), Value::Array(content));
    Ok(())
}
fn flatten(content: &[Value]) -> Vec<&Value> {
    content
        .iter()
        .flat_map(|v| {
            if let Some(a) = v.get("insertions").and_then(Value::as_array) {
                a.iter().collect()
            } else {
                vec![v]
            }
        })
        .collect()
}
/// Align three corresponding paragraphs, including nested control lists.
pub fn project(b: &mut Value, c: &mut Value, i: &mut Value) -> Result<()> {
    if paragraph(b) && paragraph(c) && paragraph(i) {
        let (bt, ct, it) = (tokens(b)?, tokens(c)?, tokens(i)?);
        let (cm, im) = (matches(&bt, &ct), matches(&bt, &it));
        if let (Some(cm), Some(im)) = (cm, im) {
            let (bm, base_len) = ((0..bt.len()).map(|n| (n, n)).collect::<Vec<_>>(), bt.len());
            annotate(b, bt, &bm, base_len)?;
            annotate(c, ct, &cm, base_len)?;
            annotate(i, it, &im, base_len)?;
        } else {
            // The bound is a review conflict rather than a partial alignment.
            for p in [&mut *b, &mut *c, &mut *i] {
                let m = p.as_object_mut().unwrap();
                let wire = json!({"text":m.remove("text"),"runs":m.remove("runs"),"controls":m.remove("controls"),"range_tags":m.remove("range_tags"),"tracks":m.remove("tracks")});
                m.insert("opaqueContent".into(), wire);
            }
            return Ok(());
        }
    }
    match (b, c, i) {
        (Value::Object(b), Value::Object(c), Value::Object(i)) => {
            // A text unit's format and wire units hold no paragraphs; only its
            // control can.
            let unit = b.contains_key("units");
            let keys = b
                .keys()
                .filter(|k| !(unit && (*k == "format" || *k == "units")))
                .filter(|k| c.contains_key(*k) && i.contains_key(*k))
                .cloned()
                .collect::<Vec<_>>();
            for k in keys {
                project(
                    b.get_mut(&k).unwrap(),
                    c.get_mut(&k).unwrap(),
                    i.get_mut(&k).unwrap(),
                )?;
            }
        }
        (Value::Array(b), Value::Array(c), Value::Array(i)) => {
            let keyed = b
                .iter()
                .chain(c.iter())
                .chain(i.iter())
                .all(|v| v["stableId"].is_string());
            if keyed {
                for bv in b {
                    let id = bv["stableId"].clone();
                    if let (Some(cv), Some(iv)) = (
                        c.iter_mut().find(|v| v["stableId"] == id),
                        i.iter_mut().find(|v| v["stableId"] == id),
                    ) {
                        project(bv, cv, iv)?;
                    }
                }
            } else if b.len() == c.len() && b.len() == i.len() {
                for ((b, c), i) in b.iter_mut().zip(c).zip(i) {
                    project(b, c, i)?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}
pub fn restore(v: &mut Value) -> Result<()> {
    if v.get("content").is_some() && v.get("header").is_some() {
        let ts = flatten(
            v["content"]
                .as_array()
                .ok_or_else(|| bad("invalid merged content"))?,
        );
        let mut text = Vec::new();
        let mut runs = Vec::<Value>::new();
        let mut controls = Vec::new();
        let mut positions = BTreeMap::new();
        let mut ids = BTreeSet::new();
        for t in ts {
            let id = t["stableId"]
                .as_str()
                .ok_or_else(|| bad("merged text has no identity"))?;
            if !ids.insert(id) {
                return Err(bad("duplicate merged text identity"));
            }
            positions.insert(id.to_string(), text.len() as u64);
            if runs.last().is_none_or(|r| r["shape"] != t["format"]) {
                runs.push(json!({"pos":text.len(),"shape":t["format"]}));
            }
            text.extend(
                t["units"]
                    .as_array()
                    .ok_or_else(|| bad("merged text has no wire units"))?
                    .iter()
                    .cloned(),
            );
            if let Some(c) = t.get("control") {
                controls.push(c.clone());
            }
        }
        let end = text.len() as u64;
        for (name, fields) in [
            ("range_tags", &["start", "end"][..]),
            ("tracks", &["pos"][..]),
        ] {
            if let Some(a) = v[name].as_array_mut() {
                for mark in a {
                    for field in fields {
                        let a = &mark[*field];
                        let n = if a["end"] == true {
                            end
                        } else {
                            let id = a["unit"]
                                .as_str()
                                .ok_or_else(|| bad("invalid merged annotation anchor"))?;
                            positions.get(id).copied().ok_or_else(||bad("merged annotation anchor was deleted; choose the paragraph version containing that annotation"))?+a["offset"].as_u64().unwrap_or(0)
                        };
                        mark[*field] = json!(n);
                    }
                }
            }
        }
        let m = v.as_object_mut().unwrap();
        m.remove("content");
        m.insert("text".into(), json!(text));
        m.insert("runs".into(), json!(runs));
        m.insert("controls".into(), json!(controls));
    }
    if let Some(wire) = v.as_object_mut().and_then(|m| m.remove("opaqueContent")) {
        v.as_object_mut().unwrap().extend(
            wire.as_object()
                .ok_or_else(|| bad("invalid opaque paragraph"))?
                .clone(),
        );
    }
    match v {
        Value::Object(m) => {
            for v in m.values_mut() {
                restore(v)?;
            }
        }
        Value::Array(a) => {
            for v in a {
                restore(v)?;
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::merge_core;
    fn para(text: &str) -> Value {
        json!({"header":{},"text":text.encode_utf16().chain([13]).collect::<Vec<_>>(),"runs":[{"pos":0,"shape":{"bold":false,"color":0}}],"controls":[],"range_tags":[],"tracks":[]})
    }
    fn merged(mut b: Value, mut c: Value, mut i: Value) -> (Value, usize) {
        project(&mut b, &mut c, &mut i).unwrap();
        let a = merge_core::analyze(&b, &c, &i);
        let n = a.conflicts.len();
        let choices = a
            .conflicts
            .iter()
            .map(|c| (c.id.clone(), merge_core::Resolution::Incoming))
            .collect();
        let mut v = merge_core::materialize(&a, &choices).unwrap();
        restore(&mut v).unwrap();
        (v, n)
    }
    #[test]
    fn disjoint_insertions_keep_formats_and_annotation_anchors() {
        let mut b = para("ab😀cd");
        b["range_tags"] = json!([{"start":2,"end":4,"tag":7}]);
        let mut c = b.clone();
        let mut i = b.clone();
        c["text"] = json!("Xab😀cd".encode_utf16().chain([13]).collect::<Vec<_>>());
        c["range_tags"][0]["start"] = json!(3);
        c["range_tags"][0]["end"] = json!(5);
        c["runs"][0]["shape"]["bold"] = json!(true);
        i["text"] = json!("ab😀cdY".encode_utf16().chain([13]).collect::<Vec<_>>());
        i["runs"][0]["shape"]["color"] = json!(42);
        let (v, n) = merged(b, c, i);
        assert_eq!(n, 0);
        assert_eq!(
            v["text"],
            json!("Xab😀cdY".encode_utf16().chain([13]).collect::<Vec<_>>())
        );
        assert_eq!(v["range_tags"][0]["start"], 3);
        assert_eq!(v["range_tags"][0]["end"], 5);
        // Existing units receive both independent format edits. Each branch's
        // new unit keeps the formatting that branch authored.
        assert_eq!(v["runs"][1]["shape"], json!({"bold":true,"color":42}));
    }
    #[test]
    fn concurrent_insertions_require_a_choice_without_leftovers() {
        let (v, n) = merged(para("ab"), para("aXXb"), para("aYb"));
        assert!(n > 0);
        assert_eq!(v["text"], para("aYb")["text"]);
    }
    #[test]
    fn inline_controls_follow_their_anchors_through_text_edits() {
        let mut b = para("ab");
        b["text"] = json!([97, 11, 0, 0, 0, 0, 0, 0, 11, 98, 13]);
        b["controls"] = json!([{"kind":{"Object":{"header":{"instance_id":7},"color":0}}}]);
        let mut c = b.clone();
        let mut i = b.clone();
        c["text"].as_array_mut().unwrap().insert(0, json!(88));
        i["controls"][0]["kind"]["Object"]["color"] = json!(8);
        let (v, n) = merged(b, c, i);
        assert_eq!(n, 0);
        assert_eq!(v["text"][2], 11);
        assert_eq!(v["controls"][0]["kind"]["Object"]["color"], 8);
        assert_eq!(v["controls"].as_array().unwrap().len(), 1);
    }
}
