//! HWPX checks: the package (container, manifest and spine against the
//! members), well-formed parts, header item counts and ids, every id
//! reference in the header and sections, tables, lists and fields.

use std::collections::{BTreeSet, HashMap};

use crate::Report;
use crate::xml::{self, Doc};
use crate::zip;

const PKG: &str = "package";
const HEAD: &str = "header";
const SEC: &str = "section";

/// Header lists and the element each counts.
const LISTS: [(&str, &str); 11] = [
    ("fontfaces", "fontface"),
    ("borderFills", "borderFill"),
    ("charProperties", "charPr"),
    ("tabProperties", "tabPr"),
    ("numberings", "numbering"),
    ("bullets", "bullet"),
    ("paraProperties", "paraPr"),
    ("styles", "style"),
    ("memoProperties", "memoPr"),
    ("trackChanges", "trackChange"),
    ("trackChangeAuthors", "trackChangeAuthor"),
];

const LANGS: [&str; 7] = [
    "hangul", "latin", "hanja", "japanese", "other", "symbol", "user",
];

/// Ids defined in the header, by item kind.
#[derive(Default)]
struct Ids {
    of: HashMap<&'static str, BTreeSet<String>>,
    fonts: HashMap<String, BTreeSet<String>>,
    bins: BTreeSet<String>,
}

impl Ids {
    fn has(&self, kind: &str, id: &str) -> bool {
        self.of.get(kind).is_some_and(|s| s.contains(id))
    }
}

pub fn check(bytes: &[u8], r: &mut Report) {
    let Some(members) = zip::read(bytes, r) else {
        return;
    };
    let get = |n: &str| members.iter().find(|m| m.name == n);
    let mut parsed: HashMap<String, Doc> = HashMap::new();
    for m in &members {
        let lower = m.name.to_ascii_lowercase();
        if lower.ends_with(".xml") || lower.ends_with(".hpf") || lower.ends_with(".rdf") {
            match xml::parse(&m.data) {
                Ok(d) => {
                    parsed.insert(m.name.clone(), d);
                }
                Err(e) => r.error("xml", format!("{}: {e}", m.name)),
            }
        }
    }
    if get("mimetype").is_none() {
        r.error(PKG, "no mimetype member");
    }
    for n in ["version.xml", "settings.xml"] {
        if get(n).is_none() {
            r.warn(PKG, format!("no {n}"));
        }
    }
    let Some(container) = parsed.get("META-INF/container.xml") else {
        if get("META-INF/container.xml").is_none() {
            r.error(PKG, "no META-INF/container.xml");
        }
        return;
    };
    let rootfile = container
        .els
        .iter()
        .find(|e| e.local() == "rootfile")
        .and_then(|e| e.attr("full-path"))
        .map(str::to_string);
    let Some(rootfile) = rootfile else {
        r.error(PKG, "container.xml names no rootfile");
        return;
    };
    let Some(hpf) = parsed.get(&rootfile) else {
        if get(&rootfile).is_none() {
            r.error(PKG, format!("rootfile {rootfile} is missing"));
        }
        return;
    };

    // Manifest and spine.
    let mut items: Vec<(String, String)> = Vec::new();
    let mut spine: Vec<String> = Vec::new();
    for e in &hpf.els {
        match e.local() {
            "item" => {
                let id = e.attr("id").unwrap_or_default().to_string();
                let href = e.attr("href").unwrap_or_default().to_string();
                if items.iter().any(|(i, _)| *i == id) {
                    r.error(PKG, format!("manifest repeats item id {id:?}"));
                }
                let linked = e.attr("isEmbeded") == Some("0");
                if !linked && get(&href).is_none() {
                    r.error(
                        PKG,
                        format!("manifest item {id} names missing member {href:?}"),
                    );
                }
                items.push((id, href));
            }
            "itemref" => spine.push(e.attr("idref").unwrap_or_default().to_string()),
            _ => {}
        }
    }
    let mut spine_hrefs = Vec::new();
    for id in &spine {
        match items.iter().find(|(i, _)| i == id) {
            Some((_, href)) => spine_hrefs.push(href.clone()),
            None => r.error(PKG, format!("spine names unknown item {id:?}")),
        }
    }
    let is_header = |h: &str| h.rsplit('/').next() == Some("header.xml");
    let Some(header) = spine_hrefs.iter().find(|h| is_header(h)).cloned() else {
        r.error(PKG, "spine has no header.xml");
        return;
    };
    let sections: Vec<String> = spine_hrefs
        .iter()
        .filter(|h| !is_header(h) && h.contains("section"))
        .cloned()
        .collect();
    if sections.is_empty() {
        r.error(PKG, "spine has no sections");
    }
    for m in &members {
        let in_manifest = items.iter().any(|(_, h)| *h == m.name);
        let lower = m.name.to_ascii_lowercase();
        if lower.starts_with("contents/section") && !spine_hrefs.contains(&m.name) {
            r.warn(PKG, format!("{} is not in the spine", m.name));
        }
        if lower.starts_with("bindata/") && !in_manifest {
            r.warn(PKG, format!("{} is not in the manifest", m.name));
        }
    }

    let mut ids = Ids::default();
    for (id, href) in &items {
        if href.starts_with("BinData/") || !href.ends_with(".xml") {
            ids.bins.insert(id.clone());
        }
    }
    let Some(head) = parsed.get(&header) else {
        return;
    };
    header_checks(head, sections.len(), &mut ids, r);
    for (k, s) in sections.iter().enumerate() {
        if let Some(d) = parsed.get(s) {
            refs(d, &ids, &format!("section {k}"), SEC, r);
            section_checks(d, k, r);
        }
    }
    let masters: Vec<&String> = items
        .iter()
        .map(|(_, h)| h)
        .filter(|h| h.contains("masterpage") || h.contains("masterPage"))
        .collect();
    for h in masters {
        if let Some(d) = parsed.get(h.as_str()) {
            refs(d, &ids, h, SEC, r);
        }
    }
}

