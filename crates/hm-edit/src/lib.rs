//! Edit operations, undo and redo, and scripted edits.

pub mod export;
pub mod lines;
pub mod ops;
pub mod script;

pub use ops::{EditError, ListPath, Op, Pos, Step, apply};

/// Operations applied together, each with its inverse, and what the host
/// asked to keep with them.
#[derive(Clone)]
struct Group {
    /// Names this history position: the group on top of the undo stack.
    id: u64,
    ops: Vec<(Op, Op)>,
    sources: Vec<ops::Sources>,
    /// Definition edits can bracket later derived shapes. Keep the complete
    /// table endpoints so redo also restores those later shape IDs.
    infos: Option<(Box<hm_model::DocInfo>, Box<hm_model::DocInfo>)>,
    /// Host data (a selection) from before and after the group.
    before: Option<String>,
    after: Option<String>,
}

/// A document with its history, kept to go back to.
#[derive(Clone)]
pub struct Snapshot {
    doc: hm_model::Document,
    undo: Vec<Group>,
    redo: Vec<Group>,
    /// The history position under the kept undo steps.
    base: u64,
    definitions: u64,
}

/// The paragraphs an edit left changed: a list and a range in it.
pub type Changed = (ListPath, u32, u32);

/// A document with its undo and redo stacks of operation groups.
pub struct Editor {
    pub doc: hm_model::Document,
    undo: Vec<Group>,
    redo: Vec<Group>,
    /// The history position with every kept undo step undone: 0, or the id
    /// of the newest group a history limit dropped, so undoing what is left
    /// does not read as the state before it.
    base: u64,
    next_id: u64,
    /// Ranges the last transaction, undo or redo changed; `None` when
    /// unknown (after a restore).
    changed: Option<Vec<Changed>>,
    /// Host data the last undo or redo returned to.
    returned: Option<String>,
    /// What the last layout keeps for the next one.
    cache: hm_layout::LayoutCache,
    /// The body paragraphs changes reached since the last layout, as
    /// [`hm_layout::incr::Touched`]; `None` when not known.
    touched: Option<Vec<hm_layout::incr::Touched>>,
    /// Names the meaning of the document's definition tables: a new value
    /// whenever an edit replaces them rather than adding to them, or the
    /// document is replaced. Kept layouts hold definitions by ID, so they
    /// are dropped when it changes.
    definitions: u64,
    next_definitions: u64,
    /// The last call committed the group on top of the undo stack, which
    /// [`Editor::amend`] may still extend.
    committed: bool,
    /// The group just committed added, removed or reordered footnotes or
    /// endnotes.
    notes_moved: bool,
    /// The group just committed changed what caption numbers count.
    numbers_moved: bool,
    /// Undo steps kept, all when `None`. With 0 nothing can be undone; the
    /// group just committed stays on the stack only for [`Editor::amend`].
    history_limit: Option<usize>,
}

impl Editor {
    pub fn new(doc: hm_model::Document) -> Self {
        Self {
            doc,
            undo: Vec::new(),
            redo: Vec::new(),
            base: 0,
            next_id: 1,
            changed: Some(Vec::new()),
            returned: None,
            cache: hm_layout::LayoutCache::new(),
            touched: None,
            definitions: 0,
            next_definitions: 1,
            committed: false,
            notes_moved: false,
            numbers_moved: false,
            history_limit: None,
        }
    }

    /// Keeps at most `limit` undo steps, dropping older ones now and as
    /// edits come; `None` keeps them all. A host that keeps its own undo
    /// passes 0: undo and redo then do nothing and snapshots carry no
    /// history, so neither grows with the session.
    pub fn set_history_limit(&mut self, limit: Option<usize>) {
        self.history_limit = limit;
        self.trim_history();
    }

