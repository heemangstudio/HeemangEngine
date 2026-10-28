//! Boxes: laying out parsed equation nodes in HU.
//!
//! Every box has a width, an ascent and a descent around its baseline and
//! the items it paints. Rows join boxes with Hancom's operator spacing:
//! relations take 0.21 em on each side, binary operators 0.14 em, a comma
//! 0.07 em after it. An operator directly after a composite box (a
//! fraction, radical, decoration or delimited group) takes no space before
//! it.

use hm_model::units::mul_div;

use super::font::{MathFonts, cell, pitch, pua};
use super::parse::{Atom, AtomKind, Node, Space, Style};
use super::table::{Big, Class, Deco, Stack};
use super::{MathItem, MathLayout};

/// The spacing behavior of a box edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Edge {
    Ord(Class),
    /// A composite box: fraction, radical, decoration, delimited group.
    Composite,
    /// A base with scripts.
    Scripted,
    /// An explicit space.
    Space,
    /// An upright function name.
    Func,
    /// A number.
    Num,
}

#[derive(Clone, Debug)]
struct Bx {
    w: i32,
    asc: i32,
    desc: i32,
    /// Clearance reserved by the outer box, separate from painted geometry.
    clear_above: i32,
    clear_below: i32,
    items: Vec<MathItem>,
    left: Edge,
    right: Edge,
    /// The box starts right after a font switch.
    fresh: bool,
    /// The box ends with a Latin capital letter.
    cap_end: bool,
    /// Width at the end that the next atom may overlap (a capital's
    /// superscript offset).
    overhang: i32,
}

impl Bx {
    fn empty(size: i32) -> Bx {
        let (asc, desc) = line_extent(size);
        Bx {
            w: 0,
            asc,
            desc,
            clear_above: 0,
            clear_below: 0,
            items: Vec::new(),
            left: Edge::Ord(Class::Ord),
            right: Edge::Ord(Class::Ord),
            fresh: false,
            cap_end: false,
            overhang: 0,
        }
    }

    /// Appends `other` with its origin at `(dx, dy)` from this box's origin.
    fn place(&mut self, other: Bx, dx: i32, dy: i32) {
        for mut it in other.items {
            match &mut it {
                MathItem::Glyph { x, y, .. } | MathItem::Rule { x, y, .. } => {
                    *x += dx;
                    *y += dy;
                }
            }
            self.items.push(it);
        }
    }
}

/// Ascent and descent of a text line at `size`: the 96-dpi pixel height
/// with 85 percent above the baseline.
fn line_extent(size: i32) -> (i32, i32) {
    let h = (size * 2 + 75) / 150 * 75;
    let asc = mul_div(h, 85, 100);
    (asc, h - asc)
}

struct Ctx<'a> {
    fonts: &'a MathFonts,
    color: u32,
    /// The equation carries no script version (converted from HWP 3):
    /// Hancom sizes its radicals by their nominal em and an eighth.
    legacy: bool,
}

/// Script size: 68 percent of the parent's.
fn script_size(size: i32) -> i32 {
    mul_div(size, 68, 100).max(1)
}

/// `em` fractions in HU, truncated.
fn em(size: i32, per_mille: i32) -> i32 {
    size * per_mille / 1000
}

pub fn typeset(node: &Node, size: i32, color: u32, fonts: &MathFonts, legacy: bool) -> MathLayout {
    let cx = Ctx {
        fonts,
        color,
        legacy,
    };
    // An eqalign that is the whole equation spaces its rows as lines.
    let b = match node {
        Node::Stack {
            kind: Stack::EqAlign,
            rows,
        } => cx.stack(Stack::EqAlign, rows, size, true),
        _ => cx.node(node, size),
    };
    MathLayout {
        width: b.w,
        ascent: b.asc,
        descent: b.desc,
        box_ascent: b.asc + b.clear_above,
        box_descent: b.desc + b.clear_below,
        items: b.items,
    }
}

