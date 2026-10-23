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
}

/// The paragraphs an edit left changed: a list and a range in it.
pub type Changed = (ListPath, u32, u32);

/// A document with its undo and redo stacks of operation groups.
pub struct Editor {
    pub doc: hm_model::Document,
    undo: Vec<Group>,
    redo: Vec<Group>,
    next_id: u64,
    /// Ranges the last transaction, undo or redo changed; `None` when
    /// unknown (after a restore).
    changed: Option<Vec<Changed>>,
    /// Host data the last undo or redo returned to.
    returned: Option<String>,
    /// What the last layout keeps for the next one.
    cache: hm_layout::LayoutCache,
}

impl Editor {
    pub fn new(doc: hm_model::Document) -> Self {
        Self {
            doc,
            undo: Vec::new(),
            redo: Vec::new(),
            next_id: 1,
            changed: Some(Vec::new()),
            returned: None,
            cache: hm_layout::LayoutCache::new(),
        }
    }

    /// Lays the document out again, formatting only the paragraphs edits
    /// changed since the last layout.
    pub fn layout(&mut self, fonts: &hm_fonts::FontSystem) -> hm_layout::DocLayout {
        hm_layout::layout_cached(
            &self.doc,
            fonts,
            hm_layout::Options::default(),
            &mut self.cache,
        )
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
        let tables = self.doc.info.clone();
        let mut group = Group {
            id: self.next_id,
            ops: Vec::new(),
            before,
            after: None,
        };
        let mut changed = Vec::new();
        let mut sources = Vec::new();
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
                    group.ops.push((op, inverse));
                    sources.push(src);
                }
            }
            Ok(())
        })();
        if let Err(e) = result {
            for ((_, inverse), src) in group.ops.iter().zip(sources).rev() {
                apply(&mut self.doc, inverse).expect("inverse of an applied operation");
                src.restore(&mut self.doc);
            }
            self.doc.info = tables;
            return Err(e);
        }
        if group.ops.is_empty() {
            self.doc.info = tables;
            return Ok(false);
        }
        self.changed = Some(changed);
        self.returned = None;
        self.next_id += 1;
        self.undo.push(group);
        self.redo.clear();
        Ok(true)
    }

    /// Reverts the last group; false when there is none.
    pub fn undo(&mut self) -> Result<bool, EditError> {
        let Some(group) = self.undo.pop() else {
            return Ok(false);
        };
        let mut changed = Vec::new();
        for (_, inverse) in group.ops.iter().rev() {
            changed.push(span_of(&apply(&mut self.doc, inverse)?));
        }
        self.changed = Some(changed);
        self.returned = group.before.clone();
        self.redo.push(group);
        Ok(true)
    }

    /// Applies the last undone group again; false when there is none.
    pub fn redo(&mut self) -> Result<bool, EditError> {
        let Some(group) = self.redo.pop() else {
            return Ok(false);
        };
        let mut again = Vec::with_capacity(group.ops.len());
        let mut changed = Vec::new();
        for (op, _) in group.ops {
            let inverse = apply(&mut self.doc, &op)?;
            changed.push(span_of(&inverse));
            again.push((op, inverse));
        }
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
        self.undo.last().map_or(0, |g| g.id)
    }

    /// The document and its history as they are now. Unchanged paragraphs
    /// share their source bytes, so layouts after [`Editor::restore`] reuse
    /// what they kept.
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            doc: self.doc.clone(),
            undo: self.undo.clone(),
            redo: self.redo.clone(),
        }
    }

    /// Puts back a snapshot's document and history.
    pub fn restore(&mut self, s: Snapshot) {
        self.doc = s.doc;
        self.undo = s.undo;
        self.redo = s.redo;
        self.changed = None;
        self.returned = None;
    }

    /// Undo and redo steps available.
    pub fn history(&self) -> (usize, usize) {
        (self.undo.len(), self.redo.len())
    }
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