    fn trim_history(&mut self) {
        let Some(limit) = self.history_limit else {
            return;
        };
        let keep = limit.max(1);
        if self.undo.len() > keep {
            let dropped = self.undo.len() - keep;
            self.base = self.undo[dropped - 1].id;
            self.undo.drain(..dropped);
        }
        if self.redo.len() > limit {
            self.redo.drain(..self.redo.len() - limit);
        }
    }

    /// History is off: nothing to undo or redo.
    fn no_history(&self) -> bool {
        self.history_limit == Some(0)
    }

    /// The definition tables now mean something else: kept layouts no
    /// longer apply.
    fn redefined(&mut self) {
        self.definitions = self.next_definitions;
        self.next_definitions += 1;
        self.forget_layout();
    }

    /// Lays the document out again, formatting only the paragraphs edits
    /// changed since the last layout.
    pub fn layout(&mut self, fonts: &hm_fonts::FontSystem) -> hm_layout::DocLayout {
        self.layout_with(fonts, hm_layout::Options::default())
    }

    /// [`Editor::layout`] with explicit [`hm_layout::Options`].
    pub fn layout_with(
        &mut self,
        fonts: &hm_fonts::FontSystem,
        options: hm_layout::Options,
    ) -> hm_layout::DocLayout {
        self.touched = Some(Vec::new());
        hm_layout::layout_cached(&self.doc, fonts, options, &mut self.cache)
    }

    /// [`Editor::layout_with`] given the layout it made last, which may
    /// lend the new layout the pages and paragraph layouts edits since then
    /// left as they were.
    pub fn relayout_with(
        &mut self,
        fonts: &hm_fonts::FontSystem,
        options: hm_layout::Options,
        last: hm_layout::DocLayout,
    ) -> hm_layout::DocLayout {
        self.relayout_until(fonts, options, last, None)
    }

    /// [`Editor::relayout_with`], stopping once more than `pages` pages
    /// are laid out when given: the result holds the complete pages so far
    /// ([`hm_layout::DocLayout::complete`] is false), and the next layout
    /// goes on from where it stopped.
    pub fn relayout_until(
        &mut self,
        fonts: &hm_fonts::FontSystem,
        options: hm_layout::Options,
        last: hm_layout::DocLayout,
        pages: Option<usize>,
    ) -> hm_layout::DocLayout {
        let touched = self.touched.replace(Vec::new());
        hm_layout::incr::relayout_cached(
            &self.doc,
            fonts,
            options,
            &mut self.cache,
            Some(last),
            touched.as_deref(),
            pages,
        )
    }

    /// Notes that the paragraph `para` of `list` changed outside
    /// [`Editor::transact`], undo and redo, as a host applying operations
    /// to [`Editor::doc`] directly must, so later layouts lay it out again.
    pub fn touch(&mut self, list: &ListPath, para: u32) {
        let t = touch_of(&self.doc, &(list.clone(), para, 1));
        self.note(vec![t]);
    }

    /// Adds what applied operations reached, each taken right after it
    /// applied ([`touch_of`]).
    fn note(&mut self, touched: Vec<Option<hm_layout::incr::Touched>>) {
        let Some(all) = self.touched.as_mut() else {
            return;
        };
        for t in touched {
            match t {
                Some(t) => all.push(t),
                None => {
                    self.touched = None;
                    return;
                }
            }
        }
    }

    /// What the last layout took from the one before without paginating
    /// it again ([`hm_layout::LayoutCache::taken`]).
    pub fn layout_taken(&self) -> (usize, usize, usize) {
        self.cache.taken()
    }

    /// Drops what earlier layouts kept, for when the fonts change.
    pub fn forget_layout(&mut self) {
        self.cache = hm_layout::LayoutCache::new();
    }

    /// Paragraphs reused and formatted by the last layout.
    pub fn layout_stats(&self) -> (u64, u64) {
        self.cache.stats()
    }

    /// Applies an operation and records it for undo.
    pub fn apply(&mut self, op: Op) -> Result<(), EditError> {
        self.transact(vec![op]).map_err(|(_, e)| e)
    }

