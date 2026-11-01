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
    /// Entries kept after the last sweep, and entries added since.
    swept: usize,
    added: usize,
}

impl Memo {
    /// Drops the entries the document's body paragraphs no longer use.
    /// After a layout that formatted every paragraph, those are the entries
    /// it did not use; after one that took paragraphs from an earlier
    /// layout, it is done once enough entries were added since the last
    /// sweep, keeping every entry of a paragraph still in the document.
    fn sweep(&mut self, doc: &Document, whole: bool) {
        let generation = self.generation;
        if !whole {
            if self.added <= self.swept + 256 {
                return;
            }
            for p in doc.sections.iter().flat_map(|s| &s.paras) {
                if let Some(entries) = identity(p).and_then(|id| self.map.get_mut(&id)) {
                    for e in entries {
                        e.used = generation;
                    }
                }
            }
        }
        self.map.retain(|_, entries| {
            entries.retain(|e| e.used == generation);
            !entries.is_empty()
        });
        self.swept = self.map.values().map(Vec::len).sum();
        self.added = 0;
    }
}

/// State kept between layouts of one document.
#[derive(Default)]
pub struct LayoutCache {
    styles: Option<StyleCache>,
    memo: Memo,
    /// What the last layout recorded per section, and what it was made
    /// with: its token, options and total page count.
    records: Vec<Option<SectionRecord>>,
    token: u64,
    options: Option<(bool, bool)>,
    total_pages: u32,
    /// What the last layout took from the one before ([`LayoutCache::taken`]).
    taken: (usize, usize, usize),
}

impl LayoutCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Paragraphs reused and formatted in the last layout.
    pub fn stats(&self) -> (u64, u64) {
        (self.memo.hits, self.memo.misses)
    }

    /// What the last layout took as it was from the layout before it,
    /// without paginating it again: sections taken whole, sections whose
    /// pagination met the earlier layout again and stopped, and body
    /// paragraphs.
    pub fn taken(&self) -> (usize, usize, usize) {
        self.taken
    }
}

/// Tokens of layouts [`LayoutCache`]s made; 0 names none.
static TOKENS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

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
            memo.added += 1;
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
    relayout_cached(doc, fonts, options, cache, None, None, None)
}

/// Lays a document out again, reusing what `cache` kept from its last
/// layout, which `old` is: the pages and paragraph layouts edits could not
/// have changed are taken from it rather than paginated again. A layout
/// `cache` did not make last is ignored.
pub fn relayout_cached(
    doc: &Document,
    fonts: &FontSystem,
    options: Options,
    cache: &mut LayoutCache,
    old: Option<DocLayout>,
    touched: Option<&[Touched]>,
    stop: Option<usize>,
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
    let opts = (options.stored_lines, options.guides);
    let old = old.filter(|o| {
        cache.token != 0
            && o.token == cache.token
            && cache.options == Some(opts)
            && (o.sections.len() == doc.sections.len()
                || (o.partial && o.sections.len() <= doc.sections.len()))
            && cache.records.len() == doc.sections.len()
    });
    let records = std::mem::take(&mut cache.records);
    // Body text showing the total page count moves lines when the count
    // changes: those documents are laid out whole.
    let totals = crate::numbers::total_in(doc);
    let mut reuse = (totals != Some(true)).then(|| {
        let usable = old.is_some();
        if usable && totals == Some(false) {
            styles.total_pages = cache.total_pages;
        }
        Reuse::new(doc, old, if usable { records } else { Vec::new() }, touched)
    });
    // Stopping after the first pages needs the records to go on from.
    let stop = stop.filter(|_| reuse.is_some());
    let mut layout = crate::run(doc, &mut styles, options, reuse.as_mut(), stop);
    let mut memo = styles.memo.take().unwrap_or_default();
    memo.sweep(doc, reuse.as_ref().is_none_or(|r| r.taken.2 == 0));
    cache.memo = memo;
    cache.total_pages = styles.total_pages;
    cache.styles = Some(styles.keep());
    cache.options = Some(opts);
    match reuse {
        Some(r) => {
            cache.taken = r.taken;
            cache.records = r.out;
            cache.token = TOKENS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        None => {
            cache.taken = (0, 0, 0);
            cache.token = 0;
        }
    }
    layout.token = cache.token;
    layout
}

/// Body paragraphs an edit reached since the last layout: the first, and
/// how many paragraphs ended the section after them right after the edit.
/// Paragraphs before the first, and those after, are the ones the last
/// layout saw, however their content would compare.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Touched {
    pub section: u32,
    pub first: usize,
    pub after: usize,
}

