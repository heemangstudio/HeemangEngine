//! The pictures OLE objects carry: the presentation stream an application
//! saves with its object (MS-OLEDS), which Hancom draws in place of charts
//! and other embedded objects.

use hm_hwp5::cfb::Cfb;

/// The metafile or bitmap of an OLE storage's first usable presentation
/// stream. The storage may follow a four-byte length.
pub fn preview(storage: &[u8]) -> Option<Vec<u8>> {
    const MAGIC: [u8; 4] = [0xD0, 0xCF, 0x11, 0xE0];
    let at = if storage.starts_with(&MAGIC) {
        0
    } else if storage.get(4..8) == Some(&MAGIC) {
        4
    } else {
        return None;
    };
    let cfb = Cfb::open(&storage[at..]).ok()?;
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
