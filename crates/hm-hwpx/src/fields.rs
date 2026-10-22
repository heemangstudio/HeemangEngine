//! HWP 5 control headers for controls the model keeps raw: fields, overlaid
//! characters (`tcps`) and ruby text (`tdut`). Their OWPML element stays the
//! saved form; the header gives readers the HWP 5 fields.

use crate::enums;
use crate::xml::{El, Ns};

fn put_str(out: &mut Vec<u8>, s: &str) {
    let units: Vec<u16> = s.encode_utf16().collect();
    out.extend_from_slice(&(units.len() as u16).to_le_bytes());
    for u in units {
        out.extend_from_slice(&u.to_le_bytes());
    }
}

/// The parameter of `el`'s `hp:parameters` named `name`.
fn param<'a>(el: El<'a>, kind: &str, name: &str) -> Option<El<'a>> {
    el.child(Ns::Hp, "parameters")?
        .children()
        .find(|p| p.is(Ns::Hp, kind) && p.attr("name").as_deref() == Some(name))
}

/// The CTRL_HEADER payload after the control id, when the element maps.
pub fn header(el: El<'_>) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    match el.local() {
        b"fieldBegin" => {
            // Bit 0 editable when read-only, bit 15 changed.
            let attr = el.flag("editable").unwrap_or(false) as u32
                | (el.flag("dirty").unwrap_or(false) as u32) << 15;
            out.extend_from_slice(&attr.to_le_bytes());
            let props = param(el, "integerParam", "Prop")
                .and_then(|p| p.text().trim().parse::<u32>().ok())
                .unwrap_or(0);
            out.push(props as u8);
            let command = param(el, "stringParam", "Command")
                .map(|p| p.text())
                .unwrap_or_default();
            put_str(&mut out, &command);
            out.extend_from_slice(&el.u32("id").unwrap_or(0).to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
        }
        b"compose" => {
            put_str(&mut out, &el.attr("composeText").unwrap_or_default());
            out.push(enums::code(COMPOSE_CIRCLE, &el.attr("circleType")?).unwrap_or(0) as u8);
            out.push(el.int("charSz").unwrap_or(0) as u8);
            out.push(enums::code(COMPOSE_TYPE, &el.attr("composeType")?).unwrap_or(0) as u8);
            let shapes: Vec<u32> = el
                .children()
                .filter(|c| c.is(Ns::Hp, "charPr"))
                .map(|c| c.u32("prIDRef").unwrap_or(0))
                .collect();
            out.push(shapes.len() as u8);
            for s in shapes {
                out.extend_from_slice(&s.to_le_bytes());
            }
        }
        b"dutmal" => {
            put_str(
                &mut out,
                &el.child(Ns::Hp, "mainText")
                    .map(|t| t.text())
                    .unwrap_or_default(),
            );
            put_str(
                &mut out,
                &el.child(Ns::Hp, "subText")
                    .map(|t| t.text())
                    .unwrap_or_default(),
            );
            let pos = el
                .attr("posType")
                .and_then(|p| enums::code(DUTMAL_POS, &p))
                .unwrap_or(0);
            for v in [
                pos,
                el.u32("szRatio").unwrap_or(0),
                el.u32("option").unwrap_or(0),
                el.u32("styleIDRef").unwrap_or(0),
                el.attr("align")
                    .and_then(|a| enums::code(enums::H_ALIGN, &a))
                    .unwrap_or(0),
            ] {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        _ => return None,
    }
    Some(out)
}

pub const COMPOSE_CIRCLE: enums::Table = &[
    ("CHAR", 0),
    ("SHAPE_CIRCLE", 1),
    ("SHAPE_REVERSAL_CIRCLE", 2),
    ("SHAPE_RECTANGLE", 3),
    ("SHAPE_REVERSAL_RECTANGLE", 4),
    ("SHAPE_TRIANGLE", 5),
    ("SHAPE_REVERSAL_TRIANGLE", 6),
    ("SHAPE_LIGHT", 7),
    ("SHAPE_RHOMBUS", 8),
    ("SHAPE_REVERSAL_RHOMBUS", 9),
    ("SHAPE_ROUNDED_RECTANGLE", 10),
    ("SHAPE_EMPTY_CIRCULATE_TRIANGLE", 11),
    ("SHAPE_THIN_CIRCULATE_TRIANGLE", 12),
];

pub const COMPOSE_TYPE: enums::Table = &[("SPREAD", 0), ("OVERLAP", 1)];

pub const DUTMAL_POS: enums::Table = &[("TOP", 0), ("BOTTOM", 1), ("CENTER", 2)];
