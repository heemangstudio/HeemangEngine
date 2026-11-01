//! Taking a document to another file format: everything the model keeps in
//! the bytes of the format it was read from (source spans, record and
//! element tails, records it does not interpret) is dropped, so the other
//! format's writer encodes the document from its typed fields alone.

use crate::*;

fn shared(b: &mut SharedBytes) {
    *b = SharedBytes::empty();
}

fn paras(ps: &mut [Paragraph]) {
    for p in ps {
        paragraph(p);
    }
}

fn paragraph(p: &mut Paragraph) {
    p.src = None;
    shared(&mut p.header.tail);
    p.extra.clear();
    for t in &mut p.tracks {
        shared(&mut t.tail);
    }
    for c in &mut p.controls {
        control(c);
    }
}

fn list(l: &mut ParaList) {
    shared(&mut l.tail);
    paras(&mut l.paras);
}

fn caption(c: &mut Caption) {
    shared(&mut c.tail);
    paras(&mut c.paras);
}

/// A fill's extra attribute block, which HWP 5 always counts: a gradient's
/// step center, as Hancom writes it, and nothing for other fills.
fn fill(f: &mut Fill) {
    f.extra = Some(match &f.gradient {
        Some(g) => SharedBytes::from(vec![g.step_center.unwrap_or(50)]),
        None => SharedBytes::empty(),
    });
}

/// A picture's effects and additional properties in the one layout both
/// formats' readers give them: the effects, the original size, the alpha.
fn picture(p: &mut Picture) {
    let mut tail = p.effect_bytes().to_vec();
    if tail.len() < 4 {
        tail = 0u32.to_le_bytes().to_vec();
    }
    // Files that stop after the effects have no additional properties.
    let ((w, h), alpha) = if p.tail.len() >= tail.len() + 9 {
        (p.original_size().unwrap_or((0, 0)), p.alpha())
    } else {
        ((0, 0), 0)
    };
    tail.extend_from_slice(&w.to_le_bytes());
    tail.extend_from_slice(&h.to_le_bytes());
    tail.push(alpha);
    p.tail = SharedBytes::from(tail);
}

fn shape(s: &mut Shape) {
    let c = &mut s.component;
    shared(&mut c.tail);
    // Hancom marks pictures in the high bits of this word, which HWPX does
    // not carry; HWP 5 readers want it. Drawings holding text get theirs
    // from the HWP 5 writer: a rectangle without text has none, and Hancom
    // refuses a file whose plain rectangle carries the text mark.
    if c.flip >> 20 == 0 && c.kind == CtrlId::from_chars(b"$pic") {
        c.flip |= 1 << 29;
    }
    if let Some(st) = &mut c.style {
        fill(&mut st.fill);
    }
    s.parts.retain(|p| !matches!(p, ShapePart::Raw(_)));
    for p in &mut s.parts {
        match p {
            ShapePart::Line(g) => shared(&mut g.tail),
            ShapePart::Rect(g) => shared(&mut g.tail),
            ShapePart::Ellipse(g) => shared(&mut g.tail),
            ShapePart::Arc(g) => shared(&mut g.tail),
            ShapePart::Polygon(g) => shared(&mut g.tail),
            ShapePart::Curve(g) => shared(&mut g.tail),
            ShapePart::Picture(g) => picture(g),
            ShapePart::Ole(g) => shared(&mut g.tail),
            ShapePart::Text(t) => {
                shared(&mut t.tail);
                paras(&mut t.paras);
            }
            ShapePart::Child(k) => shape(k),
            ShapePart::Raw(_) => {}
        }
    }
}

