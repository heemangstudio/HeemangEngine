//! Overlaid characters (글자 겹치기) and ruby text (덧말) as Mac Hancom
//! prints them. Both take one box in the line, like an object set as a
//! character, and carry their own glyphs.
//!
//! Overlaid characters: a frame shape is a symbol glyph at the run's size
//! (◯, ●, □, ...) whose advance is the box width; the characters sit
//! inside it at `100 + 10 * step` percent of the size, centered, raised
//! by a per-shape share of the size difference, in the inverse color on
//! reversed shapes. Without a frame the characters overlap at full size,
//! or spread centered over a full-width box.
//!
//! Ruby text: the main text sets the box width with its own advances; the
//! annotation, at its size ratio of the base size with ratio 100 and no
//! spacing, narrows down to 50 percent to fit the main width, and sits on
//! the main text's ascent (the face's ascent share of the size) by its own
//! descent, or that far plus both sizes lower for the bottom position. The
//! box adds the annotation's size to the line height, and the centered
//! position prints like the top one.

use std::sync::Arc;

use hm_model::units::mul_div;
use hm_model::{Compose, Control, Dutmal, Paragraph, Script};

use crate::measure::{InlineBox, Kind, Measured, natural_advance, tracked};
use crate::script::script_of;
use crate::style::{Styles, line_size};

/// One glyph of an annotation box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoxGlyph {
    pub ch: char,
    /// The style giving face and color.
    pub style: u32,
    /// Em size and horizontal scale percent.
    pub size: i32,
    pub ratio: i32,
    /// Offset from the box's left edge and from the line baseline (down).
    pub dx: i32,
    pub dy: i32,
    /// Color replacing the style's (reversed frames).
    pub color: Option<u32>,
}

/// The glyphs of an overlaid-characters or ruby box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AnnotLayout {
    pub glyphs: Vec<BoxGlyph>,
}

/// The frame glyph of each shape code. Mac Hancom prints the reversed
/// triangle as a circle.
const FRAMES: [char; 14] = [
    '\0', '◯', '●', '□', '■', '△', '◯', '☼', '◇', '◆', '▢', '♲', '♺', '♻',
];

/// How far the inner characters rise, as a share of the frame size less
/// their size: two fifths in circles, 22/75 in the triangle, a third in
/// the other shapes.
fn rise(shape: u8) -> (i32, i32) {
    match shape {
        1 | 6 | 11..=13 => (2, 5),
        5 => (22, 75),
        _ => (1, 3),
    }
}

/// The digit of Hancom's private-use circled, boxed and reversed boxed
/// digits. Inside a frame, when the run's face lacks the glyph, Hancom
/// sets the plain digit in the frame instead.
fn framed_digit(ch: char) -> Option<char> {
    let c = ch as u32;
    let d = match c {
        0xF0288..=0xF0291 => c - 0xF0288,
        0xF02B0..=0xF02B9 => c - 0xF02B0,
        0xF02CD..=0xF02D6 => c - 0xF02CD,
        _ => return None,
    };
    char::from_digit(d, 10)
}

fn has_glyph(styles: &mut Styles<'_>, cs: u32, ch: char) -> bool {
    let st = styles.style_for(cs, script(ch), None);
    styles.face(st).is_some_and(|f| f.has(ch))
}

fn reversed(shape: u8) -> bool {
    matches!(shape, 2 | 4 | 9)
}

/// Lays out an overlaid-characters or ruby control at wire position `pos`
/// into its eight units. False for other controls.
pub fn measure(
    m: &mut Measured,
    para: &Paragraph,
    pos: u32,
    index: u32,
    cs: u32,
    styles: &mut Styles<'_>,
) -> bool {
    let Some(c) = para.controls.get(index as usize) else {
        return false;
    };
    let base = styles.char_shape_ref(cs).map_or(1000, line_size);
    let (b, layout, line_style) = match c.kind {
        hm_model::ControlKind::Compose(ref k) => {
            let (b, l) = compose(k, cs, base, styles);
            (b, l, styles.style_for(cs, Script::Hangul, None))
        }
        hm_model::ControlKind::Dutmal(ref d) => {
            let (b, l) = dutmal(d, cs, base, styles);
            // Percent line spacing counts the annotation as text.
            let st = styles.style_for(cs, Script::Hangul, None);
            let h = b.height;
            (b, l, styles.derived(st, |s| s.line_size = h))
        }
        _ => return false,
    };
    let p = pos as usize;
    let n = m.styles.len();
    for k in p..(p + 8).min(n) {
        m.styles[k] = line_style;
    }
    if p < n {
        m.advances[p] = b.width;
        m.natural[p] = b.width;
    }
    m.boxes.push(InlineBox {
        pos,
        control: index,
        ..b
    });
    m.items.push((pos, Kind::Object(m.boxes.len() as u32 - 1)));
    m.annots.push((index, Arc::new(layout)));
    true
}

