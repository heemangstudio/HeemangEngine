//! Laying a document out again after edits.
//!
//! An edit leaves most paragraphs as they were read: a paragraph that still
//! holds its source bytes, with every control clean, formats exactly as
//! before given the same column width, tab interval, paragraph head and
//! page number. [`LayoutCache`] keeps the paragraph layouts and the style
//! table of the previous layout, so the next one formats only the changed
//! paragraphs and paginates the rest from their kept lines. Entries a
//! layout does not use are dropped after it.

use std::collections::HashMap;

use hm_fonts::FontSystem;
use hm_model::{Document, Paragraph};

use crate::heads::Head;
use crate::para::{ParaEnv, ParaLayout};
use crate::style::{StyleCache, Styles};
use crate::{DocLayout, Options};

/// What a paragraph's layout depends on besides the document tables.
#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    /// The paragraph's source bytes: address and length.
    src: (usize, usize),
    width: i32,
    tab: i32,
    page_number: u32,
    total_pages: u32,
    number_room: bool,
    stored_lines: bool,
    char_grid: crate::style::CharGrid,
    head: Option<(String, hm_model::ParaHead)>,
}

/// Paragraph layouts by key, with the layout generation that last used
/// each.
#[derive(Default)]
pub(crate) struct Memo {
    map: HashMap<Key, (ParaLayout, u64)>,
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

fn key(para: &Paragraph, styles: &Styles<'_>, env: ParaEnv, head: Option<&Head>) -> Option<Key> {
    let src = para.src.as_ref()?;
    if !para.is_clean() {
        return None;
    }
    Some(Key {
        src: (src.as_ptr() as usize, src.len()),
        width: env.column_width,
        tab: env.default_tab,
        page_number: styles.page_number,
        total_pages: styles.total_pages,
        number_room: std::ptr::addr_eq(para, styles.stored_number_room as *const Paragraph),
        stored_lines: styles.replay_imported_lines(),
        char_grid: styles.char_grid_of(para),
        head: head.map(|h| (h.text.clone(), h.info)),
    })
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
    let k = key(para, styles, env, head);
    if let (Some(k), Some(memo)) = (&k, styles.memo.as_mut()) {
        let generation = memo.generation;
        if let Some((hit, used)) = memo.map.get_mut(k) {
            *used = generation;
            memo.hits += 1;
            return hit.clone();
        }
    }
    let out = fresh(styles);
    if let Some(memo) = styles.memo.as_mut() {
        memo.misses += 1;
        if let Some(k) = k {
            memo.map.insert(k, (out.clone(), memo.generation));
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
    memo.map.retain(|_, (_, used)| *used == generation);
    cache.memo = memo;
    cache.styles = Some(styles.keep());
    layout
}
