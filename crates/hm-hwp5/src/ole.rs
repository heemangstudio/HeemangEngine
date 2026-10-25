//! The pictures OLE objects carry: the presentation stream an application
//! saves with its object (MS-OLEDS), which Hancom draws in place of charts
//! and other embedded objects. HWPX stores the same storages.

use hm_model::SharedBytes;
use hm_model::chart::OleView;

use crate::cfb::Cfb;

/// What an OLE storage shows: the chart of its OOXML chart part, else the
/// metafile or bitmap of its first usable presentation stream. Hancom draws
/// a chart from that part whatever picture the object saved (and drops the
/// picture when it saves), so the picture serves only objects without a
/// chart. The storage may follow a four-byte length.
pub fn view(storage: &[u8]) -> Option<OleView> {
    let cfb = compound(storage)?;
    if let Some(chart) = cfb
        .read("OOXMLChartContents")
        .and_then(|xml| crate::chart::parse(&String::from_utf8_lossy(&xml)))
    {
        return Some(OleView::Chart(Box::new(chart)));
    }
    let mut names: Vec<(String, usize)> = cfb
        .paths()
        .iter()
        .filter(|(p, _)| p.trim_start_matches('/').starts_with("\u{2}OlePres"))
        .cloned()
        .collect();
    names.sort();
    names
        .iter()
        .find_map(|(_, i)| presentation(&cfb.read_entry(*i)))
        .map(|p| OleView::Picture(SharedBytes::from(p)))
}

fn compound(storage: &[u8]) -> Option<Cfb<'_>> {
    const MAGIC: [u8; 4] = [0xD0, 0xCF, 0x11, 0xE0];
    let at = if storage.starts_with(&MAGIC) {
        0
    } else if storage.get(4..8) == Some(&MAGIC) {
        4
    } else {
        return None;
    };
    Cfb::open(&storage[at..]).ok()
}

/// Promotes a supported HwpEq5x OLE equation to an editable equation payload.
/// Its host keeps the original object header and performs the replacement.
pub fn legacy_equation(storage: &[u8]) -> Option<hm_model::Equation> {
    let contents = compound(storage)?.read("Contents")?;
    if !contents.starts_with(b"Hwp 5.0 Equation Editor(HwpEq5x)") || u32at(&contents, 52)? != 5 {
        return None;
    }
    let length = u32at(&contents, 68)? as usize;
    if length == 0 || length % 2 != 0 {
        return None;
    }
    let bytes = contents.get(72..72usize.checked_add(length)?)?;
    let utf16: Vec<_> = bytes
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect();
    let script = String::from_utf16(&utf16).ok()?;
    if script.contains('\0') {
        return None;
    }
    let script = legacy_script(&mut script.as_str(), false, 0)?;
    if script.trim().is_empty() {
        return None;
    }
    Some(hm_model::Equation {
        script: hm_model::HwpString::from_text(&script),
        size: 950,
        baseline: 60,
        font: Some(hm_model::HwpString::from_text("HYhwpEQ")),
        version: Some(hm_model::HwpString::from_text("Equation Version 60")),
        reserved: Some(0),
        ..Default::default()
    })
}

/// OVER owns two TAB-terminated slots. Each slot can hold another OVER.
fn legacy_script(rest: &mut &str, slot: bool, depth: u32) -> Option<String> {
    if depth > 64 {
        return None;
    }
    let mut out = String::new();
    loop {
        let Some(at) = rest.find('\\') else {
            if slot {
                return None;
            }
            out.push_str(rest);
            *rest = "";
            return Some(out);
        };
        out.push_str(&rest[..at]);
        *rest = &rest[at + 1..];
        let n = rest.bytes().take_while(u8::is_ascii_alphabetic).count();
        let command = &rest[..n];
        *rest = &rest[n..];
        match command {
            "TAB" if slot => return Some(out),
            "OVER" => {
                let numerator = legacy_script(rest, true, depth + 1)?;
                let denominator = legacy_script(rest, true, depth + 1)?;
                out.push_str(&format!(
                    " {{{}}} over {{{}}} ",
                    numerator.trim(),
                    denominator.trim()
                ));
            }
            "TIMES" => out.push_str(" TIMES "),
            _ => return None,
        }
    }
}

fn u32at(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(o..o + 4)?.try_into().ok()?))
}

