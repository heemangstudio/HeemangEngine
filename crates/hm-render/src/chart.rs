//! Charts drawn from their model as Hancom's chart engine lays them out:
//! frame, title, legend, axes with ticks, gridlines and labels, and the
//! plots (columns and bars, lines, areas, pies and scatter).
//!
//! Hancom draws every chart text at one size whatever its part asks, with
//! an automatic title in bold at a larger size; the margins and gaps below
//! are measured from its output at that size.

use hm_fonts::{FaceQuery, FontSystem};
use hm_layout::Rect;
use hm_model::chart::{
    Axis, Chart, DataLabels, Grouping, Plot, PlotKind, Series, TextStyle, Ticks,
};

use crate::draw::{Outline, Path, PathItem};
use crate::paint::{Brush, Dash, Stroke};
use crate::{DisplayList, GlyphRun, Item};

/// Series colors when a chart names none: Hancom's chart theme.
const ACCENTS: [u32; 6] = [0xD68261, 0x3A84FF, 0xB2B2B2, 0x00D7FF, 0x6E9B28, 0x47AD70];

/// The size Hancom draws chart text at, and its automatic title.
const TEXT: i32 = 845;
const TITLE: i32 = 1230;
/// Frame, axis and gridline width.
const RULE: i32 = 55;
/// Line series width when the series names none.
const SERIES_LINE: i32 = 225;
/// Marker size.
const MARKER: i32 = 700;
/// Tick mark length.
const TICK: i32 = 330;
/// Outer margin: from the frame to the outermost text.
const MARGIN: i32 = 885;

/// Chart text in a sans face, measured and drawn as glyph runs.
struct Text<'a> {
    fonts: &'a FontSystem,
}

impl Text<'_> {
    fn face(&self, bold: bool) -> Option<hm_fonts::FaceId> {
        // Hancom for Mac draws chart text in the system's Korean sans; the
        // sizes here are measured against 맑은 고딕, the closest face the
        // document fonts offer.
        self.fonts
            .resolve(&FaceQuery {
                name: "맑은 고딕",
                alternates: vec!["Malgun Gothic", "Apple SD Gothic Neo"],
                bold,
                italic: false,
            })
            .map(|r| r.face)
    }

    fn width(&self, s: &str, st: &TextStyle) -> i32 {
        let Some(face) = self.face(st.bold).and_then(|id| self.fonts.face(id)) else {
            return s.chars().count() as i32 * st.size / 2;
        };
        s.chars()
            .map(|c| {
                face.glyph(c).map_or(st.size / 2, |g| {
                    (g.advance as i64 * st.size as i64 / g.em.max(1) as i64) as i32
                })
            })
            .sum()
    }

    /// Draws `s` with its left edge at `x` and its baseline at `y`.
    fn draw(&self, dl: &mut DisplayList, s: &str, st: &TextStyle, x: i32, y: i32) {
        let Some(id) = self.face(st.bold) else { return };
        let Some(face) = self.fonts.face(id) else {
            return;
        };
        let mut glyphs = Vec::new();
        let mut pen = x;
        for c in s.chars() {
            if let Some(g) = face.glyph(c) {
                glyphs.push((g.glyph, pen));
                pen += (g.advance as i64 * st.size as i64 / g.em.max(1) as i64) as i32;
            } else {
                pen += st.size / 2;
            }
        }
        dl.items.push(Item::Glyphs(GlyphRun {
            face: id,
            size: st.size,
            ratio: 100,
            color: st.color,
            fake_bold: false,
            italic: false,
            baseline: y,
            glyphs,
        }));
    }

    /// Draws `s` centered on `cx` with its baseline at `y`.
    fn centered(&self, dl: &mut DisplayList, s: &str, st: &TextStyle, cx: i32, y: i32) {
        let w = self.width(s, st);
        self.draw(dl, s, st, cx - w / 2, y);
    }

    /// Draws `s` with its right edge at `x`.
    fn right(&self, dl: &mut DisplayList, s: &str, st: &TextStyle, x: i32, y: i32) {
        let w = self.width(s, st);
        self.draw(dl, s, st, x - w, y);
    }
}

/// The size chart text of `size` HU draws at: Hancom sets it in whole
/// pixels at 96 dpi, 13 for 10 points.
fn text_size(size: i32) -> i32 {
    let px = ((size as f64) * 96.0 / 7200.0).round().max(1.0);
    (TEXT as f64 * px / 13.0).round() as i32
}

/// Offset from a line's middle to the baseline of text centered on it.
fn mid_to_base(size: i32) -> i32 {
    size * 39 / 100
}

fn rect_path(x: i32, y: i32, w: i32, h: i32) -> Path {
    let mut p = Path::default();
    p.move_to((x, y));
    p.line_to((x + w, y));
    p.line_to((x + w, y + h));
    p.line_to((x, y + h));
    p.close();
    p
}

fn fill(dl: &mut DisplayList, path: Path, color: u32, outline: Option<(u32, i32)>) {
    dl.items.push(Item::Path(Box::new(PathItem {
        path,
        fill: vec![Brush::Solid(color)],
        outline: outline.map(|(c, width)| Outline {
            width,
            color: c,
            dash: Dash::Solid,
            round_cap: false,
        }),
        fill_alpha: 0,
    })));
}

fn stroke(dl: &mut DisplayList, path: Path, color: u32, width: i32) {
    dl.items.push(Item::Path(Box::new(PathItem {
        path,
        fill: Vec::new(),
        outline: Some(Outline {
            width,
            color,
            dash: Dash::Solid,
            round_cap: true,
        }),
        fill_alpha: 0,
    })));
}

fn line(dl: &mut DisplayList, (x1, y1): (i32, i32), (x2, y2): (i32, i32), color: u32, width: i32) {
    dl.items.push(Item::Line {
        x1,
        y1,
        x2,
        y2,
        stroke: Stroke {
            width,
            color,
            dash: Dash::Solid,
        },
    });
}