/// Per section, the body paragraphs no edit reached since the last layout:
/// those before `first` and the last `after`.
#[derive(Clone, Copy)]
struct Untouched {
    first: usize,
    after: usize,
}

/// What a body paragraph was known by when a section was laid out: its
/// source bytes. An edited paragraph is known only by no edit having
/// reached it since ([`Touched`]).
pub(crate) enum Known {
    Source((usize, usize, usize)),
    Unknown,
}

impl Known {
    fn of(para: &Paragraph) -> Self {
        match &para.src {
            Some(src) if para.is_clean() && !src.is_empty() => Known::Source(src.identity()),
            _ => Known::Unknown,
        }
    }

    /// Whether `para` is what was known; `untouched` says no edit reached
    /// it since, which is all an edited paragraph has.
    fn matches(&self, para: &Paragraph, untouched: bool) -> bool {
        match self {
            Known::Source(id) => para
                .src
                .as_ref()
                .is_some_and(|src| para.is_clean() && src.identity() == *id),
            Known::Unknown => untouched,
        }
    }
}

/// What a section's layout started from.
struct Start {
    heads: crate::heads::Heads,
    furniture: crate::furniture::State,
    /// Index of the section's first page in the document.
    first_page: usize,
    total_pages: u32,
}

impl Start {
    fn same(&self, other: &Start) -> bool {
        self.first_page == other.first_page
            && self.total_pages == other.total_pages
            && self.heads == other.heads
            && self.furniture.same(&other.furniture)
    }
}

/// What a later layout needs to reuse a section's layout, or to go on from
/// a page of it.
pub(crate) struct SectionRecord {
    entry: Start,
    paras: Vec<Known>,
    heads_out: crate::heads::Heads,
    furniture_out: crate::furniture::State,
    /// The section's pages.
    pages: usize,
    /// Paginator states at page starts; `None` when the section cannot go
    /// on from one (its notes took a second pass, it numbers lines or its
    /// notes renumber on each page).
    snaps: Option<Vec<crate::paginate::Snap>>,
    /// Per page of the section, what its decoration started from.
    decorations: Vec<crate::furniture::Decoration>,
    /// The layout stopped in this section after its first pages: its last
    /// snapshot is where the next layout goes on, and `decorations` ends
    /// with the state the next page's decoration starts from.
    partial: bool,
}

/// A section going on from a page of its earlier layout.
pub(crate) struct Resumed {
    pub(crate) snap: crate::paginate::Snap,
    pub(crate) paras: Vec<ParaLayout>,
    /// The earlier snapshots up to `snap`, which stay valid.
    pub(crate) snaps: Vec<crate::paginate::Snap>,
    /// The page (section-relative) decoration goes on from, the
    /// decorations of the pages before it and the state it goes on from.
    pub(crate) from: usize,
    pub(crate) decorations: Vec<crate::furniture::Decoration>,
    pub(crate) state: crate::furniture::State,
    /// What the paragraphs before `snap` were known by.
    pub(crate) known: Vec<Known>,
    /// For meeting the earlier layout again further on: its snapshots
    /// after `snap`, the paragraph from which every paragraph is as it was,
    /// its paragraph layouts from `snap` on, what its paragraphs from
    /// `same_from` on were known by, its decorations from `from` on, where
    /// its pages end in the old page list, and what it ended with.
    pub(crate) later: Vec<crate::paginate::Snap>,
    pub(crate) same_from: usize,
    tail: Vec<ParaLayout>,
    known_tail: Vec<Known>,
    later_decorations: Vec<crate::furniture::Decoration>,
    old_end: usize,
    merged_notes: Vec<i32>,
    heads_out: crate::heads::Heads,
    furniture_out: crate::furniture::State,
}

/// The previous layout of a document and what was recorded about it, for
/// [`crate::run`] to take what is still valid; and the records of the
/// layout being made.
/// What a run knows about a section's paragraphs against the last layout.
#[derive(Clone, Copy)]
enum Change {
    /// Taken whole: every paragraph is as it was.
    None,
    /// Paragraphs before `changed` are as they were, and with an unchanged
    /// paragraph count so are those from `same_from` on.
    Range { changed: usize, same_from: usize },
    /// Not known: any paragraph may have changed.
    All,
}

pub(crate) struct Reuse {
    old: Option<DocLayout>,
    /// Per section, what this run found changed.
    changes: Vec<Change>,
    records: Vec<Option<SectionRecord>>,
    /// Per section, the paragraphs edits did not reach; all touched when
    /// not known.
    untouched: Vec<Untouched>,
    out: Vec<Option<SectionRecord>>,
    /// Every section so far was taken whole from the old layout.
    unchanged: bool,
    /// Sections taken whole, sections whose pagination met the earlier
    /// layout again, and paragraphs taken from earlier layouts.
    pub(crate) taken: (usize, usize, usize),
}