fn header_checks(d: &Doc, nsec: usize, ids: &mut Ids, r: &mut Report) {
    let root = d.root();
    if root.local() != "head" {
        r.error(HEAD, format!("header root is <{}>", root.name));
    }
    match root.attr("secCnt").map(str::parse::<usize>) {
        Some(Ok(n)) if n != nsec => r.error(
            HEAD,
            format!("head secCnt is {n}, the spine has {nsec} sections"),
        ),
        Some(Err(_)) => r.error(HEAD, "head secCnt is not a number"),
        _ => {}
    }
    for e in &d.els {
        let Some(&(_, item)) = LISTS.iter().find(|(l, _)| *l == e.local()) else {
            continue;
        };
        let kids: Vec<&xml::El> = d.kids(e).filter(|k| k.local() == item).collect();
        if let Some(c) = e.attr("itemCnt") {
            if c.parse::<usize>().ok() != Some(kids.len()) {
                r.error(
                    HEAD,
                    format!(
                        "{} itemCnt is {c}, it holds {} {item}",
                        e.local(),
                        kids.len()
                    ),
                );
            }
        } else if !kids.is_empty() {
            r.warn(HEAD, format!("{} has no itemCnt", e.local()));
        }
        let set = ids.of.entry(item).or_default();
        for k in &kids {
            if item == "fontface" {
                let lang = k.attr("lang").unwrap_or_default().to_ascii_lowercase();
                let fonts: Vec<&xml::El> = d.kids(k).filter(|f| f.local() == "font").collect();
                if let Some(c) = k.attr("fontCnt")
                    && c.parse::<usize>().ok() != Some(fonts.len())
                {
                    r.error(
                        HEAD,
                        format!("fontface {lang} fontCnt is {c}, it holds {}", fonts.len()),
                    );
                }
                let fs = ids.fonts.entry(lang.clone()).or_default();
                for f in fonts {
                    let id = f.attr("id").unwrap_or_default().to_string();
                    if !fs.insert(id.clone()) {
                        r.error(HEAD, format!("fontface {lang} repeats font id {id}"));
                    }
                }
                continue;
            }
            let Some(id) = k.attr("id") else {
                r.error(HEAD, format!("{item} without an id"));
                continue;
            };
            if !set.insert(id.to_string()) {
                r.error(HEAD, format!("{item} id {id} appears twice"));
            }
        }
        // Hancom numbers items by position; HWP 5 ids are positions too.
        let base = if matches!(item, "borderFill" | "numbering" | "bullet" | "memoPr") {
            1
        } else {
            0
        };
        if item != "fontface" {
            for (pos, k) in kids.iter().enumerate() {
                if k.attr("id") != Some((pos + base).to_string().as_str()) {
                    r.warn(
                        HEAD,
                        format!(
                            "{item} {pos} has id {:?}, not its position",
                            k.attr("id").unwrap_or_default()
                        ),
                    );
                    break;
                }
            }
        }
    }
    refs(d, ids, "header", HEAD, r);
}