    /// Applies operations in order as one undo step. When one fails, the
    /// ones before it are reverted and its index is returned with the error.
    pub fn transact(&mut self, ops: Vec<Op>) -> Result<(), (usize, EditError)> {
        self.transact_with(ops, None, None)
    }

    /// [`Editor::transact`], keeping host data from before and after the
    /// step to return to on undo and redo.
    pub fn transact_with(
        &mut self,
        ops: Vec<Op>,
        before: Option<String>,
        after: Option<String>,
    ) -> Result<(), (usize, EditError)> {
        self.transact_mapped(ops, before, |_, op| Ok((vec![op], after.clone())))
            .map(|_| ())
    }

    /// Translates each request against the result of the preceding one,
    /// then commits all operations as one undo group. Failures restore
    /// paragraphs, container source spans and document tables without
    /// touching history or layout caches. The translator only changes
    /// document tables; paragraph changes must be returned as operations.
    pub fn transact_mapped<T, E: From<(usize, EditError)>>(
        &mut self,
        requests: impl IntoIterator<Item = T>,
        before: Option<String>,
        mut translate: impl FnMut(&mut hm_model::Document, T) -> Result<(Vec<Op>, Option<String>), E>,
    ) -> Result<bool, E> {
        self.committed = false;
        let tables = self.doc.info.clone();
        let bins = self.doc.bins.clone();
        let mut group = Group {
            id: self.next_id,
            ops: Vec::new(),
            sources: Vec::new(),
            infos: None,
            before,
            after: None,
        };
        let mut changed = Vec::new();
        let mut touched = Vec::new();
        let mut notes_moved = false;
        let mut numbers_moved = false;
        let result = (|| {
            for request in requests {
                let (ops, after) = translate(&mut self.doc, request)?;
                if after.is_some() {
                    group.after = after;
                }
                for op in ops {
                    let i = group.ops.len();
                    let src = ops::Sources::capture(&self.doc, &op).map_err(|e| E::from((i, e)))?;
                    let inverse = apply(&mut self.doc, &op).map_err(|e| E::from((i, e)))?;
                    changed.push(span_of(&inverse));
                    touched.push(touch_of(&self.doc, changed.last().expect("pushed")));
                    notes_moved = notes_moved || moves_notes(&self.doc, &op, &inverse);
                    numbers_moved = numbers_moved || moves_numbers(&self.doc, &op, &inverse);
                    group.ops.push((op, inverse));
                    group.sources.push(src);
                }
            }
            Ok(())
        })();
        if let Err(e) = result {
            for ((_, inverse), src) in group.ops.iter().zip(group.sources.iter()).rev() {
                apply(&mut self.doc, inverse).expect("inverse of an applied operation");
                src.clone().restore(&mut self.doc);
            }
            self.doc.info = tables;
            self.doc.bins = bins;
            return Err(e);
        }
        if group.ops.is_empty() {
            self.doc.info = tables;
            self.doc.bins = bins;
            return Ok(false);
        }
        if group
            .ops
            .iter()
            .any(|(op, _)| matches!(op, Op::SetDocInfo { .. }))
        {
            group.infos = Some((Box::new(tables), Box::new(self.doc.info.clone())));
        }
        if redefines(&group) {
            self.redefined();
        }
        self.note(touched);
        self.changed = Some(changed);
        self.returned = None;
        self.next_id += 1;
        self.undo.push(group);
        self.redo.clear();
        self.trim_history();
        self.committed = true;
        self.notes_moved = notes_moved;
        self.numbers_moved = numbers_moved;
        Ok(true)
    }

    /// Whether the group just committed added, removed or reordered notes
    /// (anywhere, nested lists included), so their numbers need renewing.
    pub fn notes_moved(&self) -> bool {
        self.committed && self.notes_moved
    }