impl Ctx<'_> {
    fn node(&self, n: &Node, size: i32) -> Bx {
        match n {
            Node::Row(v) => {
                let mut elements = v.iter().filter(|n| !matches!(n, Node::FontSwitch));
                match (elements.next(), elements.next()) {
                    // Preserve the single-element row identity after adding
                    // switches to the syntax tree. Braced groups stay boxes.
                    (Some(n), None) => self.node(n, size),
                    _ => self.row(v, size),
                }
            }
            Node::Group(v) => {
                let mut b = self.row(v, size);
                if v.iter().filter(|n| !matches!(n, Node::FontSwitch)).count() > 1 {
                    b.left = Edge::Composite;
                    b.right = Edge::Composite;
                }
                b
            }
            Node::Atom(a) => self.atom(a, size),
            Node::FontSwitch => Bx::empty(size),
            Node::Space(s) => {
                let mut b = Bx::empty(size);
                b.w = match s {
                    Space::Full => size / 2,
                    Space::Quarter => size / 8,
                };
                b.left = Edge::Space;
                b.right = Edge::Space;
                b
            }
            Node::Frac { num, den, bar } => self.frac(num, den, *bar, size),
            Node::Root { index, body } => self.root(index.as_deref(), body, size),
            Node::Scripts { base, sub, sup } => {
                self.scripts(base, sub.as_deref(), sup.as_deref(), size)
            }
            Node::LScripts { base, sub, sup } => {
                let s = script_size(size);
                let b = self.node(base, size);
                let sub = sub.as_ref().map(|n| self.node(n, s));
                let sup = sup.as_ref().map(|n| self.node(n, s));
                let w = sub.iter().chain(sup.iter()).map(|b| b.w).max().unwrap_or(0);
                let mut out = Bx::empty(size);
                out.asc = b.asc;
                out.desc = b.desc;
                if let Some(p) = sup {
                    out.asc = out.asc.max(em(size, 447) + p.asc);
                    let x = w - p.w;
                    out.place(p, x, -em(size, 447));
                }
                if let Some(q) = sub {
                    out.desc = out.desc.max(em(size, 273) + q.desc);
                    let x = w - q.w;
                    out.place(q, x, em(size, 273));
                }
                out.w = w + b.w;
                out.left = b.left;
                out.right = b.right;
                out.fresh = b.fresh;
                out.cap_end = b.cap_end;
                out.overhang = b.overhang;
                out.place(b, w, 0);
                out
            }
            Node::Big {
                op,
                style,
                sub,
                sup,
            } => self.big(*op, *style, sub.as_deref(), sup.as_deref(), size),
            Node::Lim { name, style, sub } => self.lim(name, *style, sub.as_deref(), size),
            Node::Delim { left, right, body } => self.delim(*left, *right, body, size),
            Node::Deco { kind, body } => self.deco(*kind, body, size),
            Node::Stack { kind, rows } => self.stack(*kind, rows, size, false),
            Node::Aligned { body, .. } => self.node(body, size),
            Node::LongDiv {
                divisor,
                quotient,
                rows,
            } => self.long_div(divisor, quotient, rows, size),
            Node::Color { rgb, body } => {
                let inner = Ctx {
                    fonts: self.fonts,
                    color: *rgb,
                    legacy: self.legacy,
                };
                inner.node(body, size)
            }
            Node::Rel { arrow, over, under } => self.rel(arrow, over, under.as_deref(), size),
            Node::Bigg(a) => {
                let big = mul_div(size, 150, 100);
                let mut b = self.atom(a, big);
                b.left = Edge::Ord(a.class);
                b.right = Edge::Ord(a.class);
                b
            }
            Node::Lines(lines) => {
                let rows: Vec<Vec<Node>> = lines.iter().map(|l| vec![l.clone()]).collect();
                self.stack(Stack::EqAlign, &rows, size, true)
            }
            Node::Tab => Bx::empty(size),
        }
    }

    /// A glyph run.
    fn atom(&self, a: &Atom, size: i32) -> Bx {
        let mut b = Bx::empty(size);
        let mut x = 0;
        for c in a.text.chars() {
            let (italic, shift) = match a.kind {
                AtomKind::Var => (!a.style.roman, 0),
                AtomKind::Number => (false, 0),
                AtomKind::Func => (false, 0),
                AtomKind::Symbol => (!a.style.roman, 0),
                AtomKind::Text => (false, 0),
            };
            let w = self.glyph(&mut b, c, italic, a.style.bold, size, x, shift);
            x += w;
        }
        b.w = x;
        b.fresh = a.style.after_switch;
        b.cap_end = matches!(a.kind, AtomKind::Var | AtomKind::Text)
            && a.text
                .chars()
                .last()
                .is_some_and(|c| c.is_ascii_uppercase());
        if a.kind == AtomKind::Func {
            b.left = Edge::Func;
            b.right = Edge::Func;
        } else if a.kind == AtomKind::Number {
            b.left = Edge::Num;
            b.right = Edge::Num;
        } else {
            b.left = Edge::Ord(a.class);
            b.right = Edge::Ord(a.class);
        }
        b
    }

    /// Paints one character at `x`; returns its pitch.
    #[allow(clippy::too_many_arguments)]
    fn glyph(
        &self,
        b: &mut Bx,
        c: char,
        italic: bool,
        bold: bool,
        size: i32,
        x: i32,
        shift: i32,
    ) -> i32 {
        self.glyph_with(b, c, (italic, bold), size, x, shift, false)
    }

    /// As [`Ctx::glyph`]; `natural` takes the font's own advance instead of
    /// the packed pixel pitch (limit names).
    #[allow(clippy::too_many_arguments)]
    fn glyph_with(
        &self,
        b: &mut Bx,
        c: char,
        (italic, bold): (bool, bool),
        size: i32,
        x: i32,
        shift: i32,
        natural: bool,
    ) -> i32 {
        let advance = |ch: char, adv: i32, em: i32| {
            if natural {
                mul_div(adv, size, em.max(1))
            } else if ('\u{e000}'..='\u{e019}').contains(&ch) {
                // Italic capitals keep their whole pixel advance.
                cell(adv, em, size)
            } else {
                pitch(adv, em, size)
            }
        };
        let latin_upright = c.is_ascii_alphabetic() && !italic;
        if let Some((id, face)) = &self.fonts.eq {
            let mapped = if latin_upright { None } else { pua(c, italic) };
            let candidates = [mapped, Some(c)];
            for ch in candidates.into_iter().flatten() {
                if let Some(g) = face.glyph(ch) {
                    let w = advance(ch, g.advance, g.em as i32);
                    b.items.push(MathItem::Glyph {
                        face: *id,
                        glyph: g.glyph,
                        ch,
                        size,
                        x,
                        y: shift + if natural { 0 } else { glyph_rise(ch, size) },
                        x_scale: 100,
                        bold,
                        // Hancom slants its upright capitals for italics.
                        italic: ('\u{e000}'..='\u{e019}').contains(&ch),
                        color: self.color,
                    });
                    return w;
                }
            }
        }
        for (id, face) in &self.fonts.text {
            if let Some(g) = face.glyph(c) {
                let w = advance(c, g.advance, g.em as i32);
                b.items.push(MathItem::Glyph {
                    face: *id,
                    glyph: g.glyph,
                    ch: c,
                    size,
                    x,
                    y: shift,
                    x_scale: 100,
                    bold,
                    italic: italic && c.is_alphabetic(),
                    color: self.color,
                });
                return w;
            }
        }
        em(size, 500)
    }

    fn row(&self, v: &[Node], size: i32) -> Bx {
        let mut out = Bx::empty(size);
        let mut prev: Option<Edge> = None;
        let mut x = 0;
        let mut first = None;
        let mut last = None;
        let mut last_overhang = 0;
        let mut limit_operand = false;
        for (i, n) in v.iter().enumerate() {
            let mut b = self.node(n, size);
            // Scripted limit operands keep the application's composite edge.
            // Switches are transparent; an empty operand still consumes the slot.
            if !matches!(n, Node::FontSwitch) {
                if limit_operand && b.right == Edge::Scripted {
                    b.right = Edge::Composite;
                }
                limit_operand = matches!(n, Node::Lim { .. });
            }
            if b.w == 0 && b.items.is_empty() && !matches!(n, Node::Space(_)) {
                continue;
            }
            let unary = matches!(b.left, Edge::Ord(Class::Bin))
                && matches!(
                    prev,
                    None | Some(Edge::Ord(
                        Class::Rel | Class::Bin | Class::Open | Class::Punct
                    ))
                );
            let _ = i;
            if !b.fresh {
                x -= last_overhang;
            }
            x += gap(prev, b.left, size, unary, b.fresh);
            if first.is_none() {
                first = Some(b.left);
                out.fresh = b.fresh;
            }
            last = Some(b.right);
            prev = Some(if unary {
                Edge::Ord(Class::Punct)
            } else {
                b.right
            });
            if unary {
                // A sign keeps a binary space after it.
                prev = Some(Edge::Ord(Class::Bin));
            }
            let box_asc = (out.asc + out.clear_above).max(b.asc + b.clear_above);
            let box_desc = (out.desc + out.clear_below).max(b.desc + b.clear_below);
            out.asc = out.asc.max(b.asc);
            out.desc = out.desc.max(b.desc);
            out.clear_above = box_asc - out.asc;
            out.clear_below = box_desc - out.desc;
            let w = b.w;
            last_overhang = b.overhang;
            out.cap_end = b.cap_end;
            out.place(b, x, 0);
            x += w;
        }
        out.w = x;
        out.overhang = last_overhang;
        out.left = first.unwrap_or(Edge::Ord(Class::Ord));
        out.right = last.unwrap_or(Edge::Ord(Class::Ord));
        out
    }

    fn scripts(&self, base: &Node, sub: Option<&Node>, sup: Option<&Node>, size: i32) -> Bx {
        let s = script_size(size);
        let b = self.node(base, size);
        let sub = sub.map(|n| self.node(n, s));
        let sup = sup.map(|n| self.node(n, s));
        let mut out = Bx::empty(size);
        out.asc = b.asc;
        out.desc = b.desc;
        let x = b.w;
        let left = b.left;
        let mut w = 0;
        // A superscript after a Latin capital starts 0.15 em further right;
        // the next atom may take that space back.
        let cap = if b.cap_end && sup.is_some() {
            em(size, 150)
        } else {
            0
        };
        // A superscript's bottom sits 0.525 em below the top of its base, a
        // subscript's baseline 0.119 em below the base's bottom.
        if let Some(p) = sup {
            let rise = b.asc - em(size, 525) + p.desc;
            out.asc = out.asc.max(rise + p.asc);
            w = w.max(cap + p.w);
            out.place(p, x + cap, -rise);
        }
        if let Some(q) = sub {
            let drop = b.desc + em(size, 119);
            out.desc = out.desc.max(drop + q.desc);
            w = w.max(q.w);
            out.place(q, x, drop);
        }
        out.place(b, 0, 0);
        out.w = x + w;
        out.overhang = cap;
        out.left = left;
        out.right = Edge::Scripted;
        out
    }

    fn frac(&self, num: &Node, den: &Node, bar: bool, size: i32) -> Bx {
        let n = self.node(num, size);
        let d = self.node(den, size);
        // Half an em beside the wider part, at least one em in all; the bar
        // stops a tenth of an em short of either side.
        let w = (n.w.max(d.w) + size / 2).max(size);
        let inset = size / 10;
        // The axis is the middle of the text line box; numerator and
        // denominator keep 30 percent of an em between them around it.
        let (la, ld) = line_extent(size);
        let axis = la - (la + ld) / 2;
        let gap = mul_div(size, 30, 100) / 2;
        let thick = em(size, 40).max(8);
        let mut out = Bx::empty(size);
        let n_base = -axis - gap - n.desc;
        let d_base = -axis + gap + d.asc;
        out.asc = axis + gap + n.desc + n.asc;
        out.desc = (d_base + d.desc).max(0);
        out.clear_above = n.clear_above + n.clear_below;
        out.clear_below = d.clear_above + d.clear_below;
        if bar {
            out.items.push(MathItem::Rule {
                x: inset,
                y: -axis - thick / 2,
                w: w - 2 * inset,
                h: thick,
                color: self.color,
            });
        }
        let (nw, dw) = (n.w, d.w);
        out.place(n, (w - nw) / 2, n_base);
        out.place(d, (w - dw) / 2, d_base);
        out.w = w;
        out.left = Edge::Composite;
        out.right = Edge::Composite;
        out
    }

    fn root(&self, index: Option<&Node>, body: &Node, size: i32) -> Bx {
        let b = self.node(body, size);
        let mut out = Bx::empty(size);
        let hu = |v: i32| mul_div(v, size, 1100);
        // Width uses the sign's measuring cell plus a 17 percent nominal
        // margin. Its painted attachment stays 0.96 em before the body.
        let lead = em(size, 960);
        let extra = self
            .fonts
            .eq
            .as_ref()
            .filter(|_| !self.legacy)
            .and_then(|(_, face)| face.glyph('\u{e05c}'))
            .map_or(size + size / 8, |g| {
                cell(g.advance, g.em as i32, size) + mul_div(size, 17, 100)
            });
        let tail = extra - lead;
        let w = lead + b.w + tail;
        // The radical glyph is stretched to the body's height plus a pixel,
        // its top 87 HU (at 11 pt) above the body's top, where the vinculum
        // starts.
        let top = b.asc + hu(87);
        let stretch = b.asc + b.desc + hu(75);
        if let Some((id, face)) = &self.fonts.eq
            && let Some(g) = face.glyph('\u{e05c}')
        {
            out.items.push(MathItem::Glyph {
                face: *id,
                glyph: g.glyph,
                ch: '\u{e05c}',
                size: stretch,
                x: 0,
                y: -top + mul_div(821, stretch, 1024) - hu(9),
                x_scale: mul_div(size, 100, stretch.max(1)),
                bold: false,
                italic: false,
                color: self.color,
            });
        }
        let thick = mul_div(41, size, 1024).max(8);
        out.items.push(MathItem::Rule {
            x: lead - hu(24),
            y: -top,
            w: w - hu(17) - (lead - hu(24)),
            h: thick,
            color: self.color,
        });
        out.asc = out.asc.max(top);
        out.desc = out.desc.max(b.desc);
        if let Some(i) = index {
            let ib = self.node(i, size / 2);
            let base = b.asc - em(size, 395);
            out.asc = out.asc.max(base + ib.asc);
            out.place(ib, hu(252), -base);
        }
        let bw = b.w;
        out.place(b, lead, 0);
        out.w = lead + bw + tail;
        out.left = Edge::Composite;
        out.right = Edge::Composite;
        out
    }

    fn big(&self, op: Big, style: Style, sub: Option<&Node>, sup: Option<&Node>, size: i32) -> Bx {
        let integral = op.integral();
        // Sums are set at 180 percent, integrals at 200 percent, both with
        // the font's own advance.
        let gsize = mul_div(size, if integral { 200 } else { 180 }, 100);
        let mut g = Bx::empty(gsize);
        let mut gw = 0;
        for _ in 0..op.count() {
            gw += self.glyph_with(&mut g, op.char(), (false, style.bold), gsize, gw, 0, true);
        }
        g.w = gw;
        let s = script_size(size);
        let sub = sub.map(|n| self.node(n, s));
        let sup = sup.map(|n| self.node(n, s));
        // Glyph baseline below the text baseline, the limits' bottom and top
        // edges (all at 11 pt in HU, scaled).
        let (base, sup_bottom, sub_top) = if integral {
            (em(size, 273), em(size, 879), em(size, 305))
        } else {
            (em(size, 300), em(size, 1005), em(size, 321))
        };
        let mut out = Bx::empty(size);
        out.asc = out.asc.max(gsize * 8 / 10 - base);
        out.desc = out.desc.max(base + gsize / 5);
        let (sub_x, sup_x, w) = if integral {
            let (bx, px) = (em(size, 731), em(size, 1287));
            let w = (gw + em(size, 100))
                .max(sub.as_ref().map_or(0, |q| bx + q.w))
                .max(sup.as_ref().map_or(0, |p| px + p.w));
            (bx, px, w)
        } else {
            let w = gw
                .max(sub.as_ref().map_or(0, |b| b.w))
                .max(sup.as_ref().map_or(0, |b| b.w));
            (
                (w - sub.as_ref().map_or(0, |q| q.w)) / 2,
                (w - sup.as_ref().map_or(0, |p| p.w)) / 2,
                w,
            )
        };
        if let Some(p) = sup {
            let y = -sup_bottom - p.desc;
            out.asc = out.asc.max(-y + p.asc);
            out.place(p, sup_x, y);
        }
        if let Some(q) = sub {
            let y = sub_top + q.asc;
            out.desc = out.desc.max(y + q.desc);
            out.place(q, sub_x, y);
        }
        let gx = if integral { 0 } else { (w - gw) / 2 };
        out.place(g, gx, base);
        out.w = w;
        out.left = Edge::Ord(Class::Ord);
        out.right = Edge::Composite;
        out
    }

    fn lim(&self, name: &str, style: Style, sub: Option<&Node>, size: i32) -> Bx {
        // The name is set at 120 percent with the font's own advances.
        let nsize = mul_div(size, 120, 100);
        let mut n = Bx::empty(nsize);
        let mut x = 0;
        for c in name.chars() {
            x += self.glyph_with(&mut n, c, (false, style.bold), nsize, x, 0, true);
        }
        n.w = x;
        let mut out = Bx::empty(size);
        // The name's line box is centered on the math axis; the limit hangs
        // just below it.
        let (la, ld) = line_extent(size);
        let axis = la - (la + ld) / 2;
        let ny = n.asc - (n.asc + n.desc) / 2 - axis;
        let sub = sub.map(|q| self.node(q, script_size(size)));
        let w = n.w.max(sub.as_ref().map_or(0, |b| b.w));
        if let Some(q) = sub {
            let y = ny + n.desc + mul_div(23, size, 1100) + q.asc;
            out.desc = out.desc.max(y + q.desc);
            let qw = q.w;
            out.place(q, (w - qw) / 2, y);
        }
        out.asc = out.asc.max(n.asc - ny);
        out.desc = out.desc.max(n.desc + ny);
        let nw = n.w;
        out.place(n, (w - nw) / 2, ny);
        out.w = w;
        out.left = Edge::Ord(Class::Ord);
        out.right = Edge::Composite;
        out
    }

    /// The width a delimiter takes beside its content.
    fn delim_slot(&self, c: char, size: i32) -> i32 {
        if let Some(st) = stretch(c) {
            return cell(st.slot, 1024, size);
        }
        // Angle brackets take half an em, whatever their glyph.
        if matches!(c, '〈' | '〉') {
            return em(size, 500);
        }
        if let Some((_, face)) = &self.fonts.eq
            && let Some(g) = face.glyph(pua(c, false).unwrap_or(c))
        {
            return cell(g.advance, g.em as i32, size);
        }
        // Delimiters the equation font lacks come from the text face.
        self.fonts
            .text
            .iter()
            .find_map(|(_, f)| f.glyph(c))
            .map_or(em(size, 500), |g| pitch(g.advance, g.em as i32, size))
    }

    /// A delimiter stretched over content `asc` above and `desc` below the
    /// baseline. Hancom scales the whole glyph vertically so its em box
    /// spans the content, and builds taller braces, parentheses and
    /// brackets from pieces joined by scaled extenders.
    #[allow(clippy::too_many_arguments)]
    fn delim_glyph(
        &self,
        out: &mut Bx,
        c: char,
        x: i32,
        slot: i32,
        asc: i32,
        desc: i32,
        size: i32,
    ) {
        let h = (asc + desc).max(1);
        let ch = pua(c, false).unwrap_or(c);
        let eq = self.fonts.eq.as_ref().filter(|(_, f)| f.has(ch));
        let Some((id, face)) = eq.or_else(|| self.fonts.text.iter().find(|(_, f)| f.has(c))) else {
            return;
        };
        let upem = face.units_per_em.max(1) as i32;
        let (fa, fd) = (face.ascent, face.descent.abs());
        let mut put = |ch: char, y: i32, vsize: i32, center: bool| {
            if let Some(g) = face.glyph(ch) {
                // A whole glyph narrower than its slot sits in the middle.
                let w = if center {
                    mul_div(g.advance, size, g.em.max(1) as i32)
                } else {
                    slot
                };
                out.items.push(MathItem::Glyph {
                    face: *id,
                    glyph: g.glyph,
                    ch,
                    size: vsize,
                    x: x + ((slot - w) / 2).max(0),
                    y,
                    x_scale: mul_div(100, size, vsize.max(1)).max(1),
                    bold: false,
                    italic: false,
                    color: self.color,
                });
            }
        };
        let st = stretch(c);
        let whole = st
            .as_ref()
            .map_or(if eq.is_some() { ch } else { c }, |s| s.whole);
        let (la, ld) = line_extent(size);
        let Some(st) = st.filter(|s| s.pieces_above.is_some_and(|n| h > n * (la + ld))) else {
            put(whole, desc - mul_div(h, fd, fa + fd), h, true);
            return;
        };
        let ink = |ch: char| {
            face.glyph(ch)
                .map(|g| ink_y(&face.outline(g.glyph)))
                .unwrap_or((-fd, fa))
        };
        let top_y = -asc + mul_div(size, fa, upem) - em(size, 30);
        let bottom_y = desc - mul_div(size, fd, upem);
        put(st.top, top_y, size, false);
        put(st.bottom, bottom_y, size, false);
        let mut joints = Vec::new();
        if let Some(mid) = st.mid {
            let center = (desc - asc) / 2;
            let mid_y = center + mul_div(size, fa - upem / 2, upem) - em(size, 20);
            put(mid, mid_y, size, false);
            joints.push((st.top, top_y, mid, mid_y));
            joints.push((mid, mid_y, st.bottom, bottom_y));
        } else {
            joints.push((st.top, top_y, st.bottom, bottom_y));
        }
        let (e0, e1) = ink(st.ext);
        let overlap = em(size, 100);
        for (a, ay, b, by) in joints {
            // From the upper piece's ink bottom to the lower piece's ink top.
            let g0 = ay - mul_div(ink(a).0, size, upem);
            let g1 = by - mul_div(ink(b).1, size, upem);
            if g1 <= g0 || e1 <= e0 {
                continue;
            }
            let vsize = mul_div(g1 - g0 + 2 * overlap, upem, e1 - e0);
            put(
                st.ext,
                g1 + overlap + mul_div(e0, vsize, upem),
                vsize,
                false,
            );
        }
    }

    fn delim(&self, left: Option<char>, right: Option<char>, body: &Node, size: i32) -> Bx {
        let b = self.node(body, size);
        let slot = |c: Option<char>| c.map_or(0, |c| self.delim_slot(c, size));
        let (lw, rw) = (slot(left), slot(right));
        let mut out = Bx::empty(size);
        out.asc = b.asc;
        out.desc = b.desc;
        let (asc, desc) = (b.asc, b.desc);
        if let Some(c) = left {
            self.delim_glyph(&mut out, c, 0, lw, asc, desc, size);
        }
        let bw = b.w;
        out.place(b, lw, 0);
        if let Some(c) = right {
            self.delim_glyph(&mut out, c, lw + bw, rw, asc, desc, size);
        }
        out.w = lw + bw + rw;
        out.left = Edge::Composite;
        out.right = Edge::Composite;
        out
    }

    fn deco(&self, kind: Deco, body: &Node, size: i32) -> Bx {
        let b = self.node(body, size);
        let hu = |v: i32| mul_div(v, size, 1100);
        let mut out = Bx::empty(size);
        // Decorations widen the box by about a twentieth of an em each side.
        let left = em(size, 96) / 2;
        let right = em(size, 96) - left;
        let thick = mul_div(41, size, 1024).max(8);
        let top = b.asc;
        match kind {
            Deco::Bar => {
                let y = -top - hu(73) - thick;
                out.items.push(MathItem::Rule {
                    x: left,
                    y,
                    w: b.w,
                    h: thick,
                    color: self.color,
                });
                out.asc = -y;
            }
            Deco::Under => {
                out.items.push(MathItem::Rule {
                    x: left,
                    y: b.desc + hu(40),
                    w: b.w,
                    h: thick,
                    color: self.color,
                });
                out.desc = b.desc + hu(40) + thick;
            }
            Deco::Vec | Deco::Dyad => {
                let y = -top - hu(225) - thick;
                out.items.push(MathItem::Rule {
                    x: left,
                    y,
                    w: b.w,
                    h: thick,
                    color: self.color,
                });
                // The arrowhead's baseline: its glyph rises 0.64 to 0.73 em.
                let base = -top + hu(464);
                let head = |c: char, x: i32, out: &mut Bx| {
                    let mut g = Bx::empty(size);
                    self.glyph(&mut g, c, false, false, size, 0, 0);
                    out.place(g, x, base);
                };
                head('\u{e06e}', left + b.w - hu(288), &mut out);
                if kind == Deco::Dyad {
                    head('\u{e06c}', left, &mut out);
                }
                out.asc = -y;
            }
            _ => {
                if kind == Deco::Box {
                    let pad = em(size, 60);
                    let mut path_rect = |x: i32, y: i32, w: i32, h: i32| {
                        for (rx, ry, rw, rh) in [
                            (x, y, w, thick),
                            (x, y + h - thick, w, thick),
                            (x, y, thick, h),
                            (x + w - thick, y, thick, h),
                        ] {
                            out.items.push(MathItem::Rule {
                                x: rx,
                                y: ry,
                                w: rw,
                                h: rh,
                                color: self.color,
                            });
                        }
                    };
                    path_rect(
                        left - pad,
                        -top - pad,
                        b.w + 2 * pad,
                        top + b.desc + 2 * pad,
                    );
                    out.asc = out.asc.max(top + pad);
                    out.desc = out.desc.max(b.desc + pad);
                    out.asc = out.asc.max(b.asc);
                    out.w = left + b.w + right;
                    out.place(b, left, 0);
                    out.left = Edge::Composite;
                    out.right = Edge::Composite;
                    return out;
                }
                let c = match kind {
                    Deco::Hat => '\u{e063}',
                    Deco::Check => '\u{e064}',
                    Deco::Tilde => '\u{e066}',
                    Deco::Acute => '\u{e05d}',
                    Deco::Grave => '\u{e05e}',
                    Deco::Dot => '\u{e05f}',
                    Deco::DDot => '\u{e060}',
                    Deco::Arch => '\u{e065}',
                    _ => '/',
                };
                let mut g = Bx::empty(size);
                let gw = self.glyph(&mut g, c, false, false, size, 0, 0);
                if kind == Deco::Not {
                    out.place(g, left + (b.w - gw) / 2, 0);
                } else {
                    out.place(g, left + (b.w - gw) / 2, -top + hu(464));
                    out.asc = out.asc.max(top + hu(300));
                }
            }
        }
        out.asc = out.asc.max(b.asc);
        out.desc = out.desc.max(b.desc);
        if kind == Deco::Bar {
            out.clear_above = (line_extent(size).0 + em(size, 200) - out.asc).max(0);
        }
        out.w = left + b.w + right;
        out.place(b, left, 0);
        out.left = Edge::Composite;
        out.right = Edge::Composite;
        out
    }

    fn rel(&self, arrow: &Node, over: &Node, under: Option<&Node>, size: i32) -> Bx {
        // Hancom sets the annotations at the full size and stretches the
        // arrow a little past the wider one.
        let mut a = self.node(arrow, size);
        let o = self.node(over, size);
        let u = under.map(|n| self.node(n, size));
        let w =
            a.w.max(o.w + em(size, 60))
                .max(u.as_ref().map_or(0, |b| b.w + em(size, 60)));
        if w > a.w && a.w > 0 {
            stretch_x(&mut a, w);
        }
        // The annotations sit just clear of the arrow, which lies on the
        // axis.
        let (la, ld) = line_extent(size);
        let axis = la - (la + ld) / 2;
        let gap = em(size, 100);
        let mut out = Bx::empty(size);
        let (ow, oa, od) = (o.w, o.asc, o.desc);
        out.asc = a.asc.max(axis + gap + od + oa);
        out.place(o, (w - ow) / 2, -axis - gap - od);
        out.desc = a.desc;
        if let Some(u) = u {
            let (uw, ua, ud) = (u.w, u.asc, u.desc);
            let uy = -axis + gap + ua;
            out.desc = out.desc.max(uy + ud);
            out.place(u, (w - uw) / 2, uy);
        }
        let aw = a.w;
        out.desc = out.desc.max(a.desc);
        out.place(a, (w - aw) / 2, 0);
        out.w = w;
        out.left = Edge::Ord(Class::Rel);
        out.right = Edge::Ord(Class::Rel);
        out
    }

    /// Rows of cells. Cells are centered on their row's height; an empty
    /// cell is half an em wide and an empty row an em tall. Piles, cases
    /// and matrices keep 0.15 em between rows and 0.2 em between columns
    /// and center the rows on the axis. Eqalign right-aligns its first
    /// column when others follow, sets columns an em apart, leaves empty
    /// cells empty and keeps its first row on the baseline; its rows
    /// touch, or sit 0.1 em apart when they are the equation's own lines
    /// (`lines`).
    fn stack(&self, kind: Stack, rows: &[Vec<Node>], size: i32, lines: bool) -> Bx {
        let empty = |c: &Node| matches!(c, Node::Row(v) if v.is_empty());
        let mut cells: Vec<Vec<Bx>> = rows
            .iter()
            .map(|r| {
                r.iter()
                    .map(|c| {
                        let mut b = self.node(c, size);
                        if empty(c) && kind != Stack::EqAlign {
                            b.w = em(size, 500);
                        }
                        b
                    })
                    .collect()
            })
            .collect();
        // A ladder ignores empty cells ending a row, and its last row holds
        // the final quotients under the dividends: it starts one column in.
        let ladder = matches!(kind, Stack::Ladder | Stack::SLadder);
        if ladder {
            for (r, row) in rows.iter().zip(cells.iter_mut()) {
                let keep = r.iter().rposition(|c| !empty(c)).map_or(0, |k| k + 1);
                row.truncate(keep);
            }
            if cells.len() > 1
                && let Some(last) = cells.last_mut()
            {
                let mut pad = Bx::empty(size);
                pad.w = 0;
                last.insert(0, pad);
            }
        }
        // Hancom sets a ladder in a quarter em, leaves a little under a
        // quarter em before the bar and half an em after it, and runs the
        // rules a quarter em past the last column.
        let thick = em(size, 40).max(8);
        let lead = if ladder { em(size, 250) } else { 0 };
        let (before_bar, after_bar) = (em(size, 225), em(size, 270));
        let gap_after = |k: usize, col_gap: i32| {
            if ladder && k == 0 {
                before_bar + thick + after_bar
            } else {
                col_gap
            }
        };
        let ncols = cells.iter().map(Vec::len).max().unwrap_or(0);
        let mut col_w = vec![0; ncols];
        for r in &cells {
            for (k, c) in r.iter().enumerate() {
                col_w[k] = col_w[k].max(c.w);
            }
        }
        // Columns typed one by one (`col`) touch.
        let by_column = rows
            .iter()
            .any(|r| r.iter().any(|c| matches!(c, Node::Aligned { .. })));
        let col_gap = match kind {
            Stack::EqAlign => size,
            _ if by_column => 0,
            _ => em(size, 200),
        };
        let row_gap = match kind {
            Stack::EqAlign if lines => em(size, 100),
            Stack::EqAlign => 0,
            _ => em(size, 150),
        };
        let heights: Vec<i32> = rows
            .iter()
            .zip(&cells)
            .map(|(r, c)| {
                if r.iter().all(empty) {
                    size
                } else {
                    c.iter().map(|b| b.asc + b.desc).max().unwrap_or(size)
                }
            })
            .collect();
        let mut top = 0;
        let mut first_base = None;
        let mut placed = Vec::new();
        for ((r, h), src) in cells.into_iter().zip(&heights).zip(rows) {
            let mut x = lead;
            for (k, c) in r.into_iter().enumerate() {
                let (cw, ch) = (c.w, c.asc + c.desc);
                let aligned = match src.get(k) {
                    Some(Node::Aligned { align, .. }) => Some(*align),
                    _ => None,
                };
                let dx = match (aligned, kind) {
                    (Some(0), _) => 0,
                    (Some(2), _) => col_w[k] - cw,
                    (Some(_), _) => (col_w[k] - cw) / 2,
                    (None, kind) => match kind {
                        Stack::LPile | Stack::Cases => 0,
                        Stack::RPile => col_w[k] - cw,
                        Stack::EqAlign if k == 0 && ncols > 1 => col_w[k] - cw,
                        Stack::EqAlign => 0,
                        _ => (col_w[k] - cw) / 2,
                    },
                };
                let base = top + (h - ch) / 2 + c.asc;
                first_base.get_or_insert(base);
                placed.push((c, x + dx, base));
                x += col_w[k] + gap_after(k, col_gap);
            }
            top += h + row_gap;
        }
        let height = (top - row_gap).max(0);
        // The first row on the baseline for eqalign, otherwise the
        // stack's middle on the axis.
        let (la, ld) = line_extent(size);
        let axis = la - (la + ld) / 2;
        let shift = match kind {
            Stack::EqAlign => -first_base.unwrap_or(la),
            _ => -axis - height / 2,
        };
        let mut inner = Bx::empty(size);
        inner.asc = -shift;
        inner.desc = height + shift;
        for (c, x, y) in placed {
            inner.place(c, x, y + shift);
        }
        let content: i32 = col_w.iter().sum::<i32>()
            + (0..ncols.saturating_sub(1))
                .map(|k| gap_after(k, col_gap))
                .sum::<i32>();
        inner.w = lead + content;
        if ladder && heights.len() > 1 && ncols > 1 {
            // The bar after the divisors runs down the divided rows; a rule
            // under each of them spans the dividends.
            let bar_x = lead + col_w[0] + before_bar;
            // A binary ladder's rules stop at the quotients.
            let rule_end = if kind == Stack::SLadder && ncols > 2 {
                bar_x + thick + after_bar + col_w[1]
            } else {
                lead + content
            };
            // Each divided row has its own piece of the bar, from just above
            // the row (just below the rule over it) down to its rule.
            let mut y = 0;
            let mut piece_top = -em(size, 75);
            for h in &heights[..heights.len() - 1] {
                y += h;
                let rule_y = y + row_gap / 2 - thick / 2;
                inner.items.push(MathItem::Rule {
                    x: bar_x,
                    y: shift + piece_top,
                    w: thick,
                    h: rule_y + thick - piece_top,
                    color: self.color,
                });
                inner.items.push(MathItem::Rule {
                    x: bar_x,
                    y: shift + rule_y,
                    w: rule_end - bar_x,
                    h: thick,
                    color: self.color,
                });
                piece_top = rule_y + thick + em(size, 75);
                y += row_gap;
            }
            inner.w = rule_end + em(size, 500);
        }
        let (lc, rc) = match kind {
            Stack::PMatrix => (Some('('), Some(')')),
            Stack::BMatrix => (Some('['), Some(']')),
            Stack::DMatrix => (Some('|'), Some('|')),
            Stack::Cases => (Some('{'), None),
            _ => (None, None),
        };
        let mut out = Bx::empty(size);
        let slot = |c: Option<char>| c.map_or(0, |c| self.delim_slot(c, size));
        let (lw, rw) = (slot(lc), slot(rc));
        out.asc = inner.asc;
        out.desc = inner.desc;
        let (asc, desc, iw) = (inner.asc, inner.desc, inner.w);
        if let Some(c) = lc {
            self.delim_glyph(&mut out, c, 0, lw, asc, desc, size);
        }
        out.place(inner, lw, 0);
        if let Some(c) = rc {
            self.delim_glyph(&mut out, c, lw + iw, rw, asc, desc, size);
        }
        out.w = lw + iw + rw;
        out.left = Edge::Composite;
        out.right = Edge::Composite;
        out
    }
}

