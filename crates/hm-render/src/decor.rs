//! Character decorations as Mac Hancom prints them: underline (below or
//! above the text) and strikeout in every line shape it draws, outline
//! glyphs, emboss and engrave, and emphasis marks.
//!
//! Lengths follow the size `s` of the decorated text; `w = s / 25` is the
//! single line's thickness. A solid underline's top is 15 percent of the
//! size below the baseline, an overline is centered 87 percent above it
//! and a strikeout 34.8 percent above it. Patterns start at the left end
//! of each decorated stretch. Shapes 12 and up (double wave, 3D) and the
//! centered underline print nothing.

use hm_layout::{Kind, Line, ParaLayout, Placed};

use crate::text::Ctx;
use crate::{DisplayList, GlyphRun, Item, Stroke};

/// One stroke of a shape: a band `[top, top + h)` of a solid, dashed or
/// dotted line, or a zigzag.
#[derive(Clone, Copy)]
enum Piece {
    /// Top offset and thickness, both in thousandths of `w`.
    Band(i32, i32),
    Wave,
}

/// Dash pattern in thousandths of the size: on, off, on, off, ...
fn pattern(shape: u32) -> Option<&'static [i32]> {
    Some(match shape {
        1 => &[282, 168],
        2 => &[60, 90],
        3 => &[570, 168, 60, 168],
        4 => &[570, 168, 60, 168, 60, 168],
        5 => &[570, 168],
        _ => return None,
    })
}

/// The strokes of a line shape and the band height they fill, in
/// thousandths of `w`. `None` for shapes Mac Hancom does not print.
fn pieces(shape: u32) -> Option<(&'static [Piece], i32)> {
    use Piece::Band;
    Some(match shape {
        0..=6 => (&[Band(0, 1000)], 1000),
        7 => (&[Band(0, 750), Band(2250, 750)], 3000),
        8 => (&[Band(0, 1200), Band(2400, 2550)], 4950),
        9 => (&[Band(0, 2550), Band(3800, 1200)], 5000),
        10 => (&[Band(0, 750), Band(1500, 1950), Band(4200, 750)], 4950),
        11 => (&[Piece::Wave], 2700),
        _ => return None,
    })
}

/// A decorated stretch of one line.
struct Stretch {
    x0: i32,
    x1: i32,
    baseline: i32,
    size: i32,
    shape: u32,
    color: u32,
}

/// Underlines, overlines and strikeouts of a line.
pub(crate) fn draw(
    dl: &mut DisplayList,
    ctx: &Ctx<'_>,
    pal: &ParaLayout,
    line: &Line,
    x: i32,
    y: i32,
    placed: &[Placed],
) {
    let m = &pal.measured;
    let text = |p: &Placed| matches!(p.kind, Kind::Char(_) | Kind::Space);
    // (kind: 0 underline, 1 overline, 2 strike) -> open stretch
    let mut open: [Option<Stretch>; 3] = [None, None, None];
    let flush = |dl: &mut DisplayList, which: usize, s: Stretch| paint(dl, which, &s);
    for (k, p) in placed.iter().enumerate() {
        let mut wanted: [Option<Stretch>; 3] = [None, None, None];
        if text(p) {
            let style = &ctx.styles[m.styles[p.pos as usize] as usize];
            let cs = ctx.doc.info.char_shapes.get(style.char_shape as usize);
            // A decoration runs on to the next character, over
            // justification space; the last character before the line end
            // takes its advance before character spacing.
            let w = match placed.get(k + 1) {
                Some(q) if text(q) => q.x - p.x,
                _ if matches!(p.kind, Kind::Char(_)) => m.natural[p.pos as usize],
                _ => p.advance,
            };
            let baseline = y + pal.run_baseline(line, style.line_size);
            let stretch = |shape: u32, color: u32| Stretch {
                x0: x + p.x,
                x1: x + p.x + w,
                baseline,
                size: style.size,
                shape,
                color,
            };
            let shape = cs.map_or(0, |c| c.underline_shape());
            match style.underline {
                1 => wanted[0] = Some(stretch(shape, style.underline_color)),
                3 => wanted[1] = Some(stretch(shape, style.underline_color)),
                _ => {}
            }
            if style.strike {
                let shape = cs.map_or(0, |c| (c.attr >> 26) & 15);
                let color = cs.and_then(|c| c.strike_color).unwrap_or(style.color);
                wanted[2] = Some(stretch(shape, color));
            }
        }
        for (i, want) in wanted.into_iter().enumerate() {
            match (open[i].as_mut(), want) {
                (Some(o), Some(n))
                    if o.x1 == n.x0
                        && (o.baseline, o.size, o.shape, o.color)
                            == (n.baseline, n.size, n.shape, n.color) =>
                {
                    o.x1 = n.x1;
                }
                (_, n) => {
                    if let Some(o) = open[i].take() {
                        flush(dl, i, o);
                    }
                    open[i] = n;
                }
            }
        }
    }
    for (i, o) in open.into_iter().enumerate() {
        if let Some(o) = o {
            flush(dl, i, o);
        }
    }
}

