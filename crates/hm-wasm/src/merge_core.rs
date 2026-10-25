//! Deterministic three-way property merging. Missing values remain distinct
//! from JSON null, and unresolved choices never materialize an output.
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Conflict {
    pub id: String,
    pub kind: String,
    pub path: Vec<String>,
    pub reason: String,
    pub base: Value,
    pub current: Value,
    pub incoming: Value,
    pub supports_both: bool,
    pub supports_manual: bool,
    pub fingerprint: String,
    #[serde(default = "present")]
    pub base_present: bool,
    #[serde(default = "present")]
    pub current_present: bool,
    #[serde(default = "present")]
    pub incoming_present: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub automatic: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<Value>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Analysis {
    pub analysis_version: u32,
    pub result: Value,
    pub conflicts: Vec<Conflict>,
    pub automatic_operation_count: u32,
    #[serde(default)]
    pub visited_node_count: u32,
    #[serde(default)]
    pub budget_exceeded: bool,
}
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Resolution {
    Current,
    Incoming,
    Both { order: String },
    Manual { payload: Value },
}
fn present() -> bool {
    true
}
pub type Resolutions = BTreeMap<String, Resolution>;
struct Merge {
    conflicts: Vec<Conflict>,
    automatic: u32,
    visited: u32,
    budget: bool,
}
fn digest(value: &Value) -> String {
    format!(
        "blake3:{}",
        blake3::hash(value.to_string().as_bytes()).to_hex()
    )
}
fn kind(path: &[String], value: Option<&Value>) -> String {
    if let Some(k) = value.and_then(|v| v.get("kind")).and_then(Value::as_str) {
        return k.into();
    }
    match path.last().map(String::as_str) {
        Some("text") => "text",
        Some("styles" | "char_shapes" | "para_shapes") => "style",
        Some("resources" | "bins" | "payload" | "tail" | "xml" | "opaqueContent" | "Raw") => {
            "resource"
        }
        _ => "property",
    }
    .into()
}
impl Merge {
    fn conflict(
        &mut self,
        path: &[String],
        b: Option<&Value>,
        c: Option<&Value>,
        i: Option<&Value>,
        reason: &str,
    ) -> Option<Value> {
        let opaque = (path.first().is_some_and(|p| p == "opaque") && path.len() == 2)
            || path.last().is_some_and(|s| {
                matches!(
                    s.as_str(),
                    "payload" | "tail" | "xml" | "opaqueContent" | "Raw"
                )
            });
        let fingerprint = digest(&json!([
            path,
            b,
            c,
            i,
            b.is_some(),
            c.is_some(),
            i.is_some()
        ]));
        self.conflicts.push(Conflict {
            id: fingerprint.replacen("blake3:", "conflict:", 1),
            kind: kind(path, c.or(i)),
            path: path.to_vec(),
            reason: reason.into(),
            base: b.cloned().unwrap_or(Value::Null),
            current: c.cloned().unwrap_or(Value::Null),
            incoming: i.cloned().unwrap_or(Value::Null),
            supports_both: !opaque && pure_insertions(b, c, i),
            supports_manual: !opaque,
            fingerprint,
            base_present: b.is_some(),
            current_present: c.is_some(),
            incoming_present: i.is_some(),
            automatic: false,
            position: None,
        });
        c.cloned()
    }
    fn value(
        &mut self,
        path: &[String],
        b: Option<&Value>,
        c: Option<&Value>,
        i: Option<&Value>,
        depth: usize,
    ) -> Option<Value> {
        self.visited += 1;
        if c == i {
            self.automatic += 1;
            return c.cloned();
        }
        if c == b {
            self.automatic += 1;
            return i.cloned();
        }
        if i == b {
            self.automatic += 1;
            return c.cloned();
        }
        if depth > 96 || self.visited > 100_000 {
            self.budget = true;
            return self.conflict(path, b, c, i, "budget-exceeded");
        }
        if c.is_none() || i.is_none() {
            return self.conflict(path, b, c, i, "delete-versus-edit");
        }
        if (path.first().is_some_and(|p| p == "opaque") && path.len() == 2)
            || path.last().is_some_and(|s| {
                matches!(
                    s.as_str(),
                    "payload" | "tail" | "xml" | "opaqueContent" | "Raw"
                )
            })
        {
            return self.conflict(path, b, c, i, "unknown-control-modified");
        }
        if let (Some(Value::Object(c)), Some(Value::Object(i))) = (c, i) {
            let b = b.and_then(Value::as_object);
            let keys: BTreeSet<_> = b
                .into_iter()
                .flat_map(Map::keys)
                .chain(c.keys())
                .chain(i.keys())
                .cloned()
                .collect();
            let mut out = Map::new();
            for key in keys {
                let mut p = path.to_vec();
                p.push(key.clone());
                if let Some(v) = self.value(
                    &p,
                    b.and_then(|b| b.get(&key)),
                    c.get(&key),
                    i.get(&key),
                    depth + 1,
                ) {
                    out.insert(key, v);
                }
            }
            return Some(Value::Object(out));
        }
        if let (Some(Value::Array(b)), Some(Value::Array(c)), Some(Value::Array(i))) = (b, c, i) {
            if let Some(key) = identity_key(b, c, i) {
                return Some(self.keyed(path, b, c, i, key, depth));
            }
            if b.len() == c.len() && b.len() == i.len() {
                let mut out = Vec::new();
                for n in 0..b.len() {
                    let mut p = path.to_vec();
                    p.push(n.to_string());
                    out.push(
                        self.value(&p, Some(&b[n]), Some(&c[n]), Some(&i[n]), depth + 1)
                            .unwrap_or(Value::Null),
                    );
                }
                return Some(Value::Array(out));
            }
        }
        let reason = if b.is_none() || pure_insertions(b, c, i) {
            "concurrent-insertion"
        } else {
            "same-field-changed"
        };
        self.conflict(path, b, c, i, reason)
    }
    fn keyed(
        &mut self,
        path: &[String],
        b: &[Value],
        c: &[Value],
        i: &[Value],
        key: &str,
        depth: usize,
    ) -> Value {
        let index = |values: &[Value]| -> BTreeMap<String, Value> {
            values
                .iter()
                .map(|v| (v[key].to_string(), v.clone()))
                .collect()
        };
        let bm = index(b);
        let cm = index(c);
        let im = index(i);
        let keys = |values: &[Value]| {
            values
                .iter()
                .map(|v| v[key].to_string())
                .collect::<Vec<_>>()
        };
        let bk = keys(b);
        let ck = keys(c);
        let ik = keys(i);
        let common = |keys: &[String]| {
            keys.iter()
                .filter(|k| bm.contains_key(*k) && cm.contains_key(*k) && im.contains_key(*k))
                .cloned()
                .collect::<Vec<_>>()
        };
        let (bo, co, io) = (common(&bk), common(&ck), common(&ik));
        if co != bo && io != bo && co != io {
            return self
                .conflict(
                    path,
                    Some(&Value::Array(b.to_vec())),
                    Some(&Value::Array(c.to_vec())),
                    Some(&Value::Array(i.to_vec())),
                    "incompatible-move",
                )
                .unwrap();
        }
        let mut order = if co == bo && io != bo {
            ik.clone()
        } else {
            ck.clone()
        };
        for source in [&ck, &ik, &bk] {
            for (n, id) in source.iter().enumerate() {
                if order.contains(id) {
                    continue;
                }
                let at = source[n + 1..]
                    .iter()
                    .find_map(|next| order.iter().position(|v| v == next))
                    .unwrap_or(order.len());
                order.insert(at, id.clone());
            }
        }
        let mut out = Vec::new();
        for id in order {
            let mut p = path.to_vec();
            p.push(out.len().to_string());
            let before = self.conflicts.len();
            let v = self.value(&p, bm.get(&id), cm.get(&id), im.get(&id), depth + 1);
            if let Some(v) = v {
                out.push(v);
            } else if self.conflicts.len() != before {
                out.push(Value::Null);
            }
        }
        Value::Array(out)
    }
}
fn identity_key<'a>(b: &[Value], c: &[Value], i: &[Value]) -> Option<&'a str> {
    ["stableId", "identity", "id"].into_iter().find(|key| {
        [b, c, i].iter().all(|values| {
            let mut seen = BTreeSet::new();
            values.iter().all(|v| {
                v.get(*key)
                    .is_some_and(|v| (v.is_string() || v.is_number()) && seen.insert(v.to_string()))
            })
        })
    })
}
fn subsequence(b: &[Value], v: &[Value]) -> bool {
    let mut n = 0;
    for x in v {
        if b.get(n) == Some(x) {
            n += 1;
        }
    }
    n == b.len()
}
fn pure_insertions(b: Option<&Value>, c: Option<&Value>, i: Option<&Value>) -> bool {
    match (b, c, i) {
        (Some(Value::Array(b)), Some(Value::Array(c)), Some(Value::Array(i))) => {
            c.len() > b.len() && i.len() > b.len() && subsequence(b, c) && subsequence(b, i)
        }
        (Some(Value::String(b)), Some(Value::String(c)), Some(Value::String(i))) => {
            let values = |s: &str| {
                s.chars()
                    .map(|c| Value::String(c.to_string()))
                    .collect::<Vec<_>>()
            };
            pure_insertions(
                Some(&Value::Array(values(b))),
                Some(&Value::Array(values(c))),
                Some(&Value::Array(values(i))),
            )
        }
        _ => false,
    }
}
fn combine_insertions(b: &[Value], c: &[Value], i: &[Value], first: bool) -> Vec<Value> {
    let (mut x, mut y) = (0, 0);
    let mut out = Vec::new();
    for anchor in b.iter().map(Some).chain(std::iter::once(None)) {
        let start_x = x;
        let start_y = y;
        while x < c.len() && anchor != Some(&c[x]) {
            x += 1;
        }
        while y < i.len() && anchor != Some(&i[y]) {
            y += 1;
        }
        let left = &c[start_x..x];
        let right = &i[start_y..y];
        if left == right {
            out.extend_from_slice(left);
        } else if first {
            out.extend_from_slice(left);
            out.extend_from_slice(right);
        } else {
            out.extend_from_slice(right);
            out.extend_from_slice(left);
        }
        if let Some(anchor) = anchor {
            out.push(anchor.clone());
            x += 1;
            y += 1;
        }
    }
    out
}
pub fn analyze(base: &Value, current: &Value, incoming: &Value) -> Analysis {
    let mut m = Merge {
        conflicts: Vec::new(),
        automatic: 0,
        visited: 0,
        budget: false,
    };
    let result = m
        .value(&[], Some(base), Some(current), Some(incoming), 0)
        .unwrap_or(Value::Null);
    Analysis {
        analysis_version: 1,
        result,
        conflicts: m.conflicts,
        automatic_operation_count: m.automatic,
        visited_node_count: m.visited,
        budget_exceeded: m.budget,
    }
}
fn choice(c: &Conflict, r: &Resolution) -> Result<Option<Value>, String> {
    Ok(match r {
        Resolution::Current => c.current_present.then(|| c.current.clone()),
        Resolution::Incoming => c.incoming_present.then(|| c.incoming.clone()),
        Resolution::Manual { payload } => {
            if !c.supports_manual {
                return Err(format!("manual edits are unavailable for {}", c.id));
            }
            Some(payload.clone())
        }
        Resolution::Both { order } => {
            if !c.supports_both || !matches!(order.as_str(), "current-first" | "incoming-first") {
                return Err(format!("keep-both is unavailable for {}", c.id));
            }
            let first = order == "current-first";
            Some(match (&c.base, &c.current, &c.incoming) {
                (Value::Array(b), Value::Array(c), Value::Array(i)) => {
                    Value::Array(combine_insertions(b, c, i, first))
                }
                (Value::String(b), Value::String(c), Value::String(i)) => {
                    let array = |s: &str| {
                        s.chars()
                            .map(|c| Value::String(c.to_string()))
                            .collect::<Vec<_>>()
                    };
                    Value::String(
                        combine_insertions(&array(b), &array(c), &array(i), first)
                            .iter()
                            .filter_map(Value::as_str)
                            .collect(),
                    )
                }
                _ => return Err("invalid keep-both payload".into()),
            })
        }
    })
}
#[derive(Default)]
struct Patch {
    value: Option<Option<Value>>,
    children: BTreeMap<String, Patch>,
}
impl Patch {
    fn insert(&mut self, path: &[String], value: Option<Value>) -> Result<(), String> {
        if path.is_empty() {
            if self.value.is_some() || !self.children.is_empty() {
                return Err("overlapping merge conflicts".into());
            }
            self.value = Some(value);
            return Ok(());
        }
        if self.value.is_some() {
            return Err("overlapping merge conflicts".into());
        }
        self.children
            .entry(path[0].clone())
            .or_default()
            .insert(&path[1..], value)
    }
    fn apply(&self, value: Option<&Value>) -> Result<Option<Value>, String> {
        if let Some(value) = &self.value {
            return Ok(value.clone());
        }
        if self.children.is_empty() {
            return Ok(value.cloned());
        }
        match value {
            Some(Value::Array(v)) => {
                let mut out = Vec::new();
                for (n, v) in v.iter().enumerate() {
                    if let Some(p) = self.children.get(&n.to_string()) {
                        if let Some(v) = p.apply(Some(v))? {
                            out.push(v);
                        }
                    } else {
                        out.push(v.clone());
                    }
                }
                if self
                    .children
                    .keys()
                    .any(|k| k.parse::<usize>().map_or(true, |n| n >= v.len()))
                {
                    return Err("invalid merge array path".into());
                }
                Ok(Some(Value::Array(out)))
            }
            Some(Value::Object(v)) => {
                let mut out = v.clone();
                for (k, p) in &self.children {
                    if let Some(v) = p.apply(v.get(k))? {
                        out.insert(k.clone(), v);
                    } else {
                        out.remove(k);
                    }
                }
                Ok(Some(Value::Object(out)))
            }
            _ => Err("invalid merge property path".into()),
        }
    }
}
pub fn materialize(analysis: &Analysis, resolutions: &Resolutions) -> Result<Value, String> {
    let ids: BTreeSet<_> = analysis
        .conflicts
        .iter()
        .flat_map(|c| [c.id.as_str(), c.fingerprint.as_str()])
        .collect();
    if resolutions.keys().any(|id| !ids.contains(id.as_str())) {
        return Err("resolution refers to an unknown conflict".into());
    }
    let mut patch = Patch::default();
    for c in &analysis.conflicts {
        let r = resolutions
            .get(&c.id)
            .or_else(|| resolutions.get(&c.fingerprint))
            .ok_or_else(|| format!("unresolved merge conflict {}", c.id))?;
        if resolutions
            .get(&c.id)
            .zip(resolutions.get(&c.fingerprint))
            .is_some_and(|(a, b)| a != b)
        {
            return Err("conflicting resolution aliases".into());
        }
        patch.insert(&c.path, choice(c, r)?)?;
    }
    Ok(patch.apply(Some(&analysis.result))?.unwrap_or(Value::Null))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_properties_and_keyed_moves_merge() {
        let a = analyze(
            &json!({"text":"a","color":"black"}),
            &json!({"text":"b","color":"black"}),
            &json!({"text":"a","color":"red"}),
        );
        assert!(a.conflicts.is_empty());
        assert_eq!(a.result, json!({"text":"b","color":"red"}));
        let a = analyze(
            &json!([{"id":"a","text":"a"},{"id":"b","text":"b"}]),
            &json!([{"id":"b","text":"b"},{"id":"a","text":"a"}]),
            &json!([{"id":"a","text":"a"},{"id":"b","text":"B"}]),
        );
        assert_eq!(
            a.result,
            json!([{"id":"b","text":"B"},{"id":"a","text":"a"}])
        );
        assert!(a.conflicts.is_empty());
    }
    #[test]
    fn deletions_and_null_remain_distinct() {
        let a = analyze(&json!({"p":1}), &json!({}), &json!({"p":2}));
        assert_eq!(a.conflicts[0].reason, "delete-versus-edit");
        let mut r = Resolutions::new();
        r.insert(a.conflicts[0].id.clone(), Resolution::Current);
        assert_eq!(materialize(&a, &r).unwrap(), json!({}));
        r.insert(
            a.conflicts[0].id.clone(),
            Resolution::Manual {
                payload: Value::Null,
            },
        );
        assert_eq!(materialize(&a, &r).unwrap(), json!({"p":null}));
    }
    #[test]
    fn unknown_and_unresolved_choices_fail_closed() {
        let a = analyze(&json!("a"), &json!("b"), &json!("c"));
        assert!(materialize(&a, &Resolutions::new()).is_err());
        let mut r = Resolutions::new();
        r.insert("wrong".into(), Resolution::Current);
        assert!(materialize(&a, &r).is_err());
        let a = analyze(
            &json!({"tail":"a"}),
            &json!({"tail":"b"}),
            &json!({"tail":"c"}),
        );
        r.clear();
        r.insert(
            a.conflicts[0].id.clone(),
            Resolution::Manual {
                payload: json!("x"),
            },
        );
        assert!(materialize(&a, &r).is_err());
    }
    #[test]
    fn both_keeps_insertions_without_duplicating_base_text() {
        let a = analyze(
            &json!("body"),
            &json!("current body"),
            &json!("incoming body"),
        );
        assert!(a.conflicts[0].supports_both);
        let mut r = Resolutions::new();
        r.insert(
            a.conflicts[0].id.clone(),
            Resolution::Both {
                order: "current-first".into(),
            },
        );
        assert_eq!(materialize(&a, &r).unwrap(), json!("current incoming body"));
    }
}
