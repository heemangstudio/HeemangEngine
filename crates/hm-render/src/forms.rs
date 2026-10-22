//! Form controls as Hancom for Mac prints them: push buttons, edit and combo
//! boxes in three-dimensional frames shaded from their face color, check
//! boxes and radio buttons as a gray outlined mark beside the caption.

use hm_layout::Rect;
use hm_layout::forms::FormText;
use hm_model::{FormKind, FormObject};

use crate::draw::{Outline, Path, PathItem};
use crate::paint::{Brush, Dash};
use crate::text::Ctx;
use crate::{DisplayList, GlyphRun, Item};

/// One frame ring's width.
const RING: i32 = 96;
/// The inner ring starts this far inside the outer one (the rings overlap).
const INNER: i32 = 72;
/// The face inside both rings.
const FACE: i32 = 156;
/// A combo box's drop button width.
const DROP: i32 = 1224;
const DARK: u32 = 0x404040;
const MARK_LINE: u32 = 0x808080;
/// The box outline of check boxes and radio buttons (one pixel at 96 dpi).
const PIXEL: i32 = 75;

pub(crate) fn paint(
    dl: &mut DisplayList,
    ctx: &Ctx<'_>,
    form: &FormObject,
    rect: Rect,
    text: Option<&FormText>,
) {
    if !form.printable() {
        return;
    }
    let bg = form.back_color();
    match form.form_kind() {
        FormKind::CheckBox | FormKind::RadioButton => mark(dl, ctx, form, rect, text),
        kind => {
            // The frame takes about a device pixel more than the box.
            let f = Rect {
                x: rect.x - 8,
                y: rect.y - 8,
                w: rect.w + 24,
                h: rect.h + 20,
            };
            fill(dl, f, bg);
            frame(dl, f, form.border_type(), bg);
            let Some(t) = text else { return };
            let baseline = f.y + f.h / 2 + t.rise / 2;
            match kind {
                FormKind::PushButton => {
                    glyphs(dl, ctx, t, f.x + (f.w - t.width) / 2, baseline, f.x + f.w);
                }
                FormKind::ComboBox | FormKind::ListBox => {
                    let inner = inset(f, FACE);
                    let drop = Rect {
                        x: inner.x + inner.w - DROP,
                        y: inner.y,
                        w: DROP,
                        h: inner.h,
                    };
                    fill(dl, drop, bg);
                    edges(dl, drop, raised(bg));
                    arrow(dl, drop, form.fore_color());
                    glyphs(dl, ctx, t, f.x + 240, baseline, drop.x);
                }
                _ => {
                    let x = match form.prop("EditSet", "AlignText").and_then(|v| v.as_int()) {
                        Some(1) => f.x + (f.w - t.width) / 2,
                        Some(2) => f.x + f.w - 168 - t.width,
                        _ => f.x + 168,
                    };
                    glyphs(dl, ctx, t, x, baseline, f.x + f.w - FACE);
                }
            }
        }
    }
}

fn inset(r: Rect, d: i32) -> Rect {
    Rect {
        x: r.x + d,
        y: r.y + d,
        w: (r.w - 2 * d).max(0),
        h: (r.h - 2 * d).max(0),
    }
}

fn fill(dl: &mut DisplayList, r: Rect, color: u32) {
    dl.items.push(Item::Rect {
        x: r.x,
        y: r.y,
        w: r.w,
        h: r.h,
        color,
    });
}

/// Ring colors: outer top-left, outer bottom-right, inner top-left, inner
/// bottom-right.
type Rings = [u32; 4];

fn raised(bg: u32) -> Rings {
    [luma(bg, 667), DARK, luma(bg, 400), luma(bg, -333)]
}

fn frame(dl: &mut DisplayList, f: Rect, border: i64, bg: u32) {
    let (light, soft, shadow) = (luma(bg, 667), luma(bg, 400), luma(bg, -333));
    let rings = match border {
        2 => [light, DARK, DARK, soft],
        3 => [shadow, light, soft, shadow],
        4 => raised(bg),
        5 => [shadow, light, DARK, soft],
        _ => return,
    };
    edges(dl, f, rings);
}

