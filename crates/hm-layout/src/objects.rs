//! Objects: their inner layouts, their boxes as characters, where floating
//! ones sit and which bands of the page they block.
//!
//! An object treated as a character is measured into an [`InlineBox`]; a
//! floating one is placed by [`place`] from an [`Anchor`] that callers fill
//! for the body, a header or footer frame, a note or a cell alike. Inner
//! layouts (equations, text inside shapes) ride in the paragraph's
//! [`crate::Measured::objects`].
//!
//! Top-and-bottom wrapping blocks the full width of every column the object
//! overlaps. Square, tight and through wrapping (text beside the object)
//! belong to the wrap pass.

use hm_model::units::mul_div;
use hm_model::{Control, ObjectCommon, Paragraph, Shape, TextBox};

use crate::heads::Heads;
use crate::math::{self, MathLayout};
use crate::measure::InlineBox;
use crate::paginate::Rect;
use crate::para::{ParaEnv, ParaLayout};
use crate::style::Styles;
use crate::table::{CaptionLayout, CellLine};

/// What an object holds inside its box, and its caption, laid out.
#[derive(Clone, Debug, Default)]
pub struct ObjectLayout {
    pub equation: Option<MathLayout>,
    /// The text of a drawing object's shapes, one frame per text box in
    /// the order the shape tree is walked (depth first).
    pub frames: Vec<TextFrame>,
    /// A form control's text.
    pub form: Option<crate::forms::FormText>,
    pub caption: Option<CaptionLayout>,
}

impl ObjectLayout {
    /// Where the object's own box sits inside the box it shares with its
    /// caption.
    pub fn object_offset(&self) -> (i32, i32) {
        match &self.caption {
            Some(c) if c.side == 2 => (0, c.height + c.gap),
            Some(c) if c.side == 0 => (c.width + c.gap, 0),
            _ => (0, 0),
        }
    }

    /// Where the caption sits relative to the object's box of size `(w, h)`.
    pub fn caption_offset(&self, (w, h): (i32, i32)) -> Option<(i32, i32)> {
        let c = self.caption.as_ref()?;
        Some(match c.side {
            0 => (-c.gap - c.width, 0),
            1 => (w + c.gap, 0),
            2 => (0, -c.gap - c.height),
            _ => (0, h + c.gap),
        })
    }
}

/// The paragraphs of one text box, stacked from the top of its content.
#[derive(Clone, Debug, Default)]
pub struct TextFrame {
    pub paras: Vec<ParaLayout>,
    pub lines: Vec<CellLine>,
    /// The last line's bottom.
    pub content: i32,
    /// The width the paragraphs were formatted for.
    pub width: i32,
    /// The paragraphs shown, when they belong to the box whose text flows
    /// on into this one rather than to this box.
    pub borrowed: Option<std::sync::Arc<Vec<Paragraph>>>,
}

/// Section default tab interval used inside shapes.
const TEXT_TAB: i32 = 4000;

/// Lays out every text box of a shape tree, depth first.
fn shape_text(shape: &Shape, styles: &mut Styles<'_>, out: &mut Vec<TextFrame>) {
    if let Some(t) = shape.text() {
        let (w, h) = shape.component.size;
        out.push(text_frame(t, (w as i32, h as i32), styles));
    }
    for child in shape.children() {
        shape_text(child, styles, out);
    }
}

/// The paragraphs of a text box formatted for its width less its side
/// margins; vertical text runs down the box, so its lines take the height
/// less the top and bottom margins.
pub fn text_frame(
    t: &TextBox,
    (shape_w, shape_h): (i32, i32),
    styles: &mut Styles<'_>,
) -> TextFrame {
    let [ml, mr, mt, mb] = t.margins.unwrap_or([0; 4]).map(i32::from);
    let width = if t.vertical() {
        (shape_h - mt - mb).max(0)
    } else {
        (t.max_width.filter(|&m| m > 0).unwrap_or(shape_w) - ml - mr).max(0)
    };
    let info = styles.info();
    let env = ParaEnv {
        column_width: width,
        default_tab: TEXT_TAB,
    };
    let mut heads = Heads::default();
    let (paras, lines, content) = crate::table::stack(&t.paras, info, styles, env, &mut heads);
    TextFrame {
        paras,
        lines,
        content,
        width,
        borrowed: None,
    }
}

