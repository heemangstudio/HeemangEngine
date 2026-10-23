//! Equation script parser.
//!
//! A row is a sequence of elements. An element is a group (`{...}`), a
//! word (letters and digits written together, `10a`), a symbol or a
//! command with its operands, followed by any scripts. `over`, `atop` and
//! `choose` take the element before and the element after them. Font
//! switches (`rm`, `it`, `bold`) apply to the rest of the enclosing group.
//! Serialized `scale` consumes its numeric parameter without changing size.

use super::lex::{Tok, Token, tokenize};
use super::table::{Big, Class, Deco, FontCmd, Kw, Stack, class_of};

/// What a glyph run is, for its font and spacing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AtomKind {
    /// Letters of a variable name.
    Var,
    Number,
    /// An operator or other symbol.
    Symbol,
    /// An upright function name.
    Func,
    /// Quoted or non-Latin text.
    Text,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub roman: bool,
    pub bold: bool,
    /// The element starts right after a font switch (`rm`, `it`, `bold`),
    /// which cancels an operator's leading space.
    pub after_switch: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Atom {
    pub text: String,
    pub kind: AtomKind,
    pub class: Class,
    pub style: Style,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Space {
    /// `~`
    Full,
    /// `` ` ``
    Quarter,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    /// Elements side by side.
    Row(Vec<Node>),
    /// A braced group: one element made of a row.
    Group(Vec<Node>),
    Atom(Atom),
    /// An `rm`/`it` switch is an operand even though it paints no glyph.
    FontSwitch,
    Space(Space),
    Frac {
        num: Box<Node>,
        den: Box<Node>,
        bar: bool,
    },
    Root {
        index: Option<Box<Node>>,
        body: Box<Node>,
    },
    Scripts {
        base: Box<Node>,
        sub: Option<Box<Node>>,
        sup: Option<Box<Node>>,
    },
    /// Scripts written before the base (`lsub`, `lsup`).
    LScripts {
        base: Box<Node>,
        sub: Option<Box<Node>>,
        sup: Option<Box<Node>>,
    },
    Big {
        op: Big,
        style: Style,
        sub: Option<Box<Node>>,
        sup: Option<Box<Node>>,
    },
    Lim {
        name: &'static str,
        style: Style,
        sub: Option<Box<Node>>,
    },
    /// `LEFT x ... RIGHT y`; `None` is an invisible delimiter (`.`).
    Delim {
        left: Option<char>,
        right: Option<char>,
        body: Box<Node>,
    },
    Deco {
        kind: Deco,
        body: Box<Node>,
    },
    /// Rows of cells (`#` separates rows, `&` cells).
    Stack {
        kind: Stack,
        rows: Vec<Vec<Node>>,
    },
    /// A matrix cell typed in a column (`col`, `lcol`, `rcol`): 0 left,
    /// 1 centered, 2 right.
    Aligned {
        align: u8,
        body: Box<Node>,
    },
    /// Long division: the divisor, the quotient, then the dividend and the
    /// working rows below it, each one a cell of `rows`.
    LongDiv {
        divisor: Box<Node>,
        quotient: Box<Node>,
        rows: Vec<Node>,
    },
    Color {
        rgb: u32,
        body: Box<Node>,
    },
    Rel {
        arrow: Box<Node>,
        over: Box<Node>,
        under: Option<Box<Node>>,
    },
    /// `BIGG x`: an enlarged delimiter.
    Bigg(Atom),
    /// Rows of the top level separated by `#`.
    Lines(Vec<Node>),
    /// A top-level `&`.
    Tab,
}

impl Node {
    pub fn row(mut v: Vec<Node>) -> Node {
        if v.len() == 1 {
            v.pop().unwrap()
        } else {
            Node::Row(v)
        }
    }
}

struct Parser {
    toks: Vec<Token>,
    pos: usize,
    depth: u32,
}

/// Where a row stops.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stop {
    End,
    Close,
    Right,
}

/// Nesting limit: deeper input is read as flat text.
const MAX_DEPTH: u32 = 64;

