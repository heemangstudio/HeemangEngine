//! Unmodeled container members are explicit review units. Typed field merging
//! keeps the current container; selected foreign streams are copied as bytes.
use super::types::{EngineError, Result};
use hm_model::{Document, SharedBytes};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
fn bad(s: impl std::fmt::Display) -> EngineError {
    EngineError::new("MERGE_VALIDATION_FAILED", s)
}
#[derive(Default)]
pub struct Container {
    pub view: Value,
    payloads: BTreeMap<String, Vec<u8>>,
    contexts: BTreeSet<String>,
    context_parts: BTreeMap<String, (String, Vec<u8>)>,
    zip: bool,
}
fn descriptor(bytes: &[u8]) -> Value {
    json!({"contentHash":format!("blake3:{}",blake3::hash(bytes).to_hex()),"byteLength":bytes.len()})
}
impl Container {
    pub fn read(doc: &Document) -> Result<Self> {
        let Some(source) = &doc.source else {
            return Ok(Self {
                view: json!({}),
                ..Self::default()
            });
        };
        let mut out = Self {
            view: json!({}),
            ..Self::default()
        };
        if hm_hwpx::zip::is_zip(source) {
            out.zip = true;
            let a = hm_hwpx::zip::Archive::open(source.clone()).map_err(bad)?;
            let mut modeled = BTreeSet::<String>::new();
            // Manifest destinations, rather than conventional filenames, define
            // which parts the typed reader owns.
            for e in &a.entries {
                if e.name.ends_with(".hpf") {
                    let x = hm_hwpx::xml::Xml::parse(a.read_name(&e.name).map_err(bad)?.unwrap())
                        .map_err(bad)?;
                    let mut context = Vec::new();
                    for child in x.root().children() {
                        if child.local() == b"manifest" {
                            for item in child.children() {
                                let href = item.attr("href").unwrap_or_default().into_owned();
                                let media = item.attr("media-type").unwrap_or_default();
                                let id = item.attr("id").unwrap_or_default();
                                if id == "header"
                                    || id.starts_with("section")
                                    || media.starts_with("image/")
                                    || media.contains("ole")
                                {
                                    modeled.insert(href);
                                }
                            }
                        } else if child.local() != b"spine" {
                            context.extend_from_slice(&child.source());
                        }
                    }
                    let context_name = format!("context:manifest:{}", e.name);
                    out.context(context_name.clone(), context);
                    out.context_parts.insert(
                        context_name,
                        (
                            e.name.clone(),
                            a.read_name(&e.name).map_err(bad)?.unwrap().to_vec(),
                        ),
                    );
                    modeled.insert(e.name.clone());
                }
            }
            modeled.insert("Contents/header.xml".into());
            for e in &a.entries {
                if e.name.ends_with("header.xml") {
                    let x = hm_hwpx::xml::Xml::parse(a.read_name(&e.name).map_err(bad)?.unwrap())
                        .map_err(bad)?;
                    let mut extras = Vec::new();
                    // These child groups are modeled by DocInfo. All other
                    // header nodes remain owned by the native source context.
                    for child in x.root().children() {
                        if child.local() == b"refList" {
                            for group in child.children() {
                                if ![
                                    b"fontfaces".as_slice(),
                                    b"borderFills",
                                    b"charProperties",
                                    b"tabProperties",
                                    b"numberings",
                                    b"bullets",
                                    b"paraProperties",
                                    b"styles",
                                ]
                                .contains(&group.local())
                                {
                                    extras.extend_from_slice(&group.source());
                                }
                            }
                        } else if ![b"beginNum".as_slice(), b"compatibleDocument"]
                            .contains(&child.local())
                        {
                            extras.extend_from_slice(&child.source());
                        }
                    }
                    let context_name = format!("context:header:{}", e.name);
                    out.context(context_name.clone(), extras);
                    out.context_parts
                        .insert(context_name, (e.name.clone(), x.src.to_vec()));
                    modeled.insert(e.name.clone());
                }
                if modeled.contains(&e.name)
                    || e.is_dir()
                    || e.name.starts_with("Preview/")
                    || e.name.starts_with("BinData/")
                    || e.name == "mimetype"
                    || e.name == "version.xml"
                    || e.name == "META-INF/container.xml"
                {
                    continue;
                }
                // Section roots carry namespace bindings needed by opaque
                // control tails. They are compared separately below.
                let bytes = a.read_name(&e.name).map_err(bad)?.unwrap();
                out.member(e.name.clone(), bytes.to_vec());
            }
            let mut bindings = BTreeSet::new();
            for e in &a.entries {
                if modeled.contains(&e.name) && e.name.ends_with(".xml") {
                    let x = hm_hwpx::xml::Xml::parse(a.read_name(&e.name).map_err(bad)?.unwrap())
                        .map_err(bad)?;
                    let tag = String::from_utf8_lossy(x.root().start_tag());
                    for attr in tag.split_ascii_whitespace() {
                        if attr.starts_with("xmlns=") || attr.starts_with("xmlns:") {
                            bindings.insert(attr.trim_end_matches('>').to_string());
                        }
                    }
                }
            }
            let bindings = bindings
                .into_iter()
                .collect::<Vec<_>>()
                .join(" ")
                .into_bytes();
            out.context("context:namespaces".into(), bindings);
        } else if hm_hwp5::cfb::Cfb::is_cfb(source) {
            let c = hm_hwp5::cfb::Cfb::open(source).map_err(bad)?;
            for (path, index) in c.paths() {
                if c.entries[*index].kind != 2
                    || path == "FileHeader"
                    || path == "DocInfo"
                    || path.starts_with("BodyText/")
                    || path.starts_with("BinData/")
                    || path == "PrvText"
                    || path == "PrvImage"
                {
                    continue;
                }
                out.member(path.clone(), c.read_entry(*index));
            }
        } else {
            return Err(bad("merge requires a preserving HWP5 or HWPX container"));
        }
        Ok(out)
    }
    fn member(&mut self, name: String, bytes: Vec<u8>) {
        self.view[&name] = descriptor(&bytes);
        self.payloads.insert(name, bytes);
    }
    fn context(&mut self, name: String, bytes: Vec<u8>) {
        self.contexts.insert(name.clone());
        self.member(name, bytes);
    }
    pub fn apply(
        all: &[Self; 3],
        selected: &Value,
        sources: [&Document; 3],
        target: &mut Document,
    ) -> Result<()> {
        let selected = selected
            .as_object()
            .ok_or_else(|| bad("invalid container review"))?;
        let current = &all[1];
        let mut replacements = BTreeMap::<String, Option<Vec<u8>>>::new();
        for name in current
            .view
            .as_object()
            .unwrap()
            .keys()
            .chain(selected.keys())
            .collect::<BTreeSet<_>>()
        {
            let desired = selected.get(name);
            if desired == current.view.get(name) {
                continue;
            }
            if current.contexts.contains(name) || all.iter().any(|a| a.contexts.contains(name)) {
                let owner = all
                    .iter()
                    .find(|a| a.view.get(name) == desired)
                    .ok_or_else(|| bad("unknown selected source context"))?;
                if owner.zip != current.zip {
                    return Err(bad(
                        "opaque source context cannot be transferred between HWP and HWPX containers",
                    ));
                }
                if let Some((part, bytes)) = owner.context_parts.get(name) {
                    let bytes = if name.starts_with("context:manifest:") {
                        let a = hm_hwpx::zip::Archive::open(
                            target
                                .source
                                .clone()
                                .ok_or_else(|| bad("missing source package"))?,
                        )
                        .map_err(bad)?;
                        let old = a
                            .read_name(part)
                            .map_err(bad)?
                            .ok_or_else(|| bad("missing current manifest"))?;
                        manifest_context(&old, bytes)?
                    } else {
                        bytes.clone()
                    };
                    replacements.insert(part.clone(), Some(bytes));
                    continue;
                }
                return Err(bad(format!(
                    "opaque namespace context {name} differs; retain the current context or resolve namespace bindings in the source branch"
                )));
            }
            if let Some(value) = desired {
                let owner = all
                    .iter()
                    .find(|a| a.view.get(name) == Some(value))
                    .ok_or_else(|| bad("unrecognized opaque container payload"))?;
                let source = all.iter().position(|a| std::ptr::eq(a, owner)).unwrap();
                if source != 1 && !Self::same_layout(sources[source], target) {
                    return Err(bad(format!(
                        "opaque member {name} has incompatible resource dependencies; retain the current member or resolve those dependencies in the source branch"
                    )));
                }
                if owner.zip != current.zip {
                    return Err(bad(
                        "opaque streams cannot be transferred between HWP and HWPX containers",
                    ));
                }
                replacements.insert(
                    name.clone(),
                    Some(
                        owner
                            .payloads
                            .get(name)
                            .ok_or_else(|| bad("missing opaque member bytes"))?
                            .clone(),
                    ),
                );
            } else {
                replacements.insert(name.clone(), None);
            }
        }
        if replacements.is_empty() {
            return Ok(());
        }
        let source = target
            .source
            .as_ref()
            .ok_or_else(|| bad("missing current native container"))?;
        let bytes = if current.zip {
            use hm_hwpx::zip::{Archive, Member};
            let a = Archive::open(source.clone()).map_err(bad)?;
            let mut members = Vec::new();
            let mut seen = BTreeSet::new();
            for (n, e) in a.entries.iter().enumerate() {
                seen.insert(e.name.clone());
                match replacements.get(&e.name) {
                    Some(Some(bytes)) => members.push(Member::New {
                        name: &e.name,
                        data: bytes,
                        stored: false,
                    }),
                    Some(None) => {}
                    None => members.push(Member::Copy(n)),
                }
            }
            for (name, data) in &replacements {
                if !seen.contains(name)
                    && let Some(data) = data
                {
                    members.push(Member::New {
                        name,
                        data,
                        stored: false,
                    });
                }
            }
            hm_hwpx::zip::write(Some(&a), &members)
        } else {
            let c = hm_hwp5::cfb::Cfb::open(source).map_err(bad)?;
            let mut files = BTreeMap::new();
            for (path, n) in c.paths() {
                if c.entries[*n].kind == 2 {
                    files.insert(path.clone(), c.read_entry(*n));
                }
            }
            for (name, data) in replacements {
                if let Some(data) = data {
                    files.insert(name, data);
                } else {
                    files.remove(&name);
                }
            }
            hm_hwp5::cfb::build_preserving(&files.into_iter().collect::<Vec<_>>(), &c)
        };
        target.source = Some(SharedBytes::from(bytes));
        // ZIP source tokens name archive indexes, which can shift on deletion.
        // Retain payloads and source context, invalidate every native span.
        let source = target.source.clone();
        let bins = target.bins.clone();
        let mut clean: Document =
            serde_json::from_value(serde_json::to_value(&*target).map_err(bad)?).map_err(bad)?;
        clean.source = source;
        clean.bins = bins;
        *target = clean;
        Ok(())
    }
    pub fn same_layout(a: &Document, b: &Document) -> bool {
        // Appended definitions preserve the source's numeric references.
        let av = serde_json::to_value(&a.info).unwrap();
        let bv = serde_json::to_value(&b.info).unwrap();
        // Binary entries compare without the package addressing writers assign.
        let bins = |d: &Document| {
            d.info
                .bin_data
                .iter()
                .map(|e| super::merge_resources::bin_entry(d, e).ok())
                .collect::<Vec<_>>()
        };
        if !bins(b).starts_with(&bins(a)) {
            return false;
        }
        for name in [
            "faces",
            "border_fills",
            "char_shapes",
            "tab_defs",
            "numberings",
            "bullets",
            "para_shapes",
            "styles",
        ] {
            if name == "faces" {
                for slot in 0..7 {
                    let x = av[name][slot].as_array().unwrap();
                    let y = bv[name][slot].as_array().unwrap();
                    if !y.starts_with(x) {
                        return false;
                    }
                }
            } else {
                let x = av[name].as_array().unwrap();
                let y = bv[name].as_array().unwrap();
                if !y.starts_with(x) {
                    return false;
                }
            }
        }
        for id in 1..=a.info.bin_data.len() {
            let id = id as u16;
            if a.bins.as_ref().and_then(|b| b.get(id)) != b.bins.as_ref().and_then(|b| b.get(id)) {
                return false;
            }
        }
        true
    }
}

fn manifest_context(current: &[u8], selected: &[u8]) -> Result<Vec<u8>> {
    use hm_hwpx::xml::Xml;
    let c = Xml::parse(SharedBytes::from(current)).map_err(bad)?;
    let i = Xml::parse(SharedBytes::from(selected)).map_err(bad)?;
    let r = c.root();
    let mut out = c.src[..r.span().start].to_vec();
    out.extend_from_slice(r.start_tag());
    for child in i.root().children() {
        if child.local() != b"manifest" && child.local() != b"spine" {
            out.extend_from_slice(&child.source());
        }
    }
    for child in r.children() {
        if child.local() == b"manifest" || child.local() == b"spine" {
            out.extend_from_slice(&child.source());
        }
    }
    let tag = r.start_tag();
    let name = tag[1..]
        .split(|b| b.is_ascii_whitespace() || *b == b'>')
        .next()
        .unwrap_or(b"opf:package");
    out.extend_from_slice(b"</");
    out.extend_from_slice(name);
    out.push(b'>');
    Ok(out)
}