/// A text box of a paragraph's drawing objects, for linking.
struct LinkedBox<'a> {
    object: usize,
    frame: usize,
    text: &'a TextBox,
    /// Height of the text area.
    room: i32,
}

fn text_boxes<'a>(shape: &'a Shape, out: &mut Vec<(&'a TextBox, i32)>) {
    if let Some(t) = shape.text() {
        let [_, _, mt, mb] = t.margins.unwrap_or([0; 4]).map(i32::from);
        out.push((t, shape.component.size.1 as i32 - mt - mb));
    }
    for child in shape.children() {
        text_boxes(child, out);
    }
}

/// Flows the text of linked text boxes on from box to box: the lines of a
/// box that do not fit its height go on in the box it links to, broken
/// again for that box's width. `objects` are the paragraph's measured
/// objects (control index and layout).
pub fn link_text_boxes(
    para: &Paragraph,
    objects: &mut [(u32, ObjectLayout)],
    styles: &mut Styles<'_>,
) {
    let mut boxes: Vec<LinkedBox<'_>> = Vec::new();
    for (oi, (k, _)) in objects.iter().enumerate() {
        let Some(shape) = para
            .controls
            .get(*k as usize)
            .and_then(|c| c.as_object())
            .and_then(|o| o.shape())
        else {
            continue;
        };
        let mut found = Vec::new();
        text_boxes(shape, &mut found);
        for (fi, (text, room)) in found.into_iter().enumerate() {
            boxes.push(LinkedBox {
                object: oi,
                frame: fi,
                text,
                room,
            });
        }
    }
    if !boxes.iter().any(|b| b.text.links().1 != 0) {
        return;
    }
    let info = styles.info();
    // Chains start at boxes no other box links to.
    let targets: Vec<u32> = boxes.iter().map(|b| b.text.links().1).collect();
    for head in 0..boxes.len() {
        let (id, _) = boxes[head].text.links();
        if id != 0 && targets.contains(&id) {
            continue;
        }
        let source = std::sync::Arc::new(boxes[head].text.paras.clone());
        let mut cur = head;
        let mut guard = 0;
        loop {
            guard += 1;
            let next = boxes[cur].text.links().1;
            let Some(to) = boxes
                .iter()
                .position(|b| next != 0 && b.text.links().0 == next)
            else {
                break;
            };
            if guard > boxes.len() {
                break;
            }
            let (from_o, from_f, room) = (boxes[cur].object, boxes[cur].frame, boxes[cur].room);
            let (to_o, to_f) = (boxes[to].object, boxes[to].frame);
            let Some(frame) = objects[from_o].1.frames.get(from_f).cloned() else {
                break;
            };
            let Some(cut) = frame.lines.iter().position(|l| {
                let h = frame
                    .paras
                    .get(l.para as usize)
                    .and_then(|p| p.lines.get(l.line as usize))
                    .map_or(0, |ln| ln.height);
                l.y + h > room
            }) else {
                break;
            };
            let Some(width) = objects[to_o].1.frames.get(to_f).map(|f| f.width) else {
                break;
            };
            // The lines from the cut on, broken again for the next box.
            let mut paras = frame.paras.clone();
            let first = frame.lines[cut];
            let mut lines = Vec::new();
            let mut y = 0;
            let start = first.para as usize;
            for (q, (pl, p)) in paras.iter_mut().zip(source.iter()).enumerate().skip(start) {
                let from = if q == start { first.line as usize } else { 0 };
                // The paragraph's full segment in the next box: its width
                // less the paragraph's margins.
                let margins = info
                    .para_shape(p.header.para_shape as u32)
                    .map_or(0, |s| s.left(1000) + s.right(1000));
                let full = (width - margins).max(0);
                crate::para::reflow(pl, p, info, styles, from, &mut |_| (0, full));
                if q != start {
                    y += pl.before;
                }
                for (li, l) in pl.lines.iter().enumerate().skip(from) {
                    lines.push(CellLine {
                        para: q as u32,
                        line: li as u32,
                        y,
                    });
                    y += l.advance();
                }
                y += pl.after;
            }
            let content = lines
                .last()
                .and_then(|l| {
                    paras[l.para as usize]
                        .lines
                        .get(l.line as usize)
                        .map(|ln| l.y + ln.height)
                })
                .unwrap_or(0);
            let src = &mut objects[from_o].1.frames[from_f];
            src.lines.truncate(cut);
            src.content = src.lines.last().map_or(0, |l| {
                l.y + src
                    .paras
                    .get(l.para as usize)
                    .and_then(|p| p.lines.get(l.line as usize))
                    .map_or(0, |ln| ln.height)
            });
            if let Some(dst) = objects[to_o].1.frames.get_mut(to_f) {
                *dst = TextFrame {
                    paras,
                    lines,
                    content,
                    width,
                    borrowed: Some(source.clone()),
                };
            }
            cur = to;
        }
    }
}

