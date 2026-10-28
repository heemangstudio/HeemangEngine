//! Character advances by Hancom's Mac arithmetic.

use hm_fonts::Face;
use hm_model::units::mul_div;
use hm_model::{CharShape, Paragraph, Script, Unit};

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
    /// Index of the object in the paragraph's controls.
    pub control: u32,
    pub width: i32,
    pub height: i32,
    /// Distance from the box top to the baseline it sits on.
    pub baseline: i32,
}

/// One painted glyph for an unchanged span of original UTF16 text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlyphCluster {
    pub start: u32,
    pub end: u32,
    pub glyph: u16,
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
    /// Advance before character spacing per wire unit, for the fit test.
    pub natural: Vec<i32>,
    pub boxes: Vec<InlineBox>,
    /// Inner layouts of the paragraph's objects (equations, text boxes),
    /// by control index.
    pub objects: Vec<(u32, std::sync::Arc<crate::objects::ObjectLayout>)>,
    /// Glyphs of overlaid-characters and ruby boxes, by control index.
    pub annots: Vec<(u32, std::sync::Arc<crate::annot::AnnotLayout>)>,
    /// Wire positions of fixed-width and no-break blanks, which
    /// justification leaves at their width.
    pub fixed_blanks: Vec<u32>,
    /// Visible character borders, in wire order: fill and side widths.
    pub borders: Vec<(u32, u16, [i32; 4])>,
    /// Imported default gaps, excluded from a trailing alignment ink span.
    pub fixed_tabs: Vec<u32>,
    /// Character grid pitch of the paragraph, 0 for none.
    pub grid: i32,
    /// Manuscript grids allow a short Latin suffix to hang past the margin.
    pub manuscript: bool,
    /// On a character grid, per wire unit: the glyph's offset into its
    /// advance and its own advance; `advances` then hold whole cells.
    /// Empty otherwise.
    pub grid_glyphs: Vec<(i32, i32)>,
    /// Font-selected glyphs shared by measurement and painting, sorted by start.
    pub clusters: Vec<GlyphCluster>,
}

/// Hancom's narrow class on manuscript paper.
pub(crate) fn grid_narrow(c: char) -> bool {
    matches!(c,
        'a'..='z' | '0'..='9' | '\'' | ',' | '.'
        | '\u{c0}'..='\u{24f}' | '\u{1e00}'..='\u{1eff}'
        | '\u{fb00}'..='\u{fb06}' | '\u{ff60}'..='\u{ffef}'
    )
}

/// Puts a measured paragraph on a character grid: each character and
/// blank takes the whole cells its advance needs, one at least, centred in
/// them. On manuscript paper two half-cell narrow characters share a cell;
/// narrow punctuation can also share the preceding character's cell.
pub(crate) fn to_char_grid(m: &mut Measured, grid: crate::style::CharGrid) {
    let g = grid.pitch;
    m.grid = g;
    m.manuscript = grid.manuscript;
    m.grid_glyphs = m.advances.iter().map(|&a| (0, a)).collect();
    let narrow = |kind: Kind| grid.manuscript && matches!(kind, Kind::Char(c) if grid_narrow(c));
    let mut k = 0;
    while k < m.items.len() {
        let (pos, kind) = m.items[k];
        k += 1;
        if !matches!(kind, Kind::Char(_) | Kind::Space) {
            continue;
        }
        let p = pos as usize;
        let a = m.advances[p];
        if let Some(&(next, kind2)) = m.items.get(k)
            && narrow(kind)
            && narrow(kind2)
            && a + m.advances[next as usize] <= g
            && (2 * a <= g && 2 * m.advances[next as usize] <= g
                || matches!(kind2, Kind::Char('\'' | ',' | '.')))
        {
            let (q, b) = (next as usize, m.advances[next as usize]);
            let lead = (g - a - b) / 2;
            m.grid_glyphs[p] = (lead, a);
            m.grid_glyphs[q] = (0, b);
            m.advances[p] = lead + a;
            m.advances[q] = g - lead - a;
            m.natural[p] = m.advances[p];
            m.natural[q] = m.advances[q];
            k += 1;
            continue;
        }
        let cells = ((a + g - 1) / g).max(1);
        // Standalone manuscript periods and commas sit toward the left
        // of their cell; other glyphs are centred.
        let divisor = if grid.manuscript && matches!(kind, Kind::Char('.' | ',')) {
            6
        } else {
            2
        };
        m.grid_glyphs[p] = ((cells * g - a) / divisor, a);
        m.advances[p] = cells * g;
        m.natural[p] = cells * g;
    }
}