impl Reuse {
    fn new(
        doc: &Document,
        old: Option<DocLayout>,
        records: Vec<Option<SectionRecord>>,
        touched: Option<&[Touched]>,
    ) -> Self {
        let none = Untouched { first: 0, after: 0 };
        let all = Untouched {
            first: usize::MAX,
            after: usize::MAX,
        };
        let mut untouched = vec![if touched.is_some() { all } else { none }; doc.sections.len()];
        for t in touched.unwrap_or_default() {
            match untouched.get_mut(t.section as usize) {
                Some(u) => {
                    u.first = u.first.min(t.first);
                    u.after = u.after.min(t.after);
                }
                None => untouched.iter_mut().for_each(|u| *u = none),
            }
        }
        Reuse {
            old,
            changes: Vec::new(),
            records,
            untouched,
            out: Vec::new(),
            unchanged: true,
            taken: (0, 0, 0),
        }
    }

    /// Starts a run: nothing recorded yet.
    pub(crate) fn begin(&mut self, sections: usize) {
        self.out.clear();
        self.out.resize_with(sections, || None);
        self.changes = vec![Change::All; sections];
        self.unchanged = true;
    }

    /// Whether body paragraph `para` of `section` is as the last layout saw
    /// it, as far as this run found.
    fn same_para(&self, section: u32, para: u32) -> bool {
        let p = para as usize;
        match self.changes.get(section as usize) {
            Some(Change::None) => true,
            Some(Change::Range { changed, same_from }) => p < *changed || p >= *same_from,
            _ => false,
        }
    }

    /// Whether the headers, footers and master pages a furniture state
    /// draws are in paragraphs as the last layout saw them.
    pub(crate) fn draws_same(&self, state: &crate::furniture::State) -> bool {
        state.hosts().into_iter().all(|(s, p)| self.same_para(s, p))
    }

    /// Forgets the old layout, as a run that cannot use it starts.
    pub(crate) fn forget(&mut self) {
        self.old = None;
        self.records.clear();
    }

    /// Whether the last layout left a record of section `i`: a first
    /// layout has none, and skips the rest of reuse.
    pub(crate) fn has(&self, i: usize) -> bool {
        self.old.is_some() && self.records.get(i).is_some_and(Option::is_some)
    }

    /// The old record of section `i` when it started where the section
    /// starts now.
    fn record(&mut self, i: usize, entry: &Start) -> Option<SectionRecord> {
        self.old.as_ref()?;
        let r = self.records.get_mut(i)?.take()?;
        r.entry.same(entry).then_some(r)
    }

    /// Takes section `i` whole from the old layout when none of its
    /// paragraphs changed and it starts as it did: appends its pages and
    /// returns its layout, leaving the heads and furniture state as the
    /// section left them.
    #[allow(clippy::too_many_arguments)]
    #[inline(never)]
    pub(crate) fn whole(
        &mut self,
        doc: &Document,
        i: usize,
        heads: &mut crate::heads::Heads,
        furniture: &mut crate::furniture::State,
        pages: &mut Vec<crate::Page>,
        total_pages: u32,
    ) -> Option<crate::SectionLayout> {
        let sec = &doc.sections[i];
        let entry = Start {
            heads: heads.clone(),
            furniture: furniture.clone(),
            first_page: pages.len(),
            total_pages,
        };
        // The last section places the endnotes of the sections before it.
        let last = i + 1 == doc.sections.len();
        if last && !self.unchanged {
            self.unchanged = false;
            return None;
        }
        let r = self.record(i, &entry)?;
        if r.partial || !self.draws_same(&entry.furniture) {
            self.records[i] = Some(r);
            self.unchanged = false;
            return None;
        }
        let untouched = self.untouched[i].first == usize::MAX;
        let same = r.paras.len() == sec.paras.len()
            && r.paras
                .iter()
                .zip(&sec.paras)
                .all(|(k, p)| k.matches(p, untouched));
        let old = self.old.as_mut()?;
        if !same || old.sections.len() <= i {
            // Kept for going on from a page of it.
            self.records[i] = Some(r);
            self.unchanged = false;
            return None;
        }
        let start = r.entry.first_page;
        let end = start + r.pages;
        if end > old.pages.len() {
            self.unchanged = false;
            return None;
        }
        pages.extend(old.pages[start..end].iter_mut().map(std::mem::take));
        let laid = std::mem::take(&mut old.sections[i]);
        self.changes[i] = Change::None;
        *heads = r.heads_out.clone();
        *furniture = r.furniture_out.clone();
        self.taken.0 += 1;
        self.taken.2 += sec.paras.len();
        self.out[i] = Some(r);
        Some(laid)
    }