/// Measures an object control: its box as a character when it is treated
/// as one, and its inner layout. `None` for controls that are not objects.
pub fn measure_object(
    c: &Control,
    styles: &mut Styles<'_>,
) -> Option<(Option<InlineBox>, Option<ObjectLayout>)> {
    let fonts = styles.fonts();
    let o = c.object_common()?;
    let [ml, mr, mt, mb] = o.margins.map(i32::from);
    let mut width = o.width;
    let mut height = o.height;
    let mut percent = 85;
    let mut inner = ObjectLayout::default();
    if let Some(eq) = c.as_object().and_then(|x| x.equation()) {
        // Hancom recomputes an equation's width from its script on open; the
        // stored height and baseline are kept unless missing or primed.
        // One stored under a tenth of its base size high Hancom keeps at
        // its stored box and draws as nothing.
        let collapsed = height > 0 && height.saturating_mul(10) < eq.size as i32;
        let ml_layout = if collapsed {
            MathLayout::default()
        } else {
            math::layout(eq, fonts)
        };
        if !eq.script.is_empty() && !collapsed {
            width = ml_layout.width;
        }
        // Hancom for Windows stores some primed equations with the prime
        // raised above the line box; Hancom for Mac sizes those anew. A
        // primed equation stored over a tenth taller than its script lays
        // out takes the laid out size.
        let primed = {
            let s = eq.script.to_string_lossy();
            s.contains("prime") || s.contains('\'')
        };
        let stale = primed && height * 10 > ml_layout.height() * 11;
        // A stored box well under the script's laid out size both ways was
        // not sized by Hancom (Hancom's own boxes are at least as wide as
        // the script): Hancom sizes such an equation anew.
        let foreign = ml_layout.height() * 10 > height * 12 && ml_layout.width * 10 > o.width * 11;
        if height <= 0 || stale || foreign {
            height = ml_layout.height();
            percent = ml_layout.baseline_percent();
        } else {
            percent = eq.baseline as i32;
        }
        inner.equation = Some(ml_layout);
    } else if let Some(shape) = c.as_object().and_then(|x| x.shape()) {
        // An OLE object keeps its baseline as a percentage of its height.
        if let Some(hm_model::ShapePart::Ole(ole)) = shape.kind() {
            let b = ((ole.attr >> 9) & 0x7F) as i32;
            if b > 0 {
                percent = b;
            }
        }
        shape_text(shape, styles, &mut inner.frames);
        // A text box grows to its text: its frame (the shape's current
        // size) and the line holding it take the content plus the text
        // margins when that exceeds the stored height.
        if let (Some(t), Some(frame)) = (shape.text(), inner.frames.first()) {
            let [_, _, tt, tb] = t.margins.unwrap_or([0; 4]).map(i32::from);
            height = height.max(frame.content + tt + tb);
        }
    } else if let Some(form) = c.as_form() {
        inner.form = Some(crate::forms::measure(form, styles));
    }
    if let Some(cap) = c.as_object().and_then(|x| x.caption()) {
        let side = cap.side();
        let w = if side >= 2 {
            cap.max_width.filter(|&m| m > 0).unwrap_or(width)
        } else {
            cap.width.unwrap_or(0)
        };
        let env = ParaEnv {
            column_width: w.max(0),
            default_tab: TEXT_TAB,
        };
        let info = styles.info();
        let mut heads = Heads::default();
        let (paras, lines, h) = crate::table::stack(&cap.paras, info, styles, env, &mut heads);
        inner.caption = Some(CaptionLayout {
            side,
            gap: cap.gap.unwrap_or(0) as i32,
            width: w,
            height: h,
            paras,
            lines,
        });
    }
    // A caption shares the object's box as a character.
    let (bw, bh) = match &inner.caption {
        Some(c) if c.side >= 2 => (width.max(c.width), height + c.gap + c.height),
        Some(c) => (width + c.gap + c.width, height.max(c.height)),
        None => (width, height),
    };
    let inline = o.treat_as_char().then(|| {
        let h = bh + mt + mb;
        InlineBox {
            pos: 0,
            control: 0,
            width: bw + ml + mr,
            height: h,
            baseline: mul_div(h, percent, 100),
        }
    });
    let inner = (inner.equation.is_some()
        || !inner.frames.is_empty()
        || inner.form.is_some()
        || inner.caption.is_some())
    .then_some(inner);
    Some((inline, inner))
}