/// Paints one stretch: `which` 0 underline, 1 overline, 2 strikeout.
fn paint(dl: &mut DisplayList, which: usize, s: &Stretch) {
    let Some((pieces, band)) = pieces(s.shape) else {
        return;
    };
    let w = (s.size / 25).max(1);
    let band = band * w / 1000;
    let top = match which {
        0 => s.baseline + s.size * 15 / 100,
        // An overline's band ends where a single line centered 87 percent
        // up would end; a strikeout's band is centered 34.8 percent up.
        1 => s.baseline - s.size * 87 / 100 + w / 2 - band,
        _ => s.baseline - s.size * 348 / 1000 - band / 2,
    };
    for piece in pieces {
        match *piece {
            Piece::Band(off, h) => {
                let (y, h) = (top + off * w / 1000, (h * w / 1000).max(1));
                if s.shape == 6 {
                    dots(dl, s, y + h / 2);
                } else {
                    dashes(dl, s, y, h);
                }
            }
            Piece::Wave => wave(dl, s, top, band, w * 3 / 4),
        }
    }
}

fn dashes(dl: &mut DisplayList, s: &Stretch, y: i32, h: i32) {
    let Some(p) = pattern(s.shape) else {
        dl.items.push(Item::Rect {
            x: s.x0,
            y,
            w: s.x1 - s.x0,
            h,
            color: s.color,
        });
        return;
    };
    let mut x = s.x0;
    let mut k = 0;
    while x < s.x1 {
        let len = (p[k % p.len()] * s.size / 1000).max(1);
        if k % 2 == 0 {
            dl.items.push(Item::Rect {
                x,
                y,
                w: len.min(s.x1 - x),
                h,
                color: s.color,
            });
        }
        x += len;
        k += 1;
    }
}

/// Round dots 6 percent of the size across, every 15 percent.
fn dots(dl: &mut DisplayList, s: &Stretch, cy: i32) {
    let d = (s.size * 6 / 100).max(1);
    let step = (s.size * 15 / 100).max(1);
    let mut cx = s.x0;
    while cx < s.x1 {
        dl.items.push(Item::Rect {
            x: cx - d / 2,
            y: cy - d / 2,
            w: d,
            h: d,
            color: s.color,
        });
        cx += step;
    }
}

/// A zigzag across the band, one tooth every 24 percent of the size.
fn wave(dl: &mut DisplayList, s: &Stretch, top: i32, band: i32, width: i32) {
    let half = (s.size * 12 / 100).max(1);
    let stroke = Stroke {
        width: width.max(1),
        color: s.color,
        dash: crate::Dash::Solid,
    };
    let (hi, lo) = (top + width / 2, top + band - width / 2);
    let mut x = s.x0;
    let mut up = true;
    while x < s.x1 {
        let x2 = (x + half).min(s.x1);
        let (y1, y2) = if up { (hi, lo) } else { (lo, hi) };
        let y2 = y1 + (y2 - y1) * (x2 - x) / half;
        dl.items.push(Item::Line {
            x1: x,
            y1,
            x2,
            y2,
            stroke,
        });
        x = x2;
        up = !up;
    }
}

/// Pushes a glyph run with its char shape's outline, emboss or engrave.
/// Outlines stroke the glyphs with a 0.12 pt line (twice that for the
/// thick kind) in Hancom's dash patterns; emboss and engrave print the
/// glyphs three times: gray in place, white `d` up-left (emboss) or
/// down-right (engrave), and the text color half way, `d` being 6 percent
/// of the size.
pub(crate) fn push_run(dl: &mut DisplayList, ctx: &Ctx<'_>, si: u32, run: GlyphRun) {
    let style = &ctx.styles[si as usize];
    let attr = ctx
        .doc
        .info
        .char_shapes
        .get(style.char_shape as usize)
        .map_or(0, |c| c.attr);
    let outline = (attr >> 8) & 7;
    if outline != 0 {
        // Dash lengths in units of 12 HU (0.12 pt).
        let dash: &[i32] = match outline {
            2 => &[3, 3],
            4 => &[14, 6],
            5 => &[10, 6, 3, 6],
            6 => &[11, 3, 3, 3, 3, 3],
            _ => &[],
        };
        dl.items.push(Item::Outline {
            width: if outline == 3 { 24 } else { 12 },
            dash: dash.iter().map(|d| d * 12).collect(),
            run,
        });
        return;
    }
    let (emboss, engrave) = (attr & (1 << 13) != 0, attr & (1 << 14) != 0);
    if !emboss && !engrave {
        dl.items.push(Item::Glyphs(run));
        return;
    }
    let d = (run.size * 6 / 100).max(1);
    let sign = if emboss { -1 } else { 1 };
    let shifted = |dx: i32, color: u32| GlyphRun {
        color,
        baseline: run.baseline + dx,
        glyphs: run.glyphs.iter().map(|&(g, x)| (g, x + dx)).collect(),
        ..run.clone()
    };
    let gray = shifted(0, 0x0099_9999);
    let white = shifted(sign * d, 0x00FF_FFFF);
    let (first, second) = if emboss { (gray, white) } else { (white, gray) };
    dl.items.push(Item::Glyphs(first));
    dl.items.push(Item::Glyphs(second));
    let top = shifted(sign * d / 2, run.color);
    dl.items.push(Item::Glyphs(top));
}