    /// Where section `i` can go on from in its old layout: the last page
    /// start whose state no changed paragraph can affect. Appends the pages
    /// before that page.
    #[inline(never)]
    pub(crate) fn resume(
        &mut self,
        doc: &Document,
        i: usize,
        heads: &crate::heads::Heads,
        furniture: &crate::furniture::State,
        pages: &mut Vec<crate::Page>,
        total_pages: u32,
    ) -> Option<Resumed> {
        let sec = &doc.sections[i];
        let entry = Start {
            heads: heads.clone(),
            furniture: furniture.clone(),
            first_page: pages.len(),
            total_pages,
        };
        // Pages before the one it goes on from keep their decoration.
        if !self.draws_same(&entry.furniture) {
            return None;
        }
        let r = self.record(i, &entry)?;
        let mut snaps = r.snaps?;
        // The first paragraph that changed, and the first of the unchanged
        // paragraphs that end the section at the same indices.
        let u = self.untouched[i];
        let n = sec.paras.len();
        let changed = r
            .paras
            .iter()
            .zip(&sec.paras)
            .enumerate()
            .position(|(j, (k, p))| !k.matches(p, j < u.first))
            .unwrap_or(r.paras.len().min(n));
        let same_from = if r.paras.len() == n {
            let same = r
                .paras
                .iter()
                .zip(&sec.paras)
                .rev()
                .enumerate()
                .take_while(|(back, (k, p))| k.matches(p, *back < u.after))
                .count();
            (n - same).max(changed + 1)
        } else {
            usize::MAX
        };
        self.changes[i] = Change::Range { changed, same_from };
        let at = snaps.iter().rposition(|s| s.reach < changed)?;
        let later = snaps.split_off(at + 1);
        let snap = snaps[at].clone();
        let old = self.old.as_mut()?;
        let start = r.entry.first_page;
        let old_end = start + r.pages;
        let from = snap.pages - 1 - start;
        if old.sections.len() <= i
            || old.sections[i].paras.len() < snap.para
            || old_end > old.pages.len()
            || r.decorations.len() <= from
        {
            return None;
        }
        pages.extend(
            old.pages[start..snap.pages - 1]
                .iter_mut()
                .map(std::mem::take),
        );
        let old_section = std::mem::take(&mut old.sections[i]);
        let mut paras = old_section.paras;
        let tail = paras.split_off(snap.para);
        let mut decorations = r.decorations;
        let later_decorations = decorations.split_off(from);
        let state = later_decorations[0].state.clone();
        let mut known = r.paras;
        let known_tail = if same_from <= known.len() {
            known.split_off(same_from)
        } else {
            Vec::new()
        };
        known.truncate(snap.para);
        self.taken.2 += snap.para;
        Some(Resumed {
            snap,
            paras,
            snaps,
            from,
            decorations,
            state,
            known,
            later,
            same_from,
            tail,
            known_tail,
            later_decorations,
            old_end,
            merged_notes: old_section.merged_notes,
            heads_out: r.heads_out,
            furniture_out: r.furniture_out,
        })
    }

    /// Finishes section `i` from its earlier layout after its pagination
    /// met that layout's snapshot `k` before paragraph `pi`: the page being
    /// filled and every page after it, the paragraph layouts from `pi` on
    /// and the snapshots from there are the earlier ones. Returns the
    /// section-relative page met, with the earlier decorations from there.
    #[allow(clippy::too_many_arguments)]
    #[inline(never)]
    pub(crate) fn finish(
        &mut self,
        doc: &Document,
        i: usize,
        r: &mut Resumed,
        (pi, k): (usize, usize),
        first: usize,
        pages: &mut Vec<crate::Page>,
        laid: &mut crate::SectionLayout,
        snaps: &mut Vec<crate::paginate::Snap>,
        heads: &mut crate::heads::Heads,
    ) -> Option<(usize, Vec<crate::furniture::Decoration>)> {
        let old = self.old.as_mut()?;
        let met = r.later.get(k)?.pages;
        if met > r.old_end || pi < r.snap.para || pi < r.same_from {
            return None;
        }
        pages.pop();
        pages.extend(old.pages[met - 1..r.old_end].iter_mut().map(std::mem::take));
        laid.paras.extend(r.tail.drain(pi - r.snap.para..));
        laid.merged_notes = std::mem::take(&mut r.merged_notes);
        snaps.extend(r.later.drain(k..));
        *heads = r.heads_out.clone();
        // What the paragraphs before `pi` are known by now, and the earlier
        // knowledge of the rest.
        let sec = &doc.sections[i];
        let known_before = r.known.len();
        r.known
            .extend(sec.paras[known_before..pi].iter().map(Known::of));
        let skip = pi - r.same_from;
        r.known
            .extend(std::mem::take(&mut r.known_tail).into_iter().skip(skip));
        self.taken.1 += 1;
        self.taken.2 += sec.paras.len() - pi;
        let page = met - 1 - first;
        let decorations = r.later_decorations.split_off(page - r.from);
        Some((page, decorations))
    }