/// How a floating object stacks with the text; layers draw in this order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Layer {
    /// Behind the text.
    Behind,
    /// Beside or between the text (square, tight, through, top and bottom).
    Text,
    /// In front of the text.
    Front,
}

/// A floating object placed on a page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    /// Paragraph index in its list.
    pub para: u32,
    /// Index into the paragraph's controls.
    pub control: u32,
    /// The object's box (size without outer margins), page coordinates.
    pub rect: Rect,
    pub z: i32,
    pub layer: Layer,
}

/// The layer an object not treated as a character is drawn in.
pub fn layer_of(o: &ObjectCommon) -> Layer {
    match o.text_wrap() {
        2 => Layer::Behind,
        3 => Layer::Front,
        _ => Layer::Text,
    }
}

/// Places the floating objects (those not treated as characters) of a
/// paragraph, `para` being its index in its list.
pub fn place(paragraph: &Paragraph, para: u32, a: &Anchor) -> Vec<Placement> {
    place_of(paragraph, para, a, &|_| true)
}

/// [`place`] for the controls `keep` selects.
pub fn place_of(
    paragraph: &Paragraph,
    para: u32,
    a: &Anchor,
    keep: &dyn Fn(usize) -> bool,
) -> Vec<Placement> {
    (0..paragraph.controls.len() as u32)
        .filter(|&k| keep(k as usize))
        .filter_map(|k| place_one(paragraph, para, k, a))
        .collect()
}

/// The line of a paragraph's layout a paragraph-relative object hangs
/// from: the one holding its control. Other objects, and controls not
/// found, hang from the first line.
pub fn anchor_line(paragraph: &Paragraph, pl: &ParaLayout, k: usize) -> usize {
    let relative = paragraph
        .controls
        .get(k)
        .and_then(|c| c.object_common())
        .is_some_and(|o| !o.treat_as_char() && o.vert_rel_to() == 2);
    if !relative {
        return 0;
    }
    let Some(pos) = paragraph.units().find_map(|(pos, u)| match u {
        hm_model::Unit::Extended { index, .. } if index as usize == k => Some(pos),
        _ => None,
    }) else {
        return 0;
    };
    pl.lines.iter().rposition(|l| l.start <= pos).unwrap_or(0)
}

