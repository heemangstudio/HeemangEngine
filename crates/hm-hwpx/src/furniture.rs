//! Page furniture: headers and footers, notes, numbers, page hiding and
//! master pages.

use hm_model::{
    AutoNumber, HeaderFooter, NewNumber, Note, PageHide, PageNumberPos, PageParity, ParaList,
};

use crate::body;
use crate::enums;
use crate::head::code;
use crate::object;
use crate::read::Ctx;
use crate::rest::Rest;
use crate::xml::{El, Ns};

/// A `hp:subList` as a paragraph list. Headers, footers and master pages
/// carry their text area size and reference flags.
pub fn para_list(sub: Option<El<'_>>, sized: bool, ctx: &mut Ctx) -> ParaList {
    let Some(l) = sub else {
        return ParaList::default();
    };
    let mut rest = Rest::new();
    let mut known = vec!["textDirection", "lineWrap", "vertAlign"];
    if sized {
        known.extend_from_slice(&["textWidth", "textHeight", "hasTextRef", "hasNumRef"]);
    }
    rest.attrs("", l, &known);
    for c in l.children().filter(|c| !c.is(Ns::Hp, "p")) {
        rest.child("", c);
    }
    let paras = body::paragraphs(l, ctx);
    ctx.note_rest(l, &rest);
    ParaList {
        count: paras.len() as u32,
        attr: object::list_attr(l, ctx),
        text_size: sized.then(|| {
            (
                l.i32("textWidth").unwrap_or(0),
                l.i32("textHeight").unwrap_or(0),
            )
        }),
        refs: sized.then(|| {
            (
                l.int("hasTextRef").unwrap_or(0) as u8,
                l.int("hasNumRef").unwrap_or(0) as u8,
            )
        }),
        tail: rest.into_tail(),
        paras,
    }
}

/// Records attributes outside `known` and children other than the list.
fn element_rest(el: El<'_>, known: &[&str], ctx: &mut Ctx) -> Rest {
    let mut rest = Rest::new();
    rest.attrs("", el, known);
    for c in el.children().filter(|c| !c.is(Ns::Hp, "subList")) {
        rest.child("", c);
    }
    ctx.note_rest(el, &rest);
    rest
}

pub fn header_footer(el: El<'_>, ctx: &mut Ctx) -> HeaderFooter {
    let rest = element_rest(el, &["applyPageType"], ctx);
    HeaderFooter {
        footer: el.local() == b"footer",
        attr: code(el, "applyPageType", enums::PAGE_TYPE, ctx).unwrap_or(0),
        tail: rest.into_tail(),
        list: para_list(el.child(Ns::Hp, "subList"), true, ctx),
    }
}

/// A character attribute: a number (character code) or the character.
fn char_attr(el: El<'_>, k: &str) -> u16 {
    match el.attr(k) {
        Some(v) if v.chars().all(|c| c.is_ascii_digit()) && !v.is_empty() => {
            v.parse::<u32>().unwrap_or(0) as u16
        }
        Some(v) => v.encode_utf16().next().unwrap_or(0),
        None => 0,
    }
}

pub fn note(el: El<'_>, ctx: &mut Ctx) -> Note {
    let rest = element_rest(
        el,
        &[
            "number",
            "userChar",
            "prefixChar",
            "suffixChar",
            "flag",
            "instId",
        ],
        ctx,
    );
    Note {
        endnote: el.local() == b"endNote",
        number: el.u32("number").unwrap_or(0),
        user_char: char_attr(el, "userChar"),
        prefix: char_attr(el, "prefixChar"),
        suffix: char_attr(el, "suffixChar"),
        flag: el.u32("flag").unwrap_or(0),
        inst_id: el.u32("instId"),
        tail: rest.into_tail(),
        list: para_list(el.child(Ns::Hp, "subList"), false, ctx),
    }
}

pub fn auto_number(el: El<'_>, ctx: &mut Ctx) -> AutoNumber {
    let mut rest = Rest::new();
    rest.attrs("", el, &["num", "numType"]);
    let f = el.child(Ns::Hp, "autoNumFormat");
    let mut attr = code(el, "numType", enums::AUTO_NUM_TYPE, ctx).unwrap_or(0) & 0xF;
    let mut n = AutoNumber {
        number: el.int("num").unwrap_or(0) as u16,
        ..Default::default()
    };
    if let Some(f) = f {
        rest.attrs(
            "autoNumFormat",
            f,
            &["type", "userChar", "prefixChar", "suffixChar", "supscript"],
        );
        attr |= (code(f, "type", enums::NUMBER_FORMAT, ctx).unwrap_or(0) & 0xFF) << 4;
        attr |= (f.flag("supscript").unwrap_or(false) as u32) << 12;
        n.user_char = char_attr(f, "userChar");
        n.prefix = char_attr(f, "prefixChar");
        n.suffix = char_attr(f, "suffixChar");
    }
    for c in el.children().filter(|c| !c.is(Ns::Hp, "autoNumFormat")) {
        rest.child("", c);
    }
    ctx.note_rest(el, &rest);
    n.attr = attr;
    n.tail = rest.into_tail();
    n
}

