//! Import only definitions and binary resources referenced by a clipboard fragment.
use crate::types::{EngineError, Result};
use hm_model::*;
use std::collections::HashMap;

pub struct Import<'a> {
    source: &'a Document,
    pub target: &'a mut Document,
    chars: HashMap<u32, u32>,
    paras: HashMap<u16, u16>,
    styles: HashMap<u8, u8>,
    borders: HashMap<u16, u16>,
    tabs: HashMap<u16, u16>,
    numbers: HashMap<u16, u16>,
    bullets: HashMap<u16, u16>,
    bins: HashMap<u16, u16>,
    /// The destination's definition counts before the import.
    counts: [usize; 9],
    /// A paste: styles go to the destination's of the same name.
    paste: bool,
}
fn bad() -> EngineError {
    EngineError::new(
        "BAD_REQUEST",
        "clipboard definition reference exceeds the document limits",
    )
}
/// How many definitions of each kind `info` holds.
fn counts(info: &DocInfo) -> [usize; 9] {
    [
        info.bin_data.len(),
        info.border_fills.len(),
        info.faces.iter().map(Vec::len).sum(),
        info.char_shapes.len(),
        info.tab_defs.len(),
        info.numberings.len(),
        info.bullets.len(),
        info.para_shapes.len(),
        info.styles.len(),
    ]
}
impl<'a> Import<'a> {
    pub fn new(source: &'a Document, target: &'a mut Document) -> Self {
        let counts = counts(&target.info);
        Self {
            counts,
            paste: false,
            source,
            target,
            chars: HashMap::new(),
            paras: HashMap::new(),
            styles: HashMap::new(),
            borders: HashMap::new(),
            tabs: HashMap::new(),
            numbers: HashMap::new(),
            bullets: HashMap::new(),
            bins: HashMap::new(),
        }
    }
    /// An import for pasting a fragment.
    pub fn paste(source: &'a Document, target: &'a mut Document) -> Self {
        Self {
            paste: true,
            ..Self::new(source, target)
        }
    }
    /// Ends the import: the counts record declares any definitions added.
    pub fn finish(self) {
        if counts(&self.target.info) != self.counts {
            self.target.info.id_mappings.src = None;
        }
    }
    pub fn bin(&mut self, id: u16) -> Result<u16> {
        if id == 0 {
            return Ok(0);
        }
        if let Some(id) = self.bins.get(&id) {
            return Ok(*id);
        }
        let mut entry = self
            .source
            .info
            .bin_data
            .get(id as usize - 1)
            .cloned()
            .ok_or_else(bad)?;
        let new = u16::try_from(self.target.info.bin_data.len() + 1).map_err(|_| bad())?;
        let bytes = self.source.bins.as_ref().and_then(|b| b.get(id));
        let original = self.target.bins.clone();
        let original_ole = original.clone();
        let source = self.source.bins.clone();
        self.target.bins = Some(
            BinStore::new(move |key| {
                if key == new {
                    bytes.clone()
                } else {
                    original.as_ref().and_then(|b| b.get(key))
                }
            })
            .with_ole(move |key| {
                if key == new {
                    source.as_ref().and_then(|b| b.ole(id))
                } else {
                    original_ole.as_ref().and_then(|b| b.ole(key))
                }
            }),
        );
        entry.storage_id = Some(
            self.target
                .info
                .bin_data
                .iter()
                .filter_map(|e| e.storage_id)
                .max()
                .unwrap_or(0)
                .checked_add(1)
                .ok_or_else(bad)?,
        );
        if matches!(self.source.meta.format, Format::Hwpx | Format::Hml) {
            for name in ["id", "href", "media-type", "isEmbeded"] {
                entry.tail = hm_hwpx::rest::without_attr(&entry.tail, "", name);
            }
        }
        entry.src = None;
        self.target.info.bin_data.push(entry);
        self.bins.insert(id, new);
        Ok(new)
    }
    fn fill(&mut self, f: &mut Fill) -> Result<()> {
        if let Some(image) = &mut f.image {
            image.bin_id = self.bin(image.bin_id)?;
        }
        Ok(())
    }
    pub fn border(&mut self, id: u16) -> Result<u16> {
        if id == 0 {
            return Ok(0);
        }
        if let Some(id) = self.borders.get(&id) {
            return Ok(*id);
        }
        let mut b = self
            .source
            .info
            .border_fills
            .get(id as usize - 1)
            .cloned()
            .ok_or_else(bad)?;
        if let Some(f) = &mut b.fill {
            self.fill(f)?;
        }
        b.src = None;
        let new = u16::try_from(self.target.info.border_fills.len() + 1).map_err(|_| bad())?;
        self.target.info.border_fills.push(b);
        self.borders.insert(id, new);
        Ok(new)
    }
    pub fn font(&mut self, slot: usize, id: u16) -> Result<u16> {
        let mut face = self
            .source
            .info
            .faces
            .get(slot)
            .and_then(|v| v.get(id as usize))
            .cloned()
            .ok_or_else(bad)?;
        if let Some(index) = self.target.info.faces[slot].iter().position(|other| {
            other.attr == face.attr
                && other.name == face.name
                && other.substitute == face.substitute
                && other.panose == face.panose
                && other.default_name == face.default_name
                && other.tail == face.tail
        }) {
            return u16::try_from(index).map_err(|_| bad());
        }
        let id = u16::try_from(self.target.info.faces[slot].len()).map_err(|_| bad())?;
        face.src = None;
        self.target.info.faces[slot].push(face);
        Ok(id)
    }
    pub fn character(&mut self, id: u32) -> Result<u32> {
        if let Some(id) = self.chars.get(&id) {
            return Ok(*id);
        }
        let mut c = self
            .source
            .info
            .char_shapes
            .get(id as usize)
            .cloned()
            .ok_or_else(bad)?;
        for k in 0..7 {
            if self.source.info.faces[k].get(c.faces[k] as usize).is_some() {
                c.faces[k] = self.font(k, c.faces[k])?;
            }
        }
        if let Some(id) = c.border_fill {
            c.border_fill = Some(self.border(id)?);
        }
        c.src = None;
        let new = self.target.info.char_shapes.len() as u32;
        self.target.info.char_shapes.push(c);
        self.chars.insert(id, new);
        Ok(new)
    }
    pub fn tab(&mut self, id: u16) -> Result<u16> {
        if id == 0 && self.source.info.tab_defs.is_empty() {
            return Ok(0);
        }
        if let Some(id) = self.tabs.get(&id) {
            return Ok(*id);
        }
        let mut t = self
            .source
            .info
            .tab_defs
            .get(id as usize)
            .cloned()
            .ok_or_else(bad)?;
        t.src = None;
        let new = u16::try_from(self.target.info.tab_defs.len()).map_err(|_| bad())?;
        self.target.info.tab_defs.push(t);
        self.tabs.insert(id, new);
        Ok(new)
    }
    pub fn numbering(&mut self, id: u16, bullet: bool) -> Result<u16> {
        if id == 0 {
            return Ok(0);
        }
        if let Some(id) = if bullet {
            self.bullets.get(&id)
        } else {
            self.numbers.get(&id)
        } {
            return Ok(*id);
        }
        if bullet {
            let mut b = self
                .source
                .info
                .bullets
                .get(id as usize - 1)
                .cloned()
                .ok_or_else(bad)?;
            if b.head.char_shape != u32::MAX {
                b.head.char_shape = self.character(b.head.char_shape)?;
            }
            if let Some(image) = &mut b.image {
                if *image > 0 {
                    *image = self.bin(u16::try_from(*image).map_err(|_| bad())?)? as i32;
                }
            }
            b.src = None;
            let new = u16::try_from(self.target.info.bullets.len() + 1).map_err(|_| bad())?;
            self.target.info.bullets.push(b);
            self.bullets.insert(id, new);
            Ok(new)
        } else {
            let mut n = self
                .source
                .info
                .numberings
                .get(id as usize - 1)
                .cloned()
                .ok_or_else(bad)?;
            for level in &mut n.levels {
                if level.head.char_shape != u32::MAX {
                    level.head.char_shape = self.character(level.head.char_shape)?;
                }
            }
            n.src = None;
            let new = u16::try_from(self.target.info.numberings.len() + 1).map_err(|_| bad())?;
            self.target.info.numberings.push(n);
            self.numbers.insert(id, new);
            Ok(new)
        }
    }
    pub fn paragraph_shape(&mut self, id: u16) -> Result<u16> {
        if let Some(id) = self.paras.get(&id) {
            return Ok(*id);
        }
        let mut p = self
            .source
            .info
            .para_shapes
            .get(id as usize)
            .cloned()
            .ok_or_else(bad)?;
        p.border_fill = self.border(p.border_fill)?;
        p.tab_def = self.tab(p.tab_def)?;
        if p.numbering != 0 {
            p.numbering = self.numbering(p.numbering, p.head_kind() == 3)?;
        }
        p.src = None;
        let new = u16::try_from(self.target.info.para_shapes.len()).map_err(|_| bad())?;
        self.target.info.para_shapes.push(p);
        self.paras.insert(id, new);
        Ok(new)
    }
    pub fn style(&mut self, id: u8) -> Result<u8> {
        if id == 0 && self.source.info.styles.is_empty() {
            return Ok(0);
        }
        if let Some(id) = self.styles.get(&id) {
            return Ok(*id);
        }
        let mut s = self
            .source
            .info
            .styles
            .get(id as usize)
            .cloned()
            .ok_or_else(bad)?;
        // Hancom pastes into the destination's style of the same name and
        // kind when it has one, and adds the others. (It also moves the
        // pasted paragraph onto that style's shapes, keeping only what the
        // paragraph changed from its own style; the copied shapes stay here.)
        let named = self
            .target
            .info
            .styles
            .iter()
            .position(|t| t.local_name == s.local_name && t.attr & 7 == s.attr & 7);
        if let Some(i) = named.filter(|_| self.paste) {
            let i = u8::try_from(i).map_err(|_| bad())?;
            self.styles.insert(id, i);
            return Ok(i);
        }
        let new = u8::try_from(self.target.info.styles.len()).map_err(|_| bad())?;
        self.styles.insert(id, new);
        self.target.info.styles.push(s.clone());
        s.char_shape = u16::try_from(self.character(s.char_shape as u32)?).map_err(|_| bad())?;
        s.para_shape = self.paragraph_shape(s.para_shape)?;
        s.next = self.style(s.next)?;
        s.src = None;
        self.target.info.styles[new as usize] = s;
        Ok(new)
    }
    pub fn paragraphs(&mut self, paras: &mut [Paragraph]) -> Result<()> {
        for p in paras {
            p.src = None;
            p.line_segs.clear();
            p.header.para_shape = self.paragraph_shape(p.header.para_shape)?;
            p.header.style = self.style(p.header.style)?;
            for run in &mut p.runs {
                run.shape = self.character(run.shape)?;
            }
            for c in &mut p.controls {
                self.control(c)?;
            }
        }
        Ok(())
    }
    fn shape(&mut self, s: &mut Shape) -> Result<()> {
        if let Some(style) = &mut s.component.style {
            self.fill(&mut style.fill)?;
        }
        for p in &mut s.parts {
            match p {
                ShapePart::Picture(p) => p.image.bin_id = self.bin(p.image.bin_id)?,
                ShapePart::Ole(o) => o.bin_id = self.bin(o.bin_id)?,
                ShapePart::Text(t) => self.paragraphs(&mut t.paras)?,
                ShapePart::Child(s) => self.shape(s)?,
                _ => {}
            }
        }
        Ok(())
    }
    pub fn control(&mut self, c: &mut Control) -> Result<()> {
        c.src = None;
        match &mut c.kind {
            ControlKind::Table(t) => {
                t.border_fill = self.border(t.border_fill)?;
                for cell in &mut t.cells {
                    cell.border_fill = self.border(cell.border_fill)?;
                    self.paragraphs(&mut cell.paras)?;
                }
                if let Some(c) = &mut t.caption {
                    self.paragraphs(&mut c.paras)?;
                }
                if let Some(zones) = &mut t.zones {
                    for z in zones {
                        z.border_fill = self.border(z.border_fill)?;
                    }
                }
            }
            ControlKind::Object(o) => {
                for p in &mut o.parts {
                    match p {
                        ObjectPart::Shape(s) => self.shape(s)?,
                        ObjectPart::Caption(c) => self.paragraphs(&mut c.paras)?,
                        _ => {}
                    }
                }
            }
            ControlKind::HeaderFooter(h) => self.paragraphs(&mut h.list.paras)?,
            ControlKind::Note(n) => self.paragraphs(&mut n.list.paras)?,
            ControlKind::Field(f) => {
                if let Some(memo) = &mut f.memo {
                    self.paragraphs(&mut memo.paras)?;
                }
            }
            _ => {}
        }
        self.target.info.id_mappings.src = None;
        Ok(())
    }
}