/// The data of a presentation stream when it is a metafile (format 3) or
/// a device-independent bitmap (format 8).
fn presentation(d: &[u8]) -> Option<Vec<u8>> {
    let mut at = 0;
    let marker = u32at(d, at)?;
    at += 4;
    let format = match marker {
        0xFFFF_FFFF => {
            at += 4;
            u32at(d, at - 4)?
        }
        0 => return None,
        // A registered format named by a string of this length.
        n => {
            at += n as usize;
            0
        }
    };
    let device = u32at(d, at)? as usize;
    at += device.max(4);
    // Aspect, index, advise flags, reserved, width, height, size.
    let size = u32at(d, at + 24)? as usize;
    at += 28;
    let data = d.get(at..at + size.min(d.len().saturating_sub(at)))?;
    match format {
        3 | 8 if !data.is_empty() => Some(data.to_vec()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn equation_storage(script: &str) -> Vec<u8> {
        let mut contents = vec![0; 72];
        contents[..32].copy_from_slice(b"Hwp 5.0 Equation Editor(HwpEq5x)");
        contents[52..56].copy_from_slice(&5u32.to_le_bytes());
        let utf16: Vec<_> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
        contents[68..72].copy_from_slice(&(utf16.len() as u32).to_le_bytes());
        contents.extend(utf16);
        contents.extend([0; 24]);
        crate::cfb::build(&[("Contents".into(), contents)])
    }

    #[test]
    fn legacy_equation_promotes_fraction_slots_without_changing_the_storage() {
        let raw = equation_storage(r"N``=` \OVER (0.75 \TIMES V \TIMES 1000)-(M+F) \TAB 75 \TAB ");
        for prefixed in [false, true] {
            let mut storage = Vec::new();
            if prefixed {
                storage.extend((raw.len() as u32).to_le_bytes());
            }
            storage.extend(&raw);
            let before = storage.clone();
            let eq = legacy_equation(&storage).expect("the legacy fraction is supported");
            assert_eq!(
                eq.script.to_string_lossy(),
                "N``=`  {(0.75  TIMES  V  TIMES  1000)-(M+F)} over {75}  "
            );
            assert_eq!((eq.size, eq.baseline, eq.color, eq.attr), (950, 60, 0, 0));
            assert_eq!(eq.font.unwrap().to_string_lossy(), "HYhwpEQ");
            assert_eq!(eq.version.unwrap().to_string_lossy(), "Equation Version 60");
            assert_eq!(storage, before);
        }
    }

    #[test]
    fn legacy_equation_normalizes_nested_slots_and_keeps_unicode() {
        let raw = equation_storage(r"α=\OVER \OVER a \TAB b \TAB \TAB c \TAB");
        let eq = legacy_equation(&raw).unwrap();
        assert_eq!(eq.script.to_string_lossy(), "α= {{a} over {b}} over {c} ");
        assert_eq!(
            legacy_equation(&equation_storage("α + β"))
                .unwrap()
                .script
                .to_string_lossy(),
            "α + β"
        );
        assert_eq!(
            legacy_equation(&equation_storage(r"a \TIMES b"))
                .unwrap()
                .script
                .to_string_lossy(),
            "a  TIMES  b"
        );
    }

    #[test]
    fn legacy_equation_rejects_unknown_and_unfinished_commands() {
        for script in [r"a \UNKNOWN b", r"\OVER a \TAB b", r"\TAB a", "\0", ""] {
            assert!(
                legacy_equation(&equation_storage(script)).is_none(),
                "{script:?}"
            );
        }
    }

    #[test]
    fn legacy_equation_rejects_corrupt_content_and_other_ole_formats() {
        let storage = equation_storage("a");
        let contents = compound(&storage).unwrap().read("Contents").unwrap();
        for (at, bytes) in [
            (0, b"Other".as_slice()),
            (52, 4u32.to_le_bytes().as_slice()),
            (68, 1u32.to_le_bytes().as_slice()),
            (68, u32::MAX.to_le_bytes().as_slice()),
            (72, 0xD800u16.to_le_bytes().as_slice()),
        ] {
            let mut bad = contents.clone();
            bad[at..at + bytes.len()].copy_from_slice(bytes);
            let storage = crate::cfb::build(&[("Contents".into(), bad)]);
            assert!(legacy_equation(&storage).is_none(), "corruption at {at}");
        }
        assert!(
            legacy_equation(&crate::cfb::build(&[(
                "Equation Native".into(),
                vec![5; 80]
            )]))
            .is_none()
        );
        assert!(legacy_equation(&[0; 100]).is_none());
    }
}