/// Stretches a box's glyphs and rules across to width `w`.
fn stretch_x(b: &mut Bx, w: i32) {
    let k = w as f64 / b.w as f64;
    for it in &mut b.items {
        match it {
            MathItem::Glyph { x, x_scale, .. } => {
                *x = (*x as f64 * k).round() as i32;
                *x_scale = (*x_scale as f64 * k).round() as i32;
            }
            MathItem::Rule { x, w, .. } => {
                *x = (*x as f64 * k).round() as i32;
                *w = (*w as f64 * k).round() as i32;
            }
        }
    }
    b.w = w;
}

/// The characters an element shows, spaces left out.
fn shown_chars(n: &Node) -> i32 {
    match n {
        Node::Atom(a) => a.text.chars().filter(|c| !c.is_whitespace()).count() as i32,
        Node::Row(v) | Node::Group(v) => v.iter().map(shown_chars).sum(),
        Node::Space(_) => 0,
        _ => 1,
    }
}

impl Ctx<'_> {
    /// Long division as Hancom sets it: the divisor and a bracket before
    /// the dividend, a bar over the dividend with the quotient on it
    /// flush right, and the working rows below, each flush right at a
    /// digit of the dividend: a product under the digits it was taken from
    /// and underlined, then the remainder one digit further with the next
    /// digit brought down.
    fn long_div(&self, divisor: &Node, quotient: &Node, rows: &[Node], size: i32) -> Bx {
        let thick = em(size, 40).max(8);
        let d = self.node(divisor, size);
        let q = self.node(quotient, size);
        let Some((first, rest)) = rows.split_first() else {
            return d;
        };
        let dividend = self.node(first, size);
        let digits = shown_chars(first).max(1);
        let pitch = dividend.w / digits;
        let bracket_w = em(size, 120);
        let x0 = d.w + bracket_w + em(size, 250);
        let right = x0 + dividend.w;
        let mut out = Bx::empty(size);
        // The quotient's baseline sits an em and a fifth above the
        // dividend's; the bar runs just under it.
        let bar_y = -em(size, 960);
        let q_base = -em(size, 1210);
        let (qw, qa) = (q.w, q.asc);
        out.place(q, right - qw, q_base);
        out.items.push(MathItem::Rule {
            x: d.w + em(size, 30),
            y: bar_y,
            w: right - d.w - em(size, 30),
            h: thick,
            color: self.color,
        });
        let low = em(size, 120);
        self.delim_glyph(&mut out, ')', d.w, bracket_w, -bar_y, low, size);
        let (da, dd) = (d.asc, d.desc);
        out.place(d, 0, 0);
        let mut desc = dd.max(dividend.desc).max(low);
        out.place(dividend, x0, 0);
        // Working rows: products line up with the digits used so far, the
        // remainders after them one digit further.
        let mut at = 0;
        let mut base = 0;
        for (k, r) in rest.iter().enumerate() {
            let b = self.node(r, size);
            let product = k % 2 == 0;
            at = if k == 0 {
                shown_chars(r).min(digits)
            } else if product {
                at
            } else {
                (at + 1).min(digits)
            };
            base += if k == 0 {
                em(size, 1110)
            } else {
                em(size, 1140)
            };
            let rx = x0 + pitch * at;
            let bw = b.w;
            desc = desc.max(base + b.desc);
            out.place(b, rx - bw, base);
            if product && k + 1 < rest.len() {
                // The rule spans the digits, not the spaces typed before them.
                let pad = em(size, 45);
                let shown = (pitch * shown_chars(r)).min(bw);
                out.items.push(MathItem::Rule {
                    x: rx - shown - pad,
                    y: base + em(size, 210),
                    w: shown + pad,
                    h: thick,
                    color: self.color,
                });
            }
        }
        out.w = right;
        out.asc = (qa - q_base).max(da).max(-bar_y);
        out.desc = desc;
        out.left = Edge::Composite;
        out.right = Edge::Composite;
        out
    }
}