/// The control `k` of a paragraph a paragraph-relative object kept inside
/// the body (flowing with the text) whose box, outer margins included,
/// crosses the foot of `body` at placement `p`.
pub fn spills(paragraph: &Paragraph, p: &Placement, body: Rect) -> Option<u32> {
    const FLOW_WITH_TEXT: u32 = 1 << 13;
    let o = paragraph
        .controls
        .get(p.control as usize)?
        .object_common()?;
    let mb = i32::from(o.margins[3]);
    (o.vert_rel_to() == 2
        && o.attr & FLOW_WITH_TEXT != 0
        && p.rect.y + p.rect.h + mb > body.y + body.h)
        .then_some(p.control)
}

/// Places floating object `k` of a paragraph.
pub fn place_one(paragraph: &Paragraph, para: u32, k: u32, a: &Anchor) -> Option<Placement> {
    let c = paragraph.controls.get(k as usize)?;
    let o = sized(&c.object_common()?, a);
    if o.treat_as_char() || c.as_object().is_none() {
        return None;
    }
    // The offsets place the box with its outer margins; the object
    // sits inside them.
    let [ml, mr, mt, mb] = o.margins.map(i32::from);
    let outer = ObjectCommon {
        width: ml + o.width + mr,
        height: mt + o.height + mb,
        ..o
    };
    let (x, y, _, _) = frame(&outer, a);
    let (x, y, w, h) = (x + ml, y + mt, o.width, o.height);
    Some(Placement {
        para,
        control: k,
        rect: Rect { x, y, w, h },
        z: o.z_order,
        layer: layer_of(&o),
    })
}

/// A vertical stretch of the page that lines must not overlap.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Band {
    pub top: i32,
    pub bottom: i32,
    pub left: i32,
    pub right: i32,
    /// Square wrapping: a line that fits beside the object stays there;
    /// one that does not moves down by its own pitch.
    pub square: bool,
    /// A page- or paper-anchored object at the page foot: a line clears
    /// it with its spacing, not only its height.
    pub pitch: bool,
    /// Square wrapping: the sides text takes (0 both, 1 left, 2 right,
    /// 3 the wider).
    pub side: u8,
    /// A flowing table's fragment: a stored line clears it by its stored
    /// segment, which Hancom narrows only beside objects it wraps.
    pub table: bool,
}

impl Band {
    fn overlaps_x(&self, left: i32, right: i32) -> bool {
        self.left < right && left < self.right
    }
}

/// Where an object is anchored on its page.
#[derive(Clone, Copy, Debug)]
pub struct Anchor {
    pub paper: (i32, i32),
    /// The horizontal span paper-relative objects use, when it is not the
    /// paper: objects in headers and footers measure from the body.
    pub paper_x: Option<(i32, i32)>,
    pub body: Rect,
    /// The column the anchoring paragraph is in (page coordinates).
    pub column: (i32, i32),
    /// Top of the anchoring paragraph (page coordinates).
    pub para_top: i32,
    pub para_left: i32,
}

/// The object with its size resolved: a width kept relative to the paper,
/// the page body, the column or the paragraph (attribute bits 15-17), or a
/// height relative to the paper or the page body (bits 18-19), is stored in
/// hundredths of a percent of it.
pub fn sized(o: &ObjectCommon, a: &Anchor) -> ObjectCommon {
    let of = |whole: i32, part: i32| mul_div(whole, part, 10_000);
    let width = match (o.attr >> 15) & 7 {
        0 => of(a.paper.0, o.width),
        1 => of(a.body.w, o.width),
        2 => of(a.column.1, o.width),
        3 => of(a.column.0 + a.column.1 - a.para_left, o.width),
        _ => o.width,
    };
    let height = match (o.attr >> 18) & 3 {
        0 => of(a.paper.1, o.height),
        1 => of(a.body.h, o.height),
        _ => o.height,
    };
    ObjectCommon {
        width,
        height,
        ..*o
    }
}

