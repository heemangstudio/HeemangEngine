//! Proofreading marks (교정 부호) as Mac Hancom prints them: red strokes
//! (orange ticks for the simple change and insert marks, a gray box behind
//! hyperlink marks) around the marked text of each line. Paths were
//! measured from Mac Hancom's print of each mark on 16 pt text and scale
//! with the text size; points hang from the marked text's start or end or
//! from the column's left or right edge, `y` from the baseline.

use hm_layout::{Kind, Line, ParaLayout, Placed};
use hm_model::{Paragraph, Unit};

use crate::text::Ctx;
use crate::{Dash, DisplayList, Item, Stroke};

#[derive(Clone, Copy)]
enum A {
    Start,
    End,
    ColLeft,
    ColRight,
}

struct Part {
    fill: bool,
    /// Stroke width in HU.
    width: i32,
    color: u32,
    /// `(anchor, x, y)` in HU at 16 pt.
    points: &'static [(A, i32, i32)],
}

type Mark = (&'static str, &'static [Part]);

/// Marks that print; the others (sign, sawtooth, thinking, praise, split)
/// print nothing on Mac.
const MARKS: &[Mark] = &[
    // ATTACH
    (
        "ATTACH",
        &[
            Part {
                fill: false,
                width: 60,
                color: 0xff,
                points: &[
                    (A::Start, 804, 720),
                    (A::Start, 768, 720),
                    (A::Start, 720, 684),
                    (A::Start, 672, 648),
                    (A::Start, 636, 576),
                    (A::Start, 600, 504),
                    (A::Start, 588, 432),
                    (A::Start, 576, 360),
                    (A::Start, 564, 240),
                ],
            },
            Part {
                fill: false,
                width: 60,
                color: 0xff,
                points: &[
                    (A::End, -804, 720),
                    (A::End, -768, 720),
                    (A::End, -720, 684),
                    (A::End, -672, 648),
                    (A::End, -636, 576),
                    (A::End, -600, 504),
                    (A::End, -588, 432),
                    (A::End, -576, 360),
                    (A::End, -564, 240),
                ],
            },
            Part {
                fill: false,
                width: 60,
                color: 0xff,
                points: &[(A::Start, 840, 720), (A::End, -840, 720)],
            },
        ],
    ),
    // CLIPPING
    (
        "CLIPPING",
        &[
            Part {
                fill: false,
                width: 60,
                color: 0xff,
                points: &[(A::Start, 0, -996), (A::Start, 0, -108)],
            },
            Part {
                fill: false,
                width: 60,
                color: 0xff,
                points: &[
                    (A::Start, 0, -1032),
                    (A::Start, 0, -1080),
                    (A::Start, 24, -1164),
                    (A::Start, 144, -1308),
                    (A::Start, 180, -1332),
                    (A::Start, 252, -1344),
                    (A::Start, 312, -1356),
                ],
            },
            Part {
                fill: false,
                width: 60,
                color: 0xff,
                points: &[
                    (A::Start, 0, -72),
                    (A::Start, 0, -24),
                    (A::Start, 12, 24),
                    (A::Start, 36, 84),
                    (A::Start, 156, 204),
                    (A::Start, 204, 216),
                    (A::Start, 252, 228),
                    (A::Start, 312, 240),
                ],
            },
            Part {
                fill: false,
                width: 60,
                color: 0xff,
                points: &[(A::End, 12, -996), (A::End, 12, -108)],
            },
            Part {
                fill: false,
                width: 60,
                color: 0xff,
                points: &[
                    (A::End, 12, -1032),
                    (A::End, 12, -1080),
                    (A::End, -12, -1164),
                    (A::End, -36, -1188),
                    (A::End, -132, -1308),
                    (A::End, -216, -1344),
                    (A::End, -312, -1356),
                ],
            },
            Part {
                fill: false,
                width: 60,
                color: 0xff,
                points: &[
                    (A::End, 12, -72),
                    (A::End, 12, -24),
                    (A::End, 0, 24),
                    (A::End, -36, 84),
                    (A::End, -144, 204),
                    (A::End, -240, 228),
                    (A::End, -312, 240),
                ],
            },
            Part {
                fill: false,
                width: 24,
                color: 0xff,
                points: &[
                    (A::End, 12, -1044),
                    (A::End, 36, -1008),
                    (A::End, 84, -1008),
                    (A::End, 144, -996),
                ],
            },
            Part {
                fill: false,
                width: 24,
                color: 0xff,
                points: &[
                    (A::End, 144, -996),
                    (A::End, 180, -996),
                    (A::End, 216, -1008),
                    (A::End, 264, -1044),
                    (A::End, 300, -1080),
                    (A::End, 336, -1128),
                    (A::End, 336, -1164),
                    (A::End, 360, -1224),
                    (A::End, 360, -1272),
                ],
            },
            Part {
                fill: false,
                width: 24,
                color: 0xff,
                points: &[
                    (A::End, 360, -1272),
                    (A::End, 360, -1320),
                    (A::End, 336, -1356),
                    (A::End, 324, -1356),
                ],
            },
            Part {
                fill: false,
                width: 24,
                color: 0xff,
                points: &[
                    (A::End, 324, -1356),
                    (A::End, 252, -1296),
                    (A::End, 228, -1224),
                ],
            },
            Part {
                fill: false,
                width: 24,
                color: 0xff,
                points: &[
                    (A::End, 228, -1224),
                    (A::End, 228, -1188),
                    (A::End, 240, -1152),
                    (A::End, 276, -1128),
                    (A::End, 324, -1104),
                    (A::End, 360, -1092),
                ],
            },
            Part {
                fill: false,
                width: 24,
                color: 0xff,
                points: &[
                    (A::End, 360, -1092),
                    (A::End, 408, -1092),
                    (A::End, 492, -1128),
                    (A::End, 564, -1188),
                    (A::End, 636, -1284),
                    (A::End, 648, -1344),
                    (A::End, 660, -1404),
                    (A::End, 672, -1488),
                ],
            },
            Part {
                fill: false,
                width: 24,
                color: 0xff,
                points: &[
                    (A::End, 672, -1488),
                    (A::End, 672, -1548),
                    (A::End, 660, -1596),
                    (A::End, 636, -1644),
                    (A::End, 588, -1668),
                ],
            },
            Part {
                fill: false,
                width: 24,
                color: 0xff,
                points: &[
                    (A::End, 588, -1668),
                    (A::End, 552, -1656),
                    (A::End, 516, -1596),
                    (A::End, 504, -1524),
                    (A::End, 492, -1440),
                ],
            },
            Part {
                fill: false,
                width: 24,
                color: 0xff,
                points: &[
                    (A::End, 492, -1440),
                    (A::End, 492, -1392),
                    (A::End, 516, -1320),
                    (A::End, 540, -1284),
                    (A::End, 588, -1248),
                    (A::End, 672, -1224),
                ],
            },
            Part {
                fill: false,
                width: 24,
                color: 0xff,
                points: &[
                    (A::End, 672, -1224),
                    (A::End, 708, -1224),
                    (A::End, 768, -1236),
                    (A::End, 864, -1308),
                    (A::End, 888, -1320),
                    (A::End, 936, -1404),
                    (A::End, 996, -1512),
                    (A::End, 1020, -1584),
                    (A::End, 1020, -1644),
                    (A::End, 1032, -1692),
                    (A::End, 1032, -1752),
                ],
            },
            Part {
                fill: false,
                width: 60,
                color: 0xff,
                points: &[(A::Start, 348, 240), (A::End, -348, 240)],
            },
            Part {
                fill: false,
                width: 60,
                color: 0xff,
                points: &[(A::Start, 348, -1356), (A::End, -348, -1356)],
            },
        ],
    ),
    // DELETE
    (
        "DELETE",
        &[
            Part {
                fill: false,
                width: 60,
                color: 0xff,
                points: &[(A::Start, 36, -720), (A::End, -36, -720)],
            },
            Part {
                fill: false,
                width: 60,
                color: 0xff,
                points: &[(A::Start, 36, -396), (A::End, -36, -396)],
            },
        ],
    ),
    // HYPERLINK
    (
        "HYPERLINK",
        &[Part {
            fill: true,
            width: 0,
            color: 0xd6d6d6,
            points: &[(A::Start, 0, -1356), (A::End, 12, 252)],
        }],
    ),
    // LEFTMOVE
    (
        "LEFTMOVE",
        &[
            Part {
                fill: false,
                width: 60,
                color: 0xff,
                points: &[
                    (A::ColLeft, 4, -1884),
                    (A::ColLeft, 4, -1356),
                    (A::ColLeft, 4, 1200),
                ],
            },
            Part {
                fill: false,
                width: 60,
                color: 0xff,
                points: &[
                    (A::ColLeft, 4, -1356),
                    (A::ColLeft, 4, 1200),
                    (A::ColLeft, 4, 1740),
                ],
            },
            Part {
                fill: false,
                width: 60,
                color: 0xff,
                points: &[(A::ColLeft, 4, -1320), (A::ColLeft, 4, 1164)],
            },
        ],
    ),
    // LINE
    (
        "LINE",
        &[Part {
            fill: false,
            width: 60,
            color: 0xff,
            points: &[(A::Start, 36, 96), (A::End, -36, 96)],
        }],
    ),
    // LINEATTACH
    (
        "LINEATTACH",
        &[
            Part {
                fill: false,
                width: 60,
                color: 0xff,
                points: &[
                    (A::ColLeft, -500, 240),
                    (A::ColLeft, -500, 180),
                    (A::ColLeft, -488, 108),
                    (A::ColLeft, -392, -96),
                    (A::ColLeft, -368, -108),
                    (A::ColLeft, -332, -156),
                    (A::ColLeft, -308, -168),
                    (A::ColLeft, -296, -192),
                    (A::ColLeft, -128, -276),
                    (A::ColLeft, -80, -276),
                    (A::ColLeft, 4, -288),
                ],
            },
            Part {
                fill: false,
                width: 60,
                color: 0xff,
                points: &[(A::ColLeft, -500, 276), (A::ColLeft, -500, 1164)],
            },
            Part {
                fill: false,
                width: 60,
                color: 0xff,
                points: &[
                    (A::ColLeft, -500, -1356),
                    (A::ColLeft, -500, -1296),
                    (A::ColLeft, -488, -1224),
                    (A::ColLeft, -392, -1020),
                    (A::ColLeft, -368, -1008),
                    (A::ColLeft, -332, -960),
                    (A::ColLeft, -308, -948),
                    (A::ColLeft, -296, -924),
                    (A::ColLeft, -128, -840),
                    (A::ColLeft, -80, -840),
                    (A::ColLeft, 4, -828),
                ],
            },
            Part {
                fill: false,
                width: 60,
                color: 0xff,
                points: &[(A::ColLeft, -500, -1356), (A::ColLeft, -500, -1356)],
            },
        ],
    ),
    // LINELINK
    (
        "LINELINK",
        &[
            Part {
                fill: false,
                width: 60,
                color: 0xff,
                points: &[
                    (A::Start, 312, 960),
                    (A::Start, 384, 960),
                    (A::Start, 408, 948),
                    (A::Start, 444, 948),
                    (A::Start, 504, 924),
                    (A::Start, 540, 924),
                    (A::Start, 576, 912),
                    (A::Start, 600, 888),
                    (A::Start, 660, 864),
                    (A::Start, 708, 816),
                    (A::Start, 828, 708),
                    (A::Start, 840, 672),
                    (A::Start, 876, 624),
                    (A::Start, 924, 492),
                    (A::Start, 936, 468),
                    (A::Start, 936, 432),
                    (A::Start, 948, 372),
                    (A::Start, 948, 336),
                ],
            },
            Part {
                fill: false,
                width: 60,
                color: 0xff,
                points: &[
                    (A::Start, 948, 336),
                    (A::Start, 948, 276),
                    (A::Start, 936, 240),
                    (A::Start, 936, 204),
                    (A::Start, 876, 48),
                    (A::Start, 840, 0),
                    (A::Start, 828, -36),
                    (A::Start, 732, -132),
                    (A::Start, 660, -192),
                    (A::Start, 600, -216),
                    (A::Start, 576, -240),
                    (A::Start, 540, -252),
                    (A::Start, 504, -252),
                    (A::Start, 444, -276),
                    (A::Start, 408, -276),
                    (A::Start, 384, -288),
                    (A::Start, 312, -288),
                ],
            },
            Part {
                fill: false,
                width: 60,
                color: 0xff,
                points: &[
                    (A::ColLeft, -260, 960),
                    (A::ColLeft, -308, 960),
                    (A::ColLeft, -368, 984),
                    (A::ColLeft, -416, 1008),
                    (A::ColLeft, -440, 1044),
                    (A::ColLeft, -476, 1092),
                    (A::ColLeft, -488, 1140),
                    (A::ColLeft, -500, 1200),
                ],
            },
            Part {
                fill: false,
                width: 60,
                color: 0xff,
                points: &[(A::Start, 276, 960), (A::ColLeft, -224, 960)],
            },
            Part {
                fill: false,
                width: 60,
                color: 0xff,
                points: &[(A::ColLeft, -500, 1200), (A::ColLeft, -500, 1200)],
            },
            Part {
                fill: false,
                width: 60,
                color: 0xff,
                points: &[(A::Start, 276, -288), (A::Start, 36, -288)],
            },
            Part {
                fill: false,
                width: 60,
                color: 0xff,
                points: &[(A::Start, 24, -300), (A::Start, 396, -480)],
            },
            Part {
                fill: false,
                width: 60,
                color: 0xff,
                points: &[(A::Start, 24, -276), (A::Start, 396, -96)],
            },
            Part {
                fill: false,
                width: 60,
                color: 0xff,
                points: &[
                    (A::ColLeft, -260, -552),
                    (A::ColLeft, -308, -552),
                    (A::ColLeft, -368, -576),
                    (A::ColLeft, -416, -600),
                    (A::ColLeft, -440, -636),
                    (A::ColLeft, -476, -684),
                    (A::ColLeft, -488, -732),
                    (A::ColLeft, -500, -792),
                ],
            },
            Part {
                fill: false,
                width: 60,
                color: 0xff,
                points: &[(A::ColLeft, -224, -552), (A::End, -36, -552)],
            },
            Part {
                fill: false,
                width: 60,
                color: 0xff,
                points: &[(A::ColLeft, -500, -1320), (A::ColLeft, -500, -828)],
            },
        ],
    ),
    // LINETRANSFER
    (
        "LINETRANSFER",
        &[
            Part {
                fill: false,
                width: 60,
                color: 0xff,
                points: &[(A::ColLeft, -464, -1356), (A::ColRight, -40, -1356)],
            },
            Part {
                fill: false,
                width: 60,
                color: 0xff,
                points: &[(A::ColRight, -4, -1320), (A::ColRight, -4, 1164)],
            },
            Part {
                fill: false,
                width: 60,
                color: 0xff,
                points: &[(A::ColLeft, -464, 1200), (A::ColRight, -532, 1200)],
            },
            Part {
                fill: false,
                width: 60,
                color: 0xff,
                points: &[(A::ColLeft, -500, -1320), (A::ColLeft, -500, 1164)],
            },
        ],
    ),
    // RIGHTMOVE
    (
        "RIGHTMOVE",
        &[
            Part {
                fill: false,
                width: 60,
                color: 0xff,
                points: &[
                    (A::ColLeft, 4, -1884),
                    (A::ColLeft, 4, -1356),
                    (A::ColLeft, 4, 1200),
                ],
            },
            Part {
                fill: false,
                width: 60,
                color: 0xff,
                points: &[
                    (A::ColLeft, 4, -1356),
                    (A::ColLeft, 4, 1200),
                    (A::ColLeft, 4, 1740),
                ],
            },
            Part {
                fill: false,
                width: 60,
                color: 0xff,
                points: &[(A::ColLeft, 4, -1320), (A::ColLeft, 4, 1164)],
            },
        ],
    ),
    // SIMPLECHANGE
    (
        "SIMPLECHANGE",
        &[
            Part {
                fill: false,
                width: 60,
                color: 0x4e83f2,
                points: &[(A::Start, -252, 216), (A::Start, -12, -528)],
            },
            Part {
                fill: false,
                width: 60,
                color: 0x4e83f2,
                points: &[(A::End, 12, -528), (A::End, 264, 216)],
            },
            Part {
                fill: false,
                width: 60,
                color: 0xff,
                points: &[(A::Start, 36, -552), (A::End, -36, -552)],
            },
        ],
    ),
    // SIMPLEINSERT
    (
        "SIMPLEINSERT",
        &[
            Part {
                fill: false,
                width: 60,
                color: 0x4e83f2,
                points: &[(A::Start, -264, 216), (A::Start, 0, -1332)],
            },
            Part {
                fill: false,
                width: 60,
                color: 0x4e83f2,
                points: &[(A::End, 0, -1332), (A::End, 264, 216)],
            },
            Part {
                fill: false,
                width: 60,
                color: 0xff,
                points: &[(A::Start, 36, -1356), (A::End, -36, -1356)],
            },
        ],
    ),
    // TRANSFER
    (
        "TRANSFER",
        &[
            Part {
                fill: false,
                width: 60,
                color: 0xff,
                points: &[
                    (A::Start, 0, -456),
                    (A::Start, 0, -372),
                    (A::Start, 12, -336),
                    (A::Start, 12, -288),
                    (A::Start, 48, -168),
                    (A::Start, 72, -132),
                    (A::Start, 108, -48),
                    (A::Start, 204, 84),
                    (A::Start, 240, 108),
                    (A::Start, 264, 144),
                    (A::Start, 444, 264),
                    (A::Start, 480, 276),
                    (A::Start, 516, 300),
                    (A::Start, 636, 336),
                    (A::Start, 684, 336),
                    (A::Start, 720, 348),
                    (A::Start, 804, 348),
                ],
            },
            Part {
                fill: false,
                width: 60,
                color: 0xff,
                points: &[(A::Start, 0, -1320), (A::Start, 0, -492)],
            },
            Part {
                fill: false,
                width: 60,
                color: 0xff,
                points: &[
                    (A::End, 0, -552),
                    (A::End, 0, -636),
                    (A::End, -12, -672),
                    (A::End, -12, -720),
                    (A::End, -48, -840),
                    (A::End, -72, -876),
                    (A::End, -84, -912),
                    (A::End, -204, -1092),
                    (A::End, -288, -1176),
                    (A::End, -432, -1272),
                    (A::End, -468, -1284),
                    (A::End, -504, -1308),
                    (A::End, -624, -1344),
                    (A::End, -672, -1344),
                    (A::End, -708, -1356),
                    (A::End, -792, -1356),
                ],
            },
            Part {
                fill: false,
                width: 60,
                color: 0xff,
                points: &[(A::End, 0, -516), (A::End, 0, 204)],
            },
        ],
    ),
];