fn script(c: char) -> Script {
    script_of(c).unwrap_or(Script::Latin)
}

/// The advance of `ch` in style `st` drawn at `size` and `ratio`. Blanks
/// take Hancom's width, an eighth of the size, rather than the font's.
fn advance_at(styles: &Styles<'_>, st: u32, ch: char, size: i32, ratio: i32) -> i32 {
    if ch == ' ' {
        return mul_div(size / 8, ratio, 100) * 4;
    }
    let mut style = styles.get(st).clone();
    style.size = size;
    style.ratio = ratio;
    natural_advance(&style, styles.face(st).map(|f| &**f), ch)
}

fn text_box(width: i32, height: i32) -> InlineBox {
    InlineBox {
        pos: 0,
        control: 0,
        width,
        height,
        baseline: mul_div(height, 85, 100),
    }
}

fn compose(k: &Compose, cs: u32, base: i32, styles: &mut Styles<'_>) -> (InlineBox, AnnotLayout) {
    let chars = k.chars();
    let (shape, step, spread) = k
        .frame
        .as_ref()
        .map_or((0, 0, false), |f| (f.shape, f.inner_size, f.kind == 0));
    let frame = FRAMES.get(shape as usize).copied().unwrap_or('\0');
    let mut out = AnnotLayout::default();
    // Each character in its own char shape, at the frame's size by default.
    let inner: Vec<(char, u32)> = chars
        .iter()
        .enumerate()
        .map(|(i, &ch)| {
            let id = k.char_shape(i).unwrap_or(cs);
            let ch = match framed_digit(ch) {
                Some(d) if frame != '\0' && !has_glyph(styles, id, ch) => d,
                _ => ch,
            };
            (ch, styles.style_for(id, script(ch), Some(ch)))
        })
        .collect();
    let frame_style = (frame != '\0').then(|| styles.style_for(cs, Script::Symbol, Some(frame)));
    let size_of = |st: u32, styles: &Styles<'_>| styles.get(st).size;
    match frame_style {
        Some(fs) => {
            let size = size_of(fs, styles);
            let ratio = styles.get(fs).ratio;
            let width = advance_at(styles, fs, frame, size, ratio);
            out.glyphs.push(BoxGlyph {
                ch: frame,
                style: fs,
                size,
                ratio,
                dx: 0,
                dy: 0,
                color: None,
            });
            let inner_size = mul_div(size, 100 + 10 * step as i32, 100).max(1);
            let (num, den) = rise(shape);
            let dy = -mul_div(size - inner_size, num, den);
            let color = reversed(shape).then(|| styles.get(fs).color ^ 0x00FF_FFFF);
            place_inner(
                &mut out,
                styles,
                &inner,
                inner_size,
                width,
                mul_div(width, 5, 6),
                spread,
                dy,
                color,
            );
            (text_box(width, base), out)
        }
        None if spread => {
            // A full-width box, as wide as a circle frame would be.
            let fs = styles.style_for(cs, Script::Symbol, Some('◯'));
            let st = styles.get(fs);
            let width = advance_at(styles, fs, '◯', st.size, st.ratio);
            let size = inner.first().map_or(base, |&(_, s)| size_of(s, styles));
            place_inner(&mut out, styles, &inner, size, width, width, true, 0, None);
            (text_box(width, base), out)
        }
        None => {
            let width = inner
                .iter()
                .map(|&(ch, st)| {
                    let s = styles.get(st);
                    advance_at(styles, st, ch, s.size, s.ratio)
                })
                .max()
                .unwrap_or(0);
            let size = inner.first().map_or(base, |&(_, s)| size_of(s, styles));
            place_inner(&mut out, styles, &inner, size, width, width, false, 0, None);
            (text_box(width, base), out)
        }
    }
}

