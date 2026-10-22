//! Character advances by Hancom's Mac arithmetic.

use hm_fonts::Face;
use hm_model::units::mul_div;
use hm_model::{CharShape, ControlKind, Paragraph, Script, Unit};

use crate::script::script_of;
use crate::style::{Style, Styles, line_size};

/// What a wire unit is to the line breaker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A character with an advance.
    Char(char),
    Space,
    Tab,
    LineBreak,
    ParaEnd,
    /// An inline object (treated as a character): index into `Measured::boxes`.
    Object(u32),
    /// Anything without an advance (control slot tails, field ends, ...).
    Zero,
}

/// An object laid out as a character.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InlineBox {
    pub pos: u32,
    pub width: i32,
    pub height: i32,
    /// Distance from the box top to the baseline it sits on.
    pub baseline: i32,
}

/// EQEDIT record tag.
const EQEDIT: u16 = 0x58;

/// The baseline of an inline object: an equation stores it as a percentage
/// of its height; other objects sit at 85 percent.
fn object_baseline(raw: &hm_model::RawControl, height: i32) -> i32 {
    let percent = raw
        .children
        .iter()
        .find(|c| c.tag == EQEDIT && c.level == 1)
        .and_then(|c| {
            let b = &c.payload[..];
            let n = u16::from_le_bytes(b.get(4..6)?.try_into().ok()?) as usize;
            let at = 6 + 2 * n + 8;
            Some(i16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?) as i32)
        })
        .unwrap_or(85);
    mul_div(height, percent, 100)
}

/// Per-unit measurement of one paragraph.
#[derive(Clone, Debug, Default)]
pub struct Measured {
    /// One entry per wire unit; only the first unit of an element is set.
    pub advances: Vec<i32>,
    /// Style index per wire unit (controls take the run's style).
    pub styles: Vec<u32>,
    /// `(position, kind)` per element, in order.
    pub items: Vec<(u32, Kind)>,
    pub boxes: Vec<InlineBox>,
}

/// The advance of glyph `ch` in HU.
///
/// With logical size `h`, Hancom measures in quarter units `q = |h / 4|`:
/// at ratio 100 the glyph takes `MulDiv(advance, q, upem)` units, otherwise
/// `ratio * q * advance / (upem * 100)` truncated. Character spacing then
/// adds `MulDiv(units, spacing, 100)` and the result is `units * 4` HU.
pub fn advance(style: &Style, face: Option<&Face>, ch: char) -> i32 {
    let q = (style.size / 4).abs();
    let (adv, upem) = match face.and_then(|f| {
        f.glyph(ch)
            .map(|g| (g.advance as i32, f.units_per_em as i32))
    }) {
        Some(v) => v,
        // No face has the glyph: a full em for wide scripts, half otherwise.
        None => (if wide(ch) { 1000 } else { 500 }, 1000),
    };
    let mut units = if style.ratio == 100 {
        mul_div(adv, q, upem)
    } else {
        (style.ratio as i64 * q as i64 * adv as i64 / (upem as i64 * 100)) as i32
    };
    if style.spacing != 0 {
        units += mul_div(units, style.spacing, 100);
    }
    units.wrapping_mul(4)
}

/// The blanks Hancom sizes without consulting a font.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blank {
    /// U+0020.
    Ordinary,
    /// Control 30, the no-break space (묶음 빈칸).
    NoBreak,
    /// Control 31, the fixed-width space (고정폭 빈칸).
    FixedWidth,
}

/// The advance of a blank in HU.
///
/// The width comes from the Hangul slot: its logical size divided by 8 (16
/// for the fixed-width space), scaled by its ratio, times 4. The ordinary and
/// fixed-width spaces then take the Latin slot's character spacing.
pub fn blank_advance(cs: &CharShape, blank: Blank) -> i32 {
    let h = line_size(cs, Script::Hangul);
    let divided = h / if blank == Blank::FixedWidth { 16 } else { 8 };
    let ratio = cs.ratio(Script::Hangul);
    let units = if ratio == 100 {
        divided
    } else {
        mul_div(divided, ratio, 100)
    };
    let spacing = if blank == Blank::NoBreak {
        0
    } else {
        cs.spacing(Script::Latin)
    };
    let units = units
        + if spacing != 0 {
            mul_div(units, spacing, 100)
        } else {
            0
        };
    units.wrapping_mul(4)
}

fn wide(ch: char) -> bool {
    matches!(
        script_of(ch),
        Some(Script::Hangul | Script::Hanja | Script::Japanese | Script::Symbol)
    )
}

