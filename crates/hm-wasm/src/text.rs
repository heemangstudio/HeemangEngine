//! Paragraph text with its wire offsets: caret stops between graphemes,
//! and search.

use hm_edit::{ListPath, Pos, Step};
use hm_model::{Document, Paragraph, Unit};
use unicode_segmentation::UnicodeSegmentation;

/// A control's place in the text: one grapheme of its own.
const CONTROL: char = '\u{FFFC}';

/// The paragraph as text, controls as U+FFFC, with the wire offset of each
/// char and the paragraph end's offset last.
pub fn text(p: &Paragraph) -> (String, Vec<u32>) {
    let mut s = String::new();
    let mut at = Vec::new();
    for (pos, u) in p.units() {
        match u {
            Unit::Char(c) => s.push(c),
            Unit::CharControl(13) => continue,
            Unit::CharControl(10) => s.push('\n'),
            Unit::Inline(9) => s.push('\t'),
            _ => s.push(CONTROL),
        }
        at.push(pos);
    }
    at.push(p.len().saturating_sub(1));
    (s, at)
}

/// Where the caret may stop in a paragraph: grapheme starts from after the
/// leading section and column controls, and the end mark.
pub fn stops(p: &Paragraph) -> Vec<u32> {
    let lo = hm_edit::ops::lead(p);
    let (s, at) = text(p);
    let chars: Vec<usize> = s.char_indices().map(|(b, _)| b).collect();
    let mut out: Vec<u32> = s
        .grapheme_indices(true)
        .filter_map(|(b, _)| chars.binary_search(&b).ok().map(|k| at[k]))
        .filter(|&q| q >= lo)
        .collect();
    out.push(*at.last().unwrap_or(&0));
    out.dedup();
    out
}

/// Every list in document order: each paragraph, then the lists its
/// controls hold.
pub fn lists(doc: &Document) -> Vec<ListPath> {
    fn walk(paras: &[Paragraph], list: &ListPath, out: &mut Vec<ListPath>) {
        out.push(list.clone());
        for (pi, p) in paras.iter().enumerate() {
            for (ci, c) in p.controls.iter().enumerate() {
                for (k, inner) in hm_edit::ops::control_lists(c).into_iter().enumerate() {
                    let mut path = list.clone();
                    path.steps.push(Step {
                        para: pi as u32,
                        control: ci as u32,
                        index: k as u32,
                    });
                    walk(inner, &path, out);
                }
            }
        }
    }
    let mut out = Vec::new();
    for (si, s) in doc.sections.iter().enumerate() {
        let list = ListPath {
            section: si as u32,
            steps: Vec::new(),
        };
        walk(&s.paras, &list, &mut out);
    }
    out
}

fn fold(c: char, case: bool) -> char {
    if case {
        c
    } else {
        c.to_lowercase().next().unwrap_or(c)
    }
}

/// Matches of `needle` within single paragraphs, as `[from, to)`, in list
/// order, at most `limit`. With `across`, controls are left out of the
/// text matched, so a match may span them; it still ends after its last
/// character. With `overlap`, a match may start inside the one before it,
/// as Hancom's find next and previous step ('aa' three times in 'aaaa');
/// replacing all takes them without overlap.
pub fn search(
    doc: &Document,
    needle: &str,
    case: bool,
    across: bool,
    overlap: bool,
    scope: Option<&ListPath>,
    limit: usize,
) -> Vec<(Pos, Pos)> {
    let needle: Vec<char> = needle.chars().map(|c| fold(c, case)).collect();
    let mut out = Vec::new();
    if needle.is_empty() {
        return out;
    }
    let all = lists(doc);
    let chosen = all.iter().filter(|l| scope.is_none_or(|s| *l == s));
    for list in chosen {
        let Some(paras) = hm_edit::ops::list(doc, list) else {
            continue;
        };
        for (pi, p) in paras.iter().enumerate() {
            let (s, at) = text(p);
            let units: Vec<(char, u32)> = s
                .chars()
                .zip(at.iter().copied())
                .filter(|(c, _)| !across || *c != CONTROL)
                .collect();
            let hay: Vec<char> = units.iter().map(|(c, _)| fold(*c, case)).collect();
            let pos = |offset: u32| Pos {
                list: list.clone(),
                para: pi as u32,
                offset,
            };
            let mut k = 0;
            while k + needle.len() <= hay.len() {
                if hay[k..k + needle.len()] == needle[..] {
                    let end = k + needle.len();
                    // The end is where the unit after the last matched one
                    // starts (a tab takes 8), the paragraph end mark last.
                    let last = units[end - 1].1;
                    let to = at.partition_point(|&a| a <= last);
                    out.push((pos(units[k].1), pos(at[to])));
                    if out.len() >= limit {
                        return out;
                    }
                    k = if overlap { k + 1 } else { end };
                } else {
                    k += 1;
                }
            }
        }
    }
    out
}

/// A set of characters: a bit per Basic Multilingual Plane code point.
#[derive(Clone, Debug, Default)]
pub struct CharSet {
    bmp: Vec<u64>,
    other: std::collections::BTreeSet<char>,
}

