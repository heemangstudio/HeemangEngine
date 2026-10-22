//! Hancom's font mapping table (`FontMap.dat`): for a face name, the faces
//! that stand in for it, in order.
//!
//! The file is UTF-16 (or UTF-8) text. `[Font Class]` lines name a class
//! and its faces (`TIMES=Times New Roman,Times,...`); `mapFont=Source,A,B`
//! maps a face to a list; `mapFontClass=Source,CLASS` maps it to a class's
//! list. Hancom draws a character an HFT face lacks with the face this
//! table maps it to: `HCI Poppy` falls back to `Palatino Linotype`.

use std::collections::HashMap;

use crate::normalize;

#[derive(Debug, Default)]
pub(crate) struct FontMap {
    /// Normalized source name to its stand-ins, in order.
    map: HashMap<String, Vec<String>>,
}

/// The text of a data file: UTF-16 with a byte order mark, else UTF-8.
fn decode(bytes: &[u8]) -> Option<String> {
    match bytes {
        [0xFF, 0xFE, rest @ ..] => {
            let units: Vec<u16> = rest
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            Some(String::from_utf16_lossy(&units))
        }
        [0xFE, 0xFF, rest @ ..] => {
            let units: Vec<u16> = rest
                .chunks_exact(2)
                .map(|c| u16::from_be_bytes([c[0], c[1]]))
                .collect();
            Some(String::from_utf16_lossy(&units))
        }
        _ => std::str::from_utf8(bytes).ok().map(str::to_string),
    }
}

fn list(s: &str) -> Vec<String> {
    s.trim_end_matches(';')
        .split(',')
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_string)
        .collect()
}

impl FontMap {
    /// Reads a mapping file into the table. Returns false when the bytes
    /// hold no mapping.
    pub(crate) fn add(&mut self, bytes: &[u8]) -> bool {
        let Some(text) = decode(bytes) else {
            return false;
        };
        let mut classes: HashMap<String, Vec<String>> = HashMap::new();
        let mut section = String::new();
        let mut added = false;
        for line in text.lines().map(str::trim) {
            if line.is_empty() || line.starts_with(';') {
                continue;
            }
            if line.starts_with('[') {
                section = line.to_string();
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key.trim() {
                "mapFont" => {
                    let mut names = list(value);
                    if names.len() < 2 {
                        continue;
                    }
                    let source = normalize(&names.remove(0));
                    // Some rows repeat the source among its stand-ins.
                    names.retain(|n| normalize(n) != source);
                    self.map.entry(source).or_default().extend(names);
                    added = true;
                }
                "mapFontClass" => {
                    let names = list(value);
                    if let [source, class] = names.as_slice()
                        && let Some(faces) = classes.get(class)
                    {
                        self.map
                            .entry(normalize(source))
                            .or_default()
                            .extend(faces.iter().cloned());
                        added = true;
                    }
                }
                class if section == "[Font Class]" => {
                    classes.insert(class.to_string(), list(value));
                }
                _ => {}
            }
        }
        added
    }

    /// The stand-ins for a face name, in order; empty when it has none.
    pub(crate) fn stand_ins(&self, name: &str) -> &[String] {
        self.map.get(&normalize(name)).map_or(&[], Vec::as_slice)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_maps_and_classes() {
        let text = "; comment\n[Font Class]\nTIMES=Times New Roman,Times\n[Map Fonts]\n\
                    mapFontClass=Tms Rmn,TIMES\nmapFont=HCI Poppy,Palatino Linotype\n";
        let mut m = FontMap::default();
        assert!(m.add(text.as_bytes()));
        assert_eq!(m.stand_ins("HCI Poppy"), ["Palatino Linotype"]);
        assert_eq!(m.stand_ins("tms rmn"), ["Times New Roman", "Times"]);
        assert!(m.stand_ins("Batang").is_empty());
    }
}
