//! Taking a document to another file format: everything the model keeps in
//! the bytes of the format it was read from (source spans, record and
//! element tails, records it does not interpret) is dropped, so the other
//! format's writer encodes the document from its typed fields alone.

use crate::*;

/// HWP 5's document data record.
const DOC_DATA: u16 = 0x1B;

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
            ShapePart::Arc(g) => {
                shared(&mut g.tail);
                g.byte_kind = false;
            }
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
            f.memo_in_last_section = false;
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
    // The print settings are the one raw record the other format can hold.
    if let Some(data) = i.raw.iter().find(|r| r.tag == DOC_DATA)
        && let Some(print) = PrintInfo::from_doc_data(&data.payload)
    {
        i.print = Some(print);
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
        missing_faces(&mut self.info);
        if format == Format::Hwpx {
            for s in &mut self.sections {
                ignored_new_numbers(&mut s.paras);
            }
            // OWPML has no default face name (Hancom drops it when it
            // converts); a face without a substitute keeps it as one, the
            // name the layout looks for next either way.
            for f in self.info.faces.iter_mut().flatten() {
                if f.substitute.is_none()
                    && let Some(name) = f.default_name.clone()
                {
                    f.substitute = Some((f.attr & 0x1F, name));
                    f.attr |= 0x80;
                }
            }
        }
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
            dangling_refs(self);
            for s in &mut self.sections {
                revision_fields(&mut s.paras);
            }
            // Hancom takes a heading's level from the record's last field
            // when the file's version has it; without it every heading reads
            // as level 1, in another numbering and style.
            for p in &mut self.info.para_shapes {
                if p.attr2.is_some() && p.attr3.is_some() && p.line_spacing.is_some() {
                    p.tail = SharedBytes::from(p.level().to_le_bytes().to_vec());
                }
            }
            if let Some(print) = self.info.print {
                self.info.raw.push(RawRecord {
                    tag: DOC_DATA,
                    level: 0,
                    payload: SharedBytes::from(print.to_doc_data()),
                });
                self.info.order.push(InfoSlot {
                    item: InfoItem::Raw(self.info.raw.len() as u32 - 1),
                    level: 0,
                });
            }
            for s in &mut self.sections {
                square_wraps(&mut s.paras);
                for l in s
                    .last_page_master
                    .iter_mut()
                    .chain(s.page_masters.iter_mut().map(|(_, l)| l))
                {
                    square_wraps(&mut l.paras);
                }
            }
            // Hancom writes the layout compatibility record after the
            // compatible document record and refuses a file without it.
            let i = &mut self.info;
            if let Some(compatible) = &mut i.compatible
                && i.layout_compat.is_none()
            {
                // Without that record Hancom ignores the target program in
                // HWPX and lays out for its own, but obeys it once the record
                // is there, so the record comes with Hancom's own target.
                compatible.target = 0;
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
                            // The layout flows endnotes without a shape with
                            // no separator and no gaps; Hancom's conversion
                            // writes such a shape all zero.
                            let shape = if k == 0 {
                                NoteShape::absent()
                            } else {
                                NoteShape::default()
                            };
                            s.children
                                .insert(at + 1 + k - n, SectionChild::NoteShape(shape));
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

/// Faces a char shape names but its script lacks (another writer's fault
/// both readers tolerate and both checks refuse) become the Hangul face the
/// layout falls back to, so the layout is unchanged.
fn missing_faces(info: &mut DocInfo) {
    for k in 0..info.char_shapes.len() {
        let hangul = info.char_shapes[k].face(Script::Hangul);
        for s in Script::ALL {
            let id = info.char_shapes[k].face(s);
            if info.face(s, id).is_some() {
                continue;
            }
            let Some(f) = info.face(Script::Hangul, hangul).cloned() else {
                continue;
            };
            let list = &mut info.faces[s.index()];
            let same = |g: &FaceName| {
                (g.attr, &g.name, &g.substitute, &g.default_name)
                    == (f.attr, &f.name, &f.substitute, &f.default_name)
            };
            let at = list.iter().position(same).unwrap_or_else(|| {
                list.push(f.clone());
                list.len() - 1
            });
            info.char_shapes[k].faces[s.index()] = at as u16;
        }
    }
}

/// What an HWPX document names but its header lacks, which HWP 5 refuses
/// (Hancom's conversion of such documents writes complete tables, windows1
/// captures): missing tab definitions are added empty and a missing style
/// table gets Hancom's 바탕글, a paragraph naming a style past the table
/// takes the first one, and an outline numbering the document lacks becomes
/// none (0), as Hancom writes it. The layout is unchanged.
fn dangling_refs(doc: &mut Document) {
    let info = &mut doc.info;
    let tabs = info
        .para_shapes
        .iter()
        .map(|p| usize::from(p.tab_def) + 1)
        .max();
    while tabs.is_some_and(|n| info.tab_defs.len() < n) {
        info.tab_defs.push(TabDef::default());
    }
    if info.styles.is_empty() {
        info.styles.push(Style {
            local_name: HwpString::from_text("바탕글"),
            english_name: HwpString::from_text("Normal"),
            language: 0x412,
            ..Default::default()
        });
    }
    let styles = info.styles.len();
    let numberings = info.numberings.len();
    fn walk(paras: &mut [Paragraph], styles: usize, numberings: usize) {
        for p in paras {
            if usize::from(p.header.style) >= styles {
                p.header.style = 0;
            }
            for c in &mut p.controls {
                if let ControlKind::Section(s) = &mut c.kind
                    && usize::from(s.outline_numbering) > numberings
                {
                    s.outline_numbering = 0;
                }
                for list in lists_mut(c) {
                    walk(list, styles, numberings);
                }
            }
        }
    }
    for s in &mut doc.sections {
        walk(&mut s.paras, styles, numberings);
        for l in s
            .last_page_master
            .iter_mut()
            .chain(s.page_masters.iter_mut().map(|(_, l)| l))
        {
            walk(&mut l.paras, styles, numberings);
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

/// HWP 5 keeps proofreading marks as unknown fields whose command names the
/// mark, as Hancom's own conversion writes them.
fn revision_fields(paras: &mut [Paragraph]) {
    for p in paras {
        for c in &mut p.controls {
            if let ControlKind::Field(f) = &mut c.kind
                && f.command.to_string_lossy().starts_with("$Revision")
                && f.proofreading().is_some()
            {
                f.id = CtrlId::from_chars(b"%unk");
            }
            for list in lists_mut(c) {
                revision_fields(list);
            }
        }
    }
}

/// Tight and through wraps, which only HWPX can name: Hancom reads them as
/// square (its re-save of such an HWPX says SQUARE) but reads the same numbers
/// in HWP 5 as a wrap text ignores, so the pages would close up. They are
/// written as Hancom's own conversion writes them.
fn square_wraps(paras: &mut [Paragraph]) {
    const WRAP: u32 = 7 << 21;
    for p in paras {
        for c in &mut p.controls {
            let common = match &mut c.kind {
                ControlKind::Table(t) => Some(&mut t.header.common),
                ControlKind::Object(o) => Some(&mut o.header.common),
                ControlKind::Form(f) => Some(&mut f.header.common),
                _ => None,
            };
            if let Some(o) = common
                && matches!(o.text_wrap(), 4 | 5)
            {
                o.attr &= !WRAP;
            }
            for list in lists_mut(c) {
                square_wraps(list);
            }
        }
    }
}

/// A page new-number control stored under another code than 21 (some
/// writers use the automatic number's 18), which Hancom ignores in HWP 5
/// (d-multi-table-001: its PDF keeps counting) but obeys once its own
/// conversion writes it as `hp:newNum` (the PDF of that HWPX restarts the
/// count). OWPML cannot say "ignored"; the control becomes the page parity
/// control that changes nothing, under the same code, so the pages stay.
fn ignored_new_numbers(paras: &mut [Paragraph]) {
    for p in paras {
        let ignored: Vec<usize> = p
            .units()
            .filter_map(|(_, u)| match u {
                Unit::Extended { code, index } if code != 21 => Some(index as usize),
                _ => None,
            })
            .filter(|&i| {
                p.controls.get(i).is_some_and(|c| {
                    matches!(&c.kind, ControlKind::NewNumber(n) if n.kind() == NumberKind::Page)
                })
            })
            .collect();
        for i in ignored {
            p.controls[i].kind = ControlKind::PageParity(PageParity::default());
        }
        for c in &mut p.controls {
            for list in lists_mut(c) {
                ignored_new_numbers(list);
            }
        }
    }
}
