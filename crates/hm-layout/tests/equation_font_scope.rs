#[allow(dead_code)]
mod common;

use hm_layout::math::{self, MathItem, Node};
use hm_model::{Equation, HwpString};

const NATIVE_VECTOR_EQUATION: &str = "{vec{rm CP it}} cdot {vec{rm DQ it}} = LEFT ( rm {vec{CA}} + {vec{rmAP}} RIGHT ) cdot{vec{DQ}} =rm {vec{AP}} BULLET rm {vec{DQ}} =4 BULLET 3=12";

fn letters(node: &Node, out: &mut Vec<(String, bool)>) {
    match node {
        Node::Atom(atom) if atom.text.chars().all(|c| c.is_ascii_alphabetic()) => {
            out.push((atom.text.clone(), atom.style.roman));
        }
        Node::Row(nodes) | Node::Group(nodes) => {
            for node in nodes {
                letters(node, out);
            }
        }
        Node::Delim { body, .. } | Node::Deco { body, .. } | Node::Root { body, .. } => {
            letters(body, out);
        }
        Node::Aligned { body, .. } => letters(body, out),
        Node::Stack { rows, .. } => {
            for row in rows {
                for node in row {
                    letters(node, out);
                }
            }
        }
        Node::Frac { num, den, .. } => {
            letters(num, out);
            letters(den, out);
        }
        Node::Scripts { base, sub, sup } => {
            letters(base, out);
            for script in sub.iter().chain(sup.iter()) {
                letters(script, out);
            }
        }
        _ => {}
    }
}

#[test]
fn delimiter_switches_continue_in_the_row_while_braces_and_operands_keep_their_scope() {
    for (script, expected) in [
        ("LEFT ( rm A RIGHT ) B", vec![("A", true), ("B", true)]),
        ("rm LEFT ( it A RIGHT ) B", vec![("A", false), ("B", false)]),
        ("LEFT ( {rm A} RIGHT ) B", vec![("A", true), ("B", false)]),
        ("LEFT ( rm A RIGHT ) it B", vec![("A", true), ("B", false)]),
        ("{LEFT ( rm A RIGHT )} B", vec![("A", true), ("B", false)]),
        (
            "sqrt LEFT ( rm A RIGHT ) B",
            vec![("A", true), ("B", false)],
        ),
        (
            "LEFT ( rm A RIGHT ) ^B C",
            vec![("A", true), ("B", true), ("C", true)],
        ),
        ("LEFT ( rm A RIGHT ) over B", vec![("A", true), ("B", true)]),
        (
            "{LEFT ( rm A RIGHT )} over B",
            vec![("A", true), ("B", false)],
        ),
        (
            "A over LEFT ( rm B RIGHT ) C",
            vec![("A", false), ("B", true), ("C", false)],
        ),
        (
            "A ^ LEFT ( rm B RIGHT ) C",
            vec![("A", false), ("B", true), ("C", false)],
        ),
        (
            "matrix{LEFT ( rm A RIGHT ) B & C # D & E} F",
            vec![
                ("A", true),
                ("B", true),
                ("C", true),
                ("D", false),
                ("E", false),
                ("F", false),
            ],
        ),
        (
            "matrix{lcol{LEFT ( rm A RIGHT ) B#C} rcol{D#E}} F",
            vec![
                ("A", true),
                ("B", true),
                ("D", false),
                ("C", false),
                ("E", false),
                ("F", false),
            ],
        ),
        (
            "LEFT ( rm A LEFT ( it B RIGHT ) C RIGHT ) D",
            vec![("A", true), ("B", false), ("C", false), ("D", false)],
        ),
    ] {
        let mut actual = Vec::new();
        letters(&math::parse(script), &mut actual);
        let expected: Vec<_> = expected
            .into_iter()
            .map(|(text, roman)| (text.to_string(), roman))
            .collect();
        assert_eq!(actual, expected, "{script}");
    }
    let mut actual = Vec::new();
    letters(&math::parse(NATIVE_VECTOR_EQUATION), &mut actual);
    assert_eq!(
        actual,
        ["CP", "DQ", "CA", "AP", "DQ", "AP", "DQ"].map(|s| (s.to_string(), true))
    );
}

#[test]
fn native_page20_vector_letters_use_upright_equation_glyphs_after_right() {
    let Some((_, fonts)) = common::setup() else {
        return;
    };
    let layout = math::layout(
        &Equation {
            script: HwpString::from_text(NATIVE_VECTOR_EQUATION),
            size: 900,
            font: Some(HwpString::from_text("HYhwpEQ")),
            ..Default::default()
        },
        &fonts,
    );
    let chars: Vec<_> = layout
        .items
        .iter()
        .filter_map(|item| match item {
            MathItem::Glyph { ch, .. } => Some(*ch),
            _ => None,
        })
        .collect();
    // Official page 20 uses literal D/Q glyphs for all three vectors.
    assert_eq!(chars.iter().filter(|&&ch| ch == 'D').count(), 3);
    assert_eq!(chars.iter().filter(|&&ch| ch == 'Q').count(), 3);
    assert!(!chars.contains(&math::pua('D', true).unwrap()));
    assert!(!chars.contains(&math::pua('Q', true).unwrap()));
}