fn control(c: &mut Control) {
    c.src = None;
    match &mut c.kind {
        ControlKind::Section(s) => {
            shared(&mut s.tail);
            s.children.retain(|k| !matches!(k, SectionChild::Raw(_)));
            for k in &mut s.children {
                match k {
                    SectionChild::Page(p) => shared(&mut p.tail),
                    SectionChild::NoteShape(n) => shared(&mut n.tail),
                    SectionChild::PageBorder(b) => shared(&mut b.tail),
                    SectionChild::MasterPage(l) => list(l),
                    SectionChild::Raw(_) => {}
                }
            }
        }
        ControlKind::Columns(d) => shared(&mut d.tail),
        ControlKind::Table(t) => {
            shared(&mut t.header.tail);
            shared(&mut t.tail);
            t.extra.clear();
            if let Some(c) = &mut t.caption {
                caption(c);
            }
            for cell in &mut t.cells {
                shared(&mut cell.tail);
                paras(&mut cell.paras);
            }
        }
        ControlKind::Object(o) => {
            shared(&mut o.header.tail);
            o.parts.retain(|p| !matches!(p, ObjectPart::Raw(_)));
            for p in &mut o.parts {
                match p {
                    ObjectPart::Caption(c) => caption(c),
                    ObjectPart::Shape(s) => shape(s),
                    ObjectPart::Equation(e) => shared(&mut e.tail),
                    ObjectPart::Raw(_) => {}
                }
            }
        }
        ControlKind::HeaderFooter(h) => {
            shared(&mut h.tail);
            list(&mut h.list);
        }
        ControlKind::Note(n) => {
            shared(&mut n.tail);
            list(&mut n.list);
        }
        ControlKind::AutoNumber(a) => shared(&mut a.tail),
        ControlKind::NewNumber(a) => shared(&mut a.tail),
        ControlKind::PageNumber(a) => shared(&mut a.tail),
        ControlKind::PageHide(a) => shared(&mut a.tail),
        ControlKind::PageParity(a) => shared(&mut a.tail),
        ControlKind::Field(f) => {
            shared(&mut f.tail);
            f.data.clear();
            // A memo's list is written at the section end like any other.
            if let Some(m) = &mut f.memo {
                list(m);
            }
        }
        ControlKind::Bookmark(b) => {
            shared(&mut b.tail);
            b.data.clear();
        }
        ControlKind::IndexMark(m) => shared(&mut m.tail),
        ControlKind::Form(f) => {
            shared(&mut f.header.tail);
            shared(&mut f.tail);
        }
        ControlKind::Compose(k) => shared(&mut k.tail),
        ControlKind::Dutmal(d) => shared(&mut d.tail),
        // Kept: its control character holds the text's place. Writers of
        // the other format leave out what they cannot express.
        ControlKind::Raw(r) => {
            shared(&mut r.header);
            r.children.clear();
            r.xml = None;
        }
    }
}

fn info(i: &mut DocInfo) {
    macro_rules! entries {
        ($($v:expr),*) => {$(
            for e in $v.iter_mut() {
                shared(&mut e.tail);
                e.src = None;
            }
        )*};
    }
    let p = &mut i.properties;
    shared(&mut p.tail);
    p.src = None;
    let m = &mut i.id_mappings;
    shared(&mut m.tail);
    m.src = None;
    entries!(
        i.bin_data,
        i.border_fills,
        i.char_shapes,
        i.tab_defs,
        i.numberings,
        i.bullets,
        i.para_shapes,
        i.styles
    );
    for f in i.faces.iter_mut().flatten() {
        shared(&mut f.tail);
        f.src = None;
    }
    for b in &mut i.border_fills {
        if let Some(f) = &mut b.fill {
            fill(f);
        }
    }
    if let Some(c) = &mut i.compatible {
        shared(&mut c.tail);
        c.src = None;
    }
    if let Some(c) = &mut i.layout_compat {
        shared(&mut c.tail);
        c.src = None;
    }
    i.raw.clear();
}

impl crate::Control {
    /// Readies a control taken from a document of another format, as
    /// [`Document::convert_to`] readies a whole document: what only that
    /// format's bytes carry (leftover markup, raw records, source spans) is
    /// dropped, so the writer encodes the control from its fields.
    pub fn convert_from_other_format(&mut self) {
        control(self);
    }
}