/// Sets the characters in a box `width` wide: side by side and centered
/// (narrowed to `limit` when wider), or each centered over the others.
#[allow(clippy::too_many_arguments)]
fn place_inner(
    out: &mut AnnotLayout,
    styles: &Styles<'_>,
    inner: &[(char, u32)],
    size: i32,
    width: i32,
    limit: i32,
    spread: bool,
    dy: i32,
    color: Option<u32>,
) {
    let adv = |ch: char, st: u32, ratio: i32| advance_at(styles, st, ch, size, ratio);
    if spread {
        let total: i64 = inner
            .iter()
            .map(|&(ch, st)| adv(ch, st, styles.get(st).ratio) as i64)
            .sum();
        let scale = if total > limit as i64 && total > 0 {
            (limit as i64 * 100 / total) as i32
        } else {
            100
        };
        let advs: Vec<(i32, i32)> = inner
            .iter()
            .map(|&(ch, st)| {
                let r = mul_div(styles.get(st).ratio, scale, 100).max(1);
                (r, adv(ch, st, r))
            })
            .collect();
        let used: i32 = advs.iter().map(|a| a.1).sum();
        let mut x = (width - used) / 2;
        for (&(ch, st), &(ratio, a)) in inner.iter().zip(&advs) {
            out.glyphs.push(BoxGlyph {
                ch,
                style: st,
                size,
                ratio,
                dx: x,
                dy,
                color,
            });
            x += a;
        }
    } else {
        for &(ch, st) in inner {
            let ratio = styles.get(st).ratio;
            out.glyphs.push(BoxGlyph {
                ch,
                style: st,
                size,
                ratio,
                dx: (width - adv(ch, st, ratio)) / 2,
                dy,
                color,
            });
        }
    }
}

fn dutmal(d: &Dutmal, cs: u32, base: i32, styles: &mut Styles<'_>) -> (InlineBox, AnnotLayout) {
    let mut out = AnnotLayout::default();
    // The main text: ordinary advances, the last one without its spacing.
    let main: Vec<char> = char::decode_utf16(d.main.0.iter().copied())
        .filter_map(Result::ok)
        .collect();
    let mut x = 0;
    let (mut asc, mut desc, mut em) = (0i64, 0i64, 1i64);
    for (i, &ch) in main.iter().enumerate() {
        let st = styles.style_for(cs, script(ch), Some(ch));
        let s = styles.get(st).clone();
        if let Some(f) = styles.face(st) {
            // The tallest ascent share among the faces used.
            let (a, dd) = (f.ascent.max(0) as i64, f.descent.max(0) as i64);
            if a + dd > 0 && a * (asc + desc).max(1) > asc * (a + dd) {
                (asc, desc) = (a, dd);
            }
            em = f.units_per_em.max(1) as i64;
        }
        let natural = natural_advance(&s, styles.face(st).map(|f| &**f), ch);
        out.glyphs.push(BoxGlyph {
            ch,
            style: st,
            size: s.size,
            ratio: s.ratio,
            dx: x,
            dy: 0,
            color: None,
        });
        x += if i + 1 == main.len() {
            natural
        } else {
            tracked(natural, s.spacing)
        };
    }
    let width = x;
    let (asc, desc) = if asc + desc > 0 {
        (asc, desc)
    } else {
        (850, 150)
    };
    let sub_size = mul_div(base, d.ratio(), 100).max(1);
    let sub: Vec<(char, u32)> = char::decode_utf16(d.sub.0.iter().copied())
        .filter_map(Result::ok)
        .map(|ch| (ch, styles.style_for(cs, script(ch), Some(ch))))
        .collect();
    let at = |ratio: i32, styles: &Styles<'_>| -> Vec<i32> {
        sub.iter()
            .map(|&(ch, st)| advance_at(styles, st, ch, sub_size, ratio))
            .collect()
    };
    let mut ratio = 100;
    let mut advs = at(100, styles);
    let natural: i32 = advs.iter().sum();
    if natural > width && natural > 0 {
        ratio = ((width as i64 * 100 / natural as i64) as i32).clamp(50, 100);
        advs = at(ratio, styles);
    }
    let used: i32 = advs.iter().sum();
    let start = if used > width {
        0
    } else {
        match d.align {
            0 | 1 => 0,
            2 => width - used,
            _ => (width - used) / 2,
        }
    };
    let sub_desc = (sub_size as i64 * desc / em) as i32;
    let over = mul_div(base, asc as i32, (asc + desc) as i32) + sub_desc;
    let dy = match d.position {
        1 => -over + base + sub_size,
        _ => -over,
    };
    let mut sx = start;
    for (&(ch, st), &a) in sub.iter().zip(&advs) {
        out.glyphs.push(BoxGlyph {
            ch,
            style: st,
            size: sub_size,
            ratio,
            dx: sx,
            dy,
            color: None,
        });
        sx += a;
    }
    // Hancom counts the annotation in whole quarter units. Below the main
    // text it adds a little less (fit to Mac Hancom's line advances).
    let extra = sub_size / 4 * 4;
    let (height, baseline) = match d.position {
        1 => (base + extra - mul_div(base, 9, 100), mul_div(base, 85, 100)),
        _ => (base + extra, mul_div(base, 85, 100) + extra),
    };
    (
        InlineBox {
            pos: 0,
            control: 0,
            width,
            height,
            baseline,
        },
        out,
    )
}

