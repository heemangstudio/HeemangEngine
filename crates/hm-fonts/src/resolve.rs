//! Face resolution: from a document face name to a catalog face.

use crate::metrics::Catalog;
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

/// Families tried for each class, most Hancom-like first: Hancom's faces,
/// then operating-system faces in order of how closely their advances
/// follow the full-em Hangul and third-em space of Hancom's classic faces
/// (휴먼명조, 한양신명조, HY헤드라인M; see [`OS_STAND_INS`]).
///
/// See [`class_defaults`].
const SERIF: &[&str] = &[
    "함초롬바탕",
    "HCR Batang",
    "한컴바탕",
    "바탕",
    "Batang",
    "Noto Serif KR",
    "AppleMyungjo",
    "나눔명조",
    "NanumMyeongjo",
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
    "AppleGothic",
    "나눔고딕",
    "NanumGothic",
    "Apple SD Gothic Neo",
    "Arial",
];

/// Faces of Windows and macOS that draw text set in one of the faces Hancom
/// ships when Hancom is not installed, in the order that matched Hancom's
/// PDFs best on documents set in that face. Hangul and the space carry most
/// of the measure: 함초롬 faces set Hangul at 0.97 em with a 0.3 em space,
/// close to 굴림, 돋움 and the 나눔 faces; 한컴 faces set full-em Hangul with
/// a third-em space like the Windows faces; Apple SD Gothic Neo (0.865 em
/// Hangul) comes last. The fixed-pitch 굴림체 and 돋움체 take AppleGothic
/// before the 한컴돋움 list, and HCI Poppy, Hancom's Latin face, takes the
/// face its font map gives (Palatino Linotype). A name without an entry
/// takes the faces of its design class ([`SERIF`], [`SANS`]).
const OS_STAND_INS: &[(&[&str], &[&str])] = &[
    (
        &["함초롬바탕", "HCR Batang"],
        &[
            "나눔명조",
            "NanumMyeongjo",
            "바탕",
            "Batang",
            "AppleMyungjo",
        ],
    ),
    (
        &["함초롬돋움", "HCR Dotum"],
        &[
            "굴림",
            "Gulim",
            "돋움",
            "Dotum",
            "나눔고딕",
            "NanumGothic",
            "맑은 고딕",
            "Malgun Gothic",
            "AppleGothic",
            "Apple SD Gothic Neo",
        ],
    ),
    (
        &["한컴바탕", "Haansoft Batang"],
        &[
            "바탕",
            "Batang",
            "궁서",
            "Gungsuh",
            "AppleMyungjo",
            "나눔명조",
            "NanumMyeongjo",
        ],
    ),
    (
        &["한컴돋움", "Haansoft Dotum"],
        &[
            "돋움",
            "Dotum",
            "굴림",
            "Gulim",
            "나눔고딕",
            "NanumGothic",
            "맑은 고딕",
            "Malgun Gothic",
            "AppleGothic",
            "Apple SD Gothic Neo",
        ],
    ),
    (
        &["맑은 고딕", "Malgun Gothic"],
        &[
            "나눔고딕",
            "NanumGothic",
            "AppleGothic",
            "Apple SD Gothic Neo",
        ],
    ),
    (
        &["굴림체", "GulimChe", "돋움체", "DotumChe"],
        &["AppleGothic"],
    ),
    (
        &["HCI Poppy"],
        &[
            "Palatino Linotype",
            "Palatino",
            "Book Antiqua",
            "Times New Roman",
        ],
    ),
];

/// The operating-system faces standing in for face `name`.
fn os_stand_ins(name: &str) -> &'static [&'static str] {
    let wanted = normalize(name);
    OS_STAND_INS
        .iter()
        .find(|(names, _)| names.iter().any(|n| normalize(n) == wanted))
        .map_or(&[], |(_, faces)| faces)
}