/// Every id reference in a part resolves.
fn refs(d: &Doc, ids: &Ids, part: &str, area: &'static str, r: &mut Report) {
    for e in &d.els {
        for (k, v) in &e.attrs {
            let k = k.rsplit(':').next().unwrap_or(k);
            let kind = match k {
                "paraPrIDRef" => "paraPr",
                "styleIDRef" => "style",
                "charPrIDRef" => "charPr",
                "borderFillIDRef" => "borderFill",
                "tabPrIDRef" => "tabPr",
                "nextStyleIDRef" => {
                    if ids.of.contains_key("style") && !ids.has("style", v) {
                        r.warn(
                            area,
                            format!("{part}: <{}> nextStyleIDRef={v:?} names no style", e.name),
                        );
                    }
                    continue;
                }
                "outlineShapeIDRef" => "numbering",
                "memoShapeIDRef" => "memoPr",
                "binaryItemIDRef" => {
                    if !v.is_empty() && !ids.bins.contains(v) {
                        r.error(
                            area,
                            format!(
                                "{part}: <{}> uses binary item {v:?} not in the manifest",
                                e.name
                            ),
                        );
                    }
                    continue;
                }
                "idRef" if e.local() == "heading" => match e.attr("type") {
                    Some("NUMBER" | "OUTLINE") => "numbering",
                    Some("BULLET") => "bullet",
                    _ => continue,
                },
                _ => continue,
            };
            // Unset references.
            if v == "4294967295"
                || (v == "0" && matches!(kind, "numbering" | "memoPr" | "borderFill"))
            {
                if v == "0" && kind == "numbering" && e.attr("type") == Some("NUMBER") {
                    r.error(
                        area,
                        format!("{part}: numbered heading without a numbering"),
                    );
                }
                continue;
            }
            if !ids.of.contains_key(kind) {
                // A header without the list at all (other writers omit
                // lists they do not use); Hancom fills in defaults.
                continue;
            }
            if !ids.has(kind, v) {
                r.error(
                    area,
                    format!("{part}: <{}> {k}={v:?} names no {kind}", e.name),
                );
            }
        }
        if e.local() == "fontRef" && ids.of.contains_key("fontface") {
            for lang in LANGS {
                if let Some(v) = e.attr(lang)
                    && !ids.fonts.get(lang).is_some_and(|s| s.contains(v))
                {
                    r.error(area, format!("{part}: fontRef {lang}={v:?} names no font"));
                }
            }
        }
    }
}