impl CharSet {
    pub fn insert(&mut self, c: char) {
        let v = c as usize;
        if v < 0x10000 {
            if self.bmp.len() <= v / 64 {
                self.bmp.resize(v / 64 + 1, 0);
            }
            self.bmp[v / 64] |= 1 << (v % 64);
        } else {
            self.other.insert(c);
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = char> + '_ {
        let bmp = self.bmp.iter().enumerate().flat_map(|(k, &w)| {
            let mut rest = w;
            std::iter::from_fn(move || {
                let b = (rest != 0).then(|| rest.trailing_zeros())?;
                rest &= rest - 1;
                Some(k as u32 * 64 + b)
            })
            .filter_map(char::from_u32)
        });
        bmp.chain(self.other.iter().copied())
    }
}

impl Extend<char> for CharSet {
    fn extend<I: IntoIterator<Item = char>>(&mut self, iter: I) {
        for c in iter {
            self.insert(c);
        }
    }
}

/// What a document draws text with: the `(script slot, face index)` pairs
/// its characters, controls, paragraph ends and heads take, the font names
/// its equations ask for (`None` for the default), the characters each
/// char shape draws, and every character it shows.
#[derive(Default)]
pub struct FontUse {
    pub faces: std::collections::BTreeSet<(usize, u16)>,
    pub equations: Vec<Option<String>>,
    pub shaped: Vec<(u32, CharSet)>,
    pub chars: CharSet,
}

pub fn font_use(doc: &Document) -> FontUse {
    use hm_model::{ControlKind, Script};
    let info = &doc.info;
    let mut u = FontUse::default();
    // Numbers, marks and heads take the Hangul, Latin or symbol face of
    // the shape they are drawn in: mark the shapes, add their faces last.
    let mut marked = vec![false; info.char_shapes.len()];
    let mut shape_faces = |_: &mut FontUse, id: u32| {
        if let Some(m) = marked.get_mut(id as usize) {
            *m = true;
        }
    };
    let mut numbered = false;
    // Characters per char shape, found by the shape's slot.
    let mut sets: Vec<(u32, CharSet)> = Vec::new();
    let mut slot: std::collections::HashMap<u32, usize> = Default::default();
    for list in lists(doc) {
        for p in hm_edit::ops::list(doc, &list).into_iter().flatten() {
            let kind = info
                .para_shapes
                .get(p.header.para_shape as usize)
                .map_or((0, 0), |s| (s.head_kind(), s.numbering));
            match kind {
                (0, _) => {}
                (3, n) => {
                    shape_faces(&mut u, p.shape_at(0));
                    if let Some(b) = info.bullets.get((n as usize).wrapping_sub(1)) {
                        shape_faces(&mut u, b.head.char_shape);
                    }
                }
                _ => {
                    shape_faces(&mut u, p.shape_at(0));
                    numbered = true;
                }
            }
            // The layout measures every run's shape in these slots, and
            // paragraph ends size empty lines by their font.
            for r in &p.runs {
                shape_faces(&mut u, r.shape);
            }
            let mut run = 0;
            let mut set = None;
            for (pos, unit) in p.units() {
                while run + 1 < p.runs.len() && p.runs[run + 1].pos <= pos {
                    run += 1;
                    set = None;
                }
                match unit {
                    Unit::Char(c) => {
                        let k = *set.get_or_insert_with(|| {
                            let id = p.runs.get(run).map_or(0, |r| r.shape);
                            *slot.entry(id).or_insert_with(|| {
                                sets.push((id, CharSet::default()));
                                sets.len() - 1
                            })
                        });
                        sets[k].1.insert(c);
                    }
                    _ => shape_faces(&mut u, p.shape_at(pos)),
                }
            }
            for c in &p.controls {
                if let ControlKind::Object(o) = &c.kind
                    && let Some(e) = o.equation()
                {
                    let name = e.font.as_ref().map(|n| n.to_string_lossy());
                    if !u.equations.contains(&name) {
                        u.equations.push(name);
                    }
                    u.chars.extend(e.script.to_string_lossy().chars());
                }
            }
        }
    }
    if numbered {
        for l in info.numberings.iter().flat_map(|n| &n.levels) {
            shape_faces(&mut u, l.head.char_shape);
        }
    }
    for l in info.numberings.iter().flat_map(|n| &n.levels) {
        u.chars.extend(l.format.to_string_lossy().chars());
    }
    for b in &info.bullets {
        u.chars.extend(char::from_u32(b.bullet_char as u32));
    }
    for (id, cs) in info.char_shapes.iter().enumerate() {
        if marked[id] {
            for s in [Script::Hangul, Script::Latin, Script::Symbol] {
                u.faces.insert((s.index(), cs.face(s)));
            }
        }
    }
    u.shaped = sets;
    for (id, set) in &u.shaped {
        let id = *id;
        let Some(cs) = info.char_shape(id) else {
            continue;
        };
        let mut scripts = [false; 7];
        for c in set.iter() {
            u.chars.insert(c);
            if let Some(s) = hm_layout::script::script_of(c) {
                scripts[s.index()] = true;
            }
        }
        for s in Script::ALL {
            if scripts[s.index()] {
                u.faces.insert((s.index(), cs.face(s)));
            }
        }
    }
    u
}
