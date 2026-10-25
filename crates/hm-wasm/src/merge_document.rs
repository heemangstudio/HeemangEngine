//! Structural version reviews operate on typed model properties and logical
//! identities. Materialization retains the current native container context.
use super::{
    merge_core::{self, Analysis, Conflict, Resolution, Resolutions},
    merge_resources::{Resources, hash},
    types::{EngineError, Result},
};
use hm_model::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap};
fn bad(message: impl std::fmt::Display) -> EngineError {
    EngineError::new("MERGE_VALIDATION_FAILED", message)
}
fn property<T: Serialize>(v: &T) -> Result<Value> {
    serde_json::to_value(v).map_err(bad)
}
pub fn open(bytes: &[u8]) -> Result<Document> {
    let b = SharedBytes::from(bytes);
    if hm_hwpx::is_hwpx(&b) {
        hm_hwpx::read(b).map_err(bad)
    } else if hm_hwp5::is_hwp5(&b) {
        hm_hwp5::read(b).map_err(bad)
    } else if hm_hml::is_hml(&b) {
        hm_hml::read(b).map_err(bad)
    } else if hm_hwp3::is_hwp3(&b) {
        hm_hwp3::read(b).map_err(bad)
    } else {
        Err(bad("unsupported merge document format"))
    }
}
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Manifest {
    #[serde(default)]
    pub entries: Vec<Entry>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub kind: String,
    pub path: Vec<String>,
    pub property_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<String>,
}
fn identity(manifest: &Manifest, path: &[String]) -> Option<String> {
    manifest
        .entries
        .iter()
        .find(|e| e.path == path)
        .and_then(|e| e.identity.clone().or(e.identity_hint.clone()))
}
fn named(v: &Value) -> Option<&str> {
    v.get("stableId").and_then(Value::as_str)
}
fn paragraphs(v: &Value) -> &[Value] {
    v.get("paragraphs")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}
fn sections(v: &Value) -> &[Value] {
    v.get("sections")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}
