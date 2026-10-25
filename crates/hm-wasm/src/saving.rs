//! Same-format saving, conversion and saved-state tracking.
use super::*;

#[wasm_bindgen]
impl Doc {
    /// The document written as `format` (`"hwp5"` or `"hwpx"`): in its
    /// own format as [`Doc::serialize`] writes it, in the other as a new
    /// file encoded from the model alone. The open document keeps its
    /// format; `markSaved` clears the dirty state.
    #[wasm_bindgen(js_name = serializeAs)]
    pub fn serialize_as(&mut self, format: &str, mark_saved: bool) -> Result<Vec<u8>> {
        let target = match format {
            "hwp5" => Format::Hwp5,
            "hwpx" => Format::Hwpx,
            _ => {
                return Err(EngineError::new(
                    "UNSUPPORTED_FORMAT",
                    format!("cannot write {format}"),
                ));
            }
        };
        let d = self.inner()?;
        if d.editor.doc.meta.format == target {
            return self.serialize(mark_saved);
        }
        if d.editor.doc.meta.format == Format::Hwp3 {
            return Err(EngineError::new(
                "UNSUPPORTED_FORMAT",
                "HWP 3 documents cannot be saved yet",
            ));
        }
        let doc = hm_edit::lines::prepare_conversion_with_fonts(
            &d.editor.doc,
            &d.layout,
            target,
            Some(&d.fonts.borrow()),
        )
        .map_err(|e| EngineError::new("SAVE_FAILED", e))?;
        let bytes = match target {
            Format::Hwpx => hm_hwpx::write_new(&doc).map_err(|e| e.to_string()),
            _ => hm_hwp5::write_new(&doc).map_err(|e| e.to_string()),
        }
        .map_err(|e| EngineError::new("SAVE_FAILED", e))?;
        if mark_saved {
            d.saved_epoch = d.content_epoch;
        }
        Ok(bytes)
    }

    /// The document in its own format, edited paragraphs carrying line
    /// segments from our layout. `markSaved` clears the dirty state; the
    /// content epoch never changes.
    pub fn serialize(&mut self, mark_saved: bool) -> Result<Vec<u8>> {
        let d = self.inner()?;
        let mut doc = d.editor.doc.clone();
        hm_edit::lines::fill(&mut doc, &d.layout);
        let bytes = match doc.meta.format {
            Format::Hwpx => hm_hwpx::write(&doc).map_err(|e| EngineError::new("SAVE_FAILED", e))?,
            Format::Hwp5 => hm_hwp5::write(&doc).map_err(|e| EngineError::new("SAVE_FAILED", e))?,
            Format::Hwp3 => {
                return Err(EngineError::new(
                    "UNSUPPORTED_FORMAT",
                    "HWP 3 documents cannot be saved yet",
                ));
            }
        };
        if mark_saved {
            d.saved_epoch = d.content_epoch;
            d.saved_history = d.editor.history_id();
        }
        Ok(bytes)
    }

    /// Marks the document saved as it was at `expectedEpoch` (after the
    /// host wrote what `serialize(false)` gave it); `STALE_EPOCH` when it
    /// changed since. Changes nothing else.
    #[wasm_bindgen(js_name = markSaved)]
    pub fn mark_saved(&mut self, expected_epoch: u32) -> Result<()> {
        let d = self.raw()?;
        if expected_epoch != d.content_epoch {
            return Err(EngineError::new(
                "STALE_EPOCH",
                format!(
                    "expected epoch {expected_epoch}, the document is at {}",
                    d.content_epoch
                ),
            ));
        }
        d.saved_epoch = d.content_epoch;
        d.saved_history = d.editor.history_id();
        Ok(())
    }
}
