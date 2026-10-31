//! `hp:tbl` into the table model.

use hm_model::{Cell, CellZone, Table};

use crate::body;
use crate::enums;
use crate::head::code;
use crate::object;
use crate::read::Ctx;
use crate::rest::Rest;
use crate::xml::{El, Ns};

/// Page split of a table: how Hancom stores the `pageBreak` choice.
pub const PAGE_BREAK: enums::Table = &[("NONE", 0), ("TABLE", 1), ("CELL", 2)];

pub fn table(el: El<'_>, ctx: &mut Ctx) -> Table {
    let mut hrest = Rest::new();
    let mut known: Vec<&str> = object::COMMON_ATTRS.to_vec();
    known.extend_from_slice(&[
        "pageBreak",
        "repeatHeader",
        "rowCnt",
        "colCnt",
        "cellSpacing",
        "borderFillIDRef",
        "noAdjust",
        "lock",
    ]);
    hrest.attrs("", el, &known);
    let mut header = object::header(el, &mut hrest, ctx);
    // 개체 보호하기, where HWP 5 keeps it.
    header.common.attr |= (el.flag("lock").unwrap_or(false) as u32) << 30;
    let mut t = Table {
        attr: code(el, "pageBreak", PAGE_BREAK, ctx).unwrap_or(1) & 3
            | (el.flag("repeatHeader").unwrap_or(false) as u32) << 2
            | (el.flag("noAdjust").unwrap_or(false) as u32) << 3,
        rows: el.int("rowCnt").unwrap_or(0) as u16,
        cols: el.int("colCnt").unwrap_or(0) as u16,
        spacing: el.int("cellSpacing").unwrap_or(0) as i16,
        border_fill: el.int("borderFillIDRef").unwrap_or(0) as u16,
        zones: Some(Vec::new()),
        ..Default::default()
    };
    if let Some(m) = el.child(Ns::Hp, "inMargin") {
        hrest.attrs("inMargin", m, &["left", "right", "top", "bottom"]);
        let v = |k| m.int(k).unwrap_or(0) as i16;
        t.padding = [v("left"), v("right"), v("top"), v("bottom")];
    }
    for c in el.children() {
        match (c.ns(), c.local()) {
            (Ns::Hp, b"tr") => {
                let mut n = 0u16;
                for tc in c.children().filter(|x| x.is(Ns::Hp, "tc")) {
                    t.cells.push(cell(tc, ctx));
                    n += 1;
                }
                t.row_cells.push(n);
            }
            (Ns::Hp, b"cellzoneList") => {
                let zones = t.zones.get_or_insert_with(Vec::new);
                for z in c.children().filter(|x| x.is(Ns::Hp, "cellzone")) {
                    let v = |k| z.int(k).unwrap_or(0) as u16;
                    zones.push(CellZone {
                        start_col: v("startColAddr"),
                        start_row: v("startRowAddr"),
                        end_col: v("endColAddr"),
                        end_row: v("endRowAddr"),
                        border_fill: v("borderFillIDRef"),
                    });
                }
            }
            (Ns::Hp, b"caption") => t.caption = Some(object::caption(c, ctx)),
            (Ns::Hp, b"inMargin") => {}
            _ if object::is_header_child(c) => {}
            _ => hrest.child("", c),
        }
    }
    ctx.note_rest(el, &hrest);
    header.tail = hrest.into_tail();
    t.header = header;
    t
}

fn cell(tc: El<'_>, ctx: &mut Ctx) -> Cell {
    let mut rest = Rest::new();
    rest.attrs(
        "",
        tc,
        &[
            "name",
            "header",
            "hasMargin",
            "protect",
            "editable",
            "borderFillIDRef",
        ],
    );
    let mut c = Cell {
        field_name: tc
            .attr("name")
            .filter(|n| !n.is_empty())
            .map(|n| hm_model::HwpString::from_text(&n)),
        list_attr: (tc.flag("hasMargin").unwrap_or(false) as u32) << 16
            | (tc.flag("protect").unwrap_or(false) as u32) << 17
            | (tc.flag("header").unwrap_or(false) as u32) << 18
            | (tc.flag("editable").unwrap_or(false) as u32) << 19,
        border_fill: tc.int("borderFillIDRef").unwrap_or(0) as u16,
        col_span: 1,
        row_span: 1,
        ..Default::default()
    };
    for k in tc.children() {
        match k.local() {
            b"subList" => {
                rest.attrs("subList", k, &["textDirection", "lineWrap", "vertAlign"]);
                c.list_attr |= object::list_attr(k, ctx);
                c.paras = body::paragraphs(k, ctx);
            }
            b"cellAddr" => {
                c.col = k.int("colAddr").unwrap_or(0) as u16;
                c.row = k.int("rowAddr").unwrap_or(0) as u16;
            }
            b"cellSpan" => {
                c.col_span = k.int("colSpan").unwrap_or(1) as u16;
                c.row_span = k.int("rowSpan").unwrap_or(1) as u16;
            }
            b"cellSz" => {
                c.width = k.i32("width").unwrap_or(0);
                c.height = k.i32("height").unwrap_or(0);
            }
            b"cellMargin" => {
                let v = |n| k.int(n).unwrap_or(0) as i16;
                c.padding = [v("left"), v("right"), v("top"), v("bottom")];
            }
            _ => rest.child("", k),
        }
    }
    ctx.note_rest(tc, &rest);
    c.tail = rest.into_tail();
    c
}