/// The advance of glyph `ch` in HU.
///
/// With logical size `h`, Hancom measures in quarter units `q = |h / 4|`:
/// at ratio 100 the glyph takes `MulDiv(advance, q, upem)` units, otherwise
/// `ratio * q * advance / (upem * 100)` truncated. Character spacing then
/// adds `MulDiv(units, spacing, 100)` and the result is `units * 4` HU.
pub fn advance(style: &Style, face: Option<&Face>, ch: char) -> i32 {
    tracked(natural_advance(style, face, ch), style.spacing)
}

/// A cached advance plus character spacing: `4 * MulDiv(advance / 4, spacing, 100)`.
pub fn tracked(cache: i32, spacing: i32) -> i32 {
    if spacing == 0 {
        cache
    } else {
        cache.wrapping_add(mul_div(cache / 4, spacing, 100).wrapping_mul(4))
    }
}

/// Automatic spacing classes: Latin letters on one side, CJK on the other.
fn auto_space_latin(c: u16) -> bool {
    matches!(c, 0x41..=0x5A | 0x61..=0x7A | 0xB5 | 0xC0..=0xD6 | 0xD8..=0xF6 | 0xF8..=0x10FF | 0x1E00..=0x1FFF)
}

fn auto_space_cjk(c: u16) -> bool {
    matches!(
        c,
        0x3040..=0x318F | 0x3300..=0x33FF | 0x4E00..=0x9FAF | 0xAC00..=0xD7A3 | 0xE0BC..=0xFA6A | 0xFF10..=0xFF19 | 0xFF21..=0xFF3A | 0xFF41..=0xFF5A
    )
}

/// Hancom's automatic spacing between CJK and Latin letters (paragraph
/// flags 2 bit 4) or ASCII digits (bit 5): a unit followed by one of the
/// other class gains a quarter of its char shape's base size, before
/// character spacing.
fn auto_space(para: &Paragraph, m: &mut Measured, styles: &Styles<'_>, latin: bool, digits: bool) {
    for &(pos, kind) in &m.items {
        let Kind::Char(_) = kind else { continue };
        let p = pos as usize;
        let (Some(&cur), Some(&next)) = (para.text.get(p), para.text.get(p + 1)) else {
            continue;
        };
        let pair =
            |a: fn(u16) -> bool, b: fn(u16) -> bool| (a(cur) && b(next)) || (b(cur) && a(next));
        let digit = |c: u16| matches!(c, 0x30..=0x39);
        if (latin && pair(auto_space_latin, auto_space_cjk))
            || (digits && pair(digit, auto_space_cjk))
        {
            let style = styles.get(m.styles[p]);
            let base = styles
                .char_shape_ref(style.char_shape)
                .map_or(1000, |c| c.base_size);
            m.natural[p] = m.natural[p].wrapping_add(((base as u32) >> 2) as i32);
            m.advances[p] = tracked(m.natural[p], style.spacing);
        }
    }
}

/// The advance of glyph `ch` before character spacing, in HU. The line
/// breaker tests whether a character fits with this width.
pub fn natural_advance(style: &Style, face: Option<&Face>, ch: char) -> i32 {
    natural_units(style, face, ch).wrapping_mul(4)
}