    /// The furniture state the section ended with in its earlier layout.
    pub(crate) fn ended(r: &Resumed) -> &crate::furniture::State {
        &r.furniture_out
    }

    /// Decorates every page again from what the run recorded, after the
    /// total page count changed: false, with nothing changed, when a
    /// section was not recorded.
    #[inline(never)]
    pub(crate) fn redecorate(
        &mut self,
        doc: &Document,
        styles: &mut Styles<'_>,
        pages: &mut [crate::Page],
        sections: &[crate::SectionLayout],
    ) -> bool {
        if self.out.len() != doc.sections.len() || self.out.iter().any(Option::is_none) {
            return false;
        }
        for (i, r) in self.out.iter_mut().enumerate() {
            let r = r.as_mut().expect("checked above");
            let (first, n) = (r.entry.first_page, r.pages);
            let Some(laid) = sections.get(i) else {
                return false;
            };
            if first + n > pages.len()
                || (r.decorations.len() != n && doc.sections[i].definition().is_some())
            {
                return false;
            }
            for (d, page) in r.decorations.iter().zip(&mut pages[first..first + n]) {
                d.strip(page);
            }
            let mut state = r.entry.furniture.clone();
            let mut decorations = Vec::with_capacity(n);
            crate::furniture::decorate(
                doc,
                i as u32,
                &laid.paras,
                &mut pages[first..first + n],
                styles,
                &mut state,
                crate::paginate::default_tab(&doc.sections[i]),
                (0, n, true),
                Some(&mut decorations),
            );
            r.decorations = decorations;
            r.entry.total_pages = styles.total_pages;
            r.furniture_out = state;
        }
        true
    }

    /// Records section `i` as just laid out.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn keep(
        &mut self,
        doc: &Document,
        i: usize,
        start: (crate::heads::Heads, crate::furniture::State, usize, u32),
        known: Vec<Known>,
        heads_out: &crate::heads::Heads,
        furniture_out: &crate::furniture::State,
        pages: usize,
        snaps: Option<Vec<crate::paginate::Snap>>,
        decorations: Vec<crate::furniture::Decoration>,
    ) {
        self.unchanged = false;
        let sec = &doc.sections[i];
        let mut known = known;
        known.extend(
            sec.paras[known.len().min(sec.paras.len())..]
                .iter()
                .map(Known::of),
        );
        let (heads, furniture, first_page, total_pages) = start;
        self.out[i] = Some(SectionRecord {
            entry: Start {
                heads,
                furniture,
                first_page,
                total_pages,
            },
            paras: known,
            heads_out: heads_out.clone(),
            furniture_out: furniture_out.clone(),
            pages,
            snaps,
            decorations,
            partial: false,
        });
    }

    /// Records section `i` as laid out up to where the layout stopped: its
    /// complete pages, its snapshots (the last one where the next layout
    /// goes on) and the decoration state of the page after them.
    #[allow(clippy::too_many_arguments)]
    #[inline(never)]
    pub(crate) fn keep_partial(
        &mut self,
        doc: &Document,
        i: usize,
        start: (crate::heads::Heads, crate::furniture::State, usize, u32),
        known: Vec<Known>,
        pages: usize,
        snaps: Vec<crate::paginate::Snap>,
        mut decorations: Vec<crate::furniture::Decoration>,
        furniture: &crate::furniture::State,
    ) {
        decorations.push(crate::furniture::Decoration {
            state: furniture.clone(),
            frames: 0,
            marks: 0,
        });
        self.keep(
            doc,
            i,
            start,
            known,
            &crate::heads::Heads::default(),
            furniture,
            pages,
            Some(snaps),
            decorations,
        );
        if let Some(r) = self.out[i].as_mut() {
            r.partial = true;
        }
    }
}
