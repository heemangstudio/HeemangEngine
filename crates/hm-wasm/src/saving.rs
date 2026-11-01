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
                let doc = d.own_model();
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
        // What the writer was given, edited tables fitted to their layout.
        let expected = if d.editor.doc.meta.format == Format::Hwp5 {
            d.own_model()
        } else {
            d.converted_model(Format::Hwp5)?
        };
        let losses = crate::model_losses(&expected, &merge_document::open(&bytes)?);
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
    /// The current layout, or one made from nothing when it does not match
    /// the document: a save reads every list's lines from its layout, and
    /// an incremental layout can keep a list's lines from before an edit
    /// that does not show (a cell no page draws).
    fn fresh_layout(&self) -> hm_layout::DocLayout {
        hm_layout::layout_with(
            &self.editor.doc,
            &self.fonts.borrow(),
            crate::rendering::SCREEN,
        )
    }

    /// The document as its own format writes it: edited paragraphs carry the
    /// lines of the layout.
    fn own_model(&self) -> hm_model::Document {
        let mut doc = self.editor.doc.clone();
        drop_unused_bins(&mut doc, self.opened_bins);
        if hm_edit::lines::fill_lists(&mut doc, &self.layout, Some(&self.fonts.borrow())).is_err() {
            let layout = self.fresh_layout();
            doc = self.editor.doc.clone();
            drop_unused_bins(&mut doc, self.opened_bins);
            let _ = hm_edit::lines::fill_lists(&mut doc, &layout, Some(&self.fonts.borrow()));
        }
        doc
    }

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
            let convert = |layout: &hm_layout::DocLayout| {
                hm_edit::lines::prepare_conversion_with_fonts(
                    &self.editor.doc,
                    layout,
                    target,
                    Some(&self.fonts.borrow()),
                )
                .map(|mut doc| {
                    hm_edit::lines::fit_edited_tables(&mut doc, &self.editor.doc, layout);
                    doc
                })
            };
            match convert(&self.layout) {
                Ok(doc) => doc,
                Err(_) => {
                    convert(&self.fresh_layout()).map_err(|e| EngineError::new("SAVE_FAILED", e))?
                }
            }
        };
        let mut doc = doc;
        drop_unused_bins(&mut doc, self.opened_bins);
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

/// BinData ids the document uses: its pictures and OLE objects, picture
/// fills of drawings and border fills, picture bullets, nested lists and
/// section master pages included.
fn used_bins(doc: &hm_model::Document) -> std::collections::HashSet<u16> {
    use hm_model::{ControlKind, ObjectPart, Paragraph, Shape, ShapePart};
    fn shape(s: &Shape, out: &mut std::collections::HashSet<u16>) {
        if let Some(i) = s.component.style.as_ref().and_then(|s| s.fill.image) {
            out.insert(i.bin_id);
        }
        for p in &s.parts {
            match p {
                ShapePart::Picture(p) => {
                    out.insert(p.image.bin_id);
                }
                ShapePart::Ole(o) => {
                    out.insert(o.bin_id);
                }
                ShapePart::Child(c) => shape(c, out),
                _ => {}
            }
        }
    }
    fn walk(paras: &[Paragraph], out: &mut std::collections::HashSet<u16>) {
        for p in paras {
            for c in &p.controls {
                if let ControlKind::Object(o) = &c.kind {
                    for part in &o.parts {
                        if let ObjectPart::Shape(s) = part {
                            shape(s, out);
                        }
                    }
                }
                for list in hm_edit::ops::control_lists(c) {
                    walk(list, out);
                }
            }
        }
    }
    let mut out = std::collections::HashSet::new();
    for b in &doc.info.border_fills {
        if let Some(i) = b.fill.as_ref().and_then(|f| f.image) {
            out.insert(i.bin_id);
        }
    }
    // A picture bullet pasted from another document brings its image.
    for b in &doc.info.bullets {
        if let Some(i) = b.image.and_then(|i| u16::try_from(i).ok()) {
            out.insert(i);
        }
    }
    for s in &doc.sections {
        walk(&s.paras, &mut out);
        for l in s
            .last_page_master
            .iter()
            .chain(s.page_masters.iter().map(|(_, l)| l))
        {
            walk(&l.paras, &mut out);
        }
    }
    out
}