fn natural_units(style: &Style, face: Option<&Face>, ch: char) -> i32 {
    // The zero-width space takes no room, whatever glyph a face gives it.
    if ch == '\u{200B}' {
        return 0;
    }
    let q = (style.size / 4).abs();
    let (adv, upem) = match face.and_then(|f| {
        f.glyph(ch).map(|g| {
            // HFT faces widen synthetic bold before scaling.
            let bold = if style.fake_bold {
                f.synthetic_bold_extra(&g)
            } else {
                0
            };
            (g.advance.wrapping_add(bold), g.em as i32)
        })
    }) {
        Some(v) => v,
        // No face has the glyph: a full em for wide scripts, half otherwise.
        None => (if wide(ch) { 1000 } else { 500 }, 1000),
    };
    if style.ratio == 100 {
        mul_div(adv, q, upem)
    } else {
        (style.ratio as i64 * q as i64 * adv as i64 / (upem as i64 * 100)) as i32
    }
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
    blank_width(cs, blank, true)
}

/// A blank's width, with or without character spacing.
fn blank_width(cs: &CharShape, blank: Blank, tracked: bool) -> i32 {
    let base = line_size(cs);
    let rel = cs.rel_size(Script::Hangul);
    let h = if rel == 100 {
        base
    } else {
        mul_div(base, rel, 100)
    };
    let divided = h / if blank == Blank::FixedWidth { 16 } else { 8 };
    let ratio = cs.ratio(Script::Hangul);
    let units = if ratio == 100 {
        divided
    } else {
        mul_div(divided, ratio, 100)
    };
    let spacing = if blank == Blank::NoBreak || !tracked {
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

fn contextual_neighbor(c: char) -> bool {
    c.is_ascii_graphic() || u16::try_from(c as u32).is_ok_and(auto_space_latin)
}

fn uses_font_space(styles: &mut Styles<'_>, cs: u32, contextual: bool) -> bool {
    if !contextual
        && !styles
            .char_shape_ref(cs)
            .is_some_and(|c| c.attr & (1 << 25) != 0)
    {
        return false;
    }
    let style = styles.style_for(cs, Script::Latin, Some(' '));
    // A zero HFT space leaves the ordinary blank at its declared width.
    // Synthetic bold adds a positive width even to a zero space glyph.
    !styles.face(style).is_some_and(|f| {
        f.is_hft() && !styles.get(style).fake_bold && f.glyph(' ').is_some_and(|g| g.advance == 0)
    })
}

/// Retains character-border insets for line fitting and placement. Objects
/// cause a separate composition whose glyph advances omit these insets.
pub(crate) fn border_room(para: &Paragraph, m: &mut Measured, styles: &Styles<'_>) {
    if para.controls.iter().any(|c| c.object_common().is_some()) {
        return;
    }
    for &(pos, kind) in &m.items {
        if !matches!(kind, Kind::Char(_) | Kind::Space) {
            continue;
        }
        let Some(cs) = styles.char_shape_ref(styles.get(m.styles[pos as usize]).char_shape) else {
            continue;
        };
        let Some(id) = cs.border_fill.filter(|&id| id > 0) else {
            continue;
        };
        let Some(bf) = styles.info().border_fills.get(id as usize - 1) else {
            continue;
        };
        if bf.sides.iter().any(|s| s.visible()) {
            m.borders.push((
                pos,
                id,
                bf.sides.map(|s| if s.visible() { s.width_hu() } else { 0 }),
            ));
        }
    }
}

impl Measured {
    pub fn border(&self, pos: u32) -> Option<(u16, [i32; 4])> {
        self.borders
            .binary_search_by_key(&pos, |b| b.0)
            .ok()
            .map(|k| {
                let (_, id, sides) = self.borders[k];
                (id, sides)
            })
    }

    /// Insets at this item's fragment boundaries on a physical line.
    pub fn border_room(&self, k: usize, from: usize, end: usize) -> (i32, i32) {
        let Some((id, sides)) = self.border(self.items[k].0) else {
            return (0, 0);
        };
        let same = |j: usize| self.border(self.items[j].0).is_some_and(|b| b.0 == id);
        let left = if k == from || !same(k - 1) {
            sides[0]
        } else {
            0
        };
        let right = if k + 1 == end || !same(k + 1) {
            sides[1]
        } else {
            0
        };
        (left, right)
    }
}

/// Measures generated text into the eight slots of a control at `p`.
fn generated(
    m: &mut Measured,
    p: usize,
    cs: u32,
    text: &str,
    look: crate::numbers::Look,
    styles: &mut Styles<'_>,
) {
    use crate::numbers::Look;
    let mut style = styles.style_for(cs, Script::Latin, None);
    let mut slot = p;
    for c in text.chars().take(8) {
        let script = script_of(c).unwrap_or(Script::Latin);
        style = match look {
            Look::Plain => styles.style_for(cs, script, Some(c)),
            Look::Superscript => styles.superscript_style(cs, script, Some(c)),
            Look::NoteMark => styles.note_mark_style(cs, script, Some(c)),
        };
        let face = styles.face(style).map(|f| &**f);
        if let Some(n) = m.natural.get_mut(slot) {
            *n = natural_advance(styles.get(style), face, c);
            m.advances[slot] = advance(styles.get(style), face, c);
            m.styles[slot] = style;
            m.items.push((slot as u32, Kind::Char(c)));
        }
        slot += 1;
    }
    for k in slot..(p + 8).min(m.styles.len()) {
        m.styles[k] = style;
    }
}

/// Scales the generated text in the slots at `p` so it takes the room of
/// `stored`, the text Hancom stored for the number.
fn fit_to_stored(m: &mut Measured, p: usize, cs: u32, stored: &str, styles: &mut Styles<'_>) {
    let mut room = 0;
    for c in stored.chars().take(8) {
        let script = script_of(c).unwrap_or(Script::Latin);
        let style = styles.style_for(cs, script, Some(c));
        let face = styles.face(style).map(|f| &**f);
        room += advance(styles.get(style), face, c);
    }
    let end = (p + 8).min(m.advances.len());
    let shown: i32 = m.advances[p..end].iter().sum();
    if shown <= 0 || shown == room {
        return;
    }
    for k in p..end {
        m.advances[k] = mul_div(m.advances[k], room, shown);
        m.natural[k] = mul_div(m.natural[k], room, shown);
    }
}

/// Measures every unit of a paragraph. `auto_space` carries paragraph
/// flags 2 bits 4 and 5 (automatic CJK-Latin and CJK-digit spacing), already
/// gated by the compatibility profile.
///
/// `sizes` gives laid-out `(control index, width, height)` of inline
/// objects whose size differs from what their header stores (tables).
pub fn measure(
    para: &Paragraph,
    styles: &mut Styles<'_>,
    auto: (bool, bool),
    sizes: &[(u32, i32, i32)],
) -> Measured {
    let n = para.text.len();
    let contextual = styles
        .info()
        .compatible
        .as_ref()
        .is_some_and(|c| c.target == 2);
    let mut linked: Vec<(u32, crate::objects::ObjectLayout)> = Vec::new();
    let mut m = Measured {
        natural: vec![0; n],
        advances: vec![0; n],
        styles: vec![0; n],
        items: Vec::with_capacity(n),
        boxes: Vec::new(),
        objects: Vec::new(),
        annots: Vec::new(),
        fixed_blanks: Vec::new(),
        borders: Vec::new(),
        fixed_tabs: Vec::new(),
        grid: 0,
        manuscript: false,
        grid_glyphs: Vec::new(),
        clusters: Vec::new(),
    };
    let mut has_jamo = false;
    let mut prev_script = Script::Hangul;
    let mut run = 0usize;
    let mut units = para.units().peekable();
    let mut previous = None;
    while let Some((pos, unit)) = units.next() {
        let before = previous;
        previous = if let Unit::Char(c) = unit {
            Some(c)
        } else {
            None
        };
        while run + 1 < para.runs.len() && para.runs[run + 1].pos <= pos {
            run += 1;
        }
        let cs = para.runs.get(run).map_or(0, |r| r.shape);
        let p = pos as usize;
        // Automatic numbers and note marks set generated text in the
        // control's eight wire slots.
        if let Unit::Extended { index, .. } = unit
            && let Some(c) = para.controls.get(index as usize)
            && let Some((text, look)) =
                crate::numbers::generated_with_total(c, styles.page_number, styles.total_pages)
        {
            generated(&mut m, p, cs, &text, look, styles);
            if std::ptr::addr_eq(para, styles.stored_number_room as *const Paragraph)
                && let Some((stored, _)) = crate::numbers::stored_page_number(c)
            {
                fit_to_stored(&mut m, p, cs, &stored, styles);
            }
            continue;
        }
        if let Unit::Extended { index, .. } = unit
            && crate::annot::measure(&mut m, para, pos, index, cs, styles)
        {
            continue;
        }
        let (kind, script, ch, blank) = match unit {
            // "Use the font's space" (char shape bit 25) measures U+0020 as a
            // glyph of the Latin slot's face, whatever script surrounds it.
            Unit::Char(' ')
                if uses_font_space(
                    styles,
                    cs,
                    contextual
                        && before.is_some_and(contextual_neighbor)
                        && units.peek().is_some_and(
                            |(_, u)| matches!(u, Unit::Char(c) if contextual_neighbor(*c)),
                        ),
                ) =>
            {
                (Kind::Space, Script::Latin, Some(' '), None)
            }
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
                let inline = para.controls.get(index as usize).and_then(|c| {
                    let (inline, inner) = crate::objects::measure_object(c, styles)?;
                    if let Some(inner) = inner {
                        linked.push((index, inner));
                    }
                    let mut b = inline?;
                    // Tables report their laid-out size.
                    if let Some(&(_, w, h)) = sizes.iter().find(|s| s.0 == index) {
                        let o = c.object_common()?;
                        b.width = w + o.margins[0] as i32 + o.margins[1] as i32;
                        b.height = h + o.margins[2] as i32 + o.margins[3] as i32;
                        b.baseline = mul_div(b.height, 85, 100);
                    }
                    Some(b)
                });
                match inline {
                    Some(b) => {
                        m.boxes.push(InlineBox {
                            pos,
                            control: index,
                            ..b
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
            (_, Some(b), _) => {
                let c = styles.char_shape(cs);
                m.natural[p] = blank_width(&c, b, false);
                blank_advance(&c, b)
            }
            (Kind::Char(_) | Kind::Space, None, Some(c)) => {
                let face = styles.face(style).map(|f| &**f);
                m.natural[p] = natural_advance(styles.get(style), face, c);
                advance(styles.get(style), face, c)
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
        if matches!(blank, Some(Blank::NoBreak | Blank::FixedWidth)) {
            m.fixed_blanks.push(pos);
        }
        has_jamo |= matches!(
            kind,
            Kind::Char('\u{1100}'..='\u{115f}' | '\u{a960}'..='\u{a97c}')
        );
        m.items.push((pos, kind));
    }
    // Linked text boxes flow their text on from box to box.
    crate::objects::link_text_boxes(para, &mut linked, styles);
    m.objects
        .extend(linked.into_iter().map(|(k, o)| (k, std::sync::Arc::new(o))));
    crate::annot::hide_deleted(para, &mut m);
    if has_jamo {
        compose_hangul(para, &mut m, styles);
    }
    if auto.0 || auto.1 {
        auto_space(para, &mut m, styles, auto.0, auto.1);
    }
    m
}

fn jamo(c: char) -> u8 {
    match c {
        '\u{1100}'..='\u{115f}' | '\u{a960}'..='\u{a97c}' => 1,
        '\u{1160}'..='\u{11a7}' | '\u{d7b0}'..='\u{d7c6}' => 2,
        '\u{11a8}'..='\u{11ff}' | '\u{d7cb}'..='\u{d7fb}' => 3,
        _ => 0,
    }
}

fn compose_hangul(para: &Paragraph, m: &mut Measured, styles: &Styles<'_>) {
    let mut read = 0;
    let mut write = 0;
    while read < m.items.len() {
        let (pos, kind) = m.items[read];
        let mut end = read + 1;
        if matches!(kind, Kind::Char(c) if jamo(c) == 1) {
            let mut last = 1;
            while let Some(&(p, Kind::Char(c))) = m.items.get(end) {
                let next = jamo(c);
                if p != m.items[end - 1].0 + 1
                    || !matches!((last, next), (1, 1 | 2) | (2, 2 | 3) | (3, 3))
                {
                    break;
                }
                last = next;
                end += 1;
            }
        }
        let count = end - read;
        if matches!(count, 2 | 3) {
            let si = m.styles[pos as usize];
            let mut chars = ['\0'; 3];
            let compatible = m.items[read..end].iter().enumerate().all(|(i, &(p, k))| {
                let Kind::Char(c) = k else { return false };
                chars[i] = c;
                m.styles[p as usize] == si && para.text.get(p as usize) == Some(&(c as u16))
            });
            let style = styles.get(si);
            if compatible
                && let Some(face) = style.face
                && let Some((ch, glyph)) = styles.fonts().hangul_cluster(face, &chars[..count])
            {
                let face = styles.face(si).map(|f| &**f);
                m.natural[pos as usize] = natural_advance(style, face, ch);
                m.advances[pos as usize] = advance(style, face, ch);
                for &(p, _) in &m.items[read + 1..end] {
                    m.natural[p as usize] = 0;
                    m.advances[p as usize] = 0;
                }
                m.clusters.push(GlyphCluster {
                    start: pos,
                    end: pos + count as u32,
                    glyph: glyph.glyph,
                });
                m.items[write] = m.items[read];
                write += 1;
                read = end;
                continue;
            }
        }
        for i in read..end {
            m.items[write] = m.items[i];
            write += 1;
        }
        read = end;
    }
    m.items.truncate(write);
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
            shade: None,
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
    #[ignore = "requires HEEMANG_FONTS pointing to Hancom's HFT directory"]
    fn ordinary_font_spaces_keep_positive_metrics_and_replace_zero_hft_metrics() {
        use hm_model::{CharRun, DocInfo, FaceName, HwpString};
        use std::{path::Path, sync::Arc};

        let directory = std::env::var("HEEMANG_FONTS").expect("HFT fixture directory");
        let mut fonts = hm_fonts::FontSystem::new();
        for name in ["ENSMJ.HFT", "ENGARM.HFT"] {
            let path = Path::new(&directory).join(name);
            fonts.add(name, Arc::new(std::fs::read(path).unwrap()));
        }
        for (family, positive) in [("한양신명조", false), ("AmeriGarmnd BT", true)] {
            for flag in [0, 1 << 25] {
                for (varied, bold) in [(false, false), (true, false), (false, true), (true, true)] {
                    let mut cs = shape(1150, 95, -7);
                    cs.attr = flag | if bold { 2 } else { 0 };
                    if varied {
                        cs.ratios[0] = 83;
                        cs.ratios[1] = 67;
                        cs.spacings[0] = -13;
                        cs.spacings[1] = 8;
                        cs.rel_sizes[0] = 80;
                        cs.rel_sizes[1] = 120;
                    }
                    let info = DocInfo {
                        faces: std::array::from_fn(|_| {
                            vec![FaceName {
                                name: HwpString::from_text(family),
                                ..Default::default()
                            }]
                        }),
                        char_shapes: vec![cs.clone()],
                        ..Default::default()
                    };
                    for text in ["가 \u{1e}\u{1f}\u{200b}\r", "A \u{1e}\u{1f}\u{200b}\r"] {
                        let para = Paragraph {
                            text: text.encode_utf16().collect(),
                            runs: vec![CharRun { pos: 0, shape: 0 }],
                            ..Default::default()
                        };
                        let mut styles = Styles::new(&info, &fonts);
                        let si = styles.style_for(0, Script::Latin, Some(' '));
                        let face = styles.face(si).unwrap();
                        assert!(face.is_hft());
                        assert_eq!(
                            face.glyph(' ').unwrap().advance,
                            if positive { 248 } else { 0 }
                        );
                        let font_width = advance(styles.get(si), Some(face), ' ');
                        let m = measure(&para, &mut styles, (false, false), &[]);
                        assert_eq!(
                            m.advances[1],
                            if (positive || bold) && flag != 0 {
                                font_width
                            } else {
                                blank_advance(&cs, Blank::Ordinary)
                            }
                        );
                        assert_eq!(m.advances[2], blank_advance(&cs, Blank::NoBreak));
                        assert_eq!(m.advances[3], blank_advance(&cs, Blank::FixedWidth));
                        assert_eq!(m.advances[4], 0);
                    }
                }
            }
        }
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