/// Places `extent` inside `[start, start + size)` by Hancom's alignment code:
/// 0 start, 1 center, 2 end; offsets count inward from the aligned edge.
fn align(start: i32, size: i32, extent: i32, code: u32, offset: i32) -> i32 {
    match code {
        1 => start + (size - extent) / 2 + offset,
        2 => start + size - extent - offset,
        _ => start + offset,
    }
}

/// The page rectangle `(x, y, w, h)` of an object, margins excluded.
pub fn frame(o: &ObjectCommon, a: &Anchor) -> (i32, i32, i32, i32) {
    let (vs, vh) = match o.vert_rel_to() {
        0 => (0, a.paper.1),
        1 => (a.body.y, a.body.h),
        _ => (a.para_top, a.body.y + a.body.h - a.para_top),
    };
    let (hs, hw) = match (o.attr >> 8) & 3 {
        0 => a.paper_x.unwrap_or((0, a.paper.0)),
        1 => (a.body.x, a.body.w),
        2 => a.column,
        _ => (a.para_left, a.column.0 + a.column.1 - a.para_left),
    };
    let valign = if o.vert_rel_to() == 2 {
        0
    } else {
        (o.attr >> 5) & 7
    };
    let y = align(vs, vh, o.height, valign, o.y_offset);
    let x = align(hs, hw, o.width, (o.attr >> 10) & 7, o.x_offset);
    (x, y, o.width, o.height)
}

/// Where a paragraph's top-and-bottom and square objects block the page.
#[derive(Default)]
pub struct Bands {
    /// Paper- and page-relative objects: they apply to the anchoring
    /// paragraph itself.
    pub now: Vec<Band>,
    /// Paragraph-relative objects the anchoring paragraph's own lines
    /// avoid: square ones, and top-and-bottom ones that flow with the text
    /// and may overlap other objects.
    pub own: Vec<Band>,
    /// Other paragraph-relative objects, which sit at the paragraph's flow
    /// position and push only the paragraphs after it.
    pub after: Vec<Band>,
}

/// The bands a paragraph's objects block, from its layout `pl` (for
/// captions) and anchor `a`. An object occupies its outer margins plus its
/// size and caption, starting at the anchor offset.
pub fn bands(para: &Paragraph, pl: &ParaLayout, a: &Anchor) -> Bands {
    bands_of(para, pl, a, &|_| true)
}

/// [`bands`] for the controls `keep` selects.
pub fn bands_of(
    para: &Paragraph,
    pl: &ParaLayout,
    a: &Anchor,
    keep: &dyn Fn(usize) -> bool,
) -> Bands {
    let mut out = Bands::default();
    for (k, c) in para.controls.iter().enumerate() {
        if !keep(k) {
            continue;
        }
        // Flowing tables push the content themselves, across pages.
        if crate::table::flows(c) {
            continue;
        }
        let Some(o) = c.object_common() else {
            continue;
        };
        let o = if c.as_object().is_some() {
            sized(&o, a)
        } else {
            o
        };
        if o.treat_as_char() || !matches!(o.text_wrap(), 0 | 1) || o.width <= 0 || o.height <= 0 {
            continue;
        }
        let [ml, mr, mt, mb] = o.margins.map(i32::from);
        let outer = ObjectCommon {
            width: ml + o.width + mr,
            height: mt + o.height + mb,
            ..o
        };
        let (x, y, w, h) = frame(&outer, a);
        let mut band = Band {
            top: y,
            bottom: y + h,
            left: x,
            right: x + w,
            square: o.text_wrap() == 0,
            // Only an object at the page foot keeps a line's spacing off
            // it; a line may end right above one in mid page.
            pitch: o.vert_rel_to() != 2 && (o.attr >> 5) & 7 == 2,
            side: ((o.attr >> 24) & 3) as u8,
            table: false,
        };
        let caption = pl
            .measured
            .objects
            .iter()
            .find(|(i, _)| *i as usize == k)
            .and_then(|(_, l)| l.caption.as_ref());
        if let Some(cap) = caption {
            match cap.side {
                0 => band.left -= cap.width + cap.gap,
                1 => band.right += cap.width + cap.gap,
                2 => band.top -= cap.height + cap.gap,
                _ => band.bottom += cap.height + cap.gap,
            }
        }
        const FLOW_WITH_TEXT: u32 = 1 << 13;
        const ALLOW_OVERLAP: u32 = 1 << 14;
        // Text wraps beside a square object from the line holding it on.
        // A top-and-bottom one that flows with the text pushes its own line
        // below it when that line holds text or a drawing set in the line;
        // an empty host line, or one holding a table, stays where the
        // paragraph starts.
        if o.vert_rel_to() != 2 {
            out.now.push(band);
        } else if o.text_wrap() == 0
            || o.attr & (FLOW_WITH_TEXT | ALLOW_OVERLAP) == FLOW_WITH_TEXT | ALLOW_OVERLAP
            || (o.attr & FLOW_WITH_TEXT != 0
                && o.text_wrap() == 1
                && pl
                    .lines
                    .get(anchor_line(para, pl, k))
                    .is_some_and(|l| shows_text(para, l)))
        {
            out.own.push(band);
        } else {
            out.after.push(band);
        }
    }
    out
}

