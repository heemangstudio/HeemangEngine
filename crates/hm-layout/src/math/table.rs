//! The equation script vocabulary: commands, symbols, functions and their
//! spacing classes.

/// How a symbol takes part in spacing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// Variables, numbers, letter-like symbols.
    Ord,
    /// Binary operators (`+`, `times`).
    Bin,
    /// Relations (`=`, `leq`, arrows).
    Rel,
    Open,
    Close,
    /// Separators (`,`, `;`).
    Punct,
}

/// Large operators that take limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Big {
    Sum,
    Prod,
    Coprod,
    Union,
    Inter,
    Int,
    DInt,
    TInt,
    OInt,
    ODInt,
    OTInt,
}

impl Big {
    /// The glyph Hancom paints.
    pub fn char(self) -> char {
        match self {
            Big::Sum => '∑',
            Big::Prod => '∏',
            Big::Coprod => '∐',
            Big::Union => '⋃',
            Big::Inter => '⋂',
            Big::Int | Big::DInt | Big::TInt => '∫',
            Big::OInt | Big::ODInt | Big::OTInt => '∮',
        }
    }

    /// How many integral signs are drawn.
    pub fn count(self) -> usize {
        match self {
            Big::DInt | Big::ODInt => 2,
            Big::TInt | Big::OTInt => 3,
            _ => 1,
        }
    }

    pub fn integral(self) -> bool {
        matches!(
            self,
            Big::Int | Big::DInt | Big::TInt | Big::OInt | Big::ODInt | Big::OTInt
        )
    }
}

/// Accents and decorations over or under their operand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Deco {
    Hat,
    Check,
    Tilde,
    Acute,
    Grave,
    Dot,
    DDot,
    Bar,
    Vec,
    Dyad,
    Under,
    Arch,
    /// `not`: a slash through the operand.
    Not,
    /// `box`: a frame around the operand.
    Box,
}

/// Matrix and stack kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stack {
    Matrix,
    PMatrix,
    BMatrix,
    DMatrix,
    Cases,
    Pile,
    LPile,
    RPile,
    EqAlign,
    /// Common divisors down the left with the quotients to their right
    /// (`ladder`, `sladder`): a bar after the divisors and a rule under
    /// each divided row.
    Ladder,
    /// A binary ladder (`sladder`): remainders follow the quotients after
    /// dots, outside the rules.
    SLadder,
}

/// Font switches. They apply to the rest of the enclosing group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontCmd {
    Roman,
    Italic,
    Bold,
}

