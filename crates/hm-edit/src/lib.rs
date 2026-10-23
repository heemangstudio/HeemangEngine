//! Edit operations, undo and redo, and scripted edits.

pub mod lines;
pub mod ops;
pub mod script;

pub use ops::{EditError, ListPath, Op, Pos, Step, apply};

/// Operations applied together, each with its inverse.
type Group = Vec<(Op, Op)>;

/// A document with its history, kept to go back to.
#[derive(Clone)]
pub struct Snapshot {
    doc: hm_model::Document,
    undo: Vec<Group>,
    redo: Vec<Group>,
}

/// A document with its undo and redo stacks of operation groups.
pub struct Editor {
    pub doc: hm_model::Document,
    undo: Vec<Group>,
    redo: Vec<Group>,
    /// What the last layout keeps for the next one.
    cache: hm_layout::LayoutCache,
}

impl Editor {
    pub fn new(doc: hm_model::Document) -> Self {
        Self {
            doc,
            undo: Vec::new(),
            redo: Vec::new(),
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
        let mut group: Group = Vec::with_capacity(ops.len());
        for (i, op) in ops.into_iter().enumerate() {
            match apply(&mut self.doc, &op) {
                Ok(inverse) => group.push((op, inverse)),
                Err(e) => {
                    for (_, inverse) in group.iter().rev() {
                        // Inverses of operations just applied always apply.
                        let _ = apply(&mut self.doc, inverse);
                    }
                    return Err((i, e));
                }
            }
        }
        if !group.is_empty() {
            self.undo.push(group);
            self.redo.clear();
        }
        Ok(())
    }

    /// Reverts the last group; false when there is none.
    pub fn undo(&mut self) -> Result<bool, EditError> {
        let Some(group) = self.undo.pop() else {
            return Ok(false);
        };
        for (_, inverse) in group.iter().rev() {
            apply(&mut self.doc, inverse)?;
        }
        self.redo.push(group);
        Ok(true)
    }

    /// Applies the last undone group again; false when there is none.
    pub fn redo(&mut self) -> Result<bool, EditError> {
        let Some(group) = self.redo.pop() else {
            return Ok(false);
        };
        let mut again = Vec::with_capacity(group.len());
        for (op, _) in group {
            let inverse = apply(&mut self.doc, &op)?;
            again.push((op, inverse));
        }
        self.undo.push(again);
        Ok(true)
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
    }

    /// Undo and redo steps available.
    pub fn history(&self) -> (usize, usize) {
        (self.undo.len(), self.redo.len())
    }
}
