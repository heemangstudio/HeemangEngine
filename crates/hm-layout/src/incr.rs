//! Laying a document out again after edits.
//!
//! An edit leaves most paragraphs as they were read: a paragraph that still
//! holds its source bytes, with every control clean, formats exactly as
//! before given the same column width, tab interval, paragraph head and
//! page number. So does an edited paragraph without controls whose content
//! is what it was at the last layout. [`LayoutCache`] keeps the paragraph
//! layouts and the style table of the previous layout, so the next one
//! formats only the changed paragraphs and paginates the rest from their
//! kept lines. Entries a layout does not use are dropped after it.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hash, Hasher};

use hm_fonts::FontSystem;
use hm_model::{Document, Paragraph};

use crate::heads::Head;
use crate::para::{ParaEnv, ParaLayout};
use crate::style::{StyleCache, Styles};
use crate::{DocLayout, Options};

/// What a paragraph's content is known by: its source bytes (address and
/// length), or for a changed paragraph without controls, a hash of what
/// the edit left in it.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Identity {
    Source(usize, usize),
    Content(u64),
}

/// The rest of what a paragraph's layout depends on besides the document
/// tables.
#[derive(Clone, PartialEq, Eq)]
struct Env {
    width: i32,
    tab: i32,
    page_number: u32,
    total_pages: u32,
    outline_numbering: u16,
    note_marks: [crate::numbers::MarkFormat; 2],
    /// The numbers its notes and its note's own number show, when they
    /// differ from the stored ones.
    note_numbers: u64,
    number_room: bool,
    stored_lines: bool,
    guides: bool,
    char_grid: crate::style::CharGrid,
}

/// A changed paragraph's content, compared in full before its kept layout
/// is reused.
struct Content(Paragraph);

impl Content {
    fn of(p: &Paragraph) -> Self {
        Content(Paragraph {
            src: None,
            ..p.clone()
        })
    }

    fn matches(&self, p: &Paragraph) -> bool {
        let (a, b) = (&self.0, p);
        let (h, k) = (&a.header, &b.header);
        h.para_shape == k.para_shape
            && h.style == k.style
            && h.break_kind == k.break_kind
            && h.instance_id == k.instance_id
            && h.merge_flag == k.merge_flag
            && h.tail[..] == k.tail[..]
            && h.control_mask == k.control_mask
            && a.list_restart == b.list_restart
            && a.text == b.text
            && a.runs == b.runs
            && a.line_segs == b.line_segs
            && a.range_tags == b.range_tags
            && a.extra == b.extra
            && a.tracks == b.tracks
            && b.controls.is_empty()
    }
}

/// A kept layout with what it was made for and the layout generation that
/// last used it.
struct Entry {
    env: Env,
    head: Option<Head>,
    content: Option<Content>,
    layout: ParaLayout,
    used: u64,
}

/// Hashes the small keys of the memo (FxHash): addresses and content
/// hashes need no protection against chosen collisions.
#[derive(Default)]
pub(crate) struct Fx(u64);

impl Hasher for Fx {
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.write_u64(u64::from(b));
        }
    }
    fn write_u64(&mut self, n: u64) {
        self.0 = (self.0.rotate_left(5) ^ n).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
    fn write_usize(&mut self, n: usize) {
        self.write_u64(n as u64);
    }
    fn write_u32(&mut self, n: u32) {
        self.write_u64(u64::from(n));
    }
    fn write_u16(&mut self, n: u16) {
        self.write_u64(u64::from(n));
    }
    fn finish(&self) -> u64 {
        self.0
    }
}

/// Paragraph layouts by content identity, each with the environments it
/// was laid out in.
#[derive(Default)]
pub(crate) struct Memo {
    map: HashMap<Identity, Vec<Entry>, BuildHasherDefault<Fx>>,
    generation: u64,
    hits: u64,
    misses: u64,
}

/// State kept between layouts of one document.
#[derive(Default)]
pub struct LayoutCache {
    styles: Option<StyleCache>,
    memo: Memo,
}

impl LayoutCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Paragraphs reused and formatted in the last layout.
    pub fn stats(&self) -> (u64, u64) {
        (self.memo.hits, self.memo.misses)
    }
}

fn identity(para: &Paragraph) -> Option<Identity> {
    if let Some(src) = &para.src {
        if !para.is_clean() {
            return None;
        }
        return Some(Identity::Source(src.as_ptr() as usize, src.len()));
    }
    // An edited paragraph whose controls hold content of their own lays
    // out again every time.
    if !para.controls.is_empty() {
        return None;
    }
    let mut h = Fx::default();
    para.text.hash(&mut h);
    para.header.para_shape.hash(&mut h);
    para.header.style.hash(&mut h);
    for r in &para.runs {
        h.write_u32(r.pos);
        h.write_u32(r.shape);
    }
    Some(Identity::Content(h.finish()))
}

