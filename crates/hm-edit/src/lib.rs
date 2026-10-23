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
}

impl Editor {
    pub fn new(doc: hm_model::Document) -> Self {
        Self {
            doc,
            undo: Vec::new(),
            redo: Vec::new(),
        }
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