/// The pieces of a stretchable delimiter in Hancom's equation font.
struct Stretch {
    /// The whole glyph, scaled to short content.
    whole: char,
    top: char,
    mid: Option<char>,
    bottom: char,
    /// Scaled to join the pieces.
    ext: char,
    /// Content taller than this many text lines takes the pieces instead
    /// of the scaled glyph; `None` always scales.
    pieces_above: Option<i32>,
    /// The advance, in font units of a 1024 em, that sets the slot width.
    slot: i32,
}

fn stretch(c: char) -> Option<Stretch> {
    let s = |whole, top, mid, bottom, ext, pieces_above, slot| Stretch {
        whole,
        top,
        mid,
        bottom,
        ext,
        pieces_above,
        slot,
    };
    Some(match c {
        '(' => s(
            '\u{e044}',
            '\u{e07c}',
            None,
            '\u{e07e}',
            '\u{e07d}',
            Some(3),
            398,
        ),
        ')' => s(
            '\u{e045}',
            '\u{e07f}',
            None,
            '\u{e081}',
            '\u{e080}',
            Some(3),
            398,
        ),
        '{' => s(
            '\u{e04b}',
            '\u{e078}',
            Some('\u{e079}'),
            '\u{e07a}',
            '\u{e07b}',
            Some(3),
            506,
        ),
        '}' => s(
            '\u{e04c}',
            '\u{e082}',
            Some('\u{e083}'),
            '\u{e084}',
            '\u{e07b}',
            Some(3),
            506,
        ),
        '[' => s(
            '\u{e049}',
            '\u{e100}',
            None,
            '\u{e103}',
            '\u{e101}',
            Some(2),
            506,
        ),
        ']' => s(
            '\u{e04a}',
            '\u{e102}',
            None,
            '\u{e104}',
            '\u{e105}',
            Some(2),
            506,
        ),
        '|' => s(
            '\u{e101}', '\u{e101}', None, '\u{e101}', '\u{e101}', None, 506,
        ),
        _ => return None,
    })
}