    /// Whether the group just committed added, removed or reordered
    /// pictures, tables or equations numbered as such, changed their
    /// caption numbers, or changed where those numbers start.
    pub fn numbers_moved(&self) -> bool {
        self.committed && self.numbers_moved
    }

    /// Applies `ops` as part of the group just committed: work its result
    /// implies once laid out (page-local note numbers). Undo reverts them
    /// with the group, redo reapplies them. False, with nothing applied,
    /// when no group was just committed; a failure reverts the ones applied.
    pub fn amend(&mut self, ops: Vec<Op>) -> Result<bool, (usize, EditError)> {
        if ops.is_empty() || !self.committed {
            return Ok(false);
        }
        let mut done: Vec<(Op, Op)> = Vec::new();
        let mut sources = Vec::new();
        let mut changed = Vec::new();
        let mut touched = Vec::new();
        for (i, op) in ops.into_iter().enumerate() {
            let step = ops::Sources::capture(&self.doc, &op)
                .and_then(|src| apply(&mut self.doc, &op).map(|inverse| (src, inverse)));
            match step {
                Ok((src, inverse)) => {
                    changed.push(span_of(&inverse));
                    touched.push(touch_of(&self.doc, changed.last().expect("pushed")));
                    done.push((op, inverse));
                    sources.push(src);
                }
                Err(e) => {
                    for ((_, inverse), src) in done.iter().zip(sources).rev() {
                        apply(&mut self.doc, inverse).expect("inverse of an applied operation");
                        src.restore(&mut self.doc);
                    }
                    return Err((i, e));
                }
            }
        }
        self.note(touched);
        let group = self.undo.last_mut().expect("a committed group");
        group.ops.extend(done);
        group.sources.extend(sources);
        if let Some(c) = &mut self.changed {
            c.extend(changed);
        }
        Ok(true)
    }

    /// Reverts the last group; false when there is none.
    pub fn undo(&mut self) -> Result<bool, EditError> {
        self.committed = false;
        if self.no_history() {
            return Ok(false);
        }
        let Some(group) = self.undo.pop() else {
            return Ok(false);
        };
        let mut changed = Vec::new();
        let mut touched = Vec::new();
        for ((_, inverse), src) in group.ops.iter().zip(&group.sources).rev() {
            let inverse = apply(&mut self.doc, inverse).inspect_err(|_| self.touched = None)?;
            changed.push(span_of(&inverse));
            touched.push(touch_of(&self.doc, changed.last().expect("pushed")));
            src.clone().restore(&mut self.doc);
        }
        if let Some((before, _)) = &group.infos {
            self.doc.info = *before.clone();
        }
        if redefines(&group) {
            self.redefined();
        }
        self.note(touched);
        self.changed = Some(changed);
        self.returned = group.before.clone();
        self.redo.push(group);
        Ok(true)
    }

    /// Applies the last undone group again; false when there is none.
    pub fn redo(&mut self) -> Result<bool, EditError> {
        self.committed = false;
        if self.no_history() {
            return Ok(false);
        }
        let Some(group) = self.redo.pop() else {
            return Ok(false);
        };
        let mut again = Vec::with_capacity(group.ops.len());
        let mut changed = Vec::new();
        let mut touched = Vec::new();
        for (op, _) in group.ops {
            let inverse = apply(&mut self.doc, &op).inspect_err(|_| self.touched = None)?;
            changed.push(span_of(&inverse));
            touched.push(touch_of(&self.doc, changed.last().expect("pushed")));
            again.push((op, inverse));
        }
        if let Some((_, after)) = &group.infos {
            self.doc.info = *after.clone();
        }
        if again.iter().any(|(op, _)| redefinition(op)) {
            self.redefined();
        }
        self.note(touched);
        self.changed = Some(changed);
        self.returned = group.after.clone();
        self.undo.push(Group {
            ops: again,
            ..group
        });
        Ok(true)
    }

