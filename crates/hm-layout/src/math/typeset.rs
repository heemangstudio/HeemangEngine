//! Boxes: laying out parsed equation nodes in HU.
//!
//! Every box has a width, an ascent and a descent around its baseline and
//! the items it paints. Rows join boxes with Hancom's operator spacing:
//! relations take 0.21 em on each side, binary operators 0.14 em, a comma
//! 0.07 em after it. An operator directly after a composite box (a
//! fraction, radical, decoration or delimited group) takes no space before
//! it.

use hm_model::units::mul_div;

use super::font::{MathFonts, pitch, pua};
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
}

#[derive(Clone, Debug)]
struct Bx {
    w: i32,
    asc: i32,
    desc: i32,
    items: Vec<MathItem>,
    left: Edge,
    right: Edge,
}

impl Bx {
    fn empty(size: i32) -> Bx {
        let (asc, desc) = line_extent(size);
        Bx {
            w: 0,
            asc,
            desc,
            items: Vec::new(),
            left: Edge::Ord(Class::Ord),
            right: Edge::Ord(Class::Ord),
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
}

/// Script size: 68 percent of the parent's.
fn script_size(size: i32) -> i32 {
    mul_div(size, 68, 100).max(1)
}

/// `em` fractions in HU, truncated.
fn em(size: i32, per_mille: i32) -> i32 {
    size * per_mille / 1000
}

pub fn typeset(node: &Node, size: i32, color: u32, fonts: &MathFonts) -> MathLayout {
    let cx = Ctx { fonts, color };
    let b = cx.node(node, size);
    MathLayout {
        width: b.w,
        ascent: b.asc,
        descent: b.desc,
        items: b.items,
    }
}

impl Ctx<'_> {
    fn node(&self, n: &Node, size: i32) -> Bx {
        match n {
            Node::Row(v) => self.row(v, size),
            Node::Group(v) => {
                let mut b = self.row(v, size);
                if v.len() > 1 {
                    b.left = Edge::Composite;
                    b.right = Edge::Composite;
                }
                b
            }
            Node::Atom(a) => self.atom(a, size),
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
                    out.place(p, w, -em(size, 447));
                }
                if let Some(q) = sub {
                    out.desc = out.desc.max(em(size, 273) + q.desc);
                    out.place(q, w, em(size, 273));
                }
                out.w = w + b.w;
                out.right = b.right;
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
            Node::Stack { kind, rows } => self.stack(*kind, rows, size),
            Node::Color { rgb, body } => {
                let inner = Ctx {
                    fonts: self.fonts,
                    color: *rgb,
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
                self.stack(Stack::LPile, &rows, size)
            }
            Node::Tab => Bx::empty(size),
        }
    }

    /// A glyph run.
    fn atom(&self, a: &Atom, size: i32) -> Bx {
        let size = if a.style.scale != 100 && a.style.scale > 0 {
            mul_div(size, a.style.scale as i32, 100)
        } else {
            size
        };
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
        b.left = Edge::Ord(a.class);
        b.right = Edge::Ord(a.class);
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
        let advance = |adv: i32, em: i32| {
            if natural {
                mul_div(adv, size, em.max(1))
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
                    let w = advance(g.advance, g.em as i32);
                    b.items.push(MathItem::Glyph {
                        face: *id,
                        glyph: g.glyph,
                        ch,
                        size,
                        x,
                        y: shift + if natural { 0 } else { glyph_rise(ch, size) },
                        x_scale: 100,
                        bold,
                        italic: false,
                        color: self.color,
                    });
                    return w;
                }
            }
        }
        for (id, face) in &self.fonts.text {
            if let Some(g) = face.glyph(c) {
                let w = advance(g.advance, g.em as i32);
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
        for (i, n) in v.iter().enumerate() {
            let b = self.node(n, size);
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
            x += gap(prev, b.left, size, unary);
            if first.is_none() {
                first = Some(b.left);
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
            out.asc = out.asc.max(b.asc);
            out.desc = out.desc.max(b.desc);
            let w = b.w;
            out.place(b, x, 0);
            x += w;
        }
        out.w = x;
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
        // A superscript's bottom sits 0.525 em below the top of its base, a
        // subscript's baseline 0.119 em below the base's bottom.
        if let Some(p) = sup {
            let rise = b.asc - em(size, 525) + p.desc;
            out.asc = out.asc.max(rise + p.asc);
            w = w.max(p.w);
            out.place(p, x, -rise);
        }
        if let Some(q) = sub {
            let drop = b.desc + em(size, 119);
            out.desc = out.desc.max(drop + q.desc);
            w = w.max(q.w);
            out.place(q, x, drop);
        }
        out.place(b, 0, 0);
        out.w = x + w;
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
        // The sign takes 0.96 em before the body and the box ends 0.165 em
        // after it: an em and an eighth in all.
        let lead = em(size, 960);
        let tail = size + size / 8 - lead;
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
        let gsize = mul_div(size, if integral { 200 } else { 180 }, 100);
        let a = Atom {
            text: std::iter::repeat_n(op.char(), op.count()).collect(),
            kind: AtomKind::Symbol,
            class: Class::Ord,
            style,
        };
        let mut g = self.atom(&a, gsize);
        // Big operators sit on the axis, not the text baseline.
        let lift = (g.asc - g.desc) / 2 - em(size, 270);
        let gw = g.w;
        let mut out = Bx::empty(size);
        let s = script_size(size);
        let sub = sub.map(|n| self.node(n, s));
        let sup = sup.map(|n| self.node(n, s));
        g.asc -= lift;
        g.desc += lift;
        out.asc = out.asc.max(g.asc);
        out.desc = out.desc.max(g.desc);
        if integral {
            let x = gw;
            let mut w = 0;
            if let Some(p) = sup {
                let y = -g.asc + p.asc;
                out.asc = out.asc.max(g.asc);
                w = w.max(p.w);
                out.place(p, x, y);
            }
            if let Some(q) = sub {
                let y = g.desc - q.desc;
                w = w.max(q.w);
                out.place(q, x - em(size, 200), y);
            }
            let ga = g.asc;
            out.place(g, 0, -lift);
            let _ = ga;
            out.w = gw + w;
        } else {
            let w = gw
                .max(sub.as_ref().map_or(0, |b| b.w))
                .max(sup.as_ref().map_or(0, |b| b.w));
            if let Some(p) = sup {
                let y = -g.asc - p.desc;
                out.asc = out.asc.max(g.asc + p.desc + p.asc);
                let pw = p.w;
                out.place(p, (w - pw) / 2, y);
            }
            if let Some(q) = sub {
                let y = g.desc + q.asc;
                out.desc = out.desc.max(g.desc + q.asc + q.desc);
                let qw = q.w;
                out.place(q, (w - qw) / 2, y);
            }
            out.place(g, (w - gw) / 2, -lift);
            out.w = w;
        }
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

    fn delim_glyph(&self, out: &mut Bx, c: char, x: i32, asc: i32, desc: i32, size: i32) {
        let h = asc + desc;
        let base = line_extent(size);
        let gsize = if h > base.0 + base.1 {
            mul_div(size, h, base.0 + base.1)
        } else {
            size
        };
        let a = Atom {
            text: c.to_string(),
            kind: AtomKind::Symbol,
            class: Class::Ord,
            style: Style::default(),
        };
        let g = self.atom(&a, gsize);
        let dy = (desc - mul_div(gsize, base.1, size)).max(0);
        out.place(g, x, dy);
    }

    fn delim(&self, left: Option<char>, right: Option<char>, body: &Node, size: i32) -> Bx {
        let b = self.node(body, size);
        let slot = |c: Option<char>| match c {
            None => 0,
            Some('(' | ')') => 375 * size / 1100,
            Some('|') => em(size, 500),
            Some(_) => 525 * size / 1100,
        };
        let (lw, rw) = (slot(left), slot(right));
        let mut out = Bx::empty(size);
        out.asc = out.asc.max(b.asc);
        out.desc = out.desc.max(b.desc);
        if let Some(c) = left {
            self.delim_glyph(&mut out, c, 0, b.asc, b.desc, size);
        }
        let bw = b.w;
        out.place(b, lw, 0);
        if let Some(c) = right {
            let (a, d) = (out.asc, out.desc);
            self.delim_glyph(&mut out, c, lw + bw, a, d, size);
        }
        out.w = lw + bw + rw;
        out.left = Edge::Composite;
        out.right = Edge::Composite;
        out
    }

    fn deco(&self, kind: Deco, body: &Node, size: i32) -> Bx {
        let b = self.node(body, size);
        let mut out = Bx::empty(size);
        let top = b.asc + em(size, 60);
        let thick = em(size, 40).max(8);
        match kind {
            Deco::Bar | Deco::Vec | Deco::Dyad => {
                out.items.push(MathItem::Rule {
                    x: 0,
                    y: -top,
                    w: b.w,
                    h: thick,
                    color: self.color,
                });
                out.asc = top + thick;
            }
            Deco::Under => {
                out.items.push(MathItem::Rule {
                    x: 0,
                    y: b.desc + em(size, 40),
                    w: b.w,
                    h: thick,
                    color: self.color,
                });
                out.desc = b.desc + em(size, 80);
            }
            _ => {
                let c = match kind {
                    Deco::Hat => '^',
                    Deco::Check => 'ˇ',
                    Deco::Tilde => '~',
                    Deco::Acute => '´',
                    Deco::Grave => '`',
                    Deco::Dot => '˙',
                    Deco::DDot => '¨',
                    Deco::Arch => '⌒',
                    Deco::Not => '/',
                    _ => '^',
                };
                let a = Atom {
                    text: c.to_string(),
                    kind: AtomKind::Symbol,
                    class: Class::Ord,
                    style: Style::default(),
                };
                let g = self.atom(&a, size);
                let gw = g.w;
                if kind == Deco::Not {
                    out.place(g, (b.w - gw) / 2, 0);
                } else {
                    out.place(g, (b.w - gw) / 2, -top + em(size, 700));
                    out.asc = out.asc.max(top + em(size, 300));
                }
            }
        }
        out.asc = out.asc.max(b.asc);
        out.desc = out.desc.max(b.desc);
        out.w = b.w;
        out.place(b, 0, 0);
        out.left = Edge::Composite;
        out.right = Edge::Composite;
        out
    }

    fn rel(&self, arrow: &Node, over: &Node, under: Option<&Node>, size: i32) -> Bx {
        let s = script_size(size);
        let a = self.node(arrow, size);
        let o = self.node(over, s);
        let u = under.map(|n| self.node(n, s));
        let w = a.w.max(o.w).max(u.as_ref().map_or(0, |b| b.w));
        let mut out = Bx::empty(size);
        out.asc = a.asc.max(a.asc + o.asc + o.desc);
        let ow = o.w;
        let oy = -a.asc - o.desc;
        out.place(o, (w - ow) / 2, oy);
        if let Some(u) = u {
            out.desc = a.desc + u.asc + u.desc;
            let uw = u.w;
            out.place(u, (w - uw) / 2, a.desc);
        }
        let aw = a.w;
        out.desc = out.desc.max(a.desc);
        out.place(a, (w - aw) / 2, 0);
        out.w = w;
        out.left = Edge::Ord(Class::Rel);
        out.right = Edge::Ord(Class::Rel);
        out
    }

    fn stack(&self, kind: Stack, rows: &[Vec<Node>], size: i32) -> Bx {
        let cells: Vec<Vec<Bx>> = rows
            .iter()
            .map(|r| r.iter().map(|c| self.node(c, size)).collect())
            .collect();
        let ncols = cells.iter().map(Vec::len).max().unwrap_or(0);
        let mut col_w = vec![0; ncols];
        for r in &cells {
            for (k, c) in r.iter().enumerate() {
                col_w[k] = col_w[k].max(c.w);
            }
        }
        let col_gap = match kind {
            Stack::Cases | Stack::EqAlign => em(size, 190),
            Stack::Pile | Stack::LPile | Stack::RPile => 0,
            _ => em(size, 800),
        };
        let row_gap = em(size, 150);
        let mut y = 0;
        let mut out = Bx::empty(size);
        let mut placed = Vec::new();
        for (ri, r) in cells.into_iter().enumerate() {
            let asc = r.iter().map(|c| c.asc).max().unwrap_or(line_extent(size).0);
            let desc = r
                .iter()
                .map(|c| c.desc)
                .max()
                .unwrap_or(line_extent(size).1);
            if ri > 0 {
                y += asc + row_gap;
            }
            let mut x = 0;
            for (k, c) in r.into_iter().enumerate() {
                let cw = c.w;
                let dx = match kind {
                    Stack::LPile | Stack::Cases | Stack::EqAlign => 0,
                    Stack::RPile => col_w[k] - cw,
                    _ => (col_w[k] - cw) / 2,
                };
                placed.push((c, x + dx, y));
                x += col_w[k] + col_gap;
            }
            y += desc;
        }
        let total_w: i32 = col_w.iter().sum::<i32>() + col_gap * (ncols as i32 - 1).max(0);
        // Center the rows on the axis.
        let first_asc = placed.first().map_or(0, |p| p.0.asc);
        let height = y + first_asc;
        let shift = -first_asc + (height / 2 - em(size, 270)).max(0) - (height / 2 - first_asc);
        let shift = shift.min(0);
        let mut inner = Bx::empty(size);
        inner.asc = first_asc - shift;
        inner.desc = (y + shift).max(0);
        for (c, x, y) in placed {
            inner.place(c, x, y + shift);
        }
        inner.w = total_w;
        let (lc, rc) = match kind {
            Stack::PMatrix => (Some('('), Some(')')),
            Stack::BMatrix => (Some('['), Some(']')),
            Stack::DMatrix => (Some('|'), Some('|')),
            Stack::Cases => (Some('{'), None),
            _ => (None, None),
        };
        if lc.is_none() && rc.is_none() {
            out.asc = out.asc.max(inner.asc);
            out.desc = out.desc.max(inner.desc);
            out.w = inner.w;
            out.place(inner, 0, 0);
            out.left = Edge::Composite;
            out.right = Edge::Composite;
            return out;
        }
        let slot = 525 * size / 1100;
        out.asc = out.asc.max(inner.asc);
        out.desc = out.desc.max(inner.desc);
        if let Some(c) = lc {
            self.delim_glyph(&mut out, c, 0, inner.asc, inner.desc, size);
        }
        let iw = inner.w;
        out.place(inner, slot, 0);
        out.w = slot + iw;
        if let Some(c) = rc {
            let (a, d) = (out.asc, out.desc);
            self.delim_glyph(&mut out, c, slot + iw, a, d, size);
            out.w += slot;
        }
        out.left = Edge::Composite;
        out.right = Edge::Composite;
        out
    }
}

/// How far a glyph is drawn above the equation baseline: Hancom's upright
/// Latin letters and operators sit higher than its italic letters and
/// digits.
fn glyph_rise(c: char, size: i32) -> i32 {
    let up = match c {
        'A'..='Z' | 'a'..='z' => 66,
        '\u{e046}'..='\u{e048}' | '\u{e052}' | '\u{e053}' | '\u{e055}' | '\u{e056}' => 44,
        '\u{e044}' | '\u{e045}' | '\u{e049}'..='\u{e04c}' => 33,
        _ => 0,
    };
    -em(size, up)
}

/// Space between two neighbors in a row.
fn gap(prev: Option<Edge>, next: Edge, size: i32, unary: bool) -> i32 {
    let Some(prev) = prev else { return 0 };
    let trail = match prev {
        Edge::Ord(Class::Rel) => em(size, 210),
        Edge::Ord(Class::Bin) => em(size, 140),
        Edge::Ord(Class::Punct) => em(size, 70),
        _ => 0,
    };
    let lead = if unary || matches!(prev, Edge::Composite) {
        0
    } else {
        match next {
            Edge::Ord(Class::Rel) => em(size, 210),
            Edge::Ord(Class::Bin) => em(size, 140),
            _ => 0,
        }
    };
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
