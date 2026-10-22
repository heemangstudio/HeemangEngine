//! Face resolution: from a document face name to a catalog face.

use crate::{FaceId, FontKind, FontSystem, normalize};

/// A request for one script slot of a character shape.
#[derive(Clone, Debug, Default)]
pub struct FaceQuery<'a> {
    pub name: &'a str,
    /// The face record's substitute and default names, tried in order.
    pub alternates: Vec<&'a str>,
    pub bold: bool,
    pub italic: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resolved {
    pub face: FaceId,
    /// True when the requested name itself matched.
    pub exact: bool,
    /// True when bold was asked for and the face is not bold.
    pub synthetic_bold: bool,
}

/// The design class used when no named face is available.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptClass {
    Serif,
    Sans,
}

/// Families tried for each class, most Hancom-like first.
const SERIF: &[&str] = &[
    "함초롬바탕",
    "HCR Batang",
    "한컴바탕",
    "바탕",
    "Batang",
    "Noto Serif KR",
    "AppleMyungjo",
    "Times New Roman",
];
const SANS: &[&str] = &[
    "함초롬돋움",
    "HCR Dotum",
    "한컴돋움",
    "맑은 고딕",
    "Malgun Gothic",
    "돋움",
    "Dotum",
    "Noto Sans KR",
    "Apple SD Gothic Neo",
    "Arial",
];

/// Names Hancom documents use for faces that ship under another name.
const ALIASES: &[(&str, &str)] = &[
    ("새바탕", "함초롬바탕"),
    ("새돋움", "함초롬돋움"),
    ("바탕", "바탕체"),
    ("돋움", "돋움체"),
    ("굴림", "Gulim"),
    ("궁서", "Gungsuh"),
];

/// Guesses serif or sans from a face name.
pub fn classify(name: &str) -> ScriptClass {
    const SANS_HINTS: &[&str] = &[
        "고딕",
        "돋움",
        "굴림",
        "헤드라인",
        "gothic",
        "dotum",
        "gulim",
        "sans",
        "arial",
        "helvetica",
        "verdana",
        "tahoma",
        "맑은",
        "나눔고딕",
        "그래픽",
        "디나루",
    ];
    let n = name.to_lowercase();
    if SANS_HINTS.iter().any(|h| n.contains(h)) {
        ScriptClass::Sans
    } else {
        ScriptClass::Serif
    }
}

impl FontSystem {
    /// Picks the entry of a family that best matches bold and italic. An
    /// outline (SFNT) face wins over an HFT face of the same name, whatever
    /// type the document records: Hancom for Mac draws such faces (휴먼명조
    /// and the like) with their TrueType versions.
    fn best_in(&self, ids: &[u32], bold: bool, italic: bool) -> Option<(FaceId, bool)> {
        let want = if bold { 700 } else { 400 };
        let best = ids.iter().copied().min_by_key(|&i| {
            let e = &self.entries[i as usize];
            let kind = if e.kind == FontKind::Hft { 10_000 } else { 0 };
            kind + (e.weight as i32 - want).abs() + if e.italic != italic { 1000 } else { 0 }
        })?;
        let e = &self.entries[best as usize];
        Some((FaceId(best), bold && !e.bold()))
    }

    fn by_name(&self, name: &str, bold: bool, italic: bool) -> Option<(FaceId, bool)> {
        let ids = self.family(name);
        if !ids.is_empty() {
            return self.best_in(ids, bold, italic);
        }
        // A trailing weight word ("... Bold") names a face of the family.
        let n = normalize(name);
        self.entries
            .iter()
            .position(|e| e.postscript.as_deref().is_some_and(|p| normalize(p) == n))
            .map(|i| (FaceId(i as u32), false))
    }

    /// Resolves a face name: the name, its alternates, the alias table, then
    /// the default family of its class.
    pub fn resolve(&self, q: &FaceQuery<'_>) -> Option<Resolved> {
        if let Some((face, synthetic_bold)) = self.by_name(q.name, q.bold, q.italic) {
            return Some(Resolved {
                face,
                exact: true,
                synthetic_bold,
            });
        }
        let aliases = ALIASES
            .iter()
            .filter(|(a, _)| normalize(a) == normalize(q.name))
            .map(|(_, b)| *b);
        for name in q.alternates.iter().copied().chain(aliases) {
            if let Some((face, synthetic_bold)) = self.by_name(name, q.bold, q.italic) {
                return Some(Resolved {
                    face,
                    exact: false,
                    synthetic_bold,
                });
            }
        }
        self.class_default(classify(q.name), q.bold, q.italic)
    }

    /// The first available family of a class, else any face at all.
    pub fn class_default(&self, class: ScriptClass, bold: bool, italic: bool) -> Option<Resolved> {
        let list = match class {
            ScriptClass::Serif => SERIF,
            ScriptClass::Sans => SANS,
        };
        for name in list {
            if let Some((face, synthetic_bold)) = self.by_name(name, bold, italic) {
                return Some(Resolved {
                    face,
                    exact: false,
                    synthetic_bold,
                });
            }
        }
        (!self.entries.is_empty()).then_some(Resolved {
            face: FaceId(0),
            exact: false,
            synthetic_bold: bold,
        })
    }

    /// A face that has a glyph for `ch`, preferring the class defaults.
    pub fn fallback_for(&self, ch: char, class: ScriptClass) -> Option<FaceId> {
        let (first, second) = match class {
            ScriptClass::Serif => (SERIF, SANS),
            ScriptClass::Sans => (SANS, SERIF),
        };
        for name in first.iter().chain(second) {
            for &id in self.family(name) {
                if self.face(FaceId(id)).is_some_and(|f| f.has(ch)) {
                    return Some(FaceId(id));
                }
            }
        }
        (0..self.entries.len() as u32)
            .map(FaceId)
            .find(|&id| self.face(id).is_some_and(|f| f.has(ch)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_common_korean_names() {
        assert_eq!(classify("#태고딕"), ScriptClass::Sans);
        assert_eq!(classify("신명 견명조"), ScriptClass::Serif);
        assert_eq!(classify("맑은 고딕"), ScriptClass::Sans);
        assert_eq!(classify("Times New Roman"), ScriptClass::Serif);
    }
}