/// A value as Excel's General format shows it.
fn general(v: f64) -> String {
    if v == v.trunc() && v.abs() < 1e15 {
        return format!("{}", v as i64);
    }
    let s = format!("{v:.9}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn tick_label(v: f64, percent: bool) -> String {
    if percent {
        format!("{}%", (v * 100.0).round() as i64)
    } else {
        general(v)
    }
}

fn series_color(s: &Series, k: usize) -> u32 {
    s.color.unwrap_or(ACCENTS[k % ACCENTS.len()])
}

fn stacked(kind: PlotKind) -> Option<Grouping> {
    match kind {
        PlotKind::Bar { grouping, .. }
        | PlotKind::Line { grouping, .. }
        | PlotKind::Area { grouping } => Some(grouping),
        _ => None,
    }
}

/// The lowest and highest values the plots put on the value axis.
fn data_range(plots: &[Plot], x: bool) -> (f64, f64) {
    let (mut lo, mut hi) = (f64::MAX, f64::MIN);
    for p in plots {
        if !x && stacked(p.kind) == Some(Grouping::Percent) {
            lo = lo.min(0.0);
            hi = hi.max(1.0);
            continue;
        }
        let sum = !x && stacked(p.kind) == Some(Grouping::Stacked);
        let n = p.series.iter().map(|s| s.values.len()).max().unwrap_or(0);
        for i in 0..n {
            let mut acc = 0.0;
            for s in &p.series {
                let vals = if x { &s.x } else { &s.values };
                let Some(v) = vals.get(i).copied().flatten() else {
                    continue;
                };
                if sum {
                    acc += v;
                    lo = lo.min(acc);
                    hi = hi.max(acc);
                } else {
                    lo = lo.min(v);
                    hi = hi.max(v);
                }
            }
        }
    }
    if lo > hi { (0.0, 1.0) } else { (lo, hi) }
}

/// The axis scale Hancom picks: a step of 1, 2 or 5 times a power of ten
/// giving at most one division per 2,100 HU of axis, up to the highest
/// value plus a twentieth of the range.
fn scale((lo, hi): (f64, f64), axis: Option<&Axis>, length: i32, percent: bool) -> (f64, f64, f64) {
    let min = axis
        .and_then(|a| a.min)
        .unwrap_or(if lo > 0.0 { 0.0 } else { lo });
    let fixed_max = axis.and_then(|a| a.max);
    let top = if percent {
        1.0
    } else {
        fixed_max.unwrap_or(hi + (hi - min).max(0.0) / 20.0)
    };
    let top = if top <= min { min + 1.0 } else { top };
    let divisions = (length / 2100).max(1) as f64;
    let step = axis.and_then(|a| a.major_unit).unwrap_or_else(|| {
        let mut p = 10f64.powf(((top - min) / divisions).log10().floor() - 1.0);
        loop {
            for m in [1.0, 2.0, 5.0] {
                let s = p * m;
                if ((top - min) / s - 1e-9).ceil() <= divisions {
                    return s;
                }
            }
            p *= 10.0;
        }
    });
    let max = fixed_max.unwrap_or_else(|| {
        if percent {
            1.0
        } else {
            ((top - min) / step - 1e-9).ceil() * step + min
        }
    });
    (min, max.max(min), step)
}

fn ticks(min: f64, max: f64, step: f64) -> Vec<f64> {
    let mut v = Vec::new();
    let mut k = 0;
    loop {
        let t = min + step * k as f64;
        if t > max + step * 1e-6 || v.len() > 200 {
            break;
        }
        v.push(t);
        k += 1;
    }
    v
}

/// Draws a chart filling `r`.
pub fn draw(dl: &mut DisplayList, fonts: &FontSystem, c: &Chart, r: Rect) {
    let text = Text { fonts };
    if c.fill.is_some() || c.border.is_some() {
        dl.items.push(Item::Path(Box::new(PathItem {
            path: rect_path(r.x + RULE / 2, r.y + RULE / 2, r.w - RULE, r.h - RULE),
            fill: c.fill.map(Brush::Solid).into_iter().collect(),
            outline: c.border.map(|b| Outline {
                width: RULE,
                color: b,
                dash: Dash::Solid,
                round_cap: false,
            }),
            fill_alpha: 0,
        })));
    }
    let base = TextStyle {
        size: TEXT,
        bold: false,
        color: c.text.color,
    };
    let style = |st: &TextStyle| TextStyle {
        size: text_size(st.size),
        bold: st.bold,
        color: st.color,
    };
    let (x0, mut x1) = (r.x + MARGIN - 45, r.x + r.w - MARGIN);
    let (mut y0, mut y1) = (r.y + MARGIN, r.y + r.h - MARGIN);
    // The title's baseline sits at a fixed height for its size.
    let mut title_base = None;
    if let Some(t) = c.title.as_ref().filter(|t| !t.text.is_empty()) {
        let st = TextStyle {
            size: if t.style.bold {
                TITLE
            } else {
                text_size(t.style.size)
            },
            ..t.style
        };
        let b = r.y + 1099 + st.size * 1023 / 1000;
        for (k, line) in t.text.lines().enumerate() {
            text.centered(
                dl,
                line,
                &st,
                r.x + r.w / 2,
                b + k as i32 * st.size * 12 / 10,
            );
        }
        title_base = Some(b);
        y0 = b;
    }
    // Legend entries: every series, or every category of a pie.
    let pie = c
        .plots
        .first()
        .is_some_and(|p| matches!(p.kind, PlotKind::Pie { .. }));
    let entries: Vec<(String, u32, usize)> = if pie {
        let s = c.plots.first().and_then(|p| p.series.first());
        s.map(|s| {
            (0..s.values.len())
                .map(|i| {
                    let color = s
                        .point_colors
                        .iter()
                        .find(|(k, _)| *k == i)
                        .map_or(ACCENTS[i % ACCENTS.len()], |p| p.1);
                    (s.categories.get(i).cloned().unwrap_or_default(), color, i)
                })
                .collect()
        })
        .unwrap_or_default()
    } else {
        c.plots
            .iter()
            .flat_map(|p| p.series.iter().map(move |s| (p, s)))
            .enumerate()
            .map(|(k, (_, s))| (s.name.clone(), series_color(s, k), k))
            .collect()
    };
    // Line series show a line sample with their marker as the key.
    let line_key: Vec<Option<(bool, usize)>> = c
        .plots
        .iter()
        .flat_map(|p| {
            p.series.iter().map(move |s| match p.kind {
                PlotKind::Line { markers, .. } => {
                    Some((s.markers.unwrap_or(markers), usize::from(s.line)))
                }
                PlotKind::Scatter { markers, lines, .. } => {
                    (lines && s.line).then_some((s.markers.unwrap_or(markers), 1))
                }
                _ => None,
            })
        })
        .collect();
    // Hancom lists stacked series top first, as they stack.
    let reversed = c.plots.iter().any(|p| {
        matches!(
            p.kind,
            PlotKind::Bar {
                horizontal: true,
                ..
            }
        ) || matches!(stacked(p.kind), Some(Grouping::Stacked | Grouping::Percent))
    }) && !pie;
    let mut order: Vec<usize> = (0..entries.len()).collect();
    if reversed {
        order.reverse();
    }
    let mut legend_gap = 1089;
    if let Some(l) = c.legend.filter(|_| !entries.is_empty()) {
        let st = style(&l.style);
        // Keys follow the legend's own size.
        let key = 575 * l.style.size.clamp(600, 2000) / 1000;
        let row = 1353;
        let widest = entries
            .iter()
            .map(|e| text.width(&e.0, &st))
            .max()
            .unwrap_or(0);
        // From a key's left edge to its text: a square key, or a line
        // sample with its marker.
        let key_w = |k: usize| {
            if line_key
                .get(k)
                .copied()
                .flatten()
                .is_some_and(|(_, d)| d > 0)
            {
                1157
            } else {
                key + 348
            }
        };
        let draw_key = |dl: &mut DisplayList, k: usize, x: i32, cy: i32, color: u32| {
            if let Some((markers, drawn)) = line_key.get(k).copied().flatten() {
                if drawn == 0 {
                    // No line: the marker alone, where a square key sits.
                    if markers {
                        marker(dl, k, (x + key / 2, cy), color);
                    }
                    return;
                }
                let mut p = Path::default();
                p.move_to((x, cy));
                p.line_to((x + 926, cy));
                stroke(dl, p, color, SERIES_LINE);
                if markers {
                    marker(dl, k, (x + 463, cy), color);
                }
            } else {
                fill(dl, rect_path(x, cy - key / 2, key, key), color, None);
            }
        };
        match l.position {
            b'b' | b't' => {
                let spacing = 287;
                let item = |k: usize, s: &str| key_w(k) + text.width(s, &st);
                let total: i32 = entries.iter().map(|e| item(e.2, &e.0)).sum::<i32>()
                    + spacing * (entries.len() as i32 - 1);
                let mut x = r.x + (r.w - total) / 2;
                let base_y = if l.position == b'b' {
                    r.y + r.h - 900
                } else {
                    y0 + 1200
                };
                let cy = base_y - mid_to_base(st.size);
                for &k in &order {
                    let (s, color, n) = &entries[k];
                    draw_key(dl, *n, x, cy, *color);
                    text.draw(dl, s, &st, x + key_w(*n), base_y);
                    x += item(*n, s) + spacing;
                }
                if l.position == b'b' {
                    y1 = base_y - 2469;
                } else {
                    title_base = Some(base_y - 2124 + 600);
                }
                x1 = r.x + r.w - 654;
                legend_gap = 0;
            }
            _ => {
                let wk = (0..entries.len()).map(key_w).max().unwrap_or(923);
                let text_x = if l.position == b'l' {
                    x0 + wk
                } else {
                    // Hancom centers the legend on a line 2,458 HU in from the right.
                    r.x + r.w - 2458 - (wk + widest) / 2 + wk
                };
                let top = title_base.map_or(r.y, |b| b + 230);
                let mid = (top + r.y + r.h) / 2;
                let mut cy = mid - row * (entries.len() as i32 - 1) / 2;
                for &k in &order {
                    let (s, color, n) = &entries[k];
                    draw_key(dl, *n, text_x - key_w(*n), cy, *color);
                    text.draw(dl, s, &st, text_x, cy + mid_to_base(st.size) - 30);
                    cy += row;
                }
                if l.position != b'l' {
                    x1 = text_x - 923;
                } else {
                    legend_gap = 0;
                }
            }
        }
    } else {
        x1 += MARGIN;
        legend_gap = 0;
    }
    if pie {
        if let Some(p) = c.plots.first() {
            let top = title_base.map_or(r.y + MARGIN, |b| b + 1274);
            let right = if legend_gap > 0 { x1 - 1328 } else { x1 };
            draw_pie(
                dl,
                &text,
                p,
                (x0, top, right, r.y + r.h - 651),
                base,
                c.three_d,
            );
        }
        return;
    }
    let horizontal = c.plots.iter().any(|p| {
        matches!(
            p.kind,
            PlotKind::Bar {
                horizontal: true,
                ..
            }
        )
    });
    let scatter = c
        .plots
        .iter()
        .any(|p| matches!(p.kind, PlotKind::Scatter { .. }));
    let vaxis = c.value_axis.unwrap_or_default();
    let caxis = c.category_axis.unwrap_or_default();
    let vst = style(&vaxis.labels);
    let cst = style(&caxis.labels);
    let categories: Vec<String> = c
        .plots
        .iter()
        .flat_map(|p| p.series.iter())
        .map(|s| s.categories.clone())
        .find(|c| !c.is_empty())
        .unwrap_or_default();
    let ncat = c
        .plots
        .iter()
        .flat_map(|p| p.series.iter())
        .map(|s| s.values.len())
        .max()
        .unwrap_or(0)
        .max(1);
    // The bottom label row, then the plot above it.
    let bottom_labels = if horizontal {
        !vaxis.deleted
    } else {
        !caxis.deleted
    };
    let label_base = y1 - 222;
    let py1 = if bottom_labels {
        label_base - if horizontal || scatter { 1988 } else { 2084 }
    } else {
        y1
    };
    let py0 = match title_base {
        Some(b) => b + if horizontal { 1274 } else { 2124 },
        None => r.y + MARGIN + if horizontal { 0 } else { 850 },
    };
    let py0 = py0.min(py1);
    let length_v = py1 - py0;
    let percent = vaxis.percent;
    let mut px1 = x1 - legend_gap;
    // The left edge: category names left of horizontal bars, else the
    // value labels of the vertical value axis.
    let vertical_scale = (!horizontal).then(|| {
        scale(
            data_range(&c.plots, false),
            c.value_axis.as_ref(),
            length_v,
            percent,
        )
    });
    let px0 = if horizontal {
        let cw = categories
            .iter()
            .map(|s| text.width(s, &cst))
            .max()
            .unwrap_or(0);
        x0 + if caxis.deleted { 0 } else { cw + 1161 }
    } else {
        let (a, b, st) = vertical_scale.unwrap_or((0.0, 1.0, 1.0));
        let vw = ticks(a, b, st)
            .iter()
            .map(|t| text.width(&tick_label(*t, percent), &vst))
            .max()
            .unwrap_or(0);
        x0 + if vaxis.deleted { 0 } else { vw + 1054 }
    };
    // A value axis along the bottom keeps most of its widest label's width
    // clear past the plot's right edge.
    let across = |px1: i32| {
        let (range, axis, pct, st) = if horizontal {
            (
                data_range(&c.plots, false),
                c.value_axis.as_ref(),
                percent,
                &vst,
            )
        } else {
            (
                data_range(&c.plots, true),
                c.category_axis.as_ref(),
                false,
                &cst,
            )
        };
        let sc = scale(range, axis, px1 - px0, pct);
        let w = ticks(sc.0, sc.1, sc.2)
            .iter()
            .map(|t| text.width(&tick_label(*t, pct), st))
            .max()
            .unwrap_or(0);
        (sc, w * 85 / 100 + 60)
    };
    let mut xscale = (0.0, 1.0, 1.0);
    if horizontal || scatter {
        let (_, reserve) = across(px1);
        px1 -= reserve;
        xscale = across(px1).0;
    }
    let (vmin, vmax, vstep) = if horizontal {
        xscale
    } else {
        vertical_scale.unwrap_or((0.0, 1.0, 1.0))
    };
    let vticks = ticks(vmin, vmax, vstep);
    let (xmin, xmax, xstep) = xscale;
    let xticks = if scatter {
        ticks(xmin, xmax, xstep)
    } else {
        Vec::new()
    };
    if px1 <= px0 {
        return;
    }
    if c.three_d
        && c.plots
            .iter()
            .all(|p| matches!(p.kind, PlotKind::Bar { .. }))
    {
        let frame = Frame3d {
            plot: (px0, py0, px1, py1),
            horizontal,
            label_base,
        };
        draw_bars_3d(dl, &text, c, &frame, (&vst, &cst), &categories, ncat);
        return;
    }
    // Position of a value along the value axis.
    let to = |v: f64| -> i32 {
        let f = if vmax > vmin {
            ((v - vmin) / (vmax - vmin)).clamp(-0.05, 1.05)
        } else {
            0.0
        };
        if horizontal {
            px0 + (f * (px1 - px0) as f64).round() as i32
        } else {
            py1 - (f * (py1 - py0) as f64).round() as i32
        }
    };
    let to_x = |v: f64| -> i32 {
        let f = ((v - xmin) / (xmax - xmin)).clamp(-0.05, 1.05);
        px0 + (f * (px1 - px0) as f64).round() as i32
    };
    // Gridlines and value ticks.
    for t in &vticks {
        let p = to(*t);
        let tick = if vaxis.deleted || vaxis.ticks == Ticks::None {
            0
        } else {
            TICK
        };
        if let Some(g) = vaxis.gridlines {
            if horizontal {
                line(dl, (p, py0), (p, py1 + tick), g, RULE);
            } else {
                line(dl, (px0 - tick, p), (px1, p), g, RULE);
            }
        } else if tick > 0 && vaxis.line.is_some() {
            let g = vaxis.line.unwrap_or(0);
            if horizontal {
                line(dl, (p, py1), (p, py1 + tick), g, RULE);
            } else {
                line(dl, (px0 - tick, p), (px0, p), g, RULE);
            }
        }
        if !vaxis.deleted {
            let s = tick_label(*t, percent);
            if horizontal {
                text.centered(dl, &s, &vst, p, label_base);
            } else {
                text.right(dl, &s, &vst, px0 - 1054, p + mid_to_base(vst.size));
            }
        }
    }
    if scatter {
        for t in &xticks {
            let p = to_x(*t);
            if let Some(g) = caxis.gridlines {
                line(dl, (p, py0), (p, py1), g, RULE);
            }
            if !caxis.deleted {
                if caxis.ticks != Ticks::None {
                    line(
                        dl,
                        (p, py1),
                        (p, py1 + TICK),
                        caxis.line.unwrap_or(0x86_8686),
                        RULE,
                    );
                }
                text.centered(dl, &general(*t), &cst, p, label_base);
            }
        }
    }
    let span = if horizontal { py1 - py0 } else { px1 - px0 };
    let slot = span as f64 / ncat as f64;
    // Categories run up the value axis of horizontal bars.
    let cat_at = |i: usize| -> i32 {
        let o = (slot * (i as f64 + 0.5)).round() as i32;
        if horizontal { py1 - o } else { px0 + o }
    };
    if !scatter && !caxis.deleted {
        for (i, s) in categories.iter().enumerate().take(ncat) {
            if horizontal {
                text.right(dl, s, &cst, px0 - 1161, cat_at(i) + mid_to_base(cst.size));
            } else {
                text.centered(dl, s, &cst, cat_at(i), label_base);
            }
        }
        if caxis.ticks != Ticks::None {
            let g = caxis.line.unwrap_or(0x86_8686);
            for i in 0..=ncat {
                let o = (slot * i as f64).round() as i32;
                if horizontal {
                    line(dl, (px0 - TICK, py1 - o), (px0, py1 - o), g, RULE);
                } else {
                    line(dl, (px0 + o, py1), (px0 + o, py1 + TICK), g, RULE);
                }
            }
        }
    }
    // Axis lines: the value axis along the plot's edge, the category axis
    // through the value axis's zero.
    if let Some(g) = vaxis.line.filter(|_| !vaxis.deleted) {
        if horizontal {
            line(dl, (px0, py1), (px1, py1), g, RULE);
        } else {
            line(dl, (px0, py0), (px0, py1), g, RULE);
        }
    }
    if let Some(g) = caxis.line.filter(|_| !caxis.deleted) {
        let zero = to(0f64.clamp(vmin, vmax));
        if horizontal {
            line(dl, (zero, py0), (zero, py1), g, RULE);
        } else {
            line(dl, (px0, zero), (px1, zero), g, RULE);
        }
    }
    let mut k0 = 0;
    for p in &c.plots {
        draw_plot(
            dl,
            &text,
            p,
            k0,
            (slot, &cat_at, &to, &to_x, horizontal),
            (vmin, vmax),
            base,
        );
        k0 += p.series.len();
    }
}

/// The 2D plot box a 3D bar chart is drawn in.
struct Frame3d {
    plot: (i32, i32, i32, i32),
    horizontal: bool,
    label_base: i32,
}

/// Bars as boxes seen from above and to the right: a back wall and floor
/// with their gridlines, and each bar's front, top and side faces.
fn draw_bars_3d(
    dl: &mut DisplayList,
    text: &Text<'_>,
    c: &Chart,
    f: &Frame3d,
    (vst, cst): (&TextStyle, &TextStyle),
    categories: &[String],
    ncat: usize,
) {
    let (px0, py0, px1, py1) = f.plot;
    let h = f.horizontal;
    let vaxis = c.value_axis.unwrap_or_default();
    let caxis = c.category_axis.unwrap_or_default();
    let rule = vaxis.line.or(caxis.line).unwrap_or(0x86_8686);
    // Front plane corners, and the depth shift to the back plane.
    let (fx0, fy0, bx1, fy1) = if h {
        (px0 + 834, py0 + 1018, px1 - 1180, py1 - 566)
    } else {
        (px0 + 1625, py0 + 93, px1 - 1363, py1 - 233)
    };
    // The depth runs a fixed share of the category axis.
    let dx = if h {
        ((fy1 - fy0) as f64 * 0.0611).round() as i32
    } else {
        ((bx1 - fx0) as f64 * 0.0643).round() as i32
    };
    let dy = (dx as f64 * 0.724).round() as i32;
    let fx1 = bx1 - dx;
    let fy0 = fy0 + dy;
    let len = if h { fx1 - fx0 } else { fy1 - fy0 };
    let percent = vaxis.percent;
    let (lo, hi) = data_range(&c.plots, false);
    let depth = if h { dx } else { dy };
    let (vmin, vmax, vstep) = scale3d((lo, hi), c.value_axis.as_ref(), len + depth, percent);
    // A value at depth `d` (0 front, 1 back) along the value axis.
    let val = |v: f64, d: f64| -> (i32, i32) {
        let t = if vmax > vmin {
            ((v - vmin) / (vmax - vmin)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let off = (dx as f64 * d, -(dy as f64) * d);
        if h {
            (
                (fx0 as f64 + t * len as f64 + off.0).round() as i32,
                (fy1 as f64 + off.1).round() as i32,
            )
        } else {
            (
                (fx0 as f64 + off.0).round() as i32,
                (fy1 as f64 - t * len as f64 + off.1).round() as i32,
            )
        }
    };
    let span = if h { fy1 - fy0 } else { fx1 - fx0 } as f64;
    let slot = span / ncat.max(1) as f64;
    let shift = |(x, y): (i32, i32), d: f64| {
        (
            x + (dx as f64 * d).round() as i32,
            y - (dy as f64 * d).round() as i32,
        )
    };
    // Back wall gridlines and their run down the side wall (or floor).
    for t in ticks(vmin, vmax, vstep) {
        let front = val(t, 0.0);
        let back = val(t, 1.0);
        let far = if h {
            (back.0, back.1 - (fy1 - fy0))
        } else {
            (back.0 + (fx1 - fx0), back.1)
        };
        if let Some(g) = vaxis.gridlines {
            line(dl, front, back, g, RULE);
            line(dl, back, far, g, RULE);
        }
        if !vaxis.deleted {
            let s = tick_label(t, percent);
            if h {
                line(dl, front, (front.0, front.1 + TICK), rule, RULE);
                text.centered(dl, &s, vst, front.0, f.label_base);
            } else {
                line(dl, (front.0 - TICK, front.1), front, rule, RULE);
                text.right(dl, &s, vst, front.0 - 1344, front.1 + mid_to_base(vst.size));
            }
        }
    }
    // Walls' edges: the value axis at the front, the back wall's near edge
    // and the floor.
    let corner = |a: f64, b: f64, d: f64| {
        // `a` along categories (0..1), `b` along values (0..1).
        let (x, y) = if h {
            (fx0 as f64 + b * len as f64, fy1 as f64 - a * span)
        } else {
            (fx0 as f64 + a * span, fy1 as f64 - b * len as f64)
        };
        shift((x.round() as i32, y.round() as i32), d)
    };
    line(dl, corner(0.0, 0.0, 0.0), corner(0.0, 1.0, 0.0), rule, RULE);
    line(dl, corner(0.0, 0.0, 1.0), corner(0.0, 1.0, 1.0), rule, RULE);
    line(dl, corner(0.0, 0.0, 0.0), corner(1.0, 0.0, 0.0), rule, RULE);
    line(dl, corner(1.0, 0.0, 0.0), corner(1.0, 0.0, 1.0), rule, RULE);
    line(dl, corner(0.0, 0.0, 0.0), corner(0.0, 0.0, 1.0), rule, RULE);
    // Category ticks and names along the front edge.
    if !caxis.deleted {
        for i in 0..=ncat {
            let p = corner(i as f64 / ncat as f64, 0.0, 0.0);
            if h {
                line(dl, (p.0 - TICK, p.1), p, rule, RULE);
            } else {
                line(dl, p, (p.0, p.1 + TICK), rule, RULE);
            }
        }
        for (i, s) in categories.iter().enumerate().take(ncat) {
            let p = corner((i as f64 + 0.5) / ncat as f64, 0.0, 0.0);
            if h {
                text.right(dl, s, cst, p.0 - 1336, p.1 + mid_to_base(cst.size));
            } else {
                text.centered(dl, s, cst, p.0, f.label_base);
            }
        }
    }
    // The bars, back to front and left to right.
    let mut k0 = 0;
    for p in &c.plots {
        let PlotKind::Bar { grouping, gap, .. } = p.kind else {
            continue;
        };
        let stacked = grouping != Grouping::Clustered;
        let n = p.series.len().max(1);
        let lanes = if stacked { 1.0 } else { n as f64 };
        let bar = slot / (lanes + gap as f64 / 100.0);
        let (d0, d1) = (0.3, 0.7);
        let totals: Vec<f64> = (0..ncat)
            .map(|i| {
                p.series
                    .iter()
                    .map(|s| s.values.get(i).copied().flatten().unwrap_or(0.0).abs())
                    .sum()
            })
            .collect();
        let mut boxes = Vec::new();
        let mut acc = vec![0.0; ncat];
        for (k, s) in p.series.iter().enumerate() {
            let color = series_color(s, k0 + k);
            for (i, v) in s.values.iter().enumerate().take(ncat) {
                let Some(mut v) = *v else { continue };
                if grouping == Grouping::Percent && totals[i] > 0.0 {
                    v /= totals[i];
                }
                let (from, to) = if stacked {
                    let a = acc[i];
                    acc[i] += v;
                    (a, acc[i])
                } else {
                    (vmin.max(0.0), v)
                };
                let lane = if stacked { 0.0 } else { k as f64 };
                let a0 = (slot * i as f64 + bar * gap as f64 / 200.0 + lane * bar) / span;
                let a1 = a0 + bar / span;
                let b = |v: f64| ((v - vmin) / (vmax - vmin)).clamp(0.0, 1.0);
                boxes.push((a0, a1, b(from), b(to), color));
            }
        }
        for (a0, a1, b0, b1, color) in boxes {
            let quad = |p: [(i32, i32); 4]| {
                let mut path = Path::default();
                path.move_to(p[0]);
                for q in &p[1..] {
                    path.line_to(*q);
                }
                path.close();
                path
            };
            // Front face, then the face toward the light (top of a column,
            // end of a bar) and the side away from it.
            fill(
                dl,
                quad([
                    corner(a0, b0, d0),
                    corner(a1, b0, d0),
                    corner(a1, b1, d0),
                    corner(a0, b1, d0),
                ]),
                color,
                None,
            );
            // The face at the value end and the face at the far category edge.
            let end = [
                corner(a0, b1, d0),
                corner(a1, b1, d0),
                corner(a1, b1, d1),
                corner(a0, b1, d1),
            ];
            let side = [
                corner(a1, b0, d0),
                corner(a1, b1, d0),
                corner(a1, b1, d1),
                corner(a1, b0, d1),
            ];
            let (top_face, right_face) = if h { (side, end) } else { (end, side) };
            fill(dl, quad(top_face), shade(color, 0.72), None);
            fill(dl, quad(right_face), shade(color, 0.49), None);
        }
        k0 += p.series.len();
    }
}

/// The value scale of a 3D chart, at one division per 2,100 HU of the
/// axis and its depth.
fn scale3d(
    (lo, hi): (f64, f64),
    axis: Option<&Axis>,
    length: i32,
    percent: bool,
) -> (f64, f64, f64) {
    let min = axis
        .and_then(|a| a.min)
        .unwrap_or(if lo > 0.0 { 0.0 } else { lo });
    let top = if percent {
        1.0
    } else {
        axis.and_then(|a| a.max).unwrap_or(hi)
    };
    let top = if top <= min { min + 1.0 } else { top };
    let divisions = (length / 2100).max(1) as f64;
    let step = axis.and_then(|a| a.major_unit).unwrap_or_else(|| {
        let mut p = 10f64.powf(((top - min) / divisions).log10().floor() - 1.0);
        loop {
            for m in [1.0, 2.0, 5.0] {
                let s = p * m;
                if ((top - min) / s - 1e-9).ceil() <= divisions {
                    return s;
                }
            }
            p *= 10.0;
        }
    });
    let max = ((top - min) / step - 1e-9).ceil() * step + min;
    // Native positive, automatic axes keep four intervals even when rounding
    // the data alone would give three. Explicit bounds use a different scale.
    let automatic =
        axis.is_none_or(|a| a.min.is_none() && a.max.is_none() && a.major_unit.is_none());
    let intervals = if automatic && !percent && lo > 0.0 && divisions >= 4.0 {
        4.0
    } else {
        1.0
    };
    (min, max.max(min + step * intervals), step)
}

type Geometry<'a> = (
    f64,
    &'a dyn Fn(usize) -> i32,
    &'a dyn Fn(f64) -> i32,
    &'a dyn Fn(f64) -> i32,
    bool,
);

fn label_text(l: &DataLabels, s: &Series, i: usize, v: f64) -> String {
    let mut parts = Vec::new();
    if l.series {
        parts.push(s.name.clone());
    }
    if l.category {
        parts.push(s.categories.get(i).cloned().unwrap_or_default());
    }
    if l.value {
        parts.push(general(v));
    }
    parts.join(", ")
}

/// Hancom's automatic markers by series: diamond, square, triangle, cross,
/// star, circle.
fn marker(dl: &mut DisplayList, k: usize, (x, y): (i32, i32), color: u32) {
    let h = MARKER / 2;
    let mut p = Path::default();
    match k % 6 {
        0 => {
            p.move_to((x, y - h));
            p.line_to((x + h, y));
            p.line_to((x, y + h));
            p.line_to((x - h, y));
            p.close();
        }
        1 => {
            let s = h * 85 / 100;
            p = rect_path(x - s, y - s, 2 * s, 2 * s);
        }
        2 => {
            p.move_to((x, y - h));
            p.line_to((x + h, y + h * 8 / 10));
            p.line_to((x - h, y + h * 8 / 10));
            p.close();
        }
        3 => {
            let c = h * 6 / 10;
            let mut q = Path::default();
            q.move_to((x - c, y - c));
            q.line_to((x + c, y + c));
            q.move_to((x - c, y + c));
            q.line_to((x + c, y - c));
            stroke(dl, q, color, 30);
            return;
        }
        _ => {
            let steps = 16;
            for i in 0..steps {
                let a = i as f64 / steps as f64 * std::f64::consts::TAU;
                let q = (
                    x + (h as f64 * a.cos()) as i32,
                    y + (h as f64 * a.sin()) as i32,
                );
                if i == 0 {
                    p.move_to(q);
                } else {
                    p.line_to(q);
                }
            }
            p.close();
        }
    }
    fill(dl, p, color, None);
}

/// A polyline through `pts`, or a smooth curve through them.
fn series_path(pts: &[(i32, i32)], smooth: bool) -> Path {
    let mut path = Path::default();
    let Some(&first) = pts.first() else {
        return path;
    };
    path.move_to(first);
    if !smooth || pts.len() < 3 {
        for q in &pts[1..] {
            path.line_to(*q);
        }
        return path;
    }
    // Catmull-Rom through the points as cubic Béziers.
    let f = |a: i32, b: i32| (b - a) as f64 / 6.0;
    for i in 0..pts.len() - 1 {
        let p0 = pts[i.saturating_sub(1)];
        let (p1, p2) = (pts[i], pts[i + 1]);
        let p3 = pts[(i + 2).min(pts.len() - 1)];
        let c1 = (p1.0 + f(p0.0, p2.0) as i32, p1.1 + f(p0.1, p2.1) as i32);
        let c2 = (p2.0 - f(p1.0, p3.0) as i32, p2.1 - f(p1.1, p3.1) as i32);
        path.cubic_to(c1, c2, p2);
    }
    path
}

fn draw_plot(
    dl: &mut DisplayList,
    text: &Text<'_>,
    p: &Plot,
    k0: usize,
    (slot, cat_at, to, to_x, horizontal): Geometry<'_>,
    (min, _max): (f64, f64),
    base_style: TextStyle,
) {
    let base = if min > 0.0 { min } else { 0.0 };
    let n = p.series.len().max(1);
    let ncat = p.series.iter().map(|s| s.values.len()).max().unwrap_or(0);
    let totals: Vec<f64> = (0..ncat)
        .map(|i| {
            p.series
                .iter()
                .map(|s| s.values.get(i).copied().flatten().unwrap_or(0.0).abs())
                .sum()
        })
        .collect();
    let lstyle = |l: &DataLabels| TextStyle {
        size: text_size(l.style.size),
        bold: l.style.bold,
        color: if l.style.color == 0 {
            base_style.color
        } else {
            l.style.color
        },
    };
    let mut labels = Vec::new();
    match p.kind {
        PlotKind::Bar {
            grouping,
            gap,
            overlap,
            ..
        } => {
            let stacked = grouping != Grouping::Clustered;
            let lanes = if stacked { 1.0 } else { n as f64 };
            let ov = if stacked { 1.0 } else { overlap as f64 / 100.0 };
            let bar = slot / (lanes - (lanes - 1.0) * ov + gap as f64 / 100.0);
            let group = bar * (lanes - (lanes - 1.0) * ov);
            let mut acc = vec![0.0; ncat];
            for (k, s) in p.series.iter().enumerate() {
                let color = series_color(s, k0 + k);
                for (i, v) in s.values.iter().enumerate() {
                    let Some(mut v) = *v else { continue };
                    if grouping == Grouping::Percent && totals[i] > 0.0 {
                        v /= totals[i];
                    }
                    let (from, to_v) = if stacked {
                        let a = acc[i];
                        acc[i] += v;
                        (a.max(base), acc[i])
                    } else {
                        (base, v)
                    };
                    let lane = if stacked {
                        0.0
                    } else {
                        k as f64 * bar * (1.0 - ov)
                    };
                    let (a, b) = (to(from), to(to_v));
                    let path = if horizontal {
                        // Bars stack upward from the first series.
                        let start = cat_at(i) + (group / 2.0) as i32 - lane as i32 - bar as i32;
                        rect_path(a.min(b), start, (b - a).abs(), bar.round() as i32)
                    } else {
                        let start = cat_at(i) - (group / 2.0) as i32 + lane as i32;
                        rect_path(start, a.min(b), bar.round() as i32, (b - a).abs())
                    };
                    fill(dl, path, color, None);
                    let l = s.labels.unwrap_or(p.labels);
                    if l.value || l.category || l.series {
                        let mid = if horizontal {
                            cat_at(i) + (group / 2.0) as i32 - lane as i32 - (bar / 2.0) as i32
                        } else {
                            cat_at(i) - (group / 2.0) as i32 + lane as i32 + (bar / 2.0) as i32
                        };
                        let st = lstyle(&l);
                        let at = if horizontal {
                            (b + st.size, mid + mid_to_base(st.size))
                        } else {
                            (mid, b - 801)
                        };
                        labels.push((label_text(&l, s, i, s.values[i].unwrap_or(0.0)), st, at));
                    }
                }
            }
        }
        PlotKind::Line { grouping, .. } | PlotKind::Area { grouping } => {
            let at = |i: usize, v: f64| {
                if horizontal {
                    (to(v), cat_at(i))
                } else {
                    (cat_at(i), to(v))
                }
            };
            let point = |s: &Series, i: usize| s.values.get(i).copied().flatten();
            if p.hi_low {
                for i in 0..ncat {
                    let vs: Vec<f64> = p.series.iter().filter_map(|s| point(s, i)).collect();
                    let lo = vs.iter().copied().fold(f64::MAX, f64::min);
                    let hi = vs.iter().copied().fold(f64::MIN, f64::max);
                    if lo <= hi {
                        line(dl, at(i, lo), at(i, hi), 0, 60);
                    }
                }
            }
            if let (Some(gap), Some(first), Some(last)) =
                (p.up_down, p.series.first(), p.series.last())
            {
                let w = slot / (1.0 + gap as f64 / 100.0);
                for i in 0..ncat {
                    let (Some(open), Some(close)) = (point(first, i), point(last, i)) else {
                        continue;
                    };
                    let (a, b) = (to(open), to(close));
                    let x = cat_at(i) - (w / 2.0).round() as i32;
                    let color = if close >= open { 0xFF_FFFF } else { 0x40_4040 };
                    fill(
                        dl,
                        rect_path(x, a.min(b), w.round() as i32, (a - b).abs()),
                        color,
                        Some((0, 40)),
                    );
                }
            }
            let area = matches!(p.kind, PlotKind::Area { .. });
            let mut acc = vec![0.0; ncat];
            let mut prev: Vec<i32> = (0..ncat).map(|_| to(base)).collect();
            for (k, s) in p.series.iter().enumerate() {
                let color = series_color(s, k0 + k);
                let mut pts = Vec::new();
                for (i, v) in s.values.iter().enumerate() {
                    let Some(mut v) = *v else { continue };
                    if grouping == Grouping::Percent && totals[i] > 0.0 {
                        v /= totals[i];
                    }
                    if grouping != Grouping::Clustered {
                        acc[i] += v;
                        v = acc[i];
                    }
                    let q = if horizontal {
                        (to(v), cat_at(i))
                    } else {
                        (cat_at(i), to(v))
                    };
                    pts.push((i, q));
                }
                if area && !pts.is_empty() {
                    let mut path = Path::default();
                    for (j, (_, q)) in pts.iter().enumerate() {
                        if j == 0 {
                            path.move_to(*q);
                        } else {
                            path.line_to(*q);
                        }
                    }
                    for (i, q) in pts.iter().rev() {
                        let b = prev[*i];
                        path.line_to(if horizontal { (b, q.1) } else { (q.0, b) });
                    }
                    path.close();
                    fill(dl, path, color, None);
                    for (i, q) in &pts {
                        prev[*i] = if horizontal { q.0 } else { q.1 };
                    }
                } else {
                    let line_pts: Vec<(i32, i32)> = pts.iter().map(|p| p.1).collect();
                    if s.line {
                        stroke(
                            dl,
                            series_path(&line_pts, s.smooth),
                            color,
                            s.line_width.unwrap_or(SERIES_LINE),
                        );
                    }
                    let show = match p.kind {
                        PlotKind::Line { markers, .. } => s.markers.unwrap_or(markers),
                        _ => false,
                    };
                    if show {
                        for q in &line_pts {
                            marker(dl, k0 + k, *q, color);
                        }
                    }
                }
                let l = s.labels.unwrap_or(p.labels);
                if l.value || l.category || l.series {
                    let st = lstyle(&l);
                    for (i, q) in &pts {
                        labels.push((
                            label_text(&l, s, *i, s.values[*i].unwrap_or(0.0)),
                            st,
                            (q.0, q.1 - st.size * 8 / 10),
                        ));
                    }
                }
            }
        }
        PlotKind::Scatter {
            lines,
            markers,
            smooth,
        } => {
            for (k, s) in p.series.iter().enumerate() {
                let color = series_color(s, k0 + k);
                let pts: Vec<(i32, i32)> = s
                    .values
                    .iter()
                    .enumerate()
                    .filter_map(|(i, v)| {
                        let y = to((*v)?);
                        let x = match s.x.get(i).copied().flatten() {
                            Some(x) => to_x(x),
                            None => to_x((i + 1) as f64),
                        };
                        Some((x, y))
                    })
                    .collect();
                if lines && s.line {
                    stroke(
                        dl,
                        series_path(&pts, smooth || s.smooth),
                        color,
                        s.line_width.unwrap_or(SERIES_LINE),
                    );
                }
                if s.markers.unwrap_or(markers || !lines) {
                    for q in &pts {
                        marker(dl, k0 + k, *q, color);
                    }
                }
            }
        }
        _ => {}
    }
    for (s, st, (x, y)) in labels {
        text.centered(dl, &s, &st, x, y);
    }
}

fn point_color(s: &Series, i: usize) -> u32 {
    s.point_colors
        .iter()
        .find(|(k, _)| *k == i)
        .map_or(ACCENTS[i % ACCENTS.len()], |p| p.1)
}

/// A point on an ellipse at `a` degrees clockwise from the top.
fn on_ellipse((cx, cy): (f64, f64), (rx, ry): (f64, f64), a: f64) -> (i32, i32) {
    let t = a.to_radians();
    (
        (cx + rx * t.sin()).round() as i32,
        (cy - ry * t.cos()).round() as i32,
    )
}

/// A slice from `a` sweeping `sweep` degrees clockwise; a ring when
/// `inner` is above zero.
fn slice_path(c: (f64, f64), (rx, ry): (f64, f64), inner: f64, a: f64, sweep: f64) -> Path {
    let mut path = Path::default();
    let steps = ((sweep / 3.0).ceil() as usize).max(2);
    let k = if rx > 0.0 { inner / rx } else { 0.0 };
    if inner > 0.0 {
        path.move_to(on_ellipse(c, (rx * k, ry * k), a));
    } else {
        path.move_to((c.0.round() as i32, c.1.round() as i32));
    }
    for j in 0..=steps {
        path.line_to(on_ellipse(c, (rx, ry), a + sweep * j as f64 / steps as f64));
    }
    if inner > 0.0 {
        for j in (0..=steps).rev() {
            path.line_to(on_ellipse(
                c,
                (rx * k, ry * k),
                a + sweep * j as f64 / steps as f64,
            ));
        }
    }
    path.close();
    path
}

fn shade(color: u32, f: f64) -> u32 {
    let ch = |shift: u32| ((((color >> shift) & 0xFF) as f64 * f).round() as u32).min(255) << shift;
    ch(0) | ch(8) | ch(16)
}

/// Where the line from `p` touches the circle `(c, r)`: the upper or the
/// lower tangent point.
fn tangent(p: (f64, f64), c: (f64, f64), r: f64, upper: bool) -> (i32, i32) {
    let (dx, dy) = (p.0 - c.0, p.1 - c.1);
    let d = (dx * dx + dy * dy).sqrt().max(r);
    let base = dy.atan2(dx);
    let alpha = (r / d).acos();
    let pick = |t: f64| (c.0 + r * t.cos(), c.1 + r * t.sin());
    let (a, b) = (pick(base + alpha), pick(base - alpha));
    let q = if (a.1 < b.1) == upper { a } else { b };
    (q.0.round() as i32, q.1.round() as i32)
}

fn draw_pie(
    dl: &mut DisplayList,
    text: &Text<'_>,
    p: &Plot,
    (x0, y0, x1, y1): (i32, i32, i32, i32),
    base: TextStyle,
    three_d: bool,
) {
    let Some(s) = p.series.first() else { return };
    let PlotKind::Pie {
        hole,
        first_angle,
        of_pie,
    } = p.kind
    else {
        return;
    };
    let values: Vec<f64> = s.values.iter().map(|v| v.unwrap_or(0.0).abs()).collect();
    let (w, h) = ((x1 - x0) as f64, (y1 - y0) as f64);
    let (cx, cy) = ((x0 + x1) as f64 / 2.0, (y0 + y1) as f64 / 2.0);
    if let Some(of) = of_pie {
        draw_of_pie(dl, s, &values, of, (cx, cy), (w, h));
        return;
    }
    let total: f64 = values.iter().sum();
    if total <= 0.0 {
        return;
    }
    let explode = |i: usize| {
        s.point_explosion
            .iter()
            .find(|(k, _)| *k == i)
            .map_or(s.explosion, |p| p.1)
            .max(0) as f64
            / 100.0
    };
    let most = (0..values.len()).map(explode).fold(0.0, f64::max);
    // An exploded pie shrinks so its pulled slices stay in the plot.
    let r = w.min(h) / 2.0 * 0.95 / (1.0 + most);
    // Hancom tilts a 3D pie to an ellipse with a rim below it.
    let (rx, ry, depth) = if three_d {
        (r * 1.4875, r * 1.4875 * 0.475, r * 1.4875 * 0.209)
    } else {
        (r, r, 0.0)
    };
    let center = (cx, cy - depth / 2.0);
    let inner = rx * hole.clamp(0, 90) as f64 / 100.0;
    let mut slices = Vec::new();
    let mut a = first_angle as f64;
    for (i, v) in values.iter().enumerate() {
        let sweep = v / total * 360.0;
        if sweep > 0.0 {
            let mid = (a + sweep / 2.0).to_radians();
            let off = explode(i) * rx;
            let o = (
                center.0 + off * mid.sin(),
                center.1 - off * mid.cos() * if three_d { ry / rx } else { 1.0 },
            );
            slices.push((i, o, a, sweep));
        }
        a += sweep;
    }
    if three_d {
        // The rim: each slice's part facing the front, darkening upward.
        for &(i, o, a, sweep) in &slices {
            let (from, to) = (a.max(90.0), (a + sweep).min(270.0));
            if to <= from {
                continue;
            }
            let color = point_color(s, i);
            let steps = (((to - from) / 3.0).ceil() as usize).max(2);
            let mut path = Path::default();
            for j in 0..=steps {
                let q = on_ellipse(o, (rx, ry), from + (to - from) * j as f64 / steps as f64);
                if j == 0 {
                    path.move_to(q);
                } else {
                    path.line_to(q);
                }
            }
            for j in (0..=steps).rev() {
                let q = on_ellipse(o, (rx, ry), from + (to - from) * j as f64 / steps as f64);
                path.line_to((q.0, q.1 + depth.round() as i32));
            }
            path.close();
            dl.items.push(Item::Path(Box::new(PathItem {
                path,
                fill: vec![Brush::Gradient(Box::new(crate::GradientBrush {
                    kind: 1,
                    angle: 0,
                    center_x: 50,
                    center_y: 50,
                    step: 0,
                    step_center: 0,
                    stops: vec![(0, shade(color, 0.15)), (100, color)],
                }))],
                outline: None,
                fill_alpha: 0,
            })));
        }
    }
    let l = s.labels.unwrap_or(p.labels);
    for &(i, o, a, sweep) in &slices {
        let color = point_color(s, i);
        fill(dl, slice_path(o, (rx, ry), inner, a, sweep), color, None);
        if l.value || l.percent || l.category || l.series {
            let v = values[i];
            let mut s_text = label_text(&l, s, i, v);
            if l.percent {
                let pct = format!("{}%", (v / total * 100.0).round() as i64);
                s_text = if s_text.is_empty() {
                    pct
                } else {
                    format!("{s_text}, {pct}")
                };
            }
            let st = TextStyle { size: TEXT, ..base };
            let k = ((rx + inner) / 2.0 + rx * 0.15) / rx;
            let (lx, ly) = on_ellipse(o, (rx * k, ry * k), a + sweep / 2.0);
            text.centered(dl, &s_text, &st, lx, ly + mid_to_base(TEXT));
        }
    }
}

/// A pie whose last third of points pulls out into a second pie or a
/// stacked bar beside it, joined by lines to their slice in the first.
fn draw_of_pie(
    dl: &mut DisplayList,
    s: &Series,
    values: &[f64],
    of: hm_model::chart::OfPie,
    (cx, cy): (f64, f64),
    (w, h): (f64, f64),
) {
    let n = values.len();
    let rest = n - n * 2 / 3;
    if n < 2 || rest == 0 {
        return;
    }
    let (head, tail) = values.split_at(n - rest);
    let other: f64 = tail.iter().sum();
    let total: f64 = head.iter().sum::<f64>() + other;
    if total <= 0.0 {
        return;
    }
    let size = of.second as f64 / 100.0;
    let second_w = if of.bar { size / 2.0 } else { size };
    let gap = 2.0 / 3.0 * of.gap as f64 / 100.0 * second_w;
    let ratio = 1.0 + gap + second_w;
    let d = (0.899 * h).min(0.9617 * w / ratio);
    let r = d / 2.0;
    let main = (cx - ratio * d / 2.0 + r, cy);
    let line_color = 0x20_2020;
    // The split slice centers on the right; the rest follow it clockwise.
    let other_sweep = other / total * 360.0;
    let start = 90.0 - other_sweep / 2.0;
    fill(
        dl,
        slice_path(main, (r, r), 0.0, start, other_sweep),
        ACCENTS[n % ACCENTS.len()],
        None,
    );
    let mut a = start + other_sweep;
    for (i, v) in head.iter().enumerate() {
        let sweep = v / total * 360.0;
        if sweep > 0.0 {
            fill(
                dl,
                slice_path(main, (r, r), 0.0, a, sweep),
                point_color(s, i),
                None,
            );
        }
        a += sweep;
    }
    let top = on_ellipse(main, (r, r), start);
    let bottom = on_ellipse(main, (r, r), start + other_sweep);
    let left = main.0 + r + gap * d;
    if of.bar {
        let (bw, bh) = (second_w * d, size * d);
        let mut y = cy - bh / 2.0;
        for (k, v) in tail.iter().enumerate() {
            let hh = if other > 0.0 { v / other * bh } else { 0.0 };
            fill(
                dl,
                rect_path(
                    left.round() as i32,
                    y.round() as i32,
                    bw.round() as i32,
                    (y + hh).round() as i32 - y.round() as i32,
                ),
                point_color(s, n - rest + k),
                None,
            );
            y += hh;
        }
        let l = left.round() as i32;
        line(dl, top, (l, (cy - bh / 2.0).round() as i32), line_color, 30);
        line(
            dl,
            bottom,
            (l, (cy + bh / 2.0).round() as i32),
            line_color,
            30,
        );
    } else {
        let r2 = size * r;
        let c2 = (left + r2, cy);
        let mut a = start + other_sweep;
        for (k, v) in tail.iter().enumerate() {
            let sweep = if other > 0.0 { v / other * 360.0 } else { 0.0 };
            if sweep > 0.0 {
                fill(
                    dl,
                    slice_path(c2, (r2, r2), 0.0, a, sweep),
                    point_color(s, n - rest + k),
                    None,
                );
            }
            a += sweep;
        }
        let f = |p: (i32, i32)| (p.0 as f64, p.1 as f64);
        line(dl, top, tangent(f(top), c2, r2, true), line_color, 30);
        line(
            dl,
            bottom,
            tangent(f(bottom), c2, r2, false),
            line_color,
            30,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::PathEl;

    #[test]
    fn automatic_positive_3d_axes_keep_native_major_intervals() {
        let automatic = Axis::default();
        for (high, max, step) in [
            (12.3, 20.0, 5.0),
            (6.15, 8.0, 2.0),
            (18.45, 20.0, 5.0),
            (1.3, 2.0, 0.5),
            (5.0, 5.0, 1.0),
        ] {
            assert_eq!(
                scale3d((high, high), Some(&automatic), 10752, false),
                (0.0, max, step)
            );
        }
        let manual = Axis {
            max: Some(13.0),
            major_unit: Some(5.0),
            ..Axis::default()
        };
        assert_eq!(
            scale3d((2.5, 12.3), Some(&manual), 10752, false),
            (0.0, 15.0, 5.0)
        );
        assert_eq!(scale3d((0.0, 1.0), None, 10752, true), (0.0, 1.0, 0.2));
        assert_eq!(scale3d((0.0, 0.0), None, 8400, false), (0.0, 1.0, 0.5));
        assert_eq!(scale3d((2.5, 12.3), None, 21000, false), (0.0, 14.0, 2.0));
    }

    #[test]
    fn automatic_stacked_columns_draw_at_the_native_twenty_maximum() {
        let mut chart = Chart {
            plots: vec![Plot {
                kind: PlotKind::Bar {
                    horizontal: false,
                    grouping: Grouping::Stacked,
                    gap: 150,
                    overlap: 100,
                },
                series: [
                    [4.3, 2.5, 3.5, 4.5],
                    [2.4, 4.4, 1.8, 2.8],
                    [2.0, 2.0, 3.0, 5.0],
                ]
                .into_iter()
                .map(|values| Series {
                    values: values.into_iter().map(Some).collect(),
                    ..Series::default()
                })
                .collect(),
                labels: DataLabels::default(),
                hi_low: false,
                up_down: None,
            }],
            three_d: true,
            ..Chart::default()
        };
        let fonts = FontSystem::new();
        let text = Text { fonts: &fonts };
        let frame = Frame3d {
            plot: (11328, 14401, 35520, 25479),
            horizontal: false,
            label_base: 27563,
        };
        let mut automatic = DisplayList::default();
        let style = TextStyle::default();
        draw_bars_3d(
            &mut automatic,
            &text,
            &chart,
            &frame,
            (&style, &style),
            &[],
            4,
        );
        chart.value_axis = Some(Axis {
            max: Some(20.0),
            ..Axis::default()
        });
        let mut manual = DisplayList::default();
        draw_bars_3d(&mut manual, &text, &chart, &frame, (&style, &style), &[], 4);
        assert_eq!(automatic, manual);
        let last_front = automatic
            .items
            .iter()
            .rev()
            .find_map(|item| match item {
                Item::Path(path) if path.fill == [Brush::Solid(ACCENTS[2])] => Some(&path.path),
                _ => None,
            })
            .expect("last stacked column front");
        assert_eq!(last_front.els[2], PathEl::Line(31715, 18945));
    }
}