/// The vertical ink extent of an outline in font units (y up).
fn ink_y(o: &hm_fonts::Outline) -> (i32, i32) {
    use hm_fonts::PathOp;
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for op in &o.ops {
        let ys: &[f32] = match op {
            PathOp::Move(_, y) | PathOp::Line(_, y) => &[*y],
            PathOp::Quad(_, a, _, b) => &[*a, *b],
            PathOp::Cubic(_, a, _, b, _, c) => &[*a, *b, *c],
            PathOp::Close => &[],
        };
        for &y in ys {
            lo = lo.min(y);
            hi = hi.max(y);
        }
    }
    if lo > hi {
        return (0, 0);
    }
    (lo.floor() as i32, hi.ceil() as i32)
}

/// How far a glyph is drawn above the equation baseline: Hancom's upright
/// Latin letters and operators sit higher than its italic letters and
/// digits.
fn glyph_rise(c: char, size: i32) -> i32 {
    let up = match c {
        'A'..='Z' | 'a'..='z' => 66,
        '\u{e046}'..='\u{e048}'
        | '\u{e04e}'
        | '\u{e04f}'
        | '\u{e052}'
        | '\u{e053}'
        | '\u{e055}'
        | '\u{e056}' => 44,
        '\u{e044}' | '\u{e045}' | '\u{e049}'..='\u{e04c}' => 33,
        _ => 0,
    };
    -em(size, up)
}

