//! Reading a package: container, manifest, header, sections.

use std::collections::{BTreeMap, HashMap};

use hm_model::{
    BinDataEntry, Diagnostic, DocInfo, DocMeta, Document, Format, HwpString, InfoItem, InfoSlot,
    Section, SharedBytes, Version,
};

use crate::rest::{self, Rest};
use crate::xml::{El, Ns, Xml};
use crate::zip::Archive;
use crate::{Error, body, head};

/// State shared while mapping one package.
#[derive(Default)]
pub struct Ctx {
    pub diags: Vec<Diagnostic>,
    /// Markup the mapping met but does not interpret, with counts.
    pub unknown: BTreeMap<String, u32>,
    /// Manifest item id to 1-based BinData id.
    pub bins: HashMap<String, u32>,
    pub memo_shapes: u32,
}

impl Ctx {
    pub fn diag(&mut self, message: String) {
        self.diags.push(Diagnostic { message });
    }

    pub fn unknown(&mut self, what: String) {
        *self.unknown.entry(what).or_default() += 1;
    }

    /// Counts the pieces an element left unmapped.
    pub fn note_rest(&mut self, el: El<'_>, r: &Rest) {
        if r.is_empty() {
            return;
        }
        let t = head::tag(el);
        for p in rest::pieces(r.peek()) {
            let what = if p.kind_attr {
                let name = p.bytes.split(|&c| c == b'=').next().unwrap_or_default();
                format!("{t}/{}@{}", p.path, String::from_utf8_lossy(name))
            } else {
                let end = p
                    .bytes
                    .iter()
                    .position(|&c| c == b' ' || c == b'>' || c == b'/')
                    .unwrap_or(p.bytes.len());
                format!(
                    "{t}/{}<{}",
                    p.path,
                    String::from_utf8_lossy(&p.bytes[1..end])
                )
            };
            self.unknown(what);
        }
    }

    pub fn bin_index(&self, id: &str) -> Option<u32> {
        self.bins.get(id).copied()
    }
}

/// A manifest entry of `content.hpf`.
#[derive(Clone, Debug)]
pub struct Item {
    pub id: String,
    pub href: String,
    pub span: SharedBytes,
}

/// The package parts the mapping needs.
pub struct Package {
    pub archive: Archive,
    pub items: Vec<Item>,
    /// Section part names in reading order.
    pub sections: Vec<String>,
    pub header: String,
}

fn rootfile(a: &Archive) -> Result<String, Error> {
    if let Some(c) = a.read_name("META-INF/container.xml")? {
        let x = Xml::parse(c)?;
        if let Some(files) = x.root().child(Ns::Ocf, "rootfiles") {
            for f in files.children() {
                let mt = f.attr("media-type").unwrap_or_default();
                if let Some(p) = f.attr("full-path")
                    && (mt.contains("hwpml-package") || p.ends_with(".hpf"))
                {
                    return Ok(p.into_owned());
                }
            }
        }
    }
    Ok("Contents/content.hpf".into())
}

pub fn package(bytes: SharedBytes) -> Result<Package, Error> {
    let archive = Archive::open(bytes)?;
    let content_path = rootfile(&archive)?;
    let content = archive
        .read_name(&content_path)?
        .ok_or_else(|| Error::Corrupt(format!("missing {content_path}")))?;
    let x = Xml::parse(content)?;
    let root = x.root();
    // Manifest hrefs are relative to the package root in Hancom files.
    let mut items = Vec::new();
    let mut spine = Vec::new();
    for part in root.children() {
        if part.is(Ns::Opf, "manifest") {
            for it in part.children().filter(|c| c.is(Ns::Opf, "item")) {
                items.push(Item {
                    id: it.attr("id").unwrap_or_default().into_owned(),
                    href: it.attr("href").unwrap_or_default().into_owned(),
                    span: it.source(),
                });
            }
        } else if part.is(Ns::Opf, "spine") {
            for r in part.children().filter(|c| c.is(Ns::Opf, "itemref")) {
                spine.push(r.attr("idref").unwrap_or_default().into_owned());
            }
        }
    }
    let is_section = |h: &str| {
        let name = h.rsplit('/').next().unwrap_or(h);
        name.starts_with("section") && name.ends_with(".xml")
    };
    let mut sections: Vec<String> = spine
        .iter()
        .filter_map(|id| items.iter().find(|i| &i.id == id))
        .map(|i| i.href.clone())
        .filter(|h| is_section(h))
        .collect();
    if sections.is_empty() {
        sections = items
            .iter()
            .map(|i| i.href.clone())
            .filter(|h| is_section(h))
            .collect();
    }
    if sections.is_empty() {
        let mut found: Vec<(u32, String)> = archive
            .entries
            .iter()
            .filter_map(|e| {
                let n = e
                    .name
                    .strip_prefix("Contents/section")?
                    .strip_suffix(".xml")?;
                Some((n.parse().ok()?, e.name.clone()))
            })
            .collect();
        found.sort();
        sections = found.into_iter().map(|(_, n)| n).collect();
    }
    let header = items
        .iter()
        .find(|i| i.id == "header" || i.href.ends_with("header.xml"))
        .map(|i| i.href.clone())
        .unwrap_or_else(|| "Contents/header.xml".into());
    Ok(Package {
        archive,
        items,
        sections,
        header,
    })
}

