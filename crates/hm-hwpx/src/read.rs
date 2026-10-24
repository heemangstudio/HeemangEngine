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

#[derive(Debug, Default, PartialEq, Eq)]
pub enum XmlVersion {
    #[default]
    Missing,
    Declared(String),
    Malformed,
}

impl XmlVersion {
    /// XML 1.31 introduced literal unit-bearing paragraph values. Missing
    /// declarations use current units; malformed declarations stay packed.
    pub(crate) fn packed_paragraph_values(&self) -> bool {
        match self {
            Self::Missing => false,
            Self::Malformed => true,
            Self::Declared(v) => v
                .parse::<f64>()
                .map_or(true, |v| !v.is_finite() || v < 1.31),
        }
    }
}

/// State shared while mapping one package.
#[derive(Default)]
pub struct Ctx {
    /// The package's literal `version.xml@xmlVersion`, independent of its
    /// binary record version. Absent attributes stay absent.
    pub xml_version: XmlVersion,
    pub diags: Vec<Diagnostic>,
    /// Markup the mapping met but does not interpret, with counts.
    pub unknown: BTreeMap<String, u32>,
    /// Manifest item id to 1-based BinData id.
    pub bins: HashMap<String, u32>,
    pub memo_shapes: u32,
    /// Whether to collect [`Ctx::unknown`] (reports only).
    pub report: bool,
    /// Master page parts by manifest id: archive member and content.
    pub master_parts: HashMap<String, (u32, SharedBytes)>,
    /// The last-page master page of the section being read.
    pub last_page_master: Option<hm_model::ParaList>,
}

impl Ctx {
    pub fn diag(&mut self, message: String) {
        self.diags.push(Diagnostic { message });
    }

    pub fn unknown(&mut self, what: String) {
        if !self.report {
            return;
        }
        *self.unknown.entry(what).or_default() += 1;
    }