/// Leaves out of a save the BinData entries added since opening that
/// nothing uses any more, a picture deleted or undone, as Hancom drops its
/// image (objects case pic-orphan). Unused ones at the end go; one before a
/// used one becomes an empty link, so later ids stay. Entries the file came
/// with stay, whatever refers to them.
fn drop_unused_bins(doc: &mut hm_model::Document, opened: usize) {
    if doc.info.bin_data.len() <= opened {
        return;
    }
    let used = used_bins(doc);
    let unused = |k: usize| !used.contains(&(k as u16 + 1));
    while doc.info.bin_data.len() > opened && unused(doc.info.bin_data.len() - 1) {
        doc.info.bin_data.pop();
        doc.info.id_mappings.src = None;
    }
    for k in opened..doc.info.bin_data.len() {
        if unused(k) && doc.info.bin_data[k].kind() != 0 {
            doc.info.bin_data[k] = hm_model::BinDataEntry {
                link_absolute: Some(hm_model::HwpString::from_text("")),
                link_relative: Some(hm_model::HwpString::from_text("")),
                ..Default::default()
            };
            doc.info.id_mappings.src = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const PNG: &[u8] = &[
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 4,
        0, 0, 0, 181, 28, 12, 2, 0, 0, 0, 11, 73, 68, 65, 84, 120, 218, 99, 252, 255, 31, 0, 3, 3,
        2, 0, 239, 191, 107, 153, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ];

    #[test]
    fn saves_keep_added_images_that_bullets_and_master_pages_use() {
        let mut d = crate::Engine::new()
            .create(include_bytes!("../tests/fixtures/query-equations.hwpx").to_vec())
            .unwrap();
        let opened = d.raw().unwrap().opened_bins;
        let paras: serde_json::Value = serde_json::from_str(
            &d.query(r#"{"kind":"paragraphs","list":{"section":0},"count":1}"#)
                .unwrap(),
        )
        .unwrap();
        let at = serde_json::json!({"section":0,"para":0,"offset":paras[0]["caret"][0]});
        // Two pictures inserted; a third image left unused.
        for _ in 0..3 {
            let op = serde_json::json!({"op":"insertPicture","at":at,"bytes":PNG,"extension":"png","width":3000,"height":3000});
            d.transact(&serde_json::json!({"ops":[op]}).to_string())
                .unwrap();
        }
        let mut doc = d.raw().unwrap().editor.doc.clone();
        assert_eq!(doc.info.bin_data.len(), opened + 3);
        let mut ids: Vec<u16> = doc.sections[0].paras[0]
            .controls
            .iter()
            .filter_map(|c| {
                c.as_object()?.shape()?.parts.iter().find_map(|p| match p {
                    hm_model::ShapePart::Picture(p) => Some(p.image.bin_id),
                    _ => None,
                })
            })
            .collect();
        ids.sort();
        assert_eq!(ids.len(), 3);
        // The pictures leave the body: the first image unused, the second
        // shown by a pasted picture bullet, the last by a picture on a
        // page's own master page.
        let mut master = hm_model::ParaList::default();
        let mut pictured = doc.sections[0].paras[0].clone();
        pictured.controls.retain(|c| {
            c.as_object().and_then(|o| o.shape()).is_some_and(|s| {
                s.parts.iter().any(
                    |p| matches!(p, hm_model::ShapePart::Picture(p) if p.image.bin_id == ids[2]),
                )
            })
        });
        master.paras.push(pictured);
        doc.sections[0].page_masters.push((1, master));
        doc.sections[0].paras[0]
            .controls
            .retain(|c| c.as_object().is_none_or(|o| o.shape().is_none()));
        doc.info.bullets.push(hm_model::Bullet {
            image: Some(i32::from(ids[1])),
            ..Default::default()
        });
        let unused = ids[0] as usize - 1;
        drop_unused_bins(&mut doc, opened);
        assert_eq!(
            doc.info.bin_data.len(),
            opened + 3,
            "kept before a used one"
        );
        for id in &ids[1..] {
            assert_eq!(
                doc.info.bin_data[*id as usize - 1].kind(),
                1,
                "image {id} kept"
            );
        }
        assert_eq!(
            doc.info.bin_data[unused].kind(),
            0,
            "the unused one is an empty link"
        );
    }
}