impl Document {
    /// Readies the document to be written as `format`: when that is not
    /// the format it was read from, drops what only that format's bytes
    /// carry and the source container (pictures stay readable through
    /// [`Document::bins`]).
    pub fn convert_to(&mut self, format: Format) {
        if self.meta.format == format {
            return;
        }
        info(&mut self.info);
        for s in &mut self.sections {
            s.src = None;
            s.source_index = None;
            paras(&mut s.paras);
            if let Some(master) = &mut s.last_page_master {
                list(master);
            }
            for (_, master) in &mut s.page_masters {
                list(master);
            }
        }
        self.source = None;
        self.meta.format = format;
        if format == Format::Hwp5 {
            // A style naming a shape the tables lack (another writer's
            // fault HWPX tolerates) names the first one, as Hancom's own
            // conversion does; HWP 5 refuses the dangling reference.
            let (paras, chars) = (self.info.para_shapes.len(), self.info.char_shapes.len());
            for st in &mut self.info.styles {
                if usize::from(st.para_shape) >= paras {
                    st.para_shape = 0;
                }
                if usize::from(st.char_shape) >= chars {
                    st.char_shape = 0;
                }
            }
            // Hancom writes the layout compatibility record after the
            // compatible document record and refuses a file without it.
            let i = &mut self.info;
            if i.compatible.is_some() && i.layout_compat.is_none() {
                i.layout_compat = Some(LayoutCompatibility::default());
                if let Some(at) = i.order.iter().position(|s| s.item == InfoItem::Compatible) {
                    i.order.insert(
                        at + 1,
                        InfoSlot {
                            item: InfoItem::LayoutCompat,
                            level: 1,
                        },
                    );
                }
            }
            for s in &mut self.sections {
                hwp5_lists(&mut s.paras);
                for l in s
                    .last_page_master
                    .iter_mut()
                    .chain(s.page_masters.iter_mut().map(|(_, l)| l))
                {
                    hwp5_lists(&mut l.paras);
                }
            }
        }
    }
}

/// Records every HWP 5 section definition has that the other format may
/// leave out, as Hancom writes them when it converts (windows1 captures of
/// HWPX documents saved as HWP, which Hancom refuses to open without them):
/// footnote and endnote shapes, and page border fills for both, even and
/// odd pages. Added note shapes are the ones the layout assumes for a
/// section without them, so the pages stay as they were.
fn hwp5_lists(paras: &mut [Paragraph]) {
    for p in paras {
        for c in &mut p.controls {
            match &mut c.kind {
                ControlKind::Section(s) => {
                    let count = |s: &SectionDef, f: fn(&SectionChild) -> bool| {
                        (
                            s.children.iter().filter(|k| f(k)).count(),
                            s.children.iter().rposition(f),
                        )
                    };
                    let note = |k: &SectionChild| matches!(k, SectionChild::NoteShape(_));
                    let border = |k: &SectionChild| matches!(k, SectionChild::PageBorder(_));
                    let page = |k: &SectionChild| matches!(k, SectionChild::Page(_));
                    // Footnote and endnote shapes after the page definition,
                    // then page border fills for both, even and odd pages.
                    if let (n, Some(at)) = match count(s, note) {
                        (0, _) => (0, count(s, page).1),
                        found => found,
                    } {
                        for k in n..2 {
                            s.children.insert(
                                at + 1 + k - n,
                                SectionChild::NoteShape(NoteShape::absent()),
                            );
                        }
                    }
                    if let (n, Some(at)) = match count(s, border) {
                        (0, _) => (0, count(s, note).1),
                        found => found,
                    } {
                        for k in n..3 {
                            s.children.insert(
                                at + 1 + k - n,
                                SectionChild::PageBorder(PageBorderFill {
                                    border_fill: 1,
                                    ..Default::default()
                                }),
                            );
                        }
                    }
                }
                _ => {}
            }
            for list in lists_mut(c) {
                hwp5_lists(list);
            }
        }
    }
}

/// The paragraph lists inside a control.
fn lists_mut(c: &mut Control) -> Vec<&mut Vec<Paragraph>> {
    let mut out = Vec::new();
    fn shape<'a>(s: &'a mut Shape, out: &mut Vec<&'a mut Vec<Paragraph>>) {
        for p in &mut s.parts {
            match p {
                ShapePart::Text(t) => out.push(&mut t.paras),
                ShapePart::Child(c) => shape(c, out),
                _ => {}
            }
        }
    }
    match &mut c.kind {
        ControlKind::Table(t) => {
            out.extend(t.cells.iter_mut().map(|c| &mut c.paras));
            out.extend(t.caption.iter_mut().map(|c| &mut c.paras));
        }
        ControlKind::Object(o) => {
            for p in &mut o.parts {
                match p {
                    ObjectPart::Shape(s) => shape(s, &mut out),
                    ObjectPart::Caption(c) => out.push(&mut c.paras),
                    _ => {}
                }
            }
        }
        ControlKind::HeaderFooter(h) => out.push(&mut h.list.paras),
        ControlKind::Note(n) => out.push(&mut n.list.paras),
        ControlKind::Section(s) => {
            for k in &mut s.children {
                if let SectionChild::MasterPage(l) = k {
                    out.push(&mut l.paras);
                }
            }
        }
        _ => {}
    }
    out
}