pub fn new_number(el: El<'_>, ctx: &mut Ctx) -> NewNumber {
    let rest = element_rest(el, &["num", "numType"], ctx);
    NewNumber {
        attr: code(el, "numType", enums::AUTO_NUM_TYPE, ctx).unwrap_or(0) & 0xF,
        number: el.int("num").unwrap_or(0) as u16,
        tail: rest.into_tail(),
    }
}

pub fn page_number(el: El<'_>, ctx: &mut Ctx) -> PageNumberPos {
    let rest = element_rest(el, &["pos", "formatType", "sideChar"], ctx);
    PageNumberPos {
        attr: code(el, "formatType", enums::NUMBER_FORMAT, ctx).unwrap_or(0) & 0xFF
            | (code(el, "pos", enums::PAGE_NUM_POS, ctx).unwrap_or(0) & 0xF) << 8,
        side: char_attr(el, "sideChar"),
        tail: rest.into_tail(),
        ..Default::default()
    }
}

pub const HIDE_ATTRS: [&str; 6] = [
    "hideHeader",
    "hideFooter",
    "hideMasterPage",
    "hideBorder",
    "hideFill",
    "hidePageNum",
];

pub fn page_hide(el: El<'_>, ctx: &mut Ctx) -> PageHide {
    let rest = element_rest(el, &HIDE_ATTRS, ctx);
    let attr = HIDE_ATTRS
        .iter()
        .enumerate()
        .map(|(i, k)| (el.flag(k).unwrap_or(false) as u32) << i)
        .sum();
    PageHide {
        attr,
        tail: rest.into_tail(),
    }
}

pub fn page_parity(el: El<'_>, ctx: &mut Ctx) -> PageParity {
    let rest = element_rest(el, &["pageStartsOn"], ctx);
    PageParity {
        attr: code(el, "pageStartsOn", enums::PAGE_TYPE, ctx).unwrap_or(0) & 3,
        tail: rest.into_tail(),
    }
}

/// Master page types and the section attribute bit that marks each.
pub const MASTER_TYPE: enums::Table = &[("BOTH", 29), ("EVEN", 30), ("ODD", 31)];

/// The master page part a `hp:masterPage` reference names, as a list and
/// its section attribute bit. Last-page and optional master pages have no
/// HWP 5 counterpart and stay markup.
pub fn master_page(reference: El<'_>, ctx: &mut Ctx) -> Option<(ParaList, u32)> {
    let id = reference.attr("idRef")?;
    let (index, part) = ctx.master_parts.get(id.as_ref())?.clone();
    let x = crate::xml::Xml::parse_part(part, index).ok()?;
    let root = x.root();
    let bit = crate::enums::code(MASTER_TYPE, &root.attr("type")?)?;
    let list = para_list(root.child(Ns::Hp, "subList"), true, ctx);
    Some((list, bit))
}

/// The list of a last-page master page reference (`type="LAST_PAGE"`).
pub fn last_page_master(reference: El<'_>, ctx: &mut Ctx) -> Option<ParaList> {
    let id = reference.attr("idRef")?;
    let (index, part) = ctx.master_parts.get(id.as_ref())?.clone();
    let x = crate::xml::Xml::parse_part(part, index).ok()?;
    let root = x.root();
    if root.attr("type")?.as_ref() != "LAST_PAGE" {
        return None;
    }
    Some(para_list(root.child(Ns::Hp, "subList"), true, ctx))
}

/// An extended master assigned to an individual section page.
pub fn page_master(reference: El<'_>, ctx: &mut Ctx) -> Option<(u32, ParaList)> {
    let id = reference.attr("idRef")?;
    let (index, part) = ctx.master_parts.get(id.as_ref())?.clone();
    let x = crate::xml::Xml::parse_part(part, index).ok()?;
    let root = x.root();
    if root.attr("type")?.as_ref() != "OPTIONAL_PAGE" {
        return None;
    }
    let page = root.u32("pageNumber").filter(|&p| p > 0)?;
    Some((page, para_list(root.child(Ns::Hp, "subList"), true, ctx)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hm_model::SharedBytes;

    #[test]
    fn optional_master_references_keep_their_section_page_and_list() {
        let reference = crate::xml::Xml::parse(SharedBytes::from(
            br#"<hp:masterPage xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph" idRef="chosen"/>"#.to_vec(),
        )).unwrap();
        let part = SharedBytes::from(br#"<hm:masterPage xmlns:hm="http://www.hancom.co.kr/hwpml/2011/master-page" xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph" type="OPTIONAL_PAGE" pageNumber="256"><hp:subList textWidth="42000" textHeight="70000"/></hm:masterPage>"#.to_vec());
        let mut ctx = Ctx::default();
        ctx.master_parts.insert("chosen".into(), (1, part));
        let (page, list) = page_master(reference.root(), &mut ctx).unwrap();
        assert_eq!(page, 256);
        assert_eq!(list.text_size, Some((42000, 70000)));
        assert!(last_page_master(reference.root(), &mut ctx).is_none());
    }
}