/// Two rings; the bottom-right strips go over the top-left ones at the
/// corners they share.
fn edges(dl: &mut DisplayList, f: Rect, rings: Rings) {
    for (r, tl, br) in [
        (f, rings[0], rings[1]),
        (inset(f, INNER), rings[2], rings[3]),
    ] {
        let (right, bottom) = (r.x + r.w, r.y + r.h);
        fill(dl, Rect { h: RING, ..r }, tl);
        fill(dl, Rect { w: RING, ..r }, tl);
        fill(
            dl,
            Rect {
                x: right - RING,
                w: RING,
                ..r
            },
            br,
        );
        fill(
            dl,
            Rect {
                y: bottom - RING,
                h: RING,
                ..r
            },
            br,
        );
    }
}

/// A combo box's down arrow, centered in its drop button.
fn arrow(dl: &mut DisplayList, b: Rect, color: u32) {
    let w = b.h * 267 / 1000;
    let top = b.y + b.h * 358 / 1000;
    let cx = b.x + b.w / 2;
    let mut path = Path::default();
    path.move_to((cx - w / 2, top));
    path.line_to((cx + w / 2, top));
    path.line_to((cx, top + w / 2));
    path.close();
    dl.items.push(Item::Path(Box::new(PathItem {
        path,
        fill: vec![Brush::Solid(color)],
        outline: None,
        fill_alpha: 0,
    })));
}

/// A check box or radio button: the face, a box or circle as tall as the
/// caption's type, its mark, and the caption beside it.
fn mark(dl: &mut DisplayList, ctx: &Ctx<'_>, form: &FormObject, r: Rect, text: Option<&FormText>) {
    if form.opaque() {
        fill(dl, r, form.back_color());
    }
    let size = text.map_or(1000, |t| t.size);
    let side = size * 106 / 100;
    let cy = r.y + r.h / 2;
    let snap = |v: i32| v.div_euclid(PIXEL) * PIXEL;
    let b = Rect {
        x: snap(r.x),
        y: snap(cy - side / 2),
        w: side,
        h: side,
    };
    let radio = form.form_kind() == FormKind::RadioButton;
    let mut outline = Path::default();
    if radio {
        ellipse(&mut outline, b);
    } else {
        outline.move_to((b.x, b.y));
        outline.line_to((b.x + b.w, b.y));
        outline.line_to((b.x + b.w, b.y + b.h));
        outline.line_to((b.x, b.y + b.h));
        outline.close();
    }
    dl.items.push(Item::Path(Box::new(PathItem {
        path: outline,
        fill: vec![Brush::Solid(0xFFFFFF)],
        outline: Some(Outline {
            width: PIXEL,
            color: MARK_LINE,
            dash: Dash::Solid,
            round_cap: false,
        }),
        fill_alpha: 0,
    })));
    let value = form.value();
    let ink = if value == 2 {
        MARK_LINE
    } else {
        form.fore_color()
    };
    if value != 0 {
        if radio {
            let dot = inset(b, b.w * 227 / 1000);
            let mut path = Path::default();
            ellipse(&mut path, dot);
            dl.items.push(Item::Path(Box::new(PathItem {
                path,
                fill: vec![Brush::Solid(ink)],
                outline: None,
                fill_alpha: 0,
            })));
        } else {
            let mut path = Path::default();
            path.move_to((b.x + 150, b.y + b.h * 413 / 1000));
            path.line_to((b.x + b.w * 413 / 1000, b.y + b.h * 672 / 1000));
            path.line_to((b.x + b.w * 847 / 1000, b.y + b.h * 155 / 1000));
            dl.items.push(Item::Path(Box::new(PathItem {
                path,
                fill: Vec::new(),
                outline: Some(Outline {
                    width: b.w * 173 / 1000,
                    color: ink,
                    dash: Dash::Solid,
                    round_cap: false,
                }),
                fill_alpha: 0,
            })));
        }
    }
    if let Some(t) = text {
        glyphs(dl, ctx, t, b.x + b.w + 210, cy + t.rise / 2, r.x + r.w);
    }
}