/// True when a line holds a character other than a space or a drawing
/// object set as a character (a table set in the line does not count).
fn shows_text(para: &Paragraph, line: &crate::para::Line) -> bool {
    para.units().any(|(pos, u)| {
        pos >= line.start
            && pos < line.end
            && match u {
                hm_model::Unit::Char(c) => !c.is_whitespace(),
                hm_model::Unit::Extended { index, .. } => para
                    .controls
                    .get(index as usize)
                    .and_then(|c| c.as_object())
                    .is_some_and(|o| o.header.common.treat_as_char()),
                _ => false,
            }
    })
}

/// The lowest bottom, below the paragraph's top `top`, of a paragraph's
/// top-and-bottom objects anchored to it in a stacked list (a cell or a
/// note), outer margins and captions included; `top` when it has none.
/// Tables flowing with the text place themselves.
pub fn drop_below(para: &Paragraph, pl: &ParaLayout, top: i32) -> i32 {
    let mut floor = top;
    for (k, c) in para.controls.iter().enumerate() {
        let Some(o) = c.object_common() else {
            continue;
        };
        if o.treat_as_char() || o.text_wrap() != 1 || o.vert_rel_to() != 2 || c.as_table().is_some()
        {
            continue;
        }
        let [_, _, mt, mb] = o.margins.map(i32::from);
        let caption = pl
            .measured
            .objects
            .iter()
            .find(|(i, _)| *i as usize == k)
            .and_then(|(_, l)| l.caption.as_ref())
            .filter(|cap| cap.side >= 2)
            .map_or(0, |cap| cap.height + cap.gap);
        floor = floor.max(top + o.y_offset + mt + o.height + mb + caption);
    }
    floor
}

/// The narrowest gap beside square-wrapped objects that takes text: Hancom
/// leaves a 1,070 HU gap empty and fills 1,600 HU ones.
pub const MIN_GAP: i32 = 1440;