fn item_rest(span: &SharedBytes) -> SharedBytes {
    let Ok(x) = Xml::parse(span.clone()) else {
        return SharedBytes::empty();
    };
    let mut r = Rest::new();
    r.attrs("", x.root(), &[]);
    r.into_tail()
}

fn is_bin(i: &Item) -> bool {
    i.href.starts_with("BinData/")
}

fn version(a: &Archive) -> Version {
    let Ok(Some(v)) = a.read_name("version.xml") else {
        return Version(0x0501_0000);
    };
    let Ok(x) = Xml::parse(v) else {
        return Version(0x0501_0000);
    };
    let r = x.root();
    let p = |k| r.int(k).unwrap_or(0).clamp(0, 255) as u32;
    Version(p("major") << 24 | p("minor") << 16 | p("micro") << 8 | p("buildNumber"))
}

/// Reads a whole document, returning the unmapped markup it met.
pub fn document(bytes: SharedBytes) -> Result<(Document, BTreeMap<String, u32>), Error> {
    let pkg = package(bytes.clone())?;
    let mut ctx = Ctx::default();
    let mut info = DocInfo::default();
    // BinData parts by 1-based id, read from the archive on first use.
    let bin_names: Vec<String> = pkg
        .items
        .iter()
        .filter(|i| is_bin(i))
        .map(|i| i.href.clone())
        .collect();
    for it in pkg.items.iter().filter(|i| is_bin(i)) {
        let ext = it.href.rsplit('.').next().unwrap_or("").to_string();
        let ole = ext.eq_ignore_ascii_case("ole");
        let number = it
            .id
            .trim_start_matches(|c: char| !c.is_ascii_digit())
            .parse::<u16>()
            .ok();
        info.bin_data.push(BinDataEntry {
            attr: if ole { 2 } else { 1 },
            storage_id: Some(number.unwrap_or(info.bin_data.len() as u16 + 1)),
            extension: Some(HwpString::from_text(&ext)),
            // The manifest item rides along so references and the manifest
            // can be written back by name.
            tail: item_rest(&it.span),
            src: Some(it.span.clone()),
            ..Default::default()
        });
        let n = info.bin_data.len() as u32;
        ctx.bins.insert(it.id.clone(), n);
        info.order.push(InfoSlot {
            item: InfoItem::BinData(n - 1),
            level: 1,
        });
    }
    let header = pkg
        .archive
        .read_name(&pkg.header)?
        .ok_or_else(|| Error::Corrupt("missing header.xml".into()))?;
    let hx = Xml::parse(header)?;
    let bins = std::mem::take(&mut info.order);
    head::parse(&hx, &mut info, &mut ctx);
    // BinData entries follow the id mappings in HWP 5 order.
    let at = info.order.len().min(2);
    info.order.splice(at..at, bins);
    if let Ok(Some(s)) = pkg.archive.read_name("settings.xml")
        && let Ok(sx) = Xml::parse(s)
        && let Some(c) = sx.root().child(Ns::Ha, "CaretPosition")
    {
        info.properties.caret_list = c.u32("listIDRef").unwrap_or(0);
        info.properties.caret_para = c.u32("paraIDRef").unwrap_or(0);
        info.properties.caret_pos = c.u32("pos").unwrap_or(0);
    }
    if info.properties.src.is_none() {
        info.properties.src = Some(SharedBytes::empty());
    }
    let mut sections = Vec::with_capacity(pkg.sections.len());
    for (k, name) in pkg.sections.iter().enumerate() {
        let data = pkg
            .archive
            .read_name(name)?
            .ok_or_else(|| Error::Corrupt(format!("missing {name}")))?;
        let x = Xml::parse(data.clone())?;
        let paras = body::parse_section(&x, &mut ctx);
        sections.push(Section {
            paras,
            src: Some(data),
            source_index: Some(k as u32),
        });
    }
    if info.properties.section_count as usize != sections.len() {
        info.properties.section_count = sections.len() as u16;
    }
    let archive = pkg.archive.clone();
    let bins = hm_model::BinStore::new(move |id| {
        let name = bin_names.get((id as usize).checked_sub(1)?)?;
        archive.read_name(name).ok().flatten()
    });
    let doc = Document {
        meta: DocMeta {
            format: Format::Hwpx,
            version: version(&pkg.archive),
            flags: 0,
        },
        info,
        sections,
        source: Some(bytes),
        bins: Some(bins),
        diagnostics: ctx.diags,
    };
    Ok((doc, ctx.unknown))
}