/// What a keyword means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kw {
    Sym(char, Class),
    /// A name drawn upright (`sin`, `lim` without limits, `and`).
    Func(&'static str),
    Big(Big),
    /// `lim` and `Lim`: an upright name that takes limits below.
    Lim(&'static str),
    Deco(Deco),
    Stack(Stack),
    Font(FontCmd),
    Over,
    Atop,
    Sqrt,
    Root,
    Of,
    Left,
    Right,
    Sup,
    Sub,
    LSup,
    LSub,
    From,
    To,
    Bigg,
    Rel,
    BuildRel,
    Choose,
    Binom,
    Color,
    Scale,
    Ladder,
    SLadder,
    LongDiv,
    Col(u8),
}

const GREEK: &[(&str, char, char)] = &[
    ("alpha", 'α', 'Α'),
    ("beta", 'β', 'Β'),
    ("gamma", 'γ', 'Γ'),
    ("delta", 'δ', 'Δ'),
    ("epsilon", 'ε', 'Ε'),
    ("zeta", 'ζ', 'Ζ'),
    ("eta", 'η', 'Η'),
    ("theta", 'θ', 'Θ'),
    ("iota", 'ι', 'Ι'),
    ("kappa", 'κ', 'Κ'),
    ("lambda", 'λ', 'Λ'),
    ("mu", 'μ', 'Μ'),
    ("nu", 'ν', 'Ν'),
    ("xi", 'ξ', 'Ξ'),
    ("omicron", 'ο', 'Ο'),
    ("pi", 'π', 'Π'),
    ("rho", 'ρ', 'Ρ'),
    ("sigma", 'σ', 'Σ'),
    ("tau", 'τ', 'Τ'),
    ("upsilon", 'υ', 'Υ'),
    ("phi", 'φ', 'Φ'),
    ("chi", 'χ', 'Χ'),
    ("psi", 'ψ', 'Ψ'),
    ("omega", 'ω', 'Ω'),
];

/// Symbols whose name is matched without regard to case.
const SYMBOLS: &[(&str, char, Class)] = &[
    // Letter-like symbols.
    ("aleph", 'ℵ', Class::Ord),
    ("hbar", 'ℏ', Class::Ord),
    ("imath", 'ı', Class::Ord),
    ("jmath", 'ȷ', Class::Ord),
    ("ohm", 'Ω', Class::Ord),
    ("ell", 'ℓ', Class::Ord),
    ("liter", 'ℓ', Class::Ord),
    ("wp", '℘', Class::Ord),
    ("imag", 'ℑ', Class::Ord),
    ("angstrom", 'Å', Class::Ord),
    ("vartheta", 'ϑ', Class::Ord),
    ("varpi", 'ϖ', Class::Ord),
    ("varsigma", 'ς', Class::Ord),
    ("varupsilon", 'ϒ', Class::Ord),
    ("varphi", 'ϕ', Class::Ord),
    ("varepsilon", 'ϵ', Class::Ord),
    // Set and binary operators.
    // Hancom's union and intersection glyphs carry their own side
    // bearings; like `neq`, `sim` and `simeq` they space as binary signs.
    ("cap", '∩', Class::Ord),
    ("cup", '∪', Class::Ord),
    ("smallinter", '∩', Class::Ord),
    ("smallunion", '∪', Class::Ord),
    ("sqcap", '⊓', Class::Bin),
    ("sqcup", '⊔', Class::Bin),
    ("uplus", '⊎', Class::Bin),
    ("vee", '∨', Class::Bin),
    ("wedge", '∧', Class::Bin),
    ("plusminus", '±', Class::Bin),
    ("minusplus", '∓', Class::Bin),
    ("times", '×', Class::Bin),
    ("div", '÷', Class::Bin),
    ("divide", '÷', Class::Bin),
    ("circ", '∘', Class::Bin),
    ("bullet", '∙', Class::Bin),
    ("cdot", '⋅', Class::Ord),
    ("ast", '∗', Class::Bin),
    ("star", '⋆', Class::Bin),
    ("bigcirc", '○', Class::Bin),
    ("diamond", '⋄', Class::Bin),
    ("dsum", '∔', Class::Bin),
    ("xor", '⊻', Class::Bin),
    ("oplus", '⊕', Class::Bin),
    ("ominus", '⊖', Class::Bin),
    ("otimes", '⊗', Class::Bin),
    ("odot", '⊙', Class::Bin),
    ("oslash", '⊘', Class::Bin),
    ("dagger", '†', Class::Ord),
    ("ddagger", '‡', Class::Ord),
    // Relations.
    ("subset", '⊂', Class::Rel),
    ("supset", '⊃', Class::Rel),
    ("subseteq", '⊆', Class::Rel),
    ("supseteq", '⊇', Class::Rel),
    ("in", '∈', Class::Rel),
    ("owns", '∋', Class::Rel),
    ("notin", '∉', Class::Rel),
    ("leq", '≤', Class::Rel),
    ("le", '≤', Class::Rel),
    ("geq", '≥', Class::Rel),
    ("ge", '≥', Class::Rel),
    ("sqsubset", '⊏', Class::Rel),
    ("sqsupset", '⊐', Class::Rel),
    ("sqsubseteq", '⊑', Class::Rel),
    ("sqsupseteq", '⊒', Class::Rel),
    ("lll", '⋘', Class::Rel),
    ("ggg", '⋙', Class::Rel),
    ("prec", '≺', Class::Rel),
    ("succ", '≻', Class::Rel),
    ("neq", '≠', Class::Bin),
    ("doteq", '≐', Class::Rel),
    ("image", '≓', Class::Rel),
    ("reimage", '≒', Class::Rel),
    ("sim", '∼', Class::Bin),
    ("approx", '≈', Class::Rel),
    ("simeq", '≃', Class::Bin),
    ("cong", '≅', Class::Rel),
    ("equiv", '≡', Class::Rel),
    ("identical", '≡', Class::Rel),
    ("asymp", '≍', Class::Rel),
    ("iso", '≎', Class::Rel),
    ("propto", '∝', Class::Rel),
    ("vdash", '⊢', Class::Rel),
    ("hleft", '⊣', Class::Rel),
    ("models", '⊨', Class::Rel),
    ("mapsto", '↦', Class::Ord),
    ("hookleft", '↩', Class::Ord),
    ("hookright", '↪', Class::Ord),
    ("nwarrow", '↖', Class::Ord),
    ("nearrow", '↗', Class::Ord),
    ("swarrow", '↙', Class::Ord),
    ("searrow", '↘', Class::Ord),
    // Other symbols.
    ("deg", '°', Class::Ord),
    ("emptyset", '∅', Class::Ord),
    ("therefore", '∴', Class::Ord),
    ("because", '∵', Class::Ord),
    ("exist", '∃', Class::Ord),
    ("forall", '∀', Class::Ord),
    ("prime", '′', Class::Ord),
    ("partial", '∂', Class::Ord),
    ("inf", '∞', Class::Ord),
    ("lnot", '¬', Class::Ord),
    ("cdots", '⋯', Class::Rel),
    ("ldots", '…', Class::Ord),
    ("vdots", '⋮', Class::Ord),
    ("ddots", '⋱', Class::Ord),
    ("triangle", '∆', Class::Ord),
    ("triangled", '▽', Class::Ord),
    ("angle", '∠', Class::Ord),
    ("msangle", '∡', Class::Ord),
    ("sangle", '∢', Class::Ord),
    ("rtangle", '⊾', Class::Ord),
    ("bot", '⊥', Class::Ord),
    ("top", '⊤', Class::Ord),
    ("laplace", 'ℒ', Class::Ord),
    ("centigrade", '℃', Class::Ord),
    ("fahrenheit", '℉', Class::Ord),
    ("lslant", '╱', Class::Ord),
    ("rslant", '╲', Class::Ord),
    ("att", '@', Class::Ord),
    ("hund", '‰', Class::Ord),
    ("thou", '‱', Class::Ord),
    ("well", '♯', Class::Ord),
    ("base", '⌂', Class::Ord),
    ("benzene", '⌬', Class::Ord),
    ("vert", '|', Class::Ord),
];

/// Arrows: the lowercase name is single, a capitalized name double.
const ARROWS: &[(&str, char, char)] = &[
    ("larrow", '←', '⇐'),
    ("rarrow", '→', '⇒'),
    ("uparrow", '↑', '⇑'),
    ("downarrow", '↓', '⇓'),
    ("udarrow", '↕', '⇕'),
    ("lrarrow", '↔', '⇔'),
    ("vert", '|', '‖'),
];

/// Names drawn upright. Matched exactly as written.
const FUNCTIONS: &[&str] = &[
    "sin", "cos", "tan", "cot", "sec", "csc", "cosec", "sinh", "cosh", "tanh", "coth", "arcsin",
    "arccos", "arctan", "arc", "asin", "acos", "atan", "ln", "lg", "log", "exp", "Exp", "max",
    "min", "det", "gcd", "lcm", "mod", "if", "for", "and", "hom", "ker", "deg", "arg", "dim", "Pr",
];

const COMMANDS: &[(&str, Kw)] = &[
    ("over", Kw::Over),
    ("atop", Kw::Atop),
    ("sqrt", Kw::Sqrt),
    ("root", Kw::Root),
    ("of", Kw::Of),
    ("left", Kw::Left),
    ("right", Kw::Right),
    ("sup", Kw::Sup),
    ("sub", Kw::Sub),
    ("lsup", Kw::LSup),
    ("lsub", Kw::LSub),
    ("from", Kw::From),
    ("to", Kw::To),
    ("bigg", Kw::Bigg),
    ("rel", Kw::Rel),
    ("buildrel", Kw::BuildRel),
    ("choose", Kw::Choose),
    ("binom", Kw::Binom),
    ("color", Kw::Color),
    ("scale", Kw::Scale),
    ("ladder", Kw::Ladder),
    ("sladder", Kw::SLadder),
    ("longdiv", Kw::LongDiv),
    ("col", Kw::Col(1)),
    ("lcol", Kw::Col(0)),
    ("rcol", Kw::Col(2)),
    ("rm", Kw::Font(FontCmd::Roman)),
    ("it", Kw::Font(FontCmd::Italic)),
    ("bold", Kw::Font(FontCmd::Bold)),
    ("hat", Kw::Deco(Deco::Hat)),
    ("check", Kw::Deco(Deco::Check)),
    ("tilde", Kw::Deco(Deco::Tilde)),
    ("acute", Kw::Deco(Deco::Acute)),
    ("grave", Kw::Deco(Deco::Grave)),
    ("dot", Kw::Deco(Deco::Dot)),
    ("ddot", Kw::Deco(Deco::DDot)),
    ("bar", Kw::Deco(Deco::Bar)),
    ("overline", Kw::Deco(Deco::Bar)),
    ("underline", Kw::Deco(Deco::Under)),
    ("box", Kw::Deco(Deco::Box)),
    ("vec", Kw::Deco(Deco::Vec)),
    ("dyad", Kw::Deco(Deco::Dyad)),
    ("under", Kw::Deco(Deco::Under)),
    ("arch", Kw::Deco(Deco::Arch)),
    ("not", Kw::Deco(Deco::Not)),
    ("matrix", Kw::Stack(Stack::Matrix)),
    ("pmatrix", Kw::Stack(Stack::PMatrix)),
    ("bmatrix", Kw::Stack(Stack::BMatrix)),
    ("dmatrix", Kw::Stack(Stack::DMatrix)),
    ("cases", Kw::Stack(Stack::Cases)),
    ("pile", Kw::Stack(Stack::Pile)),
    ("lpile", Kw::Stack(Stack::LPile)),
    ("rpile", Kw::Stack(Stack::RPile)),
    ("eqalign", Kw::Stack(Stack::EqAlign)),
    ("sum", Kw::Big(Big::Sum)),
    ("prod", Kw::Big(Big::Prod)),
    ("coprod", Kw::Big(Big::Coprod)),
    ("union", Kw::Big(Big::Union)),
    ("inter", Kw::Big(Big::Inter)),
    ("int", Kw::Big(Big::Int)),
    ("dint", Kw::Big(Big::DInt)),
    ("tint", Kw::Big(Big::TInt)),
    ("oint", Kw::Big(Big::OInt)),
    ("odint", Kw::Big(Big::ODInt)),
    ("otint", Kw::Big(Big::OTInt)),
];

/// Looks up a whole word. Greek letters and arrows depend on case;
/// functions match exactly; everything else ignores case.
pub fn lookup(word: &str) -> Option<Kw> {
    if word.is_empty() || !word.is_ascii() {
        return None;
    }
    let lower = word.to_ascii_lowercase();
    let capital = word.as_bytes()[0].is_ascii_uppercase();
    if let Some(&(_, small, cap)) = GREEK.iter().find(|g| g.0 == lower) {
        let all_lower = word.bytes().all(|b| b.is_ascii_lowercase());
        return Some(Kw::Sym(if all_lower { small } else { cap }, Class::Ord));
    }
    if word == "lim" || word == "Lim" {
        return Some(Kw::Lim(if word == "lim" { "lim" } else { "Lim" }));
    }
    if let Some(f) = FUNCTIONS.iter().find(|f| **f == word) {
        return Some(Kw::Func(f));
    }
    if let Some(&(_, single, double)) = ARROWS.iter().find(|a| a.0 == lower) {
        let c = if capital { double } else { single };
        // Arrows carry their own side bearings and take no relation space.
        let class = Class::Ord;
        return Some(Kw::Sym(c, class));
    }
    if let Some(&(_, kw)) = COMMANDS.iter().find(|c| c.0 == lower) {
        return Some(kw);
    }
    if let Some(&(_, c, class)) = SYMBOLS.iter().find(|s| s.0 == lower) {
        return Some(Kw::Sym(c, class));
    }
    match lower.as_str() {
        "smallprod" => Some(Kw::Sym('∏', Class::Bin)),
        _ => None,
    }
}

/// The longest keyword that starts `word`, with its length in bytes.
/// Hancom reads `rmAB` as `rm AB` and `tanx` as `tan x`.
pub fn prefix(word: &str) -> Option<(Kw, usize)> {
    let max = word.len().min(12);
    (2..=max)
        .rev()
        .filter(|&n| word.is_char_boundary(n))
        .find_map(|n| lookup(&word[..n]).map(|k| (k, n)))
}

/// The spacing class of a literal character.
pub fn class_of(c: char) -> Class {
    match c {
        '+' | '-' | '−' | '*' | '×' | '÷' | '±' | '∓' | '∘' | '∙' | '∗' | '⋆' | '≠' | '∼' | '≃'
        | '⊕' | '⊖' | '⊗' | '⊙' | '⊘' | '∧' | '∨' => Class::Bin,
        '=' | '<' | '>' | '≤' | '≥' | '≡' | '≈' | '≅' | '∝' | '∈' | '∋' | '∉' | '⊂' | '⊃' | '⊆'
        | '⊇' | '≪' | '≫' | ':' => Class::Rel,
        '(' | '[' | '{' | '⟨' => Class::Open,
        ')' | ']' | '}' | '⟩' => Class::Close,
        ',' | ';' => Class::Punct,
        _ => Class::Ord,
    }
}

/// Operators written with several characters, longest first.
pub const DIGRAPHS: &[(&str, char)] = &[
    ("<->", '↔'),
    ("<<<", '⋘'),
    (">>>", '⋙'),
    ("->", '→'),
    ("<-", '←'),
    ("<=", '≤'),
    (">=", '≥'),
    ("!=", '≠'),
    ("==", '≡'),
    ("<<", '≪'),
    (">>", '≫'),
    ("+-", '±'),
    ("-+", '∓'),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn greek_and_arrows_follow_case() {
        assert_eq!(lookup("pi"), Some(Kw::Sym('π', Class::Ord)));
        assert_eq!(lookup("PI"), Some(Kw::Sym('Π', Class::Ord)));
        assert_eq!(lookup("rarrow"), Some(Kw::Sym('→', Class::Ord)));
        assert_eq!(lookup("RARROW"), Some(Kw::Sym('⇒', Class::Ord)));
        assert_eq!(lookup("LEQ"), Some(Kw::Sym('≤', Class::Rel)));
        assert_eq!(lookup("OVER"), Some(Kw::Over));
        assert_eq!(lookup("SIN"), None);
    }

    #[test]
    fn keywords_split_glued_words() {
        assert_eq!(prefix("rmAB"), Some((Kw::Font(FontCmd::Roman), 2)));
        assert_eq!(prefix("tanx"), Some((Kw::Func("tan"), 3)));
        assert_eq!(prefix("leqb"), Some((Kw::Sym('≤', Class::Rel), 3)));
        assert_eq!(prefix("xy"), None);
    }
}
