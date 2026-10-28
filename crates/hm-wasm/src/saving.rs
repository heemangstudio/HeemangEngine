//! Same-format saving, conversion and saved-state tracking.
use super::*;

#[wasm_bindgen]
impl Doc {
    /// An explicit HWPX editor export. Preview images are stored without a
    /// second compression layer; native serialize/serializeAs keep their
    /// source-preserving contract. Export never acknowledges a file write.
    #[wasm_bindgen(js_name = exportHwpx)]
    pub fn export_hwpx(&mut self) -> Result<Vec<u8>> {
        hm_hwpx::store_preview_images(self.serialize_as("hwpx", false)?)
            .map_err(|e| EngineError::new("SAVE_FAILED", e.to_string()))
    }

    /// The document written as `format` (`"hwp5"`, `"hwpx"` or `"hml"`): in its
    /// own format as [`Doc::serialize`] writes it, in the other as a new
    /// file encoded from the model alone. An HWP 3 document, which has no
    /// writer, is always converted; as `"hwp5"` it is what
    /// [`Doc::serialize`] writes. The open document keeps its format;
    /// `markSaved` clears the dirty state.
    #[wasm_bindgen(js_name = serializeAs)]
    pub fn serialize_as(&mut self, format: &str, mark_saved: bool) -> Result<Vec<u8>> {
        let target = match format {
            "hwp5" => Format::Hwp5,
            "hwpx" => Format::Hwpx,
            "hml" => Format::Hml,
            _ => {
                return Err(EngineError::new(
                    "UNSUPPORTED_FORMAT",
                    format!("cannot write {format}"),
                ));
            }
        };
        let d = self.inner()?;
        let own = d.editor.doc.meta.format;
        if own == target || (own == Format::Hwp3 && target == Format::Hwp5) {
            return self.serialize(mark_saved);
        }
        let bytes = d.converted(target)?;
        if mark_saved {
            d.saved_epoch = d.content_epoch;
            d.saved_history = d.editor.history_id();
        }
        Ok(bytes)
    }

    /// The document in its own format, edited paragraphs carrying line
    /// segments from our layout. An HWP 3 document is written as a new
    /// HWP 5 file, the `.hwp` format this engine writes. `markSaved`
    /// clears the dirty state; the content epoch never changes.
    pub fn serialize(&mut self, mark_saved: bool) -> Result<Vec<u8>> {
        let d = self.inner()?;
        let bytes = match d.editor.doc.meta.format {
            Format::Hwp3 => d.converted(Format::Hwp5)?,
            _ => {
                let mut doc = d.editor.doc.clone();
                hm_edit::lines::fill(&mut doc, &d.layout);
                match doc.meta.format {
                    Format::Hwpx => hm_hwpx::write(&doc).map_err(|e| e.to_string()),
                    Format::Hml => hm_hml::write(&doc).map_err(|e| e.to_string()),
                    _ => hm_hwp5::write(&doc).map_err(|e| e.to_string()),
                }
                .map_err(|e| EngineError::new("SAVE_FAILED", e))?
            }
        };
        if mark_saved {
            d.saved_epoch = d.content_epoch;
            d.saved_history = d.editor.history_id();
        }
        Ok(bytes)
    }

    /// Reopens an HWP export and compares its typed properties and resource
    /// values with the model that was written, excluding source/layout caches.
    #[wasm_bindgen(js_name = verifyHwp)]
    pub fn verify_hwp(&mut self) -> Result<String> {
        let bytes = self.serialize_as("hwp5", false)?;
        let d = self.raw()?;
        let expected = if d.editor.doc.meta.format == Format::Hwp5 {
            d.editor.doc.clone()
        } else {
            d.converted_model(Format::Hwp5)?
        };
        let losses = merge_document::roundtrip_losses(&expected, &bytes)?;
        Ok(json(
            &serde_json::json!({"ok":losses.is_empty(),"recovered":true,"structureMatches":losses.is_empty(),"serializationLosses":losses,"bytes":bytes.len()}),
        ))
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

impl Inner {
    /// The document as a new file in `target`, another format than its
    /// own, with the line membership of the current layout.
    fn converted_model(&self, target: Format) -> Result<hm_model::Document> {
        let doc = if self.editor.doc.meta.format == Format::Hwp3 {
            // HWP 3 import already builds modern typed records, including line
            // segments for furniture that may never appear on a current page.
            let mut doc = self.editor.doc.clone();
            hm_edit::lines::fill(&mut doc, &self.layout);
            doc.convert_to(target);
            doc
        } else {
            let mut doc = hm_edit::lines::prepare_conversion_with_fonts(
                &self.editor.doc,
                &self.layout,
                target,
                Some(&self.fonts.borrow()),
            )
            .map_err(|e| EngineError::new("SAVE_FAILED", e))?;
            hm_edit::lines::fit_edited_tables(&mut doc, &self.editor.doc, &self.layout);
            doc
        };
        Ok(doc)
    }
    fn converted(&self, target: Format) -> Result<Vec<u8>> {
        let doc = self.converted_model(target)?;
        match target {
            Format::Hwpx => hm_hwpx::write_new(&doc).map_err(|e| e.to_string()),
            Format::Hml => hm_hml::write(&doc).map_err(|e| e.to_string()),
            _ => hm_hwp5::write_new(&doc).map_err(|e| e.to_string()),
        }
        .map_err(|e| EngineError::new("SAVE_FAILED", e))
    }
}