pub fn parse(script: &str) -> Node {
    let mut p = Parser {
        toks: tokenize(script),
        pos: 0,
        depth: 0,
    };
    let style = Style::default();
    let mut lines = Vec::new();
    loop {
        let row = p.row(Stop::End, style, true);
        lines.push(Node::row(row));
        if p.peek() != Some(&Tok::Hash) {
            break;
        }
        p.pos += 1;
        if p.pos >= p.toks.len() {
            break;
        }
    }
    if lines.len() == 1 {
        lines.pop().unwrap()
    } else {
        Node::Lines(lines)
    }
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos).map(|t| &t.tok)
    }

    fn peek_spaced(&self) -> bool {
        self.toks.get(self.pos).is_some_and(|t| t.spaced)
    }

    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).map(|t| t.tok.clone());
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    /// Reads elements until the stop token, which is left unread. Rows at
    /// the top level also stop at `#`.
    fn row(&mut self, stop: Stop, mut style: Style, top: bool) -> Vec<Node> {
        let mut out: Vec<Node> = Vec::new();
        let mut before_switch = style;
        while let Some(t) = self.peek() {
            match (t, stop) {
                (Tok::Close, Stop::Close) | (Tok::Kw(Kw::Right), Stop::Right) => break,
                (Tok::Hash, _) if top => break,
                (Tok::Close, Stop::End | Stop::Right) => {
                    self.pos += 1;
                    continue;
                }
                _ => {}
            }
            match t {
                Tok::Kw(Kw::Font(f)) => {
                    if *f != FontCmd::Bold {
                        before_switch = style;
                        out.push(Node::FontSwitch);
                    }
                    match f {
                        FontCmd::Roman => style.roman = true,
                        FontCmd::Italic => {
                            style.roman = false;
                            style.bold = false;
                        }
                        FontCmd::Bold => style.bold = true,
                    }
                    style.after_switch = true;
                    if *f == FontCmd::Bold {
                        before_switch = style;
                    }
                    let bold = *f == FontCmd::Bold;
                    self.pos += 1;
                    if bold
                        && (matches!(
                            (self.peek(), stop),
                            (None, _)
                                | (Some(Tok::Close), Stop::Close)
                                | (Some(Tok::Kw(Kw::Right)), Stop::Right)
                        ) || top && matches!(self.peek(), Some(Tok::Hash)))
                    {
                        out.push(Node::Space(Space::Full));
                    }
                    continue;
                }
                Tok::Kw(Kw::Scale) => {
                    self.pos += 1;
                    if matches!(self.peek(), Some(Tok::Number(_))) {
                        self.pos += 1;
                    }
                    before_switch = style;
                    continue;
                }
                Tok::Kw(k @ (Kw::Over | Kw::Atop | Kw::Choose)) => {
                    let k = *k;
                    self.pos += 1;
                    let num = out.pop().unwrap_or(Node::Row(Vec::new()));
                    // A switch moved into the numerator no longer changes
                    // the style of the enclosing row or its denominator.
                    if matches!(num, Node::FontSwitch) {
                        style = before_switch;
                    }
                    let den = self.infix_operand(style);
                    out.push(match k {
                        // A binomial is a pile in parentheses.
                        Kw::Choose => Node::Delim {
                            left: Some('('),
                            right: Some(')'),
                            body: Box::new(Node::Stack {
                                kind: Stack::Pile,
                                rows: vec![vec![num], vec![den]],
                            }),
                        },
                        _ => Node::Frac {
                            num: Box::new(num),
                            den: Box::new(den),
                            bar: k == Kw::Over,
                        },
                    });
                    continue;
                }
                Tok::Hash => {
                    // Only stacks give `#` a meaning below the top level.
                    self.pos += 1;
                    continue;
                }
                Tok::Amp if top => {
                    self.pos += 1;
                    out.push(Node::Tab);
                    continue;
                }
                _ => {}
            }
            match self.element(style) {
                Some(n) => out.push(n),
                None => {
                    if self.pos < self.toks.len() {
                        self.pos += 1;
                    }
                }
            }
            style.after_switch = false;
        }
        out
    }

    /// An infix command binds the switch itself, leaving the next element
    /// in the enclosing row. Prefix commands retain their operand grammar.
    fn infix_operand(&mut self, style: Style) -> Node {
        if matches!(
            self.peek(),
            Some(Tok::Kw(Kw::Font(FontCmd::Roman | FontCmd::Italic)))
        ) {
            self.pos += 1;
            Node::FontSwitch
        } else {
            self.operand(style)
        }
    }

    /// An operand: one element with its scripts, or an empty row.
    fn operand(&mut self, style: Style) -> Node {
        loop {
            match self.peek() {
                None | Some(Tok::Close | Tok::Hash | Tok::Amp) => return Node::Row(Vec::new()),
                Some(Tok::Kw(Kw::Font(f))) => {
                    let f = *f;
                    self.pos += 1;
                    let mut s = style;
                    match f {
                        FontCmd::Roman => s.roman = true,
                        FontCmd::Italic => s.roman = false,
                        FontCmd::Bold => s.bold = true,
                    }
                    return self.operand(s);
                }
                _ => {}
            }
            if let Some(n) = self.element(style) {
                return n;
            }
            if self.pos >= self.toks.len() {
                return Node::Row(Vec::new());
            }
            self.pos += 1;
        }
    }

    /// One element and its scripts; `None` for tokens that make nothing.
    fn element(&mut self, style: Style) -> Option<Node> {
        if self.depth > MAX_DEPTH {
            self.pos = self.toks.len();
            return None;
        }
        self.depth += 1;
        let base = self.primary(style);
        let out = base.map(|b| self.scripts(b, style));
        self.depth -= 1;
        out
    }

    fn atom(text: String, kind: AtomKind, class: Class, style: Style) -> Node {
        Node::Atom(Atom {
            text,
            kind,
            class,
            style,
        })
    }

    fn primary(&mut self, style: Style) -> Option<Node> {
        let tok = self.next()?;
        Some(match tok {
            Tok::Open => {
                let row = self.row(Stop::Close, style, false);
                if self.peek() == Some(&Tok::Close) {
                    self.pos += 1;
                }
                Node::Group(row)
            }
            Tok::Close => return None,
            Tok::Ident(s) => self.word(Self::atom(s, AtomKind::Var, Class::Ord, style), style),
            Tok::Number(s) => self.word(Self::atom(s, AtomKind::Number, Class::Ord, style), style),
            Tok::Quoted(s) => Self::atom(s, AtomKind::Text, Class::Ord, style),
            Tok::Char(c) => {
                if c.is_alphabetic() && !c.is_ascii() {
                    Self::atom(c.to_string(), AtomKind::Text, Class::Ord, style)
                } else if c == '\'' {
                    Self::atom("′".into(), AtomKind::Symbol, Class::Ord, style)
                } else {
                    Self::atom(c.to_string(), AtomKind::Symbol, class_of(c), style)
                }
            }
            Tok::Tilde => Node::Space(Space::Full),
            Tok::Tick => Node::Space(Space::Quarter),
            Tok::Caret | Tok::Under => {
                // A script with no base attaches to an empty one.
                self.pos -= 1;
                Node::Row(Vec::new())
            }
            Tok::Hash | Tok::Amp => return None,
            Tok::Kw(k) => return self.command(k, style),
        })
    }

    /// Joins letters and digits written without blanks into one word.
    fn word(&mut self, first: Node, style: Style) -> Node {
        let mut parts = vec![first];
        while !self.peek_spaced() {
            match self.peek().cloned() {
                Some(Tok::Ident(s)) => {
                    self.pos += 1;
                    parts.push(Self::atom(s, AtomKind::Var, Class::Ord, style));
                }
                Some(Tok::Number(s)) => {
                    self.pos += 1;
                    parts.push(Self::atom(s, AtomKind::Number, Class::Ord, style));
                }
                _ => break,
            }
        }
        Node::row(parts)
    }

    fn command(&mut self, k: Kw, style: Style) -> Option<Node> {
        Some(match k {
            Kw::Sym(c, class) => Self::atom(c.to_string(), AtomKind::Symbol, class, style),
            Kw::Func(name) => Self::atom(name.to_string(), AtomKind::Func, Class::Ord, style),
            Kw::Big(op) => {
                let (sub, sup) = self.limits(style);
                Node::Big {
                    op,
                    style,
                    sub,
                    sup,
                }
            }
            Kw::Lim(name) => {
                let (sub, _) = self.limits(style);
                Node::Lim { name, style, sub }
            }
            Kw::Deco(kind) => Node::Deco {
                kind,
                body: Box::new(self.operand(style)),
            },
            Kw::Stack(kind) => Node::Stack {
                kind,
                rows: self.stack_rows(style),
            },
            Kw::Sqrt => {
                // `sqrt 3 of 5` is the cube root of 5, as `root 3 of 5`.
                let first = self.operand(style);
                if self.peek() == Some(&Tok::Kw(Kw::Of)) {
                    self.pos += 1;
                    Node::Root {
                        index: Some(Box::new(first)),
                        body: Box::new(self.operand(style)),
                    }
                } else {
                    Node::Root {
                        index: None,
                        body: Box::new(first),
                    }
                }
            }
            Kw::Root => {
                // `root 3 of 5` takes an index; a bare `root 3` is a square root.
                let first = self.operand(style);
                if self.peek() == Some(&Tok::Kw(Kw::Of)) {
                    self.pos += 1;
                    Node::Root {
                        index: Some(Box::new(first)),
                        body: Box::new(self.operand(style)),
                    }
                } else {
                    Node::Root {
                        index: None,
                        body: Box::new(first),
                    }
                }
            }
            Kw::Left => {
                let left = self.delimiter();
                let body = self.row(Stop::Right, style, false);
                let right = if self.peek() == Some(&Tok::Kw(Kw::Right)) {
                    self.pos += 1;
                    self.delimiter()
                } else {
                    None
                };
                Node::Delim {
                    left,
                    right,
                    body: Box::new(Node::row(body)),
                }
            }
            Kw::Color => {
                let rgb = self.color();
                Node::Color {
                    rgb,
                    body: Box::new(self.operand(style)),
                }
            }
            Kw::Rel | Kw::BuildRel => {
                let arrow = self.operand(style);
                let over = self.operand(style);
                let under = (k == Kw::Rel).then(|| Box::new(self.operand(style)));
                Node::Rel {
                    arrow: Box::new(arrow),
                    over: Box::new(over),
                    under,
                }
            }
            Kw::Binom => {
                let top = self.operand(style);
                let bottom = self.operand(style);
                Node::Delim {
                    left: Some('('),
                    right: Some(')'),
                    body: Box::new(Node::Stack {
                        kind: Stack::Pile,
                        rows: vec![vec![top], vec![bottom]],
                    }),
                }
            }
            Kw::Bigg => {
                let c = self.delimiter().unwrap_or('(');
                Node::Bigg(Atom {
                    text: c.to_string(),
                    kind: AtomKind::Symbol,
                    class: class_of(c),
                    style,
                })
            }
            Kw::Ladder => Node::Stack {
                kind: Stack::Ladder,
                rows: self.stack_rows(style),
            },
            Kw::SLadder => {
                let dots = || {
                    Node::Atom(Atom {
                        text: "\u{22EF}".into(),
                        kind: AtomKind::Symbol,
                        class: Class::Ord,
                        style,
                    })
                };
                let rows = self.stack_rows(style);
                let n = rows.len();
                // The last row holds the final quotient without a divisor.
                let rows = rows
                    .into_iter()
                    .enumerate()
                    .map(|(i, mut r)| {
                        let at = if n > 1 && i + 1 == n { 1 } else { 2 };
                        if let Some(rem) = r.get_mut(at)
                            && !matches!(rem, Node::Row(v) if v.is_empty())
                        {
                            let r = std::mem::replace(rem, Node::Row(Vec::new()));
                            *rem = Node::Row(vec![dots(), Node::Space(Space::Quarter), r]);
                        }
                        r
                    })
                    .collect();
                Node::Stack {
                    kind: Stack::SLadder,
                    rows,
                }
            }
            Kw::LongDiv => {
                let divisor = Box::new(self.operand(style));
                let quotient = Box::new(self.operand(style));
                let rows = self.stack_rows(style).into_iter().map(Node::row).collect();
                Node::LongDiv {
                    divisor,
                    quotient,
                    rows,
                }
            }
            Kw::LSup | Kw::LSub => {
                let script = self.operand(style);
                let base = self.operand(style);
                let (sub, sup) = if k == Kw::LSub {
                    (Some(Box::new(script)), None)
                } else {
                    (None, Some(Box::new(script)))
                };
                Node::LScripts {
                    base: Box::new(base),
                    sub,
                    sup,
                }
            }
            Kw::Sup | Kw::Sub => {
                self.pos -= 1;
                Node::Row(Vec::new())
            }
            Kw::From
            | Kw::To
            | Kw::Right
            | Kw::Of
            | Kw::Over
            | Kw::Atop
            | Kw::Choose
            | Kw::Col(_) => return None,
            Kw::Font(_) | Kw::Scale => return None,
        })
    }

    /// Reads `_`/`sub`/`from` and `^`/`sup`/`to` limits in any order.
    fn limits(&mut self, style: Style) -> (Option<Box<Node>>, Option<Box<Node>>) {
        let (mut sub, mut sup) = (None, None);
        loop {
            match self.peek() {
                Some(Tok::Under | Tok::Kw(Kw::Sub | Kw::From)) if sub.is_none() => {
                    self.pos += 1;
                    sub = Some(Box::new(self.operand_no_scripts(style)));
                }
                Some(Tok::Caret | Tok::Kw(Kw::Sup | Kw::To)) if sup.is_none() => {
                    self.pos += 1;
                    sup = Some(Box::new(self.operand_no_scripts(style)));
                }
                _ => return (sub, sup),
            }
        }
    }

    fn scripts(&mut self, base: Node, style: Style) -> Node {
        let mut base = base;
        loop {
            let (sub, sup) = match self.peek() {
                Some(Tok::Under | Tok::Kw(Kw::Sub)) => (true, false),
                Some(Tok::Caret | Tok::Kw(Kw::Sup)) => (false, true),
                Some(Tok::Char('\'')) if !self.peek_spaced() => {
                    self.pos += 1;
                    base = Node::Row(vec![
                        base,
                        Self::atom("′".into(), AtomKind::Symbol, Class::Ord, style),
                    ]);
                    continue;
                }
                _ => return base,
            };
            self.pos += 1;
            let operand = Some(Box::new(self.operand_no_scripts(style)));
            base = match base {
                Node::Scripts {
                    base: b,
                    sub: s,
                    sup: p,
                } if (sub && s.is_none()) || (sup && p.is_none()) => Node::Scripts {
                    base: b,
                    sub: if sub { operand.clone() } else { s },
                    sup: if sup { operand } else { p },
                },
                other => Node::Scripts {
                    base: Box::new(other),
                    sub: if sub { operand.clone() } else { None },
                    sup: if sup { operand } else { None },
                },
            };
        }
    }

    /// A script operand: a primary without its own scripts, so `x^2_3`
    /// puts both scripts on `x`.
    fn operand_no_scripts(&mut self, style: Style) -> Node {
        let first = loop {
            match self.peek() {
                None | Some(Tok::Close | Tok::Hash | Tok::Amp) => return Node::Row(Vec::new()),
                Some(Tok::Caret | Tok::Under) => return Node::Row(Vec::new()),
                _ => {}
            }
            if self.depth > MAX_DEPTH {
                self.pos = self.toks.len();
                return Node::Row(Vec::new());
            }
            let group = self.peek() == Some(&Tok::Open);
            self.depth += 1;
            let p = self.primary(style);
            self.depth -= 1;
            if let Some(n) = p {
                if group {
                    return n;
                }
                break n;
            }
            if self.pos >= self.toks.len() {
                return Node::Row(Vec::new());
            }
        };
        // An unbraced script runs to the next blank: `_t->0+` is `t→0+`.
        let mut items = vec![first];
        while !self.peek_spaced() {
            let more = matches!(
                self.peek(),
                Some(
                    Tok::Ident(_)
                        | Tok::Number(_)
                        | Tok::Char(_)
                        | Tok::Quoted(_)
                        | Tok::Kw(Kw::Sym(..) | Kw::Func(_))
                )
            );
            if !more {
                break;
            }
            match self.primary(style) {
                Some(n) => items.push(n),
                None => break,
            }
        }
        Node::row(items)
    }

    /// A delimiter after `LEFT`, `RIGHT` or `BIGG`.
    fn delimiter(&mut self) -> Option<char> {
        match self.next()? {
            Tok::Char('.') => None,
            Tok::Char(c) => Some(c),
            Tok::Open => Some('{'),
            Tok::Close => Some('}'),
            Tok::Kw(Kw::Sym(c, _)) => Some(c),
            Tok::Ident(s) if s.eq_ignore_ascii_case("lbrace") => Some('{'),
            Tok::Ident(s) if s.eq_ignore_ascii_case("rbrace") => Some('}'),
            Tok::Ident(s) => match s.to_ascii_lowercase().as_str() {
                "langle" => Some('〈'),
                "rangle" => Some('〉'),
                "lfloor" => Some('⌊'),
                "rfloor" => Some('⌋'),
                "lceil" => Some('⌈'),
                "rceil" => Some('⌉'),
                "dline" => Some('∥'),
                "line" => Some('|'),
                _ => {
                    self.pos -= 1;
                    None
                }
            },
            _ => {
                self.pos -= 1;
                None
            }
        }
    }

    fn color(&mut self) -> u32 {
        let mut parts = Vec::new();
        if self.peek() == Some(&Tok::Open) {
            self.pos += 1;
            while let Some(t) = self.next() {
                match t {
                    Tok::Close => break,
                    Tok::Number(n) => parts.push(n.parse::<u32>().unwrap_or(0).min(255)),
                    _ => {}
                }
            }
        }
        let c = |i: usize| parts.get(i).copied().unwrap_or(0);
        c(0) | (c(1) << 8) | (c(2) << 16)
    }

    /// The braced body of a stack: rows split at `#`, cells at `&`.
    fn stack_rows(&mut self, style: Style) -> Vec<Vec<Node>> {
        let mut rows = Vec::new();
        if self.peek() != Some(&Tok::Open) {
            rows.push(vec![self.operand(style)]);
            return rows;
        }
        self.pos += 1;
        // Columns typed one by one (`col{a#b} rcol{c#d}`) make the rows.
        if matches!(self.peek(), Some(Tok::Kw(Kw::Col(_)))) {
            let mut cols: Vec<(u8, Vec<Node>)> = Vec::new();
            while let Some(Tok::Kw(Kw::Col(a))) = self.peek() {
                let a = *a;
                self.pos += 1;
                let cells = self.stack_rows(style).into_iter().map(Node::row).collect();
                cols.push((a, cells));
            }
            while let Some(t) = self.next() {
                if t == Tok::Close {
                    break;
                }
            }
            let n = cols.iter().map(|c| c.1.len()).max().unwrap_or(0);
            return (0..n)
                .map(|i| {
                    cols.iter()
                        .map(|(a, c)| Node::Aligned {
                            align: *a,
                            body: Box::new(c.get(i).cloned().unwrap_or(Node::Row(Vec::new()))),
                        })
                        .collect()
                })
                .collect();
        }
        let mut cells: Vec<Node> = Vec::new();
        let mut cur: Vec<Node> = Vec::new();
        let mut cell_style = style;
        loop {
            match self.peek() {
                None => break,
                Some(Tok::Close) => {
                    self.pos += 1;
                    break;
                }
                Some(Tok::Hash) => {
                    self.pos += 1;
                    cells.push(Node::row(std::mem::take(&mut cur)));
                    rows.push(std::mem::take(&mut cells));
                    cell_style = style;
                    continue;
                }
                Some(Tok::Amp) => {
                    self.pos += 1;
                    cells.push(Node::row(std::mem::take(&mut cur)));
                    continue;
                }
                _ => {}
            }
            // Read one element at a time so `#` and `&` stay visible here.
            let mut one = self.row_until_separator(cell_style);
            if let Some(s) = one.1 {
                cell_style = s;
            }
            cur.append(&mut one.0);
        }
        cells.push(Node::row(cur));
        rows.push(cells);
        rows
    }

    /// Reads a row up to the next `#`, `&` or closing brace, returning the
    /// style in force at its end.
    fn row_until_separator(&mut self, style: Style) -> (Vec<Node>, Option<Style>) {
        let mut style = style;
        let mut before_switch = style;
        let mut out: Vec<Node> = Vec::new();
        while let Some(t) = self.peek() {
            match t {
                Tok::Close | Tok::Hash | Tok::Amp => break,
                Tok::Kw(Kw::Font(f)) => {
                    if *f != FontCmd::Bold {
                        before_switch = style;
                        out.push(Node::FontSwitch);
                    }
                    match f {
                        FontCmd::Roman => style.roman = true,
                        FontCmd::Italic => {
                            style.roman = false;
                            style.bold = false;
                        }
                        FontCmd::Bold => style.bold = true,
                    }
                    if *f == FontCmd::Bold {
                        before_switch = style;
                    }
                    let bold = *f == FontCmd::Bold;
                    self.pos += 1;
                    if bold && matches!(self.peek(), None | Some(Tok::Close | Tok::Hash | Tok::Amp))
                    {
                        out.push(Node::Space(Space::Full));
                    }
                }
                Tok::Kw(k @ (Kw::Over | Kw::Atop)) => {
                    let bar = *k == Kw::Over;
                    self.pos += 1;
                    let num = out.pop().unwrap_or(Node::Row(Vec::new()));
                    if matches!(num, Node::FontSwitch) {
                        style = before_switch;
                    }
                    let den = self.infix_operand(style);
                    out.push(Node::Frac {
                        num: Box::new(num),
                        den: Box::new(den),
                        bar,
                    });
                }
                _ => match self.element(style) {
                    Some(n) => out.push(n),
                    None => {
                        if self.pos < self.toks.len() {
                            self.pos += 1;
                        }
                    }
                },
            }
        }
        (out, Some(style))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(n: &Node) -> String {
        match n {
            Node::Atom(a) => a.text.clone(),
            Node::Row(v) | Node::Group(v) => v.iter().map(text).collect::<Vec<_>>().join(""),
            Node::Frac { num, den, .. } => format!("({})/({})", text(num), text(den)),
            Node::Scripts { base, sub, sup } => format!(
                "{}{}{}",
                text(base),
                sub.as_ref()
                    .map_or(String::new(), |s| format!("_({})", text(s))),
                sup.as_ref()
                    .map_or(String::new(), |s| format!("^({})", text(s)))
            ),
            Node::Root { index, body } => format!(
                "root[{}]({})",
                index.as_ref().map_or(String::new(), |i| text(i)),
                text(body)
            ),
            Node::Delim { left, right, body } => format!(
                "{}{}{}",
                left.unwrap_or('.'),
                text(body),
                right.unwrap_or('.')
            ),
            Node::Space(_) => " ".into(),
            Node::FontSwitch => String::new(),
            other => format!("{other:?}"),
        }
    }

    #[test]
    fn missing_bold_operand_reserves_a_blank_in_its_row() {
        assert_eq!(text(&parse("bold")), " ");
        assert_eq!(text(&parse("ABC bold")), "ABC ");
        assert_eq!(text(&parse("{ABC bold} D")), "ABC D");
        assert_eq!(text(&parse("bold ABC")), "ABC");
        assert_eq!(text(&parse("bold bold ABC")), "ABC");
        let Node::Stack { rows, .. } = parse("matrix{A bold & B#C & D bold}") else {
            panic!()
        };
        assert_eq!(text(&rows[0][0]), "A ");
        assert_eq!(text(&rows[1][1]), "D ");
    }

    #[test]
    fn over_takes_neighbouring_elements() {
        let n = parse("{a_{4}} over {a_{2}}+{a_{2}} over {a_{1}}=30");
        assert_eq!(text(&n), "(a_(4))/(a_(2))+(a_(2))/(a_(1))=30");
        assert_eq!(text(&parse("10a^3 over b^2")), "(10a^(3))/(b^(2))");
    }

    #[test]
    fn roots_delimiters_and_fonts() {
        assert_eq!(text(&parse("sqrt {3} of {5}")), "root[3](5)");
        assert_eq!(text(&parse("root {3} of {5}")), "root[3](5)");
        assert_eq!(text(&parse("f left(2+h right)")), "f(2+h)");
        let n = parse("rm AB it x");
        let Node::Row(v) = n else { panic!() };
        let roman: Vec<bool> = v
            .iter()
            .filter_map(|n| match n {
                Node::Atom(a) => Some(a.style.roman),
                _ => None,
            })
            .collect();
        assert_eq!(roman, vec![true, false]);
    }

    #[test]
    fn matrix_columns_typed_one_by_one_become_rows() {
        let Node::Stack { rows, .. } = parse("matrix{lcol{a#bb} rcol{1#22}}") else {
            panic!()
        };
        let cells: Vec<Vec<(u8, String)>> = rows
            .iter()
            .map(|r| {
                r.iter()
                    .map(|c| match c {
                        Node::Aligned { align, body } => (*align, text(body)),
                        other => (9, text(other)),
                    })
                    .collect()
            })
            .collect();
        assert_eq!(
            cells,
            vec![
                vec![(0, "a".into()), (2, "1".into())],
                vec![(0, "bb".into()), (2, "22".into())]
            ]
        );
    }

    #[test]
    fn stacks_split_rows_and_cells() {
        let n = parse("cases{a & b # c & d}");
        let Node::Stack { rows, .. } = n else {
            panic!()
        };
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].len(), 2);
    }
}
