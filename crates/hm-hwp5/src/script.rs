//! The document script (`Scripts/DefaultJScript`): Hancom fills combo and
//! list boxes there with `Name.InsertString("text", index)` calls instead
//! of storing their entries in the form records.

use hm_model::{
    Control, ControlKind, FormKind, HwpString, ObjectPart, Paragraph, SectionChild, Shape,
    ShapePart,
};
use std::collections::HashMap;

const SCRIPT_LIMIT: usize = 16 << 20;

/// The script's text: raw deflate (stored as is when that fails), UTF-16LE.
fn script_text(raw: &[u8]) -> Option<String> {
    let bytes = crate::deflate::inflate(raw, SCRIPT_LIMIT).unwrap_or_else(|_| raw.to_vec());
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    (!units.is_empty()).then(|| String::from_utf16_lossy(&units))
}

/// `Name.InsertString("text", index)` calls by control name, in index order.
fn entries(script: &str) -> HashMap<String, Vec<String>> {
    let mut found: HashMap<String, Vec<(u32, String)>> = HashMap::new();
    const CALL: &str = ".InsertString(";
    let mut rest = script;
    while let Some(i) = rest.find(CALL) {
        let name_start = rest[..i]
            .rfind(|c: char| !(c.is_alphanumeric() || c == '_'))
            .map_or(0, |k| {
                k + rest[k..].chars().next().map_or(1, char::len_utf8)
            });
        let name = rest[name_start..i].to_string();
        let args = &rest[i + CALL.len()..];
        rest = args;
        let Some(args) = args.trim_start().strip_prefix('"') else {
            continue;
        };
        let Some(close) = args.find('"') else {
            continue;
        };
        let text = args[..close].to_string();
        let tail = args[close + 1..].trim_start();
        let Some(tail) = tail.strip_prefix(',') else {
            continue;
        };
        let digits: String = tail
            .trim_start()
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        let index = digits.parse().unwrap_or(u32::MAX);
        if !name.is_empty() {
            found.entry(name).or_default().push((index, text));
        }
    }
    found
        .into_iter()
        .map(|(name, mut v)| {
            v.sort_by_key(|(k, _)| *k);
            (name, v.into_iter().map(|(_, t)| t).collect())
        })
        .collect()
}

fn fill_shape(s: &mut Shape, items: &HashMap<String, Vec<String>>) {
    for p in &mut s.parts {
        match p {
            ShapePart::Text(t) => fill(&mut t.paras, items),
            ShapePart::Child(c) => fill_shape(c, items),
            _ => {}
        }
    }
}

fn fill_control(c: &mut Control, items: &HashMap<String, Vec<String>>) {
    match &mut c.kind {
        ControlKind::Form(f) => {
            let listed = matches!(f.form_kind(), FormKind::ComboBox | FormKind::ListBox);
            let name = f
                .prop("CommonSet", "Name")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            if listed
                && f.items.is_empty()
                && let Some(list) = name.and_then(|n| items.get(&n))
            {
                f.items = list
                    .iter()
                    .map(|t| (HwpString::from_text(t), HwpString::from_text(t)))
                    .collect();
            }
        }
        ControlKind::Table(t) => {
            for cell in &mut t.cells {
                fill(&mut cell.paras, items);
            }
            if let Some(cap) = &mut t.caption {
                fill(&mut cap.paras, items);
            }
        }
        ControlKind::Object(o) => {
            for part in &mut o.parts {
                match part {
                    ObjectPart::Shape(s) => fill_shape(s, items),
                    ObjectPart::Caption(cap) => fill(&mut cap.paras, items),
                    _ => {}
                }
            }
        }
        ControlKind::HeaderFooter(h) => fill(&mut h.list.paras, items),
        ControlKind::Note(n) => fill(&mut n.list.paras, items),
        ControlKind::Section(s) => {
            for child in &mut s.children {
                if let SectionChild::MasterPage(l) = child {
                    fill(&mut l.paras, items);
                }
            }
        }
        _ => {}
    }
}

fn fill(paras: &mut [Paragraph], items: &HashMap<String, Vec<String>>) {
    for p in paras {
        for c in &mut p.controls {
            fill_control(c, items);
        }
    }
}

/// Gives combo and list boxes without stored entries the entries the
/// document script inserts. The records stay as read, so an untouched
/// section still writes back byte for byte.
pub fn fill_form_items(paras: &mut [Paragraph], raw: &[u8]) {
    let Some(script) = script_text(raw) else {
        return;
    };
    let items = entries(&script);
    if !items.is_empty() {
        fill(paras, &items);
    }
}

#[cfg(test)]
mod tests {
    use super::entries;

    #[test]
    fn insert_string_calls_give_entries_in_index_order() {
        let e = entries(
            "function OnDocument_New()\r\n{\r\n Combo.InsertString(\"여름\", 1);\r\nCombo.InsertString(\"봄\",0);\r\n  other.InsertString( \"x\" , 0 );\r\n}",
        );
        assert_eq!(e["Combo"], vec!["봄", "여름"]);
        assert_eq!(e["other"], vec!["x"]);
    }
}