/// The size the paths were measured at.
const MEASURED: i32 = 1596;

/// Draws the marks of proofreading fields in line `line` whose column
/// starts at `x`, line top `y`. `behind` draws the fills that go under the
/// text; otherwise the strokes.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw(
    dl: &mut DisplayList,
    ctx: &Ctx<'_>,
    para: &Paragraph,
    pal: &ParaLayout,
    line: &Line,
    x: i32,
    y: i32,
    placed: &[Placed],
    behind: bool,
) {
    for (name, start, end) in ranges(para) {
        let Some((_, parts)) = MARKS.iter().find(|m| m.0 == name) else {
            continue;
        };
        let inside: Vec<&Placed> = placed
            .iter()
            .filter(|p| {
                p.pos >= start && p.pos < end && matches!(p.kind, Kind::Char(_) | Kind::Space)
            })
            .collect();
        let (Some(first), Some(last)) = (inside.first(), inside.last()) else {
            continue;
        };
        let m = &pal.measured;
        let size = ctx.styles[m.styles[first.pos as usize] as usize]
            .size
            .max(1);
        let tail = match last.kind {
            Kind::Char(_) => m.natural[last.pos as usize],
            _ => last.advance,
        };
        let (x0, x1) = (x + first.x, x + last.x + tail);
        let (cl, cr) = (x, x + pal.column_width);
        let base = y + line.baseline;
        let at = |&(a, dx, dy): &(A, i32, i32)| {
            let o = match a {
                A::Start => x0,
                A::End => x1,
                A::ColLeft => cl,
                A::ColRight => cr,
            };
            (o + dx * size / MEASURED, base + dy * size / MEASURED)
        };
        for part in parts.iter().filter(|p| p.fill == behind) {
            if part.fill {
                let (a, b) = (at(&part.points[0]), at(&part.points[1]));
                dl.items.push(Item::Rect {
                    x: a.0,
                    y: a.1,
                    w: b.0 - a.0,
                    h: b.1 - a.1,
                    color: part.color,
                });
                continue;
            }
            let stroke = Stroke {
                width: part.width,
                color: part.color,
                dash: Dash::Solid,
            };
            for w in part.points.windows(2) {
                let (a, b) = (at(&w[0]), at(&w[1]));
                dl.items.push(Item::Line {
                    x1: a.0,
                    y1: a.1,
                    x2: b.0,
                    y2: b.1,
                    stroke,
                });
            }
        }
    }
}

/// `(mark, start, end)` of the proofreading fields in a paragraph; a field
/// left open runs to the paragraph end.
fn ranges(para: &Paragraph) -> Vec<(&'static str, u32, u32)> {
    let mut out = Vec::new();
    // Only fields marked for proofreading open a range.
    if !para
        .controls
        .iter()
        .any(|c| c.as_field().is_some_and(|f| f.proofreading().is_some()))
    {
        return out;
    }
    let mut open: Vec<Option<(&'static str, u32)>> = Vec::new();
    for (pos, u) in para.units() {
        match u {
            Unit::Extended { index, .. } => {
                if let Some(f) = para.controls.get(index as usize).and_then(|c| c.as_field()) {
                    open.push(f.proofreading().map(|n| (n, pos + 8)));
                }
            }
            Unit::Inline(4) => {
                if let Some(Some((n, s))) = open.pop() {
                    out.push((n, s, pos));
                }
            }
            _ => {}
        }
    }
    let end = para.len().saturating_sub(1);
    out.extend(open.into_iter().flatten().map(|(n, s)| (n, s, end)));
    out
}