fn env_of(para: &Paragraph, styles: &Styles<'_>, env: ParaEnv) -> Env {
    Env {
        width: env.column_width,
        tab: env.default_tab,
        page_number: styles.page_number,
        total_pages: styles.total_pages,
        outline_numbering: styles.outline_numbering,
        note_marks: styles.note_marks,
        note_numbers: note_numbers(para, styles),
        number_room: std::ptr::addr_eq(para, styles.stored_number_room as *const Paragraph),
        stored_lines: styles.replay_imported_lines(),
        guides: styles.guides,
        char_grid: styles.char_grid_of(para),
    }
}

/// The numbers shown instead of the stored ones by the notes a paragraph's
/// layout draws marks of: its own, and those in the cells of its tables and
/// the text boxes of its drawings, which its layout holds too.
fn note_numbers(para: &Paragraph, styles: &Styles<'_>) -> u64 {
    fn walk(paras: &[Paragraph], styles: &Styles<'_>, h: &mut u64) {
        for c in paras.iter().flat_map(|p| &p.controls) {
            if let Some(n) = c.as_note() {
                if let Some(&v) = styles.note_numbers.get(&crate::numbers::key(n)) {
                    *h = h
                        .wrapping_mul(0x100_0000_01b3)
                        .wrapping_add(u64::from(v) + 1);
                }
            } else if let Some(t) = c.as_table() {
                for cell in &t.cells {
                    walk(&cell.paras, styles, h);
                }
            } else if let Some(o) = c.as_object() {
                for tb in crate::notes::text_boxes(o) {
                    walk(&tb.paras, styles, h);
                }
            }
        }
    }
    let mut h = styles.note_number.map_or(0, |n| u64::from(n) + 1);
    if !styles.note_numbers.is_empty() {
        walk(std::slice::from_ref(para), styles, &mut h);
    }
    h
}

/// Formats through the memo when the styles carry one.
pub(crate) fn memoized(
    para: &Paragraph,
    styles: &mut Styles<'_>,
    env: ParaEnv,
    head: Option<&Head>,
    fresh: impl FnOnce(&mut Styles<'_>) -> ParaLayout,
) -> ParaLayout {
    if styles.memo.is_none() {
        return fresh(styles);
    }
    let id = identity(para);
    let key = id.map(|id| (id, env_of(para, styles, env)));
    if let (Some((id, e)), Some(memo)) = (&key, styles.memo.as_mut()) {
        let generation = memo.generation;
        let hit = memo.map.get_mut(id).and_then(|entries| {
            entries.iter_mut().find(|k| {
                k.env == *e
                    && k.head.as_ref() == head
                    && k.content.as_ref().is_none_or(|c| c.matches(para))
            })
        });
        if let Some(hit) = hit {
            hit.used = generation;
            memo.hits += 1;
            return hit.layout.clone();
        }
    }
    let out = fresh(styles);
    if let Some(memo) = styles.memo.as_mut() {
        memo.misses += 1;
        if let Some((id, env)) = key {
            let content = matches!(id, Identity::Content(_)).then(|| Content::of(para));
            memo.map.entry(id).or_default().push(Entry {
                env,
                head: head.cloned(),
                content,
                layout: out.clone(),
                used: memo.generation,
            });
        }
    }
    out
}

/// Lays a document out, reusing what `cache` kept from its last layout.
pub fn layout_cached(
    doc: &Document,
    fonts: &FontSystem,
    options: Options,
    cache: &mut LayoutCache,
) -> DocLayout {
    let mut styles = match cache.styles.take() {
        Some(kept) => Styles::resume(&doc.info, fonts, kept),
        None => Styles::new(&doc.info, fonts),
    };
    let mut memo = std::mem::take(&mut cache.memo);
    memo.generation += 1;
    memo.hits = 0;
    memo.misses = 0;
    styles.memo = Some(memo);
    let layout = crate::run(doc, &mut styles, options);
    let mut memo = styles.memo.take().unwrap_or_default();
    let generation = memo.generation;
    memo.map.retain(|_, entries| {
        entries.retain(|e| e.used == generation);
        !entries.is_empty()
    });
    cache.memo = memo;
    cache.styles = Some(styles.keep());
    layout
}