/// Text inside tracked deletions takes no room and is not drawn, as in
/// Mac Hancom's print of the changed document.
pub fn hide_deleted(para: &Paragraph, m: &mut Measured) {
    if para.tracks.is_empty() {
        return;
    }
    let ranges = hm_model::deleted_ranges(&para.tracks, para.len());
    if ranges.is_empty() {
        return;
    }
    for (pos, kind) in &mut m.items {
        if matches!(kind, Kind::ParaEnd) {
            continue;
        }
        if ranges.iter().any(|&(a, b)| *pos >= a && *pos < b) {
            *kind = Kind::Zero;
            m.advances[*pos as usize] = 0;
            m.natural[*pos as usize] = 0;
        }
    }
}

/// The annotation layout of control `index`, when it has one.
pub fn of(m: &Measured, index: u32) -> Option<&AnnotLayout> {
    m.annots
        .iter()
        .find(|(k, _)| *k == index)
        .map(|(_, l)| &**l)
}

/// True for controls laid out here.
pub fn handles(c: &Control) -> bool {
    c.as_compose().is_some() || c.as_dutmal().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hm_fonts::FontSystem;
    use hm_model::{DocInfo, PARA_END, TrackKind, TrackMark};

    fn mark(pos: u32, kind: TrackKind) -> TrackMark {
        TrackMark {
            pos,
            kind,
            id: 1,
            change: 1,
            ..Default::default()
        }
    }

    #[test]
    fn deleted_text_takes_no_room() {
        let (info, fonts) = (DocInfo::default(), FontSystem::new());
        let mut styles = Styles::new(&info, &fonts);
        let mut p = Paragraph {
            text: "가나다라".encode_utf16().chain([PARA_END]).collect(),
            ..Default::default()
        };
        let full = crate::measure::measure(&p, &mut styles, (false, false), &[]);
        p.tracks = vec![
            mark(1, TrackKind::DeleteBegin),
            mark(3, TrackKind::DeleteEnd),
            mark(3, TrackKind::InsertBegin),
            mark(4, TrackKind::InsertEnd),
        ];
        let m = crate::measure::measure(&p, &mut styles, (false, false), &[]);
        let sum = |m: &Measured| m.advances.iter().sum::<i32>();
        assert_eq!(sum(&m), sum(&full) - full.advances[1] - full.advances[2]);
        assert_eq!(m.items[1].1, Kind::Zero);
        assert_eq!(m.items[3].1, Kind::Char('라'));
    }

    #[test]
    fn open_deletions_run_to_the_paragraph_end() {
        let marks = [mark(2, TrackKind::DeleteBegin)];
        assert_eq!(hm_model::deleted_ranges(&marks, 6), [(2, 5)]);
        let marks = [mark(3, TrackKind::DeleteEnd)];
        assert_eq!(hm_model::deleted_ranges(&marks, 6), [(0, 3)]);
    }
}
