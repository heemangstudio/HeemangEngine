//! Edit operations, undo and redo, and scripted edits.

pub mod lines;
pub mod ops;
pub mod script;

pub use ops::{CellStep, EditError, ListPath, Op, Pos, apply};

/// A document with its undo and redo stacks.
pub struct Editor {
    pub doc: hm_model::Document,
    undo: Vec<(Op, Op)>,
    redo: Vec<(Op, Op)>,
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

    /// Paragraphs reused and formatted by the last layout.
    pub fn layout_stats(&self) -> (u64, u64) {
        self.cache.stats()
    }

    /// Applies an operation and records it for undo.
    pub fn apply(&mut self, op: Op) -> Result<(), EditError> {
        let inverse = apply(&mut self.doc, &op)?;
        self.undo.push((op, inverse));
        self.redo.clear();
        Ok(())
    }

    /// Reverts the last operation; false when there is none.
    pub fn undo(&mut self) -> Result<bool, EditError> {
        let Some((op, inverse)) = self.undo.pop() else {
            return Ok(false);
        };
        apply(&mut self.doc, &inverse)?;
        self.redo.push((op, inverse));
        Ok(true)
    }

    /// Applies the last undone operation again.
    pub fn redo(&mut self) -> Result<bool, EditError> {
        let Some((op, _)) = self.redo.pop() else {
            return Ok(false);
        };
        let inverse = apply(&mut self.doc, &op)?;
        self.undo.push((op, inverse));
        Ok(true)
    }
}