/// Hancom's stand-ins for a missing face name (see [`SUBSTITUTES`]).
fn substitutes(name: &str) -> impl Iterator<Item = &'static str> + use<> {
    let wanted = normalize(name);
    SUBSTITUTES
        .iter()
        .filter(move |(names, _)| names.iter().any(|n| normalize(n) == wanted))
        .flat_map(|(_, faces)| faces.iter().copied())
}

/// Names Hancom for Mac draws with another of its faces when they are
/// missing, tried in order. Only these names: the English names of the same
/// Windows faces (`Batang`, `Dotum`) and the font map's classes fall to the
/// default face instead. The 신명 and 명조 entries stand in for HFT faces a
/// host may not have.
const SUBSTITUTES: &[(&[&str], &[&str])] = &[
    (&["새바탕", "새궁서"], &["함초롬바탕"]),
    (&["새돋움"], &["함초롬돋움"]),
    (
        &[
            "바탕",
            "바탕체",
            "궁서",
            "궁서체",
            "신명 신명조",
            "신명 견명조",
            "신명 중명조",
            "명조",
            "새문명조",
        ],
        &["한컴바탕", "함초롬바탕"],
    ),
    (&["HY신명조", "한양신명조"], &["함초롬바탕", "한컴바탕"]),
    (&["HY중고딕"], &["한컴돋움", "함초롬돋움"]),
    (
        &["돋움", "돋움체", "굴림", "굴림체"],
        &["한컴돋움", "함초롬돋움"],
    ),
];