    /// Counts the pieces an element left unmapped.
    pub fn note_rest(&mut self, el: El<'_>, r: &Rest) {
        if !self.report || r.is_empty() {
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
    pub media_type: String,
    /// `isEmbeded`: absent for package parts, 0 for linked pictures.
    pub embedded: Option<bool>,
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
                    media_type: it.attr("media-type").unwrap_or_default().into_owned(),
                    embedded: it.flag("isEmbeded"),
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

impl Package {
    /// Hancom opens only the sections the header counts (`secCnt`); spine
    /// sections past that count stay in the package unread.
    pub(crate) fn limit_sections(&mut self, head: El<'_>) {
        if let Some(n) = head.int("secCnt").filter(|&n| n > 0)
            && (n as usize) < self.sections.len()
        {
            self.sections.truncate(n as usize);
        }
    }
}

fn item_rest(span: &SharedBytes) -> SharedBytes {
    let Ok(x) = Xml::parse(span.clone()) else {
        return SharedBytes::empty();
    };
    let mut r = Rest::new();
    r.attrs("", x.root(), &[]);
    r.into_tail()
}

/// Binary items: embedded parts under `BinData/` and pictures linked by path.
fn is_bin(i: &Item) -> bool {
    i.href.starts_with("BinData/") || i.embedded.is_some() || i.media_type.starts_with("image/")
}

/// An archive member by name, with its index for [`Xml::parse_part`].
fn part(a: &Archive, name: &str) -> Result<Option<(u32, SharedBytes)>, Error> {
    a.find(name)
        .map(|i| a.read(i).map(|b| (i as u32, b)))
        .transpose()
}

pub(crate) fn version(a: &Archive) -> (Version, XmlVersion) {
    let v = match a.read_name("version.xml") {
        Ok(Some(v)) => v,
        Ok(None) => return (Version(0x0501_0000), XmlVersion::Missing),
        Err(_) => return (Version(0x0501_0000), XmlVersion::Malformed),
    };
    let Ok(x) = Xml::parse(v) else {
        return (Version(0x0501_0000), XmlVersion::Malformed);
    };
    let r = x.root();
    let p = |k| r.int(k).unwrap_or(0).clamp(0, 255) as u32;
    (
        Version(p("major") << 24 | p("minor") << 16 | p("micro") << 8 | p("buildNumber")),
        r.attr("xmlVersion").map_or(XmlVersion::Missing, |v| {
            XmlVersion::Declared(v.into_owned())
        }),
    )
}

/// Reads a whole document, returning the unmapped markup it met.
pub fn document(
    bytes: SharedBytes,
    report: bool,
) -> Result<(Document, BTreeMap<String, u32>), Error> {
    let mut pkg = package(bytes.clone())?;
    let (version, xml_version) = version(&pkg.archive);
    let mut ctx = Ctx {
        xml_version,
        report,
        ..Ctx::default()
    };
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
        let linked = it.embedded == Some(false) && !it.href.starts_with("BinData/");
        info.bin_data.push(BinDataEntry {
            attr: if linked {
                0
            } else if ole {
                2
            } else {
                1
            },
            link_absolute: linked.then(|| HwpString::from_text(&it.href)),
            link_relative: linked.then(HwpString::default),
            storage_id: (!linked).then(|| number.unwrap_or(info.bin_data.len() as u16 + 1)),
            extension: (!linked).then(|| HwpString::from_text(&ext)),
            // The manifest item rides along so references and the manifest
            // can be written back by name.
            tail: item_rest(&it.span),
            src: Some(it.span.clone()),
        });
        let n = info.bin_data.len() as u32;
        ctx.bins.insert(it.id.clone(), n);
        info.order.push(InfoSlot {
            item: InfoItem::BinData(n - 1),
            level: 1,
        });
    }
    let (hi, header) = part(&pkg.archive, &pkg.header)?
        .ok_or_else(|| Error::Corrupt("missing header.xml".into()))?;
    let bins = std::mem::take(&mut info.order);
    let head_xml = Xml::parse_part(header, hi)?;
    head::parse(&head_xml, &mut info, &mut ctx);
    pkg.limit_sections(head_xml.root());
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
    for it in pkg.items.iter().filter(|i| i.href.contains("masterpage")) {
        if let Ok(Some(b)) = part(&pkg.archive, &it.href) {
            ctx.master_parts.insert(it.id.clone(), b);
        }
    }
    let mut sections = Vec::with_capacity(pkg.sections.len());
    for (k, name) in pkg.sections.iter().enumerate() {
        let (i, data) =
            part(&pkg.archive, name)?.ok_or_else(|| Error::Corrupt(format!("missing {name}")))?;
        let x = Xml::parse_part(data, i)?;
        let paras = body::parse_section(&x, &mut ctx);
        sections.push(Section {
            paras,
            src: Some(x.root().origin()),
            source_index: Some(k as u32),
            last_page_master: ctx.last_page_master.take(),
        });
    }
    ctx.master_parts.clear();
    if info.properties.section_count as usize != sections.len() {
        info.properties.section_count = sections.len() as u16;
    }
    let archive = pkg.archive.clone();
    let bins = hm_hwp5::with_ole_previews(hm_model::BinStore::new(move |id| {
        let name = bin_names.get((id as usize).checked_sub(1)?)?;
        archive.read_name(name).ok().flatten()
    }));
    let doc = Document {
        meta: DocMeta {
            format: Format::Hwpx,
            version,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zip::Member;

    #[test]
    fn package_xml_version_keeps_independent_binary_fields() {
        for value in [Some("1.2"), Some("1.4"), Some("1.5"), Some("broken"), None] {
            let attr = value.map_or(String::new(), |v| format!(" xmlVersion=\"{v}\""));
            let xml =
                format!("<version major=\"5\" minor=\"1\" micro=\"bad\" buildNumber=\"7\"{attr}/>");
            let bytes = crate::zip::write(
                None,
                &[Member::New {
                    name: "version.xml",
                    data: xml.as_bytes(),
                    stored: true,
                }],
            );
            let (binary, context) = version(&Archive::open(bytes.into()).unwrap());
            assert_eq!(binary, Version(0x0501_0007));
            assert_eq!(
                context,
                value.map_or(XmlVersion::Missing, |v| XmlVersion::Declared(v.into()))
            );
        }
        let bytes = crate::zip::write(None, &[]);
        assert_eq!(
            version(&Archive::open(bytes.into()).unwrap()),
            (Version(0x0501_0000), XmlVersion::Missing)
        );
        let bytes = crate::zip::write(
            None,
            &[Member::New {
                name: "version.xml",
                data: b"<version xmlVersion=\"1.5\"",
                stored: true,
            }],
        );
        assert_eq!(
            version(&Archive::open(bytes.into()).unwrap()),
            (Version(0x0501_0000), XmlVersion::Malformed)
        );
    }

    #[test]
    fn spine_sections_past_the_header_count_stay_unread_and_saved() {
        let head = |count: &str| {
            format!(
                r#"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head" version="1.2"{count}><hh:refList/></hh:head>"#
            )
        };
        let section = |text: &str| {
            format!(
                r#"<hs:sec xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section" xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph"><hp:p id="0" paraPrIDRef="0" styleIDRef="0"><hp:run charPrIDRef="0"><hp:t>{text}</hp:t></hp:run></hp:p></hs:sec>"#
            )
        };
        let package = |count: &str| {
            let files = [
                ("mimetype", "application/hwp+zip".to_string()),
                ("version.xml", r#"<version major="5" minor="1"/>"#.to_string()),
                ("Contents/header.xml", head(count)),
                (
                    "Contents/content.hpf",
                    r#"<opf:package xmlns:opf="http://www.idpf.org/2007/opf/"><opf:manifest><opf:item id="header" href="Contents/header.xml"/><opf:item id="section0" href="Contents/section0.xml"/><opf:item id="section1" href="Contents/section1.xml"/></opf:manifest><opf:spine><opf:itemref idref="section0"/><opf:itemref idref="section1"/></opf:spine></opf:package>"#.to_string(),
                ),
                ("Contents/section0.xml", section("first")),
                ("Contents/section1.xml", section("second")),
            ];
            crate::zip::write(
                None,
                &files
                    .iter()
                    .map(|(name, data)| Member::New {
                        name,
                        data: data.as_bytes(),
                        stored: *name == "mimetype",
                    })
                    .collect::<Vec<_>>(),
            )
        };
        let text = |doc: &hm_model::Document| -> Vec<String> {
            doc.sections
                .iter()
                .map(|s| String::from_utf16_lossy(&s.paras[0].text))
                .collect()
        };
        for (count, expected) in [
            (r#" secCnt="1""#, vec!["first"]),
            (r#" secCnt="2""#, vec!["first", "second"]),
            (r#" secCnt="3""#, vec!["first", "second"]),
            (r#" secCnt="0""#, vec!["first", "second"]),
            ("", vec!["first", "second"]),
        ] {
            let mut doc = crate::read(package(count).into()).unwrap();
            let got = text(&doc);
            assert_eq!(got.len(), expected.len(), "{count}");
            assert!(got.iter().zip(&expected).all(|(g, e)| g.starts_with(e)));
            // An edited document saves; the unread section rides along.
            doc.sections[0].paras[0].src = None;
            let saved = crate::write(&doc).unwrap();
            let archive = Archive::open(saved.clone().into()).unwrap();
            assert!(
                archive
                    .read_name("Contents/section1.xml")
                    .unwrap()
                    .is_some()
            );
            assert_eq!(
                text(&crate::read(saved.into()).unwrap()).len(),
                expected.len()
            );
        }
    }
}
