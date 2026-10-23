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

/// A fill gets an empty extra attribute block: HWP 5 always counts its
/// length.
fn fill(f: &mut Fill) {
    f.extra = Some(SharedBytes::empty());
}

/// A picture's additional properties in the one layout both formats'
/// readers give them: no effects, the original size, the alpha.
fn picture(p: &mut Picture) {
    let mut tail = 0u32.to_le_bytes().to_vec();
    let (w, h) = p.original_size().unwrap_or((0, 0));
    tail.extend_from_slice(&w.to_le_bytes());
    tail.extend_from_slice(&h.to_le_bytes());
    tail.push(p.alpha());
    p.tail = SharedBytes::from(tail);
}

fn shape(s: &mut Shape) {
    let c = &mut s.component;
    shared(&mut c.tail);
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
        }
        self.source = None;
        self.meta.format = format;
    }
}