/// The face Hancom for Mac draws a missing face with when nothing above
/// names one, whatever the name or design class: its default 함초롬돋움.
const DEFAULT_FACES: &[&str] = &["함초롬돋움", "HCR Dotum"];

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

    /// Entries Hancom for Mac lists under `name` (see [`FaceEntry::local`]).
    fn by_local_name(&self, name: &str, bold: bool, italic: bool) -> Option<(FaceId, bool)> {
        let n = normalize(name);
        let ids: Vec<u32> = self
            .family(name)
            .iter()
            .copied()
            .filter(|&i| normalize(&self.entries[i as usize].local) == n)
            .collect();
        self.best_in(&ids, bold, italic)
    }

    /// Resolves a face name as Hancom for Mac does: the name itself (any of
    /// the face's family names), Hancom's stand-ins for a few missing names,
    /// the record's alternates (the substitute face, then the default name)
    /// by the name Hancom lists faces under, then its default face. Without
    /// Hancom's faces, an operating-system face standing in for the first of
    /// those names that has one (see [`OS_STAND_INS`]), else the first
    /// available face of the name's design class.
    pub fn resolve(&self, q: &FaceQuery<'_>) -> Option<Resolved> {
        if let Some((face, synthetic_bold)) = self.by_name(q.name, q.bold, q.italic) {
            return Some(Resolved {
                face,
                exact: true,
                synthetic_bold,
            });
        }
        let found = substitutes(q.name)
            .filter_map(|n| self.by_name(n, q.bold, q.italic))
            .chain(
                q.alternates
                    .iter()
                    .filter_map(|n| self.by_local_name(n, q.bold, q.italic)),
            )
            .chain(
                DEFAULT_FACES
                    .iter()
                    .filter_map(|n| self.by_name(n, q.bold, q.italic)),
            )
            .chain(
                Self::hancom_faces(q.name, &q.alternates)
                    .flat_map(os_stand_ins)
                    .filter_map(|n| self.by_name(n, q.bold, q.italic)),
            )
            .next();
        let r = match found {
            Some((face, synthetic_bold)) => Some(Resolved {
                face,
                exact: false,
                synthetic_bold,
            }),
            None => self.class_default(classify(q.name), q.bold, q.italic),
        }?;
        Some(self.emulate(q, r))
    }

    /// A face drawing in place of a missing face Hancom ships lays text out
    /// with that face's metrics, so lines and pages break as Hancom's do:
    /// the metrics of the face Hancom draws for the name (the name itself,
    /// a face standing in for it, the record's alternates, else its default
    /// face), when that face is missing and no face of the chain before it
    /// is here.
    fn emulate(&self, q: &FaceQuery<'_>, r: Resolved) -> Resolved {
        let catalog = Catalog::embedded();
        // The first name of Hancom's chain that Hancom ships or this catalog
        // has decides; a face found here before any Hancom face is the one
        // Hancom would draw too.
        let decided = Self::hancom_faces(q.name, &q.alternates)
            .chain(DEFAULT_FACES.iter().copied())
            .find_map(|n| match catalog.table(n, q.bold) {
                Some(t) => Some(Some(t)),
                None => (!self.family(n).is_empty()).then_some(None),
            });
        let Some(Some(table)) = decided else {
            return r;
        };
        let drawn = self.entry(r.face);
        let same = |f: &String| table.names.iter().any(|n| normalize(n) == normalize(f));
        if drawn.families.iter().any(same) {
            return r;
        }
        // Hancom widens synthetic bold HFT glyphs; real bold widths come
        // with a bold table.
        let synthetic_bold = q.bold && (!drawn.bold() || (table.hft && !table.bold));
        Resolved {
            face: self.emulated_face(r.face, table),
            exact: false,
            synthetic_bold,
        }
    }

    /// The face name, the faces Hancom draws in its place and the record's
    /// alternates.
    fn hancom_faces<'n>(name: &'n str, alternates: &'n [&'n str]) -> impl Iterator<Item = &'n str> {
        std::iter::once(name)
            .chain(substitutes(name).map(|n| -> &'n str { n }))
            .chain(alternates.iter().copied())
    }

    /// The families `resolve` tries for a face name, in order: the name,
    /// the alias table and its alternates, then the operating-system faces
    /// standing in for Hancom's; then the font map's stand-ins, which only
    /// fill glyphs the face lacks. Hosts that load fonts on demand register
    /// the first candidate they have, its stand-ins, and the first family
    /// they have of each list in [`class_defaults`].
    pub fn face_candidates(&self, name: &str, alternates: &[&str]) -> (Vec<String>, Vec<String>) {
        let mut candidates = Self::hancom_names(name, alternates);
        for n in Self::hancom_faces(name, alternates).flat_map(os_stand_ins) {
            if !candidates.iter().any(|c| c == n) {
                candidates.push(n.to_string());
            }
        }
        (candidates, self.font_map.stand_ins(name).to_vec())
    }

    /// The name, its alias table entries and the alternates, without
    /// repeats.
    pub(crate) fn hancom_names(name: &str, alternates: &[&str]) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for n in std::iter::once(name)
            .chain(substitutes(name).map(|n| -> &str { n }))
            .chain(alternates.iter().copied())
        {
            if !n.is_empty() && !out.iter().any(|c| c == n) {
                out.push(n.to_string());
            }
        }
        out
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

    /// A face of the families Hancom draws in place of a missing face
    /// `name` that has a glyph for `ch`.
    pub fn substitute_glyph(&self, name: &str, ch: char) -> Option<FaceId> {
        substitutes(name).find_map(|n| self.family_glyph(n, ch))
    }

    /// A face that has a glyph for `ch`: Hancom's default face, then the
    /// class defaults.
    pub fn fallback_for(&self, ch: char, class: ScriptClass) -> Option<FaceId> {
        let (first, second) = match class {
            ScriptClass::Serif => (SERIF, SANS),
            ScriptClass::Sans => (SANS, SERIF),
        };
        for name in DEFAULT_FACES.iter().chain(first).chain(second) {
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

/// Hancom's default face, then the serif and sans families, tried in order
/// when no candidate of a face name is available and for glyphs no chosen
/// face has.
pub fn class_defaults() -> [&'static [&'static str]; 3] {
    [DEFAULT_FACES, SERIF, SANS]
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