fn section_checks(d: &Doc, k: usize, r: &mut Report) {
    let w = format!("section {k}");
    let mut begins: Vec<String> = Vec::new();
    let mut ends: Vec<String> = Vec::new();
    let mut paras = 0;
    for (i, e) in d.els.iter().enumerate() {
        match e.local() {
            "p" => {
                paras += 1;
                if !d.kids(e).any(|c| c.local() == "run") {
                    r.warn(SEC, format!("{w}: paragraph without a run"));
                }
                if let Some(segs) = d.kids(e).find(|c| c.local() == "linesegarray") {
                    let pos: Vec<i64> = d
                        .kids(segs)
                        .filter_map(|s| s.attr("textpos")?.parse().ok())
                        .collect();
                    if pos.windows(2).any(|p| p[1] < p[0]) {
                        r.warn(SEC, format!("{w}: line segments out of order"));
                    }
                    if pos.first().is_some_and(|&p| p != 0) {
                        r.warn(SEC, format!("{w}: first line segment does not start at 0"));
                    }
                }
            }
            "subList" => {
                // The later boxes of a linked text box chain show the first
                // box's text; how Hancom stores them is not established.
                let linked = e.attr("linkListIDRef").is_some_and(|v| v != "0");
                if !d.kids(e).any(|c| c.local() == "p") {
                    if linked {
                        r.warn(SEC, format!("{w}: linked <{}> has no paragraph", e.name));
                    } else {
                        r.error(SEC, format!("{w}: <{}> has no paragraph", e.name));
                    }
                }
            }
            "tbl" => table(d, i, &w, r),
            "fieldBegin" => begins.push(e.attr("id").unwrap_or_default().to_string()),
            "fieldEnd" => ends.push(e.attr("beginIDRef").unwrap_or_default().to_string()),
            _ => {}
        }
    }
    if paras == 0 {
        r.error(SEC, format!("{w}: no paragraphs"));
    }
    // Hancom opens a field whose end was deleted (it keeps the begin).
    for b in &begins {
        if !ends.contains(b) {
            r.warn(SEC, format!("{w}: field {b} has no end"));
        }
    }
    for e in &ends {
        if !begins.contains(e) {
            r.error(SEC, format!("{w}: field end {e} has no begin"));
        }
    }
}

fn num(e: &xml::El, k: &str) -> Option<usize> {
    e.attr(k)?.parse().ok()
}

fn table(d: &Doc, i: usize, w: &str, r: &mut Report) {
    let t = &d.els[i];
    let (Some(rows), Some(cols)) = (num(t, "rowCnt"), num(t, "colCnt")) else {
        r.error(SEC, format!("{w}: table lacks rowCnt or colCnt"));
        return;
    };
    if rows == 0 || cols == 0 || rows > 10_000 || cols > 10_000 {
        r.error(SEC, format!("{w}: table is {rows}x{cols}"));
        return;
    }
    let trs: Vec<&xml::El> = d.kids(t).filter(|c| c.local() == "tr").collect();
    if trs.len() != rows {
        r.error(
            SEC,
            format!("{w}: table rowCnt {rows} with {} rows", trs.len()),
        );
    }
    let mut grid = vec![0u8; rows * cols];
    if trs.iter().any(|tr| !d.kids(tr).any(|c| c.local() == "tc")) {
        r.error(SEC, format!("{w}: table has a row without cells"));
    }
    for (ri, tr) in trs.iter().enumerate() {
        for tc in d.kids(tr).filter(|c| c.local() == "tc") {
            let addr = d.kids(tc).find(|c| c.local() == "cellAddr");
            let span = d.kids(tc).find(|c| c.local() == "cellSpan");
            let (Some(addr), Some(span)) = (addr, span) else {
                r.error(SEC, format!("{w}: table cell lacks its address or span"));
                continue;
            };
            let (Some(c0), Some(r0)) = (num(addr, "colAddr"), num(addr, "rowAddr")) else {
                r.error(SEC, format!("{w}: table cell address is not numeric"));
                continue;
            };
            let cs = num(span, "colSpan").unwrap_or(1);
            let rs = num(span, "rowSpan").unwrap_or(1);
            if r0 != ri {
                r.error(SEC, format!("{w}: cell row {r0} sits in table row {ri}"));
            }
            if cs == 0 || rs == 0 || r0 + rs > rows || c0 + cs > cols {
                r.error(
                    SEC,
                    format!("{w}: cell at {r0},{c0} spans {rs}x{cs} outside a {rows}x{cols} table"),
                );
                continue;
            }
            if !d.kids(tc).any(|c| c.local() == "subList") {
                r.error(SEC, format!("{w}: cell at {r0},{c0} has no paragraph list"));
            }
            for rr in r0..r0 + rs {
                for cc in c0..c0 + cs {
                    grid[rr * cols + cc] += 1;
                }
            }
        }
    }
    if let Some(k) = grid.iter().position(|&n| n != 1) {
        r.error(
            SEC,
            format!(
                "{w}: table grid slot {},{} is covered {} times",
                k / cols,
                k % cols,
                grid[k]
            ),
        );
    }
}