    /// The paragraph ranges the last transaction, undo or redo changed,
    /// as they are now; `None` after a restore.
    pub fn changed(&self) -> Option<&[Changed]> {
        self.changed.as_deref()
    }

    /// The host data the last undo (from before its group) or redo (from
    /// after it) returned to.
    pub fn returned(&self) -> Option<&str> {
        self.returned.as_deref()
    }

    /// The history position: the id of the group on top of the undo
    /// stack, 0 with none. Undo and redo return to earlier ids.
    pub fn history_id(&self) -> u64 {
        self.undo.last().map_or(self.base, |g| g.id)
    }

    /// The document and its history as they are now. Unchanged paragraphs
    /// share their source bytes, so layouts after [`Editor::restore`] reuse
    /// what they kept.
    pub fn snapshot(&self) -> Snapshot {
        let history = !self.no_history();
        Snapshot {
            doc: self.doc.clone(),
            undo: if history {
                self.undo.clone()
            } else {
                Vec::new()
            },
            redo: if history {
                self.redo.clone()
            } else {
                Vec::new()
            },
            base: if history {
                self.base
            } else {
                self.history_id()
            },
            definitions: self.definitions,
        }
    }

    /// Puts in another document with an empty history. Ids go on from this
    /// history's, so a position from before never reads as one after.
    pub fn replace_document(&mut self, doc: hm_model::Document) {
        self.committed = false;
        self.doc = doc;
        self.undo.clear();
        self.redo.clear();
        self.base = 0;
        self.changed = None;
        self.touched = None;
        self.returned = None;
        // Its definition IDs mean other things.
        self.redefined();
    }

    /// Puts back a snapshot's document and history.
    pub fn restore(&mut self, s: Snapshot) {
        self.committed = false;
        self.doc = s.doc;
        self.undo = s.undo;
        self.redo = s.redo;
        self.base = s.base;
        self.trim_history();
        self.changed = None;
        self.touched = None;
        self.returned = None;
        if s.definitions != self.definitions {
            self.forget_layout();
            self.definitions = s.definitions;
        }
    }

    /// Undo and redo steps available.
    pub fn history(&self) -> (usize, usize) {
        if self.no_history() {
            return (0, 0);
        }
        (self.undo.len(), self.redo.len())
    }
}

/// The body paragraphs an operation just applied reached, from what its
/// inverse says it changed: the first, and how many paragraphs end the
/// section after them. `None` when that is not known.
fn touch_of(doc: &hm_model::Document, span: &Changed) -> Option<hm_layout::incr::Touched> {
    let (list, start, count) = span;
    let n = doc.sections.get(list.section as usize)?.paras.len();
    let (first, end) = match list.steps.first() {
        Some(step) => (step.para as usize, step.para as usize + 1),
        None if *count != u32::MAX => (*start as usize, *start as usize + *count as usize),
        None => return None,
    };
    Some(hm_layout::incr::Touched {
        section: list.section,
        first,
        after: n.saturating_sub(end),
    })
}

/// Whether an operation replaces definitions or document properties that
/// layouts read by ID or number, rather than adding to them.
fn redefinition(op: &Op) -> bool {
    matches!(op, Op::SetDocInfo { .. } | Op::SetProperties { .. })
}

/// Whether `op` (applied, with `inverse`) changed the sequence of notes in
/// the paragraphs it replaced. Typing and formatting never do.
fn moves_notes(doc: &hm_model::Document, op: &Op, inverse: &Op) -> bool {
    if !matches!(op, Op::Delete { .. } | Op::Replace { .. }) {
        return false;
    }
    let Op::Replace {
        list,
        start,
        count,
        paras: old,
    } = inverse
    else {
        return false;
    };
    let new = ops::list(doc, list)
        .and_then(|l| l.get(*start as usize..(*start + *count) as usize))
        .unwrap_or_default();
    let (mut a, mut b) = (Vec::new(), Vec::new());
    note_marks(old, &mut a);
    note_marks(new, &mut b);
    a != b
}