/// The combining mark each emphasis code prints over (or under) every
/// character but blanks. Code 5 prints nothing on Mac.
fn mark(code: u32) -> Option<char> {
    Some(match code {
        1 => '\u{0307}',
        2 => '\u{030A}',
        3 => '\u{0303}',
        4 => '\u{030C}',
        6 => '\u{302F}',
        7 => '\u{0300}',
        8 => '\u{0301}',
        9 => '\u{0302}',
        10 => '\u{0304}',
        11 => '\u{0309}',
        12 => '\u{0323}',
        _ => return None,
    })
}

/// Emphasis marks (char shape bits 21-24): the mark's glyph from the
/// character's face, 38 percent of the size above where the font puts it
/// (marks below and the side dots stay), its ink centered 41 percent into
/// a wide character or a tenth of the size before the end of a narrow one,
/// where Mac Hancom's mark attachment leaves it; the side dots end where
/// the character starts.
pub(crate) fn emphasis(
    dl: &mut DisplayList,
    ctx: &Ctx<'_>,
    pal: &ParaLayout,
    line: &Line,
    x: i32,
    y: i32,
    placed: &[Placed],
) {
    let m = &pal.measured;
    for p in placed {
        let Kind::Char(c) = p.kind else { continue };
        let style = &ctx.styles[m.styles[p.pos as usize] as usize];
        let Some(cs) = ctx.doc.info.char_shapes.get(style.char_shape as usize) else {
            continue;
        };
        let Some(ch) = mark((cs.attr >> 21) & 15) else {
            continue;
        };
        let Some(face_id) = style.face else { continue };
        let Some(face) = ctx.fonts.face(face_id) else {
            continue;
        };
        let Some(glyph) = face.glyph(ch) else {
            continue;
        };
        let outline = face.outline(glyph.glyph);
        let em = if outline.units_per_em > 0 {
            outline.units_per_em
        } else {
            face.units_per_em
        } as f32;
        let (lo, hi) = ink_x(&outline.ops);
        let center =
            ((lo + hi) / 2.0 * style.size as f32 * style.ratio as f32 / (em * 100.0)) as i32;
        let adv = m.natural[p.pos as usize];
        let wide = matches!(
            hm_layout::script::script_of(c),
            Some(hm_model::Script::Hangul | hm_model::Script::Hanja | hm_model::Script::Japanese)
        );
        // The side dots (a Hangul tone mark) end where the character starts.
        if ch == '\u{302F}' {
            let right = (hi * style.size as f32 * style.ratio as f32 / (em * 100.0)) as i32;
            dl.items.push(Item::Glyphs(GlyphRun {
                face: face_id,
                size: style.size,
                ratio: style.ratio,
                color: style.color,
                fake_bold: false,
                italic: false,
                baseline: y + pal.run_baseline(line, style.line_size) + style.offset,
                glyphs: vec![(glyph.glyph, x + p.x - right)],
            }));
            continue;
        }
        let target = if wide {
            adv * 41 / 100
        } else {
            adv - style.size / 10
        };
        let rise = if ch == '\u{0323}' {
            0
        } else {
            style.size * 38 / 100
        };
        dl.items.push(Item::Glyphs(GlyphRun {
            face: face_id,
            size: style.size,
            ratio: style.ratio,
            color: style.color,
            fake_bold: false,
            italic: false,
            baseline: y + pal.run_baseline(line, style.line_size) + style.offset - rise,
            glyphs: vec![(glyph.glyph, x + p.x + target - center)],
        }));
    }
}

/// Horizontal extent of an outline's points.
fn ink_x(ops: &[hm_fonts::PathOp]) -> (f32, f32) {
    use hm_fonts::PathOp::*;
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    let mut see = |x: f32| {
        lo = lo.min(x);
        hi = hi.max(x);
    };
    for op in ops {
        match *op {
            Move(x, _) | Line(x, _) => see(x),
            Quad(x1, _, x, _) => {
                see(x1);
                see(x);
            }
            Cubic(x1, _, x2, _, x, _) => {
                see(x1);
                see(x2);
                see(x);
            }
            Close => {}
        }
    }
    if lo > hi { (0.0, 0.0) } else { (lo, hi) }
}