/// The gaps square-wrapped objects leave, left to right, beside a line of
/// `height` at `top` in the column `[left, right)` once top-and-bottom
/// objects have pushed the line down: `(line top, gaps)`, the gaps empty
/// when no square object meets the line. Each object keeps text to the
/// sides it allows; gaps narrower than [`MIN_GAP`] stay empty.
pub fn square_gaps(
    bands: &[Band],
    top: i32,
    height: i32,
    left: i32,
    right: i32,
) -> (i32, Vec<(i32, i32)>) {
    let full: Vec<Band> = bands.iter().filter(|b| !b.square).copied().collect();
    let top = clear(&full, top, height, left, right);
    let beside: Vec<&Band> = bands
        .iter()
        .filter(|b| {
            b.square && b.overlaps_x(left, right) && b.top < top + height.max(1) && top < b.bottom
        })
        .collect();
    if beside.is_empty() {
        return (top, Vec::new());
    }
    let mut gaps = vec![(left, right)];
    for b in &beside {
        gaps = gaps
            .into_iter()
            .flat_map(|(a, z)| {
                if b.right <= a || z <= b.left {
                    return vec![(a, z)];
                }
                // The parts left and right of the object, as it allows.
                let keep_left = b.side != 2;
                let keep_right = b.side != 1;
                let mut out = Vec::new();
                if keep_left && a < b.left {
                    out.push((a, b.left));
                }
                if keep_right && b.right < z {
                    out.push((b.right, z));
                }
                out
            })
            .collect();
    }
    if beside.iter().any(|b| b.side == 3)
        && let Some(&widest) = gaps.iter().max_by_key(|g| g.1 - g.0)
    {
        gaps = vec![widest];
    }
    gaps.retain(|g| g.1 - g.0 >= MIN_GAP);
    (top, gaps)
}

/// Places a line of `height` and `pitch` (height plus spacing) whose
/// content spans `span` from the column's left edge `left`: below every
/// top-and-bottom band it overlaps; beside a square-wrapped object when
/// its content clears the object or fits the wider gap there (moving it
/// right), else one pitch further down until it clears. A line beside an
/// object that leaves half the column or more keeps its place. Returns the
/// line top and how far right it moves.
pub fn clear_line(
    bands: &[Band],
    mut top: i32,
    (height, pitch): (i32, i32),
    span: (i32, i32),
    stored: Option<(i32, i32)>,
    left: i32,
    right: i32,
) -> (i32, i32) {
    let full: Vec<Band> = bands.iter().filter(|b| !b.square).copied().collect();
    for _ in 0..10_000 {
        top = clear_pitch(&full, top, (height, pitch), left, right);
        let hit = bands.iter().find(|b| {
            b.square && b.overlaps_x(left, right) && b.top < top + height.max(1) && top < b.bottom
        });
        let Some(b) = hit else {
            return (top, 0);
        };
        let span = match stored {
            Some(seg) if b.table => seg,
            _ => span,
        };
        if span.1 <= span.0 || !b.overlaps_x(left + span.0, left + span.1) {
            return (top, 0);
        }
        let after = right - b.right;
        if span.1 <= after {
            return (top, b.right - left);
        }
        let before = b.left - left;
        if before.max(after) * 2 >= right - left {
            return (top, 0);
        }
        top = if pitch > 0 { top + pitch } else { b.bottom };
    }
    (top, 0)
}

/// Like [`clear`], a line clearing page- and paper-anchored bands with its
/// whole pitch: Hancom keeps a line's spacing off such an object, so a
/// line whose spacing would reach an object anchored at the page foot
/// moves below it, and with it to the next page.
fn clear_pitch(
    bands: &[Band],
    mut top: i32,
    (height, pitch): (i32, i32),
    left: i32,
    right: i32,
) -> i32 {
    loop {
        let hit = bands
            .iter()
            .filter(|b| {
                let reach = if b.pitch { pitch.max(height) } else { height };
                b.overlaps_x(left, right) && b.top < top + reach.max(1) && top < b.bottom
            })
            .map(|b| b.bottom)
            .max();
        match hit {
            Some(bottom) if bottom > top => top = bottom,
            _ => return top,
        }
    }
}

/// Moves `top` below every band that a line `[top, top + height)` spanning
/// `[left, right)` would overlap.
pub fn clear(bands: &[Band], mut top: i32, height: i32, left: i32, right: i32) -> i32 {
    loop {
        let hit = bands
            .iter()
            .filter(|b| b.overlaps_x(left, right) && b.top < top + height.max(1) && top < b.bottom)
            .map(|b| b.bottom)
            .max();
        match hit {
            Some(bottom) if bottom > top => top = bottom,
            _ => return top,
        }
    }
}