fn ellipse(path: &mut Path, r: Rect) {
    let (rx, ry) = (r.w as f64 / 2.0, r.h as f64 / 2.0);
    let (cx, cy) = (r.x as f64 + rx, r.y as f64 + ry);
    let k = 0.552_284_75;
    let p = |x: f64, y: f64| (x.round() as i32, y.round() as i32);
    path.move_to(p(cx + rx, cy));
    path.cubic_to(
        p(cx + rx, cy + k * ry),
        p(cx + k * rx, cy + ry),
        p(cx, cy + ry),
    );
    path.cubic_to(
        p(cx - k * rx, cy + ry),
        p(cx - rx, cy + k * ry),
        p(cx - rx, cy),
    );
    path.cubic_to(
        p(cx - rx, cy - k * ry),
        p(cx - k * rx, cy - ry),
        p(cx, cy - ry),
    );
    path.cubic_to(
        p(cx + k * rx, cy - ry),
        p(cx + rx, cy - k * ry),
        p(cx + rx, cy),
    );
    path.close();
}

/// The text from `x` on one baseline, cut where it would pass `limit`.
fn glyphs(dl: &mut DisplayList, ctx: &Ctx<'_>, t: &FormText, x: i32, baseline: i32, limit: i32) {
    let mut run: Option<(u32, GlyphRun)> = None;
    for &(ch, si, gx) in &t.glyphs {
        if x + gx >= limit {
            break;
        }
        let style = &ctx.styles[si as usize];
        let Some(face) = style.face else { continue };
        let Some(glyph) = ctx.fonts.face(face).and_then(|f| f.glyph(ch)) else {
            continue;
        };
        if run.as_ref().is_none_or(|(s, _)| *s != si) {
            if let Some((_, r)) = run.take() {
                dl.items.push(Item::Glyphs(r));
            }
            run = Some((
                si,
                GlyphRun {
                    face,
                    size: style.size,
                    ratio: style.ratio,
                    color: style.color,
                    fake_bold: style.fake_bold,
                    italic: style.italic,
                    baseline: baseline + style.offset,
                    glyphs: Vec::new(),
                },
            ));
        }
        if let Some((_, r)) = run.as_mut() {
            r.glyphs.push((glyph.glyph, x + gx));
        }
    }
    if let Some((_, r)) = run {
        dl.items.push(Item::Glyphs(r));
    }
}

/// Moves a COLORREF's lightness toward white (`per_mille` > 0) or black,
/// keeping hue and saturation, as Hancom shades control frames.
fn luma(color: u32, per_mille: i32) -> u32 {
    let ch = |s: u32| ((color >> s) & 0xFF) as f64 / 255.0;
    let (r, g, b) = (ch(0), ch(8), ch(16));
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.0;
    let d = max - min;
    let s = if d == 0.0 {
        0.0
    } else {
        d / (1.0 - (2.0 * l - 1.0).abs())
    };
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    let k = per_mille as f64 / 1000.0;
    let l = if k >= 0.0 {
        l + (1.0 - l) * k
    } else {
        l * (1.0 + k)
    };
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - (h.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match h as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let q = |v: f64| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u32;
    q(r) | q(g) << 8 | q(b) << 16
}

#[cfg(test)]
mod tests {
    use super::luma;

    #[test]
    fn frame_shades_follow_the_face() {
        // Faces and the shades Hancom printed around them.
        let near =
            |c: u32, rgb: [u32; 3]| (0..3).all(|i| (c >> (8 * i) & 0xFF).abs_diff(rgb[i]) <= 4);
        assert!(near(luma(0xF0F0F0, -333), [159, 159, 159]));
        assert!(near(luma(0xF0F0F0, 667), [250, 250, 250]));
        assert!(near(luma(0xFFFFFF, -333), [170, 170, 170]));
        assert!(near(luma(0xD8E9EC, -333), [185, 174, 113]));
    }
}
