//! Equation script tokens.
//!
//! Blanks only separate tokens. Letter runs split into keywords and plain
//! letters (`rmAB` is `rm` then `AB`); digit runs keep their decimal point.

use super::table::{self, DIGRAPHS, Kw};

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    /// Letters that are not a keyword.
    Ident(String),
    /// Digits, possibly with a decimal point.
    Number(String),
    Kw(Kw),
    /// Any other character, digraphs already folded (`->` is `→`).
    Char(char),
    /// Text between double quotes.
    Quoted(String),
    Open,
    Close,
    /// `^`
    Caret,
    /// `_`
    Under,
    /// `~`: a full space.
    Tilde,
    /// `` ` ``: a quarter space.
    Tick,
    /// `#`: a new row.
    Hash,
    /// `&`: a column mark.
    Amp,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub tok: Tok,
    /// A blank came before the token.
    pub spaced: bool,
}

fn is_letter(c: char) -> bool {
    c.is_ascii_alphabetic()
}

pub fn tokenize(script: &str) -> Vec<Token> {
    let chars: Vec<char> = script.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let mut spaced = false;
    let push = |out: &mut Vec<Token>, tok: Tok, spaced: &mut bool| {
        out.push(Token {
            tok,
            spaced: *spaced,
        });
        *spaced = false;
    };
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            spaced = true;
            i += 1;
            continue;
        }
        if is_letter(c) {
            let start = i;
            while i < chars.len() && is_letter(chars[i]) {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            // Keywords are recognized at the start of the run and right after
            // another keyword; the first plain letter makes the rest plain.
            let mut rest = word.as_str();
            while let Some((kw, n)) = table::prefix(rest) {
                push(&mut out, Tok::Kw(kw), &mut spaced);
                rest = &rest[n..];
            }
            if !rest.is_empty() {
                push(&mut out, Tok::Ident(rest.to_string()), &mut spaced);
            }
            continue;
        }
        if c.is_ascii_digit() || (c == '.' && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit()))
        {
            let start = i;
            while i < chars.len()
                && (chars[i].is_ascii_digit()
                    || (chars[i] == '.' && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit())))
            {
                i += 1;
            }
            push(
                &mut out,
                Tok::Number(chars[start..i].iter().collect()),
                &mut spaced,
            );
            continue;
        }
        let tok = match c {
            '{' => Tok::Open,
            '}' => Tok::Close,
            '^' => Tok::Caret,
            '_' => Tok::Under,
            '~' => Tok::Tilde,
            '`' => Tok::Tick,
            '#' => Tok::Hash,
            '&' => Tok::Amp,
            '"' => {
                let start = i + 1;
                let mut j = start;
                while j < chars.len() && chars[j] != '"' {
                    j += 1;
                }
                let text: String = chars[start..j].iter().collect();
                i = (j + 1).min(chars.len());
                push(&mut out, Tok::Quoted(text), &mut spaced);
                continue;
            }
            _ => {
                let rest: String = chars[i..chars.len().min(i + 3)].iter().collect();
                if let Some(&(d, folded)) = DIGRAPHS.iter().find(|(d, _)| rest.starts_with(d)) {
                    i += d.chars().count();
                    push(&mut out, Tok::Char(folded), &mut spaced);
                    continue;
                }
                Tok::Char(c)
            }
        };
        i += 1;
        push(&mut out, tok, &mut spaced);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::table::{Class, FontCmd};

    fn toks(s: &str) -> Vec<Tok> {
        tokenize(s).into_iter().map(|t| t.tok).collect()
    }

    #[test]
    fn glued_keywords_split() {
        assert_eq!(
            toks("rmAB"),
            vec![Tok::Kw(Kw::Font(FontCmd::Roman)), Tok::Ident("AB".into())]
        );
        assert_eq!(
            toks("1 le x"),
            vec![
                Tok::Number("1".into()),
                Tok::Kw(Kw::Sym('≤', Class::Rel)),
                Tok::Ident("x".into())
            ]
        );
        assert_eq!(toks("ax"), vec![Tok::Ident("ax".into())]);
    }

    #[test]
    fn digraphs_and_numbers() {
        assert_eq!(
            toks("x->0.5"),
            vec![
                Tok::Ident("x".into()),
                Tok::Char('→'),
                Tok::Number("0.5".into())
            ]
        );
        let t = tokenize("a ^{2}");
        assert!(t[1].spaced && !t[2].spaced);
    }
}