/// Measures every unit of a paragraph.
pub fn measure(para: &Paragraph, styles: &mut Styles<'_>) -> Measured {
    let n = para.text.len();
    let mut m = Measured {
        advances: vec![0; n],
        styles: vec![0; n],
        items: Vec::with_capacity(n),
        boxes: Vec::new(),
    };
    let mut prev_script = Script::Hangul;
    let mut run = 0usize;
    for (pos, unit) in para.units() {
        while run + 1 < para.runs.len() && para.runs[run + 1].pos <= pos {
            run += 1;
        }
        let cs = para.runs.get(run).map_or(0, |r| r.shape);
        let p = pos as usize;
        let (kind, script, ch, blank) = match unit {
            Unit::Char(' ') => (Kind::Space, prev_script, None, Some(Blank::Ordinary)),
            Unit::Char(c) => match script_of(c) {
                Some(s) => (Kind::Char(c), s, Some(c), None),
                // Other literal blanks have glyphs: U+00A0 in the Latin slot,
                // U+2007 and U+3000 in the Symbol slot.
                None => {
                    let s = match c {
                        '\u{a0}' => Script::Latin,
                        '\u{2007}' | '\u{3000}' => Script::Symbol,
                        _ => prev_script,
                    };
                    (Kind::Space, s, Some(c), None)
                }
            },
            Unit::CharControl(30) => (Kind::Space, prev_script, None, Some(Blank::NoBreak)),
            Unit::CharControl(31) => (Kind::Space, prev_script, None, Some(Blank::FixedWidth)),
            Unit::CharControl(10) => (Kind::LineBreak, prev_script, None, None),
            Unit::CharControl(13) => (Kind::ParaEnd, prev_script, None, None),
            Unit::Inline(9) => (Kind::Tab, prev_script, None, None),
            Unit::Extended { index, .. } => {
                let obj = para
                    .controls
                    .get(index as usize)
                    .and_then(|c| match &c.kind {
                        ControlKind::Raw(r) => r
                            .object_common()
                            .filter(|o| o.treat_as_char())
                            .map(|o| (r, o)),
                        _ => None,
                    });
                match obj {
                    Some((raw, o)) => {
                        let width = o.width + o.margins[0] as i32 + o.margins[1] as i32;
                        let height = o.height + o.margins[2] as i32 + o.margins[3] as i32;
                        let baseline = object_baseline(raw, height);
                        m.boxes.push(InlineBox {
                            pos,
                            width,
                            height,
                            baseline,
                        });
                        (
                            Kind::Object(m.boxes.len() as u32 - 1),
                            prev_script,
                            None,
                            None,
                        )
                    }
                    None => (Kind::Zero, prev_script, None, None),
                }
            }
            _ => (Kind::Zero, prev_script, None, None),
        };
        if matches!(kind, Kind::Char(_)) {
            prev_script = script;
        }
        let style = styles.style_for(cs, script, ch);
        m.styles[p] = style;
        m.advances[p] = match (kind, blank, ch) {
            (_, Some(b), _) => styles.char_shape_ref(cs).map_or(0, |c| blank_advance(c, b)),
            (Kind::Char(_) | Kind::Space, None, Some(c)) => {
                advance(styles.get(style), styles.face(style).map(|f| &**f), c)
            }
            (Kind::Object(i), _, _) => m.boxes[i as usize].width,
            _ => 0,
        };
        let w = match unit {
            Unit::Char(c) => c.len_utf16(),
            Unit::CharControl(_) => 1,
            _ => 8,
        };
        for k in p + 1..(p + w).min(n) {
            m.styles[k] = style;
        }
        m.items.push((pos, kind));
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use hm_model::Script;

    fn style(size: i32, ratio: i32, spacing: i32) -> Style {
        Style {
            char_shape: 0,
            script: Script::Hangul,
            face: None,
            exact: true,
            size,
            line_size: size,
            ratio,
            spacing,
            offset: 0,
            color: 0,
            fake_bold: false,
            italic: false,
            underline: 0,
            underline_color: 0,
            strike: false,
            units_per_em: 1000,
        }
    }

    fn shape(size: i32, ratio: u8, spacing: i8) -> CharShape {
        CharShape {
            base_size: size,
            ratios: [ratio; 7],
            spacings: [spacing; 7],
            rel_sizes: [100; 7],
            ..Default::default()
        }
    }

    #[test]
    fn blanks_follow_the_hangul_slot() {
        // 10 pt: 1000 / 8 = 125 units = 500 HU; the fixed-width space is half.
        assert_eq!(blank_advance(&shape(1000, 100, 0), Blank::Ordinary), 500);
        assert_eq!(blank_advance(&shape(1000, 100, 0), Blank::FixedWidth), 248);
        // 15 pt with -6% spacing: 187 units less 11 = 176 units.
        assert_eq!(blank_advance(&shape(1500, 100, -6), Blank::Ordinary), 704);
        // The no-break space ignores spacing.
        assert_eq!(blank_advance(&shape(1500, 100, -6), Blank::NoBreak), 748);
        // Ratio 90: MulDiv(125, 90, 100) = 113 units.
        assert_eq!(blank_advance(&shape(1000, 90, 0), Blank::Ordinary), 452);
    }

    #[test]
    fn spacing_rounds_half_away_from_zero() {
        // A full-em glyph at 10 pt is 250 units; -5% removes 13 (12.5 rounded away).
        assert_eq!(advance(&style(1000, 100, -5), None, '가'), (250 - 13) * 4);
    }

    #[test]
    fn non_100_ratio_truncates() {
        // 0.5 em at ratio 95: 95 * 250 * 500 / 100000 = 118.75 -> 118 units.
        assert_eq!(advance(&style(1000, 95, 0), None, 'a'), 118 * 4);
    }
}