/// Space between two neighbors in a row.
fn gap(prev: Option<Edge>, next: Edge, size: i32, unary: bool, fresh: bool) -> i32 {
    let Some(prev) = prev else { return 0 };
    // A relation joins a binary sign or another relation; punctuation
    // follows it without a gap. Font switches keep their run spacing.
    if !fresh
        && matches!(
            (prev, next),
            (Edge::Ord(Class::Bin), Edge::Ord(Class::Rel))
                | (Edge::Ord(Class::Rel), Edge::Ord(Class::Rel | Class::Punct))
        )
    {
        return 0;
    }
    let trail = match prev {
        Edge::Ord(Class::Rel) => em(size, 210),
        Edge::Ord(Class::Bin) => em(size, 140),
        Edge::Ord(Class::Punct) => em(size, 70),
        _ => 0,
    };
    // A sign right after an operator joins it, unless a font switch starts
    // a new run between them.
    if unary {
        return if fresh { trail } else { 0 };
    }
    let lead = if fresh || matches!(prev, Edge::Composite) {
        0
    } else {
        match next {
            Edge::Ord(Class::Rel) => em(size, 210),
            Edge::Ord(Class::Bin) => em(size, 140),
            // Bare parentheses keep 0.07 em from a letter or number before
            // them; they touch operators, spaces and other parentheses.
            Edge::Ord(Class::Open | Class::Close)
                if matches!(prev, Edge::Ord(Class::Ord) | Edge::Num | Edge::Func) =>
            {
                em(size, 70)
            }
            _ => 0,
        }
    };
    // A function name keeps an eighth of an em from an ordinary atom before it.
    if next == Edge::Func
        && matches!(
            prev,
            Edge::Ord(Class::Ord | Class::Close) | Edge::Scripted | Edge::Func
        )
    {
        return trail + size / 8;
    }
    let lead = if matches!(prev, Edge::Scripted) {
        match next {
            Edge::Ord(Class::Rel) => em(size, 300),
            Edge::Ord(Class::Bin) => em(size, 200),
            _ => 0,
        }
    } else {
        lead
    };
    trail + lead
}
