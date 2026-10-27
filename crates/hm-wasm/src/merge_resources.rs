//! Model property views carry resource values rather than foreign numeric IDs.
//! Exact source resources use the clipboard importer; combined values create
//! one typed definition after their dependencies have been resolved.
use super::types::{EngineError, Result};
use hm_model::*;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
#[derive(Clone)]
struct Owner {
    source: usize,
    kind: String,
    id: u32,
}
fn bad(message: impl std::fmt::Display) -> EngineError {
    EngineError::new("MERGE_VALIDATION_FAILED", message)
}
fn property<T: serde::Serialize>(v: &T) -> Result<Value> {
    serde_json::to_value(v).map_err(bad)
}
fn implicit(v: &Value) -> bool {
    v.get("$defaultTab").is_some()
        || v.get("$defaultFont").is_some()
        || v.as_array().is_some_and(|a| a.iter().any(implicit))
        || v.as_object().is_some_and(|o| o.values().any(implicit))
}
/// A binary entry without its package addressing (HWPX item id, path, media
/// type and embedding flag, and the storage id), which writers assign anew.
pub fn bin_entry(doc: &Document, entry: &BinDataEntry) -> Result<Value> {
    let mut normalized = entry.clone();
    if matches!(doc.meta.format, Format::Hwpx | Format::Hml) {
        for name in ["id", "href", "media-type", "isEmbeded"] {
            normalized.tail = hm_hwpx::rest::without_attr(&normalized.tail, "", name);
        }
    }
    let mut value = property(&normalized)?;
    value.as_object_mut().unwrap().remove("storage_id");
    Ok(value)
}
pub fn hash(v: &Value) -> String {
    format!("blake3:{}", blake3::hash(v.to_string().as_bytes()).to_hex())
}
pub struct Resources<'a> {
    pub sources: &'a [Document; 3],
    owners: HashMap<String, Owner>,
    cache: HashMap<(usize, String, u32), Value>,
    payloads: HashMap<String, SharedBytes>,
    pub style_keys: [HashMap<u32, String>; 3],
}
impl<'a> Resources<'a> {
    pub fn new(sources: &'a [Document; 3]) -> Self {
        Self {
            sources,
            owners: HashMap::new(),
            cache: HashMap::new(),
            payloads: HashMap::new(),
            style_keys: std::array::from_fn(|_| HashMap::new()),
        }
    }
    fn resource(
        &mut self,
        source: usize,
        kind: &str,
        id: u32,
        styles: &mut Vec<u32>,
    ) -> Result<Value> {
        if (matches!(kind, "bin" | "border" | "numbering" | "bullet") && id == 0) || id == u32::MAX
        {
            return Ok(json!(id));
        }
        if kind == "style"
            && let Some(at) = styles.iter().position(|v| *v == id)
        {
            return Ok(json!({"$styleCycle":styles.len()-at-1}));
        }
        if styles.len() > 256 {
            return Err(bad("style dependency graph exceeds 256 definitions"));
        }
        let key = (source, kind.into(), id);
        if styles.is_empty()
            && let Some(v) = self.cache.get(&key)
        {
            return Ok(v.clone());
        }
        let doc = &self.sources[source];
        if kind == "tab"
            && (id == 0 && doc.info.tab_defs.is_empty()
                || doc
                    .info
                    .tab_defs
                    .get(id as usize)
                    .is_some_and(|v| v.attr == 0 && v.tabs.is_empty() && v.tail.is_empty()))
        {
            return Ok(json!({"$defaultTab":true}));
        }
        if let Some(slot) = kind
            .strip_prefix("font:")
            .and_then(|s| s.parse::<usize>().ok())
        {
            let font = doc.info.faces.get(slot).and_then(|v| v.get(id as usize));
            if id == 0 && font.is_none()
                || font.is_some_and(|f| {
                    f.attr == 0
                        && f.name.0.is_empty()
                        && f.substitute.is_none()
                        && f.panose.is_none()
                        && f.default_name.is_none()
                        && f.tail.is_empty()
                })
            {
                return Ok(json!({"$defaultFont":slot}));
            }
        }
        let mut value = match kind {
            "char" => property(
                doc.info
                    .char_shapes
                    .get(id as usize)
                    .ok_or_else(|| bad("missing character shape"))?,
            )?,
            "para" => property(
                doc.info
                    .para_shapes
                    .get(id as usize)
                    .ok_or_else(|| bad("missing paragraph shape"))?,
            )?,
            "style" => property(
                doc.info
                    .styles
                    .get(id as usize)
                    .ok_or_else(|| bad("missing style"))?,
            )?,
            "border" => property(
                doc.info
                    .border_fills
                    .get(id as usize - 1)
                    .ok_or_else(|| bad("missing border fill"))?,
            )?,
            "tab" => property(
                doc.info
                    .tab_defs
                    .get(id as usize)
                    .ok_or_else(|| bad("missing tab definition"))?,
            )?,
            "numbering" => property(
                doc.info
                    .numberings
                    .get(id as usize - 1)
                    .ok_or_else(|| bad("missing numbering"))?,
            )?,
            "bullet" => property(
                doc.info
                    .bullets
                    .get(id as usize - 1)
                    .ok_or_else(|| bad("missing bullet"))?,
            )?,
            "bin" => {
                let entry = doc
                    .info
                    .bin_data
                    .get(id as usize - 1)
                    .ok_or_else(|| bad("missing binary resource"))?;
                let value = bin_entry(doc, entry)?;
                let bytes = doc.bins.as_ref().and_then(|b| b.get(id as u16));
                let payload = bytes
                    .as_ref()
                    .map(|b| format!("blake3:{}", blake3::hash(b).to_hex()));
                if let (Some(key), Some(bytes)) = (&payload, bytes) {
                    self.payloads.insert(key.clone(), bytes);
                }
                json!({"entry":value,"payloadHash":payload})
            }
            _ if kind.starts_with("font:") => {
                let slot = kind[5..].parse::<usize>().map_err(bad)?;
                property(
                    doc.info
                        .faces
                        .get(slot)
                        .and_then(|v| v.get(id as usize))
                        .ok_or_else(|| bad("missing font face"))?,
                )?
            }
            _ => return Err(bad(format!("unknown resource kind {kind}"))),
        };
        if kind == "style" {
            styles.push(id);
            let next = value["next"]
                .as_u64()
                .ok_or_else(|| bad("invalid next style"))? as u32;
            value["next"] = self.resource(source, "style", next, styles)?;
        }
        if kind == "char" {
            if let Some(faces) = value["faces"].as_array_mut() {
                for (slot, face) in faces.iter_mut().enumerate() {
                    let id = face.as_u64().ok_or_else(|| bad("invalid font reference"))?;
                    *face = self.resource(source, &format!("font:{slot}"), id as u32, styles)?;
                }
            }
        }
        self.expand_fields(source, &mut value, styles)?;
        if kind == "style" {
            styles.pop();
        }
        let wrapper = json!({"$resource":kind,"value":value});
        let fingerprint = hash(&wrapper);
        // Current source wins equivalent-resource ownership without re-importing it.
        // The merge target starts as the current document, so its own ids stay
        // valid even for resources that reference an implicit default tab or font.
        let owner = Owner {
            source,
            kind: kind.into(),
            id,
        };
        if (source == 1 || !implicit(&wrapper))
            && (kind != "style" || styles.is_empty())
            && self
                .owners
                .get(&fingerprint)
                .is_none_or(|old| old.source != 1 && source == 1)
        {
            self.owners.insert(fingerprint, owner);
        }
        if styles.is_empty() {
            self.cache.insert(key, wrapper.clone());
        }
        Ok(wrapper)
    }
    pub fn style_key(&self, source: usize, id: u32) -> Result<String> {
        if let Some(key) = self.style_keys[source].get(&id) {
            return Ok(key.clone());
        }
        let style = self.sources[source]
            .info
            .styles
            .get(id as usize)
            .ok_or_else(|| bad("missing named style"))?;
        Ok(format!(
            "style:{}",
            hash(&json!([
                style.local_name.0,
                style.english_name.0,
                style.attr & 3
            ]))
        ))
    }
    pub fn styles(&mut self, source: usize) -> Result<Value> {
        let mut out = serde_json::Map::new();
        for id in 0..self.sources[source].info.styles.len() {
            let style = &self.sources[source].info.styles[id];
            let mut properties = property(style)?;
            properties["next"] = json!({"$styleRef":self.style_key(source,style.next as u32)?});
            self.expand(source, &mut properties)?;
            let key = self.style_key(source, id as u32)?;
            if out.insert(key, properties).is_some() {
                return Err(bad("duplicate named style identity"));
            }
        }
        Ok(Value::Object(out))
    }
    pub fn expand(&mut self, source: usize, value: &mut Value) -> Result<()> {
        self.expand_fields(source, value, &mut Vec::new())
    }
    fn expand_fields(
        &mut self,
        source: usize,
        value: &mut Value,
        styles: &mut Vec<u32>,
    ) -> Result<()> {
        match value {
            Value::Array(v) => {
                for v in v {
                    self.expand_fields(source, v, styles)?;
                }
            }
            Value::Object(v) => {
                if v.contains_key("$resource") || v.contains_key("$styleCycle") {
                    return Ok(());
                }
                let run = v.contains_key("pos") && v.contains_key("shape") && v.len() == 2;
                let header = v.contains_key("break_kind") && v.contains_key("para_shape");
                let bullet = v
                    .get("attr1")
                    .and_then(Value::as_u64)
                    .is_some_and(|a| (a >> 23) & 3 == 3);
                for key in v.keys().cloned().collect::<Vec<_>>() {
                    let field = v.get_mut(&key).unwrap();
                    let kind = match key.as_str() {
                        "char_shape" => Some("char"),
                        "para_shape" => Some("para"),
                        "shape" if run => Some("char"),
                        "style" if header => Some("style"),
                        "border_fill" => Some("border"),
                        "tab_def" => Some("tab"),
                        "outline_numbering" => Some("numbering"),
                        "numbering" => Some(if bullet { "bullet" } else { "numbering" }),
                        "bin_id" => Some("bin"),
                        "image" if field.is_number() => Some("bin"),
                        _ => None,
                    };
                    if let (Some(kind), Some(id)) = (kind, field.as_u64()) {
                        if kind == "style" {
                            if !self.sources[source].info.styles.is_empty() {
                                *field = json!({"$styleRef":self.style_key(source,id as u32)?});
                            }
                        } else {
                            *field = self.resource(source, kind, id as u32, styles)?;
                        }
                    } else if key == "char_shapes"
                        && field
                            .as_array()
                            .is_some_and(|a| a.iter().all(Value::is_number))
                    {
                        for v in field.as_array_mut().unwrap() {
                            *v =
                                self.resource(source, "char", v.as_u64().unwrap() as u32, styles)?;
                        }
                    } else {
                        self.expand_fields(source, field, styles)?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub fn materialize(&self, value: &mut Value, target: &mut Document) -> Result<()> {
        fn needed(v: &Value, out: &mut BTreeMap<String, Value>) {
            if v.get("$resource").is_some() {
                out.insert(hash(v), v.clone());
            }
            if let Some(a) = v.as_array() {
                for v in a {
                    needed(v, out);
                }
            }
            if let Some(o) = v.as_object() {
                for v in o.values() {
                    needed(v, out);
                }
            }
        }
        for slot in 0..7 {
            if target.info.faces[slot].is_empty() {
                target.info.faces[slot].push(FaceName::default());
            }
        }
        if target.info.tab_defs.is_empty() {
            target.info.tab_defs.push(TabDef::default());
        }
        let mut refs = BTreeMap::new();
        needed(value, &mut refs);
        let mut ids = HashMap::new();
        for (source, doc) in self.sources.iter().enumerate() {
            let mut importer = crate::clip_import::Import::new(doc, target);
            for key in refs.keys() {
                let Some(owner) = self.owners.get(key).filter(|o| o.source == source) else {
                    continue;
                };
                let id = if source == 1 {
                    owner.id
                } else {
                    match owner.kind.as_str() {
                        "char" => importer.character(owner.id)?,
                        "para" => importer.paragraph_shape(owner.id as u16)? as u32,
                        "style" => importer.style(owner.id as u8)? as u32,
                        "border" => importer.border(owner.id as u16)? as u32,
                        "tab" => importer.tab(owner.id as u16)? as u32,
                        "numbering" => importer.numbering(owner.id as u16, false)? as u32,
                        "bullet" => importer.numbering(owner.id as u16, true)? as u32,
                        "bin" => importer.bin(owner.id as u16)? as u32,
                        k if k.starts_with("font:") => {
                            let slot = k[5..].parse::<usize>().map_err(bad)?;
                            importer.font(slot, owner.id as u16)? as u32
                        }
                        _ => return Err(bad("invalid resource owner")),
                    }
                };
                ids.insert(key.clone(), id);
            }
        }
        self.factor(value, target, &mut ids, &mut Vec::new())?;
        let style_values = value.get("styles").and_then(Value::as_object).cloned();
        let mut order = Vec::new();
        let mut named = HashMap::new();
        if let Some(styles) = &style_values {
            for id in 0..self.sources[1].info.styles.len() {
                let key = self.style_key(1, id as u32)?;
                if styles.contains_key(&key) && !order.contains(&key) {
                    order.push(key);
                }
            }
            for key in styles.keys() {
                if !order.contains(key) {
                    order.push(key.clone());
                }
            }
            for (index, key) in order.iter().enumerate() {
                named.insert(key.clone(), u8::try_from(index).map_err(bad)?);
            }
        } else {
            for id in 0..self.sources[1].info.styles.len() {
                named.insert(self.style_key(1, id as u32)?, id as u8);
            }
        }
        fn resolve(value: &mut Value, named: &HashMap<String, u8>) -> Result<()> {
            if let Some(key) = value.get("$styleRef").and_then(Value::as_str) {
                *value = json!(
                    named
                        .get(key)
                        .ok_or_else(|| bad("a selected paragraph references a removed style"))?
                );
                return Ok(());
            }
            if let Some(a) = value.as_array_mut() {
                for v in a {
                    resolve(v, named)?;
                }
            }
            if let Some(o) = value.as_object_mut() {
                for v in o.values_mut() {
                    resolve(v, named)?;
                }
            }
            Ok(())
        }
        resolve(value, &named)?;
        if style_values.is_some() {
            target.info.styles = order
                .iter()
                .map(|key| serde_json::from_value(value["styles"][key].clone()).map_err(bad))
                .collect::<Result<Vec<Style>>>()?;
        }
        Ok(())
    }
    fn factor(
        &self,
        value: &mut Value,
        target: &mut Document,
        ids: &mut HashMap<String, u32>,
        styles: &mut Vec<u32>,
    ) -> Result<()> {
        if value.get("$defaultTab").is_some() {
            let at = target
                .info
                .tab_defs
                .iter()
                .position(|v| v.attr == 0 && v.tabs.is_empty() && v.tail.is_empty());
            let id = if let Some(id) = at {
                id
            } else {
                let id = target.info.tab_defs.len();
                target.info.tab_defs.push(TabDef::default());
                id
            };
            *value = json!(u16::try_from(id).map_err(bad)?);
            return Ok(());
        }
        if let Some(slot) = value.get("$defaultFont").and_then(Value::as_u64) {
            let slot = usize::try_from(slot).map_err(bad)?;
            let faces = target
                .info
                .faces
                .get_mut(slot)
                .ok_or_else(|| bad("invalid default-font slot"))?;
            let at = faces.iter().position(|f| {
                f.attr == 0
                    && f.name.0.is_empty()
                    && f.substitute.is_none()
                    && f.panose.is_none()
                    && f.default_name.is_none()
                    && f.tail.is_empty()
            });
            let id = if let Some(id) = at {
                id
            } else {
                let id = faces.len();
                faces.push(FaceName::default());
                id
            };
            *value = json!(u16::try_from(id).map_err(bad)?);
            return Ok(());
        }
        if let Some(back) = value.get("$styleCycle").and_then(Value::as_u64) {
            let at = styles
                .len()
                .checked_sub(back as usize + 1)
                .ok_or_else(|| bad("invalid cyclic style reference"))?;
            *value = json!(styles[at]);
            return Ok(());
        }
        if let Some(kind) = value
            .get("$resource")
            .and_then(Value::as_str)
            .map(str::to_owned)
        {
            let key = hash(value);
            if let Some(id) = ids.get(&key) {
                *value = json!(id);
                return Ok(());
            }
            let mut properties = value["value"].clone();
            let reserved = if kind == "style" {
                let id = u8::try_from(target.info.styles.len()).map_err(bad)? as u32;
                target.info.styles.push(Style::default());
                styles.push(id);
                Some(id)
            } else {
                None
            };
            self.factor(&mut properties, target, ids, styles)?;
            macro_rules! append {
                ($ty:ty,$field:ident,$offset:expr) => {{
                    let id = u16::try_from(target.info.$field.len() + $offset).map_err(bad)? as u32;
                    let entry: $ty = serde_json::from_value(properties).map_err(bad)?;
                    target.info.$field.push(entry);
                    id
                }};
            }
            let id = match kind.as_str() {
                "char" => append!(CharShape, char_shapes, 0),
                "para" => append!(ParaShape, para_shapes, 0),
                "border" => append!(BorderFill, border_fills, 1),
                "tab" => append!(TabDef, tab_defs, 0),
                "numbering" => append!(Numbering, numberings, 1),
                "bullet" => append!(Bullet, bullets, 1),
                "style" => {
                    let id = reserved.unwrap();
                    target.info.styles[id as usize] =
                        serde_json::from_value(properties).map_err(bad)?;
                    styles.pop();
                    id
                }
                "bin" => {
                    let mut entry: BinDataEntry =
                        serde_json::from_value(properties["entry"].clone()).map_err(bad)?;
                    let payload = properties["payloadHash"]
                        .as_str()
                        .map(|key| {
                            self.payloads
                                .get(key)
                                .cloned()
                                .ok_or_else(|| bad("unknown merged binary payload"))
                        })
                        .transpose()?;
                    let id = u16::try_from(target.info.bin_data.len() + 1).map_err(bad)?;
                    let previous = target.bins.clone();
                    let ole = previous.clone();
                    target.bins = Some(
                        BinStore::new(move |key| {
                            if key == id {
                                payload.clone()
                            } else {
                                previous.as_ref().and_then(|b| b.get(key))
                            }
                        })
                        .with_ole(move |key| ole.as_ref().and_then(|b| b.ole(key))),
                    );
                    entry.storage_id = Some(
                        target
                            .info
                            .bin_data
                            .iter()
                            .filter_map(|e| e.storage_id)
                            .max()
                            .unwrap_or(0)
                            .checked_add(1)
                            .ok_or_else(|| bad("binary storage id overflow"))?,
                    );
                    entry.src = None;
                    target.info.bin_data.push(entry);
                    id as u32
                }
                k if k.starts_with("font:") => {
                    let slot = k[5..].parse::<usize>().map_err(bad)?;
                    let font: FaceName = serde_json::from_value(properties).map_err(bad)?;
                    let existing = target.info.faces[slot].iter().position(|other| {
                        other.attr == font.attr
                            && other.name == font.name
                            && other.substitute == font.substitute
                            && other.panose == font.panose
                            && other.default_name == font.default_name
                            && other.tail == font.tail
                    });
                    if let Some(id) = existing {
                        id as u32
                    } else {
                        let id = u16::try_from(target.info.faces[slot].len()).map_err(bad)?;
                        target.info.faces[slot].push(font);
                        id as u32
                    }
                }
                _ => return Err(bad("unsupported synthetic resource")),
            };
            ids.insert(key, id);
            *value = json!(id);
            return Ok(());
        }
        if let Some(a) = value.as_array_mut() {
            for v in a {
                self.factor(v, target, ids, styles)?;
            }
        }
        if let Some(o) = value.as_object_mut() {
            for v in o.values_mut() {
                self.factor(v, target, ids, styles)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source() -> Document {
        hm_hwpx::read(
            include_bytes!("../tests/fixtures/query-equations.hwpx")
                .as_slice()
                .into(),
        )
        .unwrap()
    }
    #[test]
    fn foreign_ids_compare_by_resource_values_and_disjoint_formats_combine() {
        let base = source();
        let mut current = base.clone();
        let mut incoming = base.clone();
        let original = base.sections[0].paras[0].runs[0].shape as usize;
        let mut bold = base.info.char_shapes[original].clone();
        bold.attr |= 2;
        bold.src = None;
        current.info.char_shapes.push(bold);
        current.sections[0].paras[0].runs[0].shape = current.info.char_shapes.len() as u32 - 1;
        let mut colored = base.info.char_shapes[original].clone();
        colored.color = 0x123456;
        colored.src = None;
        incoming.info.char_shapes.push(colored);
        incoming.sections[0].paras[0].runs[0].shape = incoming.info.char_shapes.len() as u32 - 1;
        let docs = [base, current, incoming];
        let mut codec = Resources::new(&docs);
        let mut views = Vec::new();
        for (s, doc) in docs.iter().enumerate() {
            let mut view = property(&doc.sections[0].paras[0]).unwrap();
            codec.expand(s, &mut view).unwrap();
            views.push(view);
        }
        let analysis = crate::merge_core::analyze(&views[0], &views[1], &views[2]);
        assert!(analysis.conflicts.is_empty(), "{:?}", analysis.conflicts);
        let mut result = analysis.result;
        let mut target = docs[1].clone();
        codec.materialize(&mut result, &mut target).unwrap();
        let paragraph: Paragraph = serde_json::from_value(result).unwrap();
        let look = &target.info.char_shapes[paragraph.runs[0].shape as usize];
        assert!(look.bold());
        assert_eq!(look.color, 0x123456);
    }
    #[test]
    fn duplicate_native_ids_and_font_metadata_preserve_their_semantics() {
        let base = source();
        let mut incoming = base.clone();
        let original = base.sections[0].paras[0].runs[0].shape as usize;
        incoming
            .info
            .char_shapes
            .push(incoming.info.char_shapes[original].clone());
        incoming.sections[0].paras[0].runs[0].shape = incoming.info.char_shapes.len() as u32 - 1;
        let docs = [base.clone(), base, incoming];
        let mut codec = Resources::new(&docs);
        let mut current = property(&docs[1].sections[0].paras[0]).unwrap();
        let mut incoming = property(&docs[2].sections[0].paras[0]).unwrap();
        codec.expand(1, &mut current).unwrap();
        codec.expand(2, &mut incoming).unwrap();
        assert_eq!(current, incoming);
        let mut original = docs[1].clone();
        original.info.faces[0].push(FaceName {
            name: HwpString::from_text("Same Font"),
            ..FaceName::default()
        });
        let mut changed = original.clone();
        changed.info.faces[0][0].panose = Some([42; 10]);
        let docs = [original.clone(), original, changed];
        let mut codec = Resources::new(&docs);
        let mut properties = property(&docs[2].sections[0].paras[0]).unwrap();
        codec.expand(2, &mut properties).unwrap();
        let mut target = docs[1].clone();
        codec.materialize(&mut properties, &mut target).unwrap();
        let paragraph: Paragraph = serde_json::from_value(properties).unwrap();
        let look = &target.info.char_shapes[paragraph.runs[0].shape as usize];
        assert_eq!(
            target.info.faces[0][look.faces[0] as usize].panose,
            Some([42; 10])
        );
    }
}