fn find<'a>(values: &'a [Value], id: &str) -> Option<&'a Value> {
    values.iter().find(|v| named(v) == Some(id))
}
fn plain(v: &Value) -> String {
    let mut raw = v.clone();
    if super::merge_text::restore(&mut raw).is_err() {
        return String::new();
    }
    let units = raw
        .get("text")
        .and_then(Value::as_array)
        .map(|v| {
            v.iter()
                .filter_map(|v| v.as_u64().map(|n| n as u16))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let p = Paragraph {
        text: units,
        ..Default::default()
    };
    p.plain_text()
}
fn control_count(v: &Value) -> usize {
    let mut v = v.clone();
    if super::merge_text::restore(&mut v).is_err() {
        return 0;
    }
    v["controls"].as_array().map_or(0, Vec::len)
}
fn brief(v: Option<&Value>) -> Value {
    v.map(|v|{if v.get("header").is_some(){json!({"kind":"paragraph","text":plain(v),"controls":control_count(v)})}else if v.get("paragraphs").is_some(){json!({"kind":"section","paragraphs":paragraphs(v).iter().map(|p|json!({"text":plain(p),"controls":control_count(p)})).collect::<Vec<_>>()})}else{v.clone()}}).unwrap_or(Value::Null)
}
fn normalized_para(p: &Paragraph) -> Result<Value> {
    let mut value = property(p)?;
    value["header"]["instance_id"] = json!(0);
    Ok(value)
}
fn align(base: &[Value], other: &mut [Value], prefix: &str) {
    let mut hashes = HashMap::<String, Vec<String>>::new();
    for v in base {
        if let Some(id) = named(v) {
            let mut p = v.clone();
            p.as_object_mut().unwrap().remove("stableId");
            hashes.entry(hash(&p)).or_default().push(id.to_string());
        }
    }
    let mut used: BTreeSet<String> = other.iter().filter_map(named).map(str::to_owned).collect();
    for (n, v) in other.iter_mut().enumerate() {
        if named(v).is_some() {
            continue;
        }
        let key = hash(v);
        let id = hashes
            .get(&key)
            .and_then(|ids| {
                let unused = ids
                    .iter()
                    .filter(|id| !used.contains(*id))
                    .collect::<Vec<_>>();
                (unused.len() == 1).then(|| unused[0].clone())
            })
            .unwrap_or_else(|| format!("{prefix}:{n}:{key}"));
        used.insert(id.clone());
        v["stableId"] = json!(id);
    }
    // Equal-length edited siblings stay positioned only when unmatched slots are
    // unambiguous after unchanged anchors have claimed their identities.
    if base.len() == other.len() {
        for (n, v) in other.iter_mut().enumerate() {
            let base_id = named(&base[n]).unwrap();
            if named(v).is_some_and(|id| id.starts_with(prefix)) && !used.contains(base_id) {
                used.remove(named(v).unwrap());
                used.insert(base_id.to_string());
                v["stableId"] = json!(base_id);
            }
        }
    }
}
fn align_insertions(base: &[Value], current: &mut [Value], incoming: &mut [Value]) {
    fn semantic(v: &Value) -> Value {
        match v {
            Value::Object(m) => Value::Object(
                m.iter()
                    .filter(|(k, _)| k.as_str() != "stableId")
                    .map(|(k, v)| (k.clone(), semantic(v)))
                    .collect(),
            ),
            Value::Array(a) => Value::Array(a.iter().map(semantic).collect()),
            _ => v.clone(),
        }
    }
    let ids = base.iter().filter_map(named).collect::<BTreeSet<_>>();
    let mut matches = HashMap::<String, Vec<String>>::new();
    for v in current {
        if let Some(id) = named(v)
            && !ids.contains(id)
        {
            matches
                .entry(hash(&semantic(v)))
                .or_default()
                .push(id.to_string());
        }
    }
    let mut counts = HashMap::<String, usize>::new();
    for v in incoming.iter() {
        if named(v).is_some_and(|id| !ids.contains(id)) {
            *counts.entry(hash(&semantic(v))).or_default() += 1;
        }
    }
    for v in incoming {
        if named(v).is_some_and(|id| !ids.contains(id)) {
            let h = hash(&semantic(v));
            if counts.get(&h) == Some(&1)
                && let Some(ids) = matches.get(&h)
                && ids.len() == 1
            {
                v["stableId"] = json!(ids[0]);
            }
        }
    }
}
#[derive(Clone)]
enum Scope {
    Root(String),
    OpaqueMember(String),
    Section(String),
    SectionProperties(String),
    Paragraph(String, String),
    Order(String),
}
#[derive(Clone)]
struct Group {
    scope: Scope,
    conflict: Conflict,
    incoming: Option<Value>,
}
pub struct Prepared<'a> {
    pub resources: Resources<'a>,
    pub views: [Value; 3],
    containers: [super::merge_container::Container; 3],
}
impl<'a> Prepared<'a> {
    pub fn new(docs: &'a [Document; 3], manifests: &[Manifest; 3]) -> Result<Self> {
        let mut resources = Resources::new(docs);
        let containers = [
            super::merge_container::Container::read(&docs[0])?,
            super::merge_container::Container::read(&docs[1])?,
            super::merge_container::Container::read(&docs[2])?,
        ];
        for (source, doc) in docs.iter().enumerate() {
            for id in 0..doc.info.styles.len() {
                let p = vec!["docInfo".into(), "styles".into(), id.to_string()];
                if let Some(key) = identity(&manifests[source], &p) {
                    resources.style_keys[source].insert(id as u32, key);
                }
            }
        }
        let mut views: [Value; 3] = std::array::from_fn(|_| Value::Null);
        for (source, doc) in docs.iter().enumerate() {
            let mut secs = Vec::new();
            for (si, section) in doc.sections.iter().enumerate() {
                let mut properties = property(section)?;
                properties.as_object_mut().unwrap().remove("paras");
                resources.expand(source, &mut properties)?;
                let mut ps = Vec::new();
                for (pi, p) in section.paras.iter().enumerate() {
                    let mut p = normalized_para(p)?;
                    resources.expand(source, &mut p)?;
                    let path = vec![
                        "sections".into(),
                        si.to_string(),
                        "paragraphs".into(),
                        pi.to_string(),
                    ];
                    if let Some(id) = identity(&manifests[source], &path) {
                        p["stableId"] = json!(id);
                    }
                    ps.push(p);
                }
                let mut s = json!({"properties":properties,"paragraphs":ps});
                let path = vec!["sections".into(), si.to_string()];
                if let Some(id) = identity(&manifests[source], &path) {
                    s["stableId"] = json!(id);
                }
                secs.push(s);
            }

            let mut metadata = json!({"properties":doc.info.properties,"compatible":doc.info.compatible,"layoutCompatibility":doc.info.layout_compat,"raw":doc.info.raw,"mappingCounts":doc.info.id_mappings.counts.get(15..).unwrap_or(&[]),"mappingTail":doc.info.id_mappings.tail});
            resources.expand(source, &mut metadata)?;
            views[source] = json!({"properties":metadata,"styles":resources.styles(source)?,"sections":secs,"opaque":containers[source].view});
        }
        // Base identities are deterministic without a persisted manifest.
        for (si, s) in views[0]["sections"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .enumerate()
        {
            if named(s).is_none() {
                s["stableId"] = json!(format!("section:{si}"));
            }
            for (pi, p) in s["paragraphs"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .enumerate()
            {
                if named(p).is_none() {
                    p["stableId"] = json!(format!("paragraph:{si}:{pi}"));
                }
            }
        }
        for source in [1usize, 2] {
            let base = sections(&views[0]).to_vec();
            align(
                &base,
                views[source]["sections"].as_array_mut().unwrap(),
                &format!("section-new-{source}"),
            );
            let source_sections = views[source]["sections"].as_array_mut().unwrap();
            for s in source_sections {
                let id = named(s).unwrap().to_string();
                if let Some(b) = find(&base, &id) {
                    align(
                        paragraphs(b),
                        s["paragraphs"].as_array_mut().unwrap(),
                        &format!("paragraph-new-{source}-{id}"),
                    );
                } else {
                    for (n, p) in s["paragraphs"]
                        .as_array_mut()
                        .unwrap()
                        .iter_mut()
                        .enumerate()
                    {
                        if named(p).is_none() {
                            p["stableId"] = json!(format!("paragraph-new-{source}-{id}:{n}"));
                        }
                    }
                }
            }
        }
        let [base, current, incoming] = &mut views;
        align_insertions(
            sections(base),
            current["sections"].as_array_mut().unwrap(),
            incoming["sections"].as_array_mut().unwrap(),
        );
        for c in current["sections"].as_array_mut().unwrap() {
            let id = named(c).unwrap().to_string();
            if let Some(i) = incoming["sections"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|v| named(v) == Some(&id))
            {
                align_insertions(
                    find(sections(base), &id).map(paragraphs).unwrap_or(&[]),
                    c["paragraphs"].as_array_mut().unwrap(),
                    i["paragraphs"].as_array_mut().unwrap(),
                );
            }
        }

        super::merge_text::project(base, current, incoming)?;
        Ok(Self {
            resources,
            views,
            containers,
        })
    }
    fn candidate(
        base: Option<&Value>,
        current: Option<&Value>,
        incoming: Option<&Value>,
    ) -> Result<(Option<Value>, bool)> {
        if incoming == base {
            return Ok((current.cloned(), false));
        }
        if current == base {
            return Ok((incoming.cloned(), false));
        }
        if current == incoming {
            return Ok((current.cloned(), false));
        }
        if current.is_none() || incoming.is_none() {
            return Ok((incoming.cloned(), true));
        }
        let a = merge_core::analyze(
            base.unwrap_or(&Value::Null),
            current.unwrap(),
            incoming.unwrap(),
        );
        let r = a
            .conflicts
            .iter()
            .map(|c| (c.id.clone(), Resolution::Incoming))
            .collect();
        let v = merge_core::materialize(&a, &r).map_err(bad)?;
        Ok((Some(v), !a.conflicts.is_empty()))
    }
    fn groups(&self) -> Result<Vec<Group>> {
        let [base, current, incoming] = &self.views;
        let mut out = Vec::new();
        fn push(
            out: &mut Vec<Group>,
            scope: Scope,
            path: Vec<String>,
            b: Option<&Value>,
            c: Option<&Value>,
            i: Option<&Value>,
            position: Option<Value>,
        ) -> Result<()> {
            if i == b {
                return Ok(());
            }
            let (candidate, conflicted) = if matches!(&scope, Scope::OpaqueMember(_)) {
                (i.cloned(), c != b && c != i)
            } else {
                Prepared::candidate(b, c, i)?
            };
            let fingerprint = hash(&json!([path, b, c, i]));
            let mut conflict = Conflict {
                id: fingerprint.replacen("blake3:", "conflict:", 1),
                kind: "document-change".into(),
                path,
                reason: "same-field-changed".into(),
                base: brief(b),
                current: brief(c),
                incoming: brief(candidate.as_ref()),
                supports_both: false,
                supports_manual: false,
                fingerprint,
                base_present: b.is_some(),
                current_present: c.is_some(),
                incoming_present: candidate.is_some(),
                automatic: !conflicted,
                position: None,
            };
            if let Some(position) = position {
                conflict.position = Some(position);
            }
            out.push(Group {
                scope,
                conflict,
                incoming: candidate,
            });
            Ok(())
        }
        for key in ["properties", "styles"] {
            push(
                &mut out,
                Scope::Root(key.into()),
                vec![key.into()],
                base.get(key),
                current.get(key),
                incoming.get(key),
                None,
            )?;
        }
        let opaque_keys = base["opaque"]
            .as_object()
            .unwrap()
            .keys()
            .chain(incoming["opaque"].as_object().unwrap().keys())
            .collect::<BTreeSet<_>>();
        for key in opaque_keys {
            push(
                &mut out,
                Scope::OpaqueMember(key.clone()),
                vec!["opaque".into(), key.clone()],
                base["opaque"].get(key),
                current["opaque"].get(key),
                incoming["opaque"].get(key),
                None,
            )?;
        }
        let bsecs = sections(base);
        let csecs = sections(current);
        let isecs = sections(incoming);
        let ids: BTreeSet<_> = bsecs
            .iter()
            .chain(isecs)
            .filter_map(named)
            .map(str::to_owned)
            .collect();
        for sid in ids {
            let b = find(bsecs, &sid);
            let c = find(csecs, &sid);
            let i = find(isecs, &sid);
            let si = csecs
                .iter()
                .position(|v| named(v) == Some(&sid))
                .or_else(|| isecs.iter().position(|v| named(v) == Some(&sid)))
                .unwrap_or(0);
            if b.is_none() || c.is_none() || i.is_none() {
                push(
                    &mut out,
                    Scope::Section(sid.clone()),
                    vec!["sections".into(), sid.clone()],
                    b,
                    c,
                    i,
                    Some(json!({"section":si,"paragraph":0})),
                )?;
                continue;
            }
            let (b, c, i) = (b.unwrap(), c.unwrap(), i.unwrap());
            push(
                &mut out,
                Scope::SectionProperties(sid.clone()),
                vec!["sections".into(), si.to_string(), "properties".into()],
                b.get("properties"),
                c.get("properties"),
                i.get("properties"),
                Some(json!({"section":si,"paragraph":0})),
            )?;
            let (bp, cp, ip) = (paragraphs(b), paragraphs(c), paragraphs(i));
            let ids: BTreeSet<_> = bp
                .iter()
                .chain(ip)
                .filter_map(named)
                .map(str::to_owned)
                .collect();
            for pid in ids {
                let position = cp
                    .iter()
                    .position(|v| named(v) == Some(&pid))
                    .or_else(|| ip.iter().position(|v| named(v) == Some(&pid)))
                    .unwrap_or(0);
                push(
                    &mut out,
                    Scope::Paragraph(sid.clone(), pid.clone()),
                    vec![
                        "sections".into(),
                        si.to_string(),
                        "paragraphs".into(),
                        pid.clone(),
                    ],
                    find(bp, &pid),
                    find(cp, &pid),
                    find(ip, &pid),
                    Some(json!({"section":si,"paragraph":position})),
                )?;
            }
            let order = |p: &[Value]| {
                Value::Array(p.iter().filter_map(named).map(|id| json!(id)).collect())
            };
            let (bo, co, io) = (order(bp), order(cp), order(ip));
            let base_order: BTreeSet<_> = bp.iter().filter_map(named).collect();
            let common = |p: &[Value]| {
                p.iter()
                    .filter_map(named)
                    .filter(|id| base_order.contains(id))
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            };
            if common(bp) != common(ip) {
                push(
                    &mut out,
                    Scope::Order(sid.clone()),
                    vec!["sections".into(), si.to_string(), "order".into()],
                    Some(&bo),
                    Some(&co),
                    Some(&io),
                    Some(json!({"section":si,"paragraph":0})),
                )?;
            }
        }
        Ok(out)
    }
    pub fn review(&self) -> Result<Analysis> {
        let groups = self.groups()?;
        Ok(Analysis {
            analysis_version: 3,
            result: self.summary(&self.views[1]),
            automatic_operation_count: groups.iter().filter(|g| g.conflict.automatic).count()
                as u32,
            conflicts: groups.into_iter().map(|g| g.conflict).collect(),
            visited_node_count: 0,
            budget_exceeded: false,
        })
    }
    pub fn analyze(&self) -> Analysis {
        merge_core::analyze(&self.views[0], &self.views[1], &self.views[2])
    }
    pub fn manifest(&self) -> Vec<Entry> {
        /// `hash` of the value with its `stableId` removed, without copying it.
        fn hash_without_identity(v: &Value) -> String {
            let Value::Object(m) = v else { return hash(v) };
            if !m.contains_key("stableId") {
                return hash(v);
            }
            let mut text = String::from("{");
            for (k, item) in m.iter().filter(|(k, _)| *k != "stableId") {
                if text.len() > 1 {
                    text.push(',');
                }
                text.push_str(&Value::String(k.clone()).to_string());
                text.push(':');
                text.push_str(&item.to_string());
            }
            text.push('}');
            format!("blake3:{}", blake3::hash(text.as_bytes()).to_hex())
        }
        fn entry(
            out: &mut Vec<Entry>,
            kind: &str,
            path: Vec<String>,
            v: &Value,
            hint: Option<String>,
        ) {
            out.push(Entry {
                kind: kind.into(),
                path,
                property_hash: hash_without_identity(v),
                identity_hint: hint,
                identity: None,
            });
        }
        let mut out = Vec::new();
        let view = &self.views[1];
        entry(&mut out, "document", vec![], view, None);
        for (si, s) in sections(view).iter().enumerate() {
            entry(
                &mut out,
                "section",
                vec!["sections".into(), si.to_string()],
                s,
                None,
            );
            for (pi, p) in paragraphs(s).iter().enumerate() {
                let path = vec![
                    "sections".into(),
                    si.to_string(),
                    "paragraphs".into(),
                    pi.to_string(),
                ];
                let original = &self.resources.sources[1].sections[si].paras[pi];
                entry(
                    &mut out,
                    "paragraph",
                    path.clone(),
                    p,
                    (original.header.instance_id != 0)
                        .then(|| format!("paragraph-instance:{}", original.header.instance_id)),
                );
                let mut raw = p.clone();
                if super::merge_text::restore(&mut raw).is_ok() {
                    for (ci, c) in raw["controls"].as_array().into_iter().flatten().enumerate() {
                        let mut path = path.clone();
                        path.extend(["controls".into(), ci.to_string()]);
                        entry(&mut out, "control", path, c, None);
                    }
                }
            }
        }
        for (id, style) in self.resources.sources[1].info.styles.iter().enumerate() {
            if let Ok(key) = self.resources.style_key(1, id as u32) {
                entry(
                    &mut out,
                    "style",
                    vec!["docInfo".into(), "styles".into(), id.to_string()],
                    &view["styles"][&key],
                    Some(format!(
                        "style-name:{}:{}",
                        style.local_name.to_string_lossy(),
                        style.english_name.to_string_lossy()
                    )),
                );
            }
        }
        out
    }
    pub fn summary(&self, view: &Value) -> Value {
        json!({"kind":"document","sections":sections(view).iter().map(|s|json!({"kind":"section","stableId":named(s),"paragraphs":paragraphs(s).iter().map(|p|json!({"kind":"paragraph","stableId":named(p),"text":plain(p),"controlCount":control_count(p)})).collect::<Vec<_>>()})).collect::<Vec<_>>()})
    }
    pub fn resolve_review(&self, resolutions: &Resolutions) -> Result<Value> {
        let groups = self.groups()?;
        if resolutions.keys().any(|id| {
            !groups.iter().any(|g| {
                &g.conflict.id == id
                    || &g.conflict.fingerprint == id
                    || g.conflict.position.as_ref().is_some_and(|p| {
                        id == &format!("review-pos:{}:{}", p["section"], p["paragraph"])
                    })
            })
        }) {
            return Err(bad(
                "resolution refers to an unknown or stale document change",
            ));
        }
        let mut root = self.views[1].clone();
        fn set_item(values: &mut Vec<Value>, id: &str, value: Option<Value>, incoming: &[Value]) {
            let at = values.iter().position(|v| named(v) == Some(id));
            match (at, value) {
                (Some(at), Some(value)) => values[at] = value,
                (Some(at), None) => {
                    values.remove(at);
                }
                (None, Some(value)) => {
                    let at = incoming
                        .iter()
                        .position(|v| named(v) == Some(id))
                        .and_then(|at| {
                            incoming[at + 1..]
                                .iter()
                                .filter_map(named)
                                .find_map(|next| values.iter().position(|v| named(v) == Some(next)))
                        })
                        .unwrap_or(values.len());
                    values.insert(at, value);
                }
                _ => {}
            }
        }
        for group in groups {
            let resolution = resolutions
                .get(&group.conflict.id)
                .or_else(|| resolutions.get(&group.conflict.fingerprint))
                .ok_or_else(|| bad(format!("unresolved document change {}", group.conflict.id)))?;
            let selected = match resolution {
                Resolution::Current => continue,
                Resolution::Incoming => group.incoming,
                _ => {
                    return Err(bad(
                        "this document review unit requires a current or incoming choice",
                    ));
                }
            };
            match group.scope {
                Scope::OpaqueMember(key) => {
                    if let Some(value) = selected {
                        root["opaque"][&key] = value;
                    } else {
                        root["opaque"].as_object_mut().unwrap().remove(&key);
                    }
                }
                Scope::Root(key) => {
                    root[&key] = selected.unwrap_or(Value::Null);
                }
                Scope::Section(id) => set_item(
                    root["sections"].as_array_mut().unwrap(),
                    &id,
                    selected,
                    sections(&self.views[2]),
                ),
                Scope::SectionProperties(sid) => {
                    let s = root["sections"]
                        .as_array_mut()
                        .unwrap()
                        .iter_mut()
                        .find(|v| named(v) == Some(&sid))
                        .ok_or_else(|| {
                            bad("selected section properties refer to a deleted section")
                        })?;
                    s["properties"] = selected.unwrap_or(Value::Null);
                }
                Scope::Paragraph(sid, pid) => {
                    let s = root["sections"]
                        .as_array_mut()
                        .unwrap()
                        .iter_mut()
                        .find(|v| named(v) == Some(&sid))
                        .ok_or_else(|| bad("selected paragraph refers to a deleted section"))?;
                    let incoming = find(sections(&self.views[2]), &sid)
                        .map(paragraphs)
                        .unwrap_or(&[]);
                    set_item(
                        s["paragraphs"].as_array_mut().unwrap(),
                        &pid,
                        selected,
                        incoming,
                    );
                }
                Scope::Order(sid) => {
                    let s = root["sections"]
                        .as_array_mut()
                        .unwrap()
                        .iter_mut()
                        .find(|v| named(v) == Some(&sid))
                        .ok_or_else(|| bad("paragraph order refers to a deleted section"))?;
                    let original = s["paragraphs"].as_array().unwrap().clone();
                    let ids = selected.unwrap_or(Value::Null);
                    let mut out = Vec::new();
                    for id in ids
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                    {
                        if let Some(v) = find(&original, id) {
                            out.push(v.clone());
                        }
                    }
                    for v in original {
                        if !out.iter().any(|p| named(p) == named(&v)) {
                            out.push(v);
                        }
                    }
                    s["paragraphs"] = Value::Array(out);
                }
            }
        }
        Ok(root)
    }
    fn validate_opaque(&self, selected: &Value, target: &Document) -> Result<()> {
        fn collect(v: &Value, out: &mut BTreeSet<String>, body: bool) {
            match v {
                Value::Object(m) => {
                    if m.contains_key("$resource") {
                        return;
                    }
                    for (k, v) in m {
                        if (matches!(k.as_str(), "Raw" | "extra" | "raw") || body && k == "tail")
                            && v.as_array().is_none_or(|a| !a.is_empty())
                            && !v.is_null()
                        {
                            out.insert(hash(v));
                        }
                        collect(v, out, body || k == "sections");
                    }
                }
                Value::Array(a) => {
                    for v in a {
                        collect(v, out, body);
                    }
                }
                _ => {}
            }
        }
        let mut chosen = BTreeSet::new();
        collect(selected, &mut chosen, false);
        let mut current = BTreeSet::new();
        collect(&self.views[1], &mut current, false);
        for fingerprint in chosen.difference(&current) {
            let mut found = false;
            for source in [0, 2] {
                let mut records = BTreeSet::new();
                collect(&self.views[source], &mut records, false);
                if records.contains(fingerprint) {
                    found = true;
                    if self.containers[source].view.get("context:namespaces")
                        != self.containers[1].view.get("context:namespaces")
                    {
                        return Err(bad(
                            "selected opaque fragments require different namespace bindings; retain their source context or keep the current fragment",
                        ));
                    }
                    if !super::merge_container::Container::same_layout(
                        &self.resources.sources[source],
                        target,
                    ) {
                        return Err(bad(
                            "selected opaque records refer to a different resource table; retain the current opaque record or resolve its resource dependencies in the source branch",
                        ));
                    }
                }
            }
            if !found {
                return Err(bad("opaque records cannot be synthesized by field merging"));
            }
        }
        Ok(())
    }
    pub fn model(&self, mut view: Value) -> Result<Document> {
        fn resources(v: &Value) -> Result<()> {
            match v {
                Value::Object(m) => {
                    if m.get("$resource").and_then(Value::as_str) == Some("bin")
                        && v["value"]["payloadHash"].is_null()
                    {
                        return Err(bad(
                            "a selected merge image or embedded object has no loaded bytes",
                        ));
                    }
                    for v in m.values() {
                        resources(v)?;
                    }
                }
                Value::Array(a) => {
                    for v in a {
                        resources(v)?;
                    }
                }
                _ => {}
            }
            Ok(())
        }
        resources(&view)?;
        let mut target = self.resources.sources[1].clone();
        super::merge_text::restore(&mut view)?;
        self.resources.materialize(&mut view, &mut target)?;
        let metadata = &view["properties"];
        target.info.properties =
            serde_json::from_value(metadata["properties"].clone()).map_err(bad)?;
        target.info.compatible =
            serde_json::from_value(metadata["compatible"].clone()).map_err(bad)?;
        target.info.layout_compat =
            serde_json::from_value(metadata["layoutCompatibility"].clone()).map_err(bad)?;
        target.info.raw = serde_json::from_value(metadata["raw"].clone()).map_err(bad)?;
        target.info.id_mappings.counts.resize(15, 0);
        target.info.id_mappings.counts.truncate(15);
        target.info.id_mappings.counts.extend(
            serde_json::from_value::<Vec<i32>>(metadata["mappingCounts"].clone()).map_err(bad)?,
        );
        target.info.id_mappings.tail =
            serde_json::from_value(metadata["mappingTail"].clone()).map_err(bad)?;
        let old_styles = self.resources.sources[1].info.styles.len();
        target.info.order.retain(|s| match s.item {
            InfoItem::Style(i) => (i as usize) < target.info.styles.len(),
            InfoItem::Raw(i) => (i as usize) < target.info.raw.len(),
            _ => true,
        });
        if target.info.styles.len() != old_styles {
            target.info.id_mappings.counts[IdMappings::STYLES] = target
                .info
                .order
                .iter()
                .filter(|s| matches!(s.item, InfoItem::Style(_)))
                .count() as i32;
        }
        for i in 0..target.info.raw.len() {
            if !target
                .info
                .order
                .iter()
                .any(|s| s.item == InfoItem::Raw(i as u32))
            {
                target.info.order.push(InfoSlot {
                    item: InfoItem::Raw(i as u32),
                    level: target.info.raw[i].level,
                });
            }
        }

        let mut secs = Vec::new();
        for section in sections(&view) {
            let mut value = section["properties"].clone();
            let mut ps = paragraphs(section).to_vec();
            for p in &mut ps {
                p.as_object_mut().unwrap().remove("stableId");
            }
            value["paras"] = Value::Array(ps);
            let mut s: Section = serde_json::from_value(value).map_err(bad)?;
            if let Some(current) = find(sections(&self.views[1]), named(section).unwrap()) {
                let old_index = sections(&self.views[1])
                    .iter()
                    .position(|s| named(s) == named(current))
                    .unwrap();
                let old = &self.resources.sources[1].sections[old_index];
                for (pi, p) in s.paras.iter_mut().enumerate() {
                    if let Some(id) = named(&paragraphs(section)[pi])
                        && let Some(old_pi) = paragraphs(current)
                            .iter()
                            .position(|p| named(p) == Some(id))
                    {
                        let old_p = &old.paras[old_pi];
                        p.header.instance_id = old_p.header.instance_id;
                        if property(p)? == property(old_p)? {
                            *p = old_p.clone();
                        }
                    }
                }
                if property(&s)? == property(old)? {
                    s = old.clone();
                }
            }
            if s.paras.is_empty() {
                return Err(bad("a merged section must retain a paragraph"));
            }
            secs.push(s);
        }
        if secs.is_empty() {
            return Err(bad("a merged document must retain a section"));
        }
        target.sections = secs;
        target.info.properties.section_count = u16::try_from(target.sections.len()).map_err(bad)?;
        target.info.id_mappings.src = None;
        super::merge_container::Container::apply(
            &self.containers,
            &view["opaque"],
            self.resources.sources,
            &mut target,
        )?;
        self.validate_opaque(&view, &target)?;
        validate(&target)?;
        Ok(target)
    }
}
pub fn write(doc: &Document) -> Result<Vec<u8>> {
    match doc.meta.format {
        Format::Hwp5 => hm_hwp5::write(doc).map_err(bad),
        Format::Hwpx => hm_hwpx::write(doc).map_err(bad),
        Format::Hml => hm_hml::write(doc).map_err(bad),
        Format::Hwp3 => {
            let mut doc = doc.clone();
            doc.convert_to(Format::Hwp5);
            hm_hwp5::write_new(&doc).map_err(bad)
        }
    }
}
fn validate(doc: &Document) -> Result<()> {
    fn list(doc: &Document, ps: &[Paragraph], depth: usize) -> Result<()> {
        if depth > 128 {
            return Err(bad("merged control nesting exceeds 128 lists"));
        }
        for p in ps {
            if p.text.last() != Some(&PARA_END) {
                return Err(bad("merged paragraph has no end marker"));
            }
            if p.runs.iter().any(|r| {
                r.pos >= p.text.len() as u32 || doc.info.char_shapes.get(r.shape as usize).is_none()
            }) {
                return Err(bad(
                    "merged paragraph has an invalid character format reference",
                ));
            }
            if !doc.info.para_shapes.is_empty()
                && doc
                    .info
                    .para_shapes
                    .get(p.header.para_shape as usize)
                    .is_none()
            {
                return Err(bad(
                    "merged paragraph has an invalid paragraph format reference",
                ));
            }
            let count = p
                .units()
                .filter(|(_, u)| matches!(u, Unit::Extended { .. }))
                .count();
            if count != p.controls.len() {
                return Err(bad(
                    "merged paragraph control anchors do not match its controls",
                ));
            }
            for c in &p.controls {
                for ps in hm_edit::ops::control_lists(c) {
                    list(doc, ps, depth + 1)?;
                }
            }
        }
        Ok(())
    }
    for s in &doc.sections {
        list(doc, &s.paras, 0)?;
    }
    Ok(())
}

/// Properties that did not survive the native writer. Resources are compared
/// by their values and bytes; source spans and generated layout are absent.
pub fn roundtrip_losses(expected: &Document, bytes: &[u8]) -> Result<Vec<String>> {
    let actual = open(bytes)?;
    let docs = [expected.clone(), expected.clone(), actual];
    let p = Prepared::new(&docs, &Default::default())?;
    fn diff(a: &Value, b: &Value, path: &str, out: &mut Vec<String>) {
        if a == b || out.len() >= 32 {
            return;
        }
        match (a, b) {
            (Value::Object(a), Value::Object(b)) => {
                for key in a.keys().chain(b.keys()).collect::<BTreeSet<_>>() {
                    diff(
                        a.get(key).unwrap_or(&Value::Null),
                        b.get(key).unwrap_or(&Value::Null),
                        &format!("{path}/{key}"),
                        out,
                    );
                }
            }
            (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
                for (n, (a, b)) in a.iter().zip(b).enumerate() {
                    diff(a, b, &format!("{path}/{n}"), out);
                }
            }
            _ => out.push(path.into()),
        }
    }
    let mut views = p.views;
    fn strip(v: &mut Value) {
        match v {
            Value::Object(m) => {
                m.remove("stableId");
                for v in m.values_mut() {
                    strip(v);
                }
            }
            Value::Array(a) => {
                for v in a {
                    strip(v);
                }
            }
            _ => {}
        }
    }
    for view in &mut views {
        super::merge_text::restore(view)?;
        strip(view);
    }
    let mut out = Vec::new();
    for key in ["properties", "styles", "sections"] {
        diff(&views[1][key], &views[2][key], key, &mut out);
    }

    Ok(out)
}