/// Every note (`(0 or 1 for an endnote, number)`) and note number restart
/// (`(2 or 3, number)`) in `paras`, nested lists included.
fn note_marks(paras: &[hm_model::Paragraph], out: &mut Vec<(u8, u32)>) {
    use hm_model::{ControlKind, NumberKind};
    for p in paras {
        for c in &p.controls {
            match &c.kind {
                ControlKind::Note(n) => out.push((u8::from(n.endnote), n.number)),
                ControlKind::NewNumber(n) => match n.kind() {
                    NumberKind::Footnote => out.push((2, n.number as u32)),
                    NumberKind::Endnote => out.push((3, n.number as u32)),
                    _ => {}
                },
                _ => {
                    for l in ops::control_lists(c) {
                        note_marks(l, out);
                    }
                }
            }
        }
    }
}

/// Whether `op` (applied, with `inverse`) changed what caption numbers
/// count in the paragraphs it replaced, or the start numbers.
fn moves_numbers(doc: &hm_model::Document, op: &Op, inverse: &Op) -> bool {
    if matches!(op, Op::SetProperties { .. }) {
        return true;
    }
    if !matches!(op, Op::Delete { .. } | Op::Replace { .. }) {
        return false;
    }
    let Op::Replace {
        list,
        start,
        count,
        paras: old,
    } = inverse
    else {
        return false;
    };
    let new = ops::list(doc, list)
        .and_then(|l| l.get(*start as usize..(*start + *count) as usize))
        .unwrap_or_default();
    let (mut a, mut b) = (Vec::new(), Vec::new());
    number_marks(old, &mut a);
    number_marks(new, &mut b);
    a != b
}

/// 0, 1 and 2 for figure, table and equation numbers.
fn number_slot(kind: hm_model::NumberKind) -> Option<u8> {
    use hm_model::NumberKind;
    match kind {
        NumberKind::Figure => Some(0),
        NumberKind::Table => Some(1),
        NumberKind::Equation => Some(2),
        _ => None,
    }
}

/// Every object numbered as a picture, table or equation (`(1..=3, 0)`),
/// figure, table and equation number (`(4..=6, number)`), restart of one
/// (`(7..=9, number)`) and section start number (`(10..=12, number)`) in
/// `paras`, nested lists included.
fn number_marks(paras: &[hm_model::Paragraph], out: &mut Vec<(u8, u32)>) {
    use hm_model::ControlKind;
    for p in paras {
        for c in &p.controls {
            if let Some(o) = c.object_common() {
                let k = (o.attr >> 26) & 7;
                if (1..=3).contains(&k) {
                    out.push((k as u8, 0));
                }
            }
            match &c.kind {
                ControlKind::AutoNumber(a) => {
                    if let Some(k) = number_slot(a.kind()) {
                        out.push((4 + k, a.number as u32));
                    }
                }
                ControlKind::NewNumber(n) => {
                    if let Some(k) = number_slot(n.kind()) {
                        out.push((7 + k, n.number as u32));
                    }
                }
                ControlKind::Section(s) => out.extend([
                    (10, s.figure_number as u32),
                    (11, s.table_number as u32),
                    (12, s.equation_number as u32),
                ]),
                _ => {}
            }
            for l in ops::control_lists(c) {
                number_marks(l, out);
            }
        }
    }
}

fn redefines(group: &Group) -> bool {
    group.ops.iter().any(|(op, _)| redefinition(op))
}

/// The range an applied operation's inverse covers: what the operation
/// left changed.
fn span_of(inverse: &Op) -> Changed {
    match inverse {
        Op::Replace {
            list, start, count, ..
        } => (list.clone(), *start, *count),
        // Inverses are always replacements; anything else changed all.
        _ => (ListPath::default(), 0, u32::MAX),
    }
}
