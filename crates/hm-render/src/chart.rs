//! Charts drawn from their model: title, legend, axes with gridlines and
//! labels, and the plots (columns and bars, lines, areas, pies and
//! scatter) in the default Office theme's colors.

use hm_fonts::{FaceQuery, FontSystem};
use hm_layout::Rect;
use hm_model::chart::{Axis, Chart, DataLabels, Grouping, Plot, PlotKind, Series, TextStyle};

use crate::draw::{Outline, Path, PathItem};
use crate::paint::{Brush, Dash, Stroke};
use crate::{DisplayList, GlyphRun, Item};

/// Series colors when a chart names none: the theme's accents.
const ACCENTS: [u32; 6] = [0xC47244, 0x317DED, 0xA5A5A5, 0x00C0FF, 0xD59B5B, 0x47AD70];

/// Chart text in a sans face, measured and drawn as glyph runs.
struct Text<'a> {
    fonts: &'a FontSystem,
}

impl Text<'_> {
    fn face(&self, bold: bool) -> Option<hm_fonts::FaceId> {
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

    /// Draws `s` centered on `cx`, its middle at `cy`.
    fn centered(&self, dl: &mut DisplayList, s: &str, st: &TextStyle, cx: i32, cy: i32) {
        let w = self.width(s, st);
        self.draw(dl, s, st, cx - w / 2, cy + st.size * 35 / 100);
    }
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

fn fill(dl: &mut DisplayList, path: Path, color: u32, outline: Option<u32>) {
    dl.items.push(Item::Path(Box::new(PathItem {
        path,
        fill: vec![Brush::Solid(color)],
        outline: outline.map(|c| Outline {
            width: 0,
            color: c,
            dash: Dash::Solid,
            round_cap: false,
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

/// A round step for about `n` divisions of `range`: 1, 2 or 5 times a
/// power of ten.
fn nice_step(range: f64, n: f64) -> f64 {
    if range <= 0.0 {
        return 1.0;
    }
    let raw = range / n;
    let p = 10f64.powf(raw.log10().floor());
    let m = raw / p;
    p * if m <= 1.0 {
        1.0
    } else if m <= 2.0 {
        2.0
    } else if m <= 5.0 {
        5.0
    } else {
        10.0
    }
}

fn series_color(s: &Series, k: usize) -> u32 {
    s.color.unwrap_or(ACCENTS[k % ACCENTS.len()])
}

/// The value range the plots need on the value axis.
fn value_range(plots: &[Plot], axis: Option<&Axis>) -> (f64, f64, f64) {
    let (mut lo, mut hi) = (f64::MAX, f64::MIN);
    for p in plots {
        let stacked = matches!(
            p.kind,
            PlotKind::Bar {
                grouping: Grouping::Stacked,
                ..
            } | PlotKind::Line {
                grouping: Grouping::Stacked,
                ..
            } | PlotKind::Area {
                grouping: Grouping::Stacked
            }
        );
        let percent = matches!(
            p.kind,
            PlotKind::Bar {
                grouping: Grouping::Percent,
                ..
            } | PlotKind::Line {
                grouping: Grouping::Percent,
                ..
            } | PlotKind::Area {
                grouping: Grouping::Percent
            }
        );
        if percent {
            lo = lo.min(0.0);
            hi = hi.max(1.0);
            continue;
        }
        let n = p.series.iter().map(|s| s.values.len()).max().unwrap_or(0);
        for i in 0..n {
            let mut sum = 0.0;
            for s in &p.series {
                let v = s.values.get(i).copied().flatten().unwrap_or(0.0);
                if stacked {
                    sum += v;
                    lo = lo.min(sum);
                    hi = hi.max(sum);
                } else if s.values.get(i).copied().flatten().is_some() {
                    lo = lo.min(v);
                    hi = hi.max(v);
                }
            }
        }
    }
    if lo > hi {
        (lo, hi) = (0.0, 1.0);
    }
    if lo > 0.0 {
        lo = 0.0;
    }
    let min = axis.and_then(|a| a.min).unwrap_or(lo);
    let step = axis
        .and_then(|a| a.major_unit)
        .unwrap_or_else(|| nice_step(hi.max(min + 1e-9) - min, 5.0));
    let max = axis
        .and_then(|a| a.max)
        .unwrap_or_else(|| ((hi - min) / step).ceil() * step + min)
        .max(min + step);
    (min, max, step)
}

/// Draws a chart filling `r`.
pub fn draw(dl: &mut DisplayList, fonts: &FontSystem, c: &Chart, r: Rect) {
    let text = Text { fonts };
    dl.items.push(Item::Path(Box::new(PathItem {
        path: rect_path(r.x, r.y, r.w, r.h),
        fill: c.fill.map(Brush::Solid).into_iter().collect(),
        outline: c.border.map(|b| Outline {
            width: 36,
            color: b,
            dash: Dash::Solid,
            round_cap: false,
        }),
        fill_alpha: 0,
    })));
    // Margins as Hancom's chart engine leaves them: about a tenth of an
    // inch inside the frame and 400 HU between blocks.
    let pad = 720.min(r.h / 6).min(r.w / 6);
    let gap = 400.min(r.h / 12);
    let (mut x0, mut y0, mut x1, mut y1) = (r.x + pad, r.y + pad, r.x + r.w - pad, r.y + r.h - pad);
    if let Some(t) = c.title.as_ref().filter(|t| !t.text.is_empty()) {
        let center = y0 + t.style.size * 97 / 100;
        text.centered(dl, &t.text, &t.style, r.x + r.w / 2, center);
        y0 = center + t.style.size * 6 / 10 + gap;
    }
    // Legend entries: every series, or every category of a pie.
    let pie = c
        .plots
        .first()
        .is_some_and(|p| matches!(p.kind, PlotKind::Pie { .. }));
    let entries: Vec<(String, u32)> = if pie {
        let s = c.plots.first().and_then(|p| p.series.first());
        s.map(|s| {
            (0..s.values.len())
                .map(|i| {
                    let color = s
                        .point_colors
                        .iter()
                        .find(|(k, _)| *k == i)
                        .map_or(ACCENTS[i % ACCENTS.len()], |p| p.1);
                    (s.categories.get(i).cloned().unwrap_or_default(), color)
                })
                .collect()
        })
        .unwrap_or_default()
    } else {
        c.plots
            .iter()
            .flat_map(|p| p.series.iter())
            .enumerate()
            .map(|(k, s)| (s.name.clone(), series_color(s, k)))
            .collect()
    };
    if let Some(l) = c.legend.filter(|_| !entries.is_empty()) {
        let st = l.style;
        let key = st.size * 6 / 10;
        let item = |s: &str| key + st.size / 4 + text.width(s, &st);
        let row = st.size * 14 / 10;
        match l.position {
            b'b' | b't' => {
                let spacing = st.size / 4;
                let total: i32 = entries.iter().map(|(s, _)| item(s)).sum::<i32>()
                    + spacing * (entries.len() as i32 - 1);
                let mut x = r.x + (r.w - total) / 2;
                let cy = if l.position == b'b' {
                    y1 - st.size * 6 / 10
                } else {
                    y0 + st.size * 6 / 10
                };
                for (s, color) in &entries {
                    fill(dl, rect_path(x, cy - key / 2, key, key), *color, None);
                    text.draw(dl, s, &st, x + key + st.size / 4, cy + st.size * 35 / 100);
                    x += item(s) + spacing;
                }
                if l.position == b'b' {
                    y1 = cy - st.size * 6 / 10 - gap;
                } else {
                    y0 = cy + st.size * 6 / 10 + gap;
                }
            }
            _ => {
                let widest = entries.iter().map(|(s, _)| item(s)).max().unwrap_or(0);
                let total = row * entries.len() as i32;
                let x = if l.position == b'l' { x0 } else { x1 - widest };
                let mut cy = (y0 + y1) / 2 - total / 2 + row / 2;
                for (s, color) in &entries {
                    fill(dl, rect_path(x, cy - key / 2, key, key), *color, None);
                    text.draw(dl, s, &st, x + key + st.size / 4, cy + st.size * 35 / 100);
                    cy += row;
                }
                if l.position == b'l' {
                    x0 += widest + pad;
                } else {
                    x1 -= widest + pad;
                }
            }
        }
    }
    if pie {
        if let Some(p) = c.plots.first() {
            draw_pie(dl, &text, p, (x0, y0, x1, y1));
        }
        return;
    }
    let (min, max, step) = value_range(&c.plots, c.value_axis.as_ref());
    let vaxis = c.value_axis.unwrap_or_default();
    let caxis = c.category_axis.unwrap_or_default();
    let horizontal = c.plots.iter().any(|p| {
        matches!(
            p.kind,
            PlotKind::Bar {
                horizontal: true,
                ..
            }
        )
    });
    // Value labels on the left (or below for bars), categories below (or left).
    let ticks: Vec<f64> = {
        let mut v = Vec::new();
        let mut t = min;
        while t <= max + step * 1e-6 && v.len() < 200 {
            v.push(t);
            t += step;
        }
        v
    };
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
    let vlabel_w = if vaxis.deleted {
        0
    } else {
        ticks
            .iter()
            .map(|t| text.width(&general(*t), &vaxis.labels))
            .max()
            .unwrap_or(0)
            + vaxis.labels.size * 3 / 2
    };
    // Category labels keep their row even when the axis is hidden.
    let clabel_h = caxis.labels.size * 13 / 10;
    // Data labels outside the bar ends need a row above the plot.
    let label_row = c
        .plots
        .iter()
        .flat_map(|p| p.series.iter().map(move |s| s.labels.unwrap_or(p.labels)))
        .filter(|l| l.value || l.category || l.series)
        .map(|l| l.style.size * 13 / 10)
        .max()
        .unwrap_or(0);
    let (px0, py0, px1, py1) = if horizontal {
        let cw = categories
            .iter()
            .map(|s| text.width(s, &caxis.labels))
            .max()
            .unwrap_or(0);
        (
            x0 + if caxis.deleted { 0 } else { cw + pad / 2 },
            y0,
            x1,
            y1 - if vaxis.deleted {
                0
            } else {
                vaxis.labels.size * 14 / 10
            },
        )
    } else {
        (
            x0 + vlabel_w,
            y0 + vaxis.labels.size / 2 + label_row,
            x1,
            y1 - clabel_h,
        )
    };
    // A plot with no room left collapses to its baseline.
    let py0 = py0.min(py1);
    if px1 <= px0 {
        return;
    }
    // Position of a value along the value axis.
    let to = |v: f64| -> i32 {
        let f = ((v - min) / (max - min)).clamp(-0.05, 1.05);
        if horizontal {
            px0 + (f * (px1 - px0) as f64) as i32
        } else {
            py1 - (f * (py1 - py0) as f64) as i32
        }
    };
    let mut last_label: Option<i32> = None;
    for t in &ticks {
        let p = to(*t);
        if let Some(g) = vaxis.gridlines {
            if horizontal {
                line(dl, (p, py0), (p, py1), g, 0);
            } else {
                line(dl, (px0, p), (px1, p), g, 0);
            }
        }
        let crowded = last_label.is_some_and(|q| (q - p).abs() < vaxis.labels.size);
        if !vaxis.deleted && !crowded {
            last_label = Some(p);
            let s = general(*t);
            if horizontal {
                text.centered(dl, &s, &vaxis.labels, p, py1 + vaxis.labels.size * 7 / 10);
            } else {
                let w = text.width(&s, &vaxis.labels);
                let x = px0 - vaxis.labels.size * 3 / 2 - w;
                text.draw(dl, &s, &vaxis.labels, x, p + vaxis.labels.size * 35 / 100);
            }
        }
    }
    let span = if horizontal { py1 - py0 } else { px1 - px0 };
    let slot = span as f64 / ncat as f64;
    let cat_at = |i: usize| -> i32 {
        let o = (slot * (i as f64 + 0.5)) as i32;
        if horizontal { py0 + o } else { px0 + o }
    };
    if !caxis.deleted {
        for (i, s) in categories.iter().enumerate().take(ncat) {
            if horizontal {
                let w = text.width(s, &caxis.labels);
                text.draw(
                    dl,
                    s,
                    &caxis.labels,
                    px0 - pad / 2 - w,
                    cat_at(i) + caxis.labels.size * 35 / 100,
                );
            } else {
                text.centered(dl, s, &caxis.labels, cat_at(i), py1 + clabel_h / 2);
            }
        }
        if let Some(l) = caxis.line {
            if horizontal {
                line(dl, (px0, py0), (px0, py1), l, 0);
            } else {
                line(dl, (px0, py1), (px1, py1), l, 0);
            }
        }
    }
    let mut k0 = 0;
    for p in &c.plots {
        draw_plot(
            dl,
            &text,
            p,
            k0,
            (slot, &cat_at, &to, horizontal),
            (min, max),
        );
        k0 += p.series.len();
    }
}

type Geometry<'a> = (f64, &'a dyn Fn(usize) -> i32, &'a dyn Fn(f64) -> i32, bool);

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

fn draw_plot(
    dl: &mut DisplayList,
    text: &Text<'_>,
    p: &Plot,
    k0: usize,
    (slot, cat_at, to, horizontal): Geometry<'_>,
    (min, _max): (f64, f64),
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
                    let start = cat_at(i) - (group / 2.0) as i32 + lane as i32;
                    let (a, b) = (to(from), to(to_v));
                    let path = if horizontal {
                        rect_path(a.min(b), start, (b - a).abs(), bar as i32)
                    } else {
                        rect_path(start, a.min(b), bar as i32, (b - a).abs())
                    };
                    fill(dl, path, color, None);
                    let l = s.labels.unwrap_or(p.labels);
                    if l.value || l.category || l.series {
                        let mid = start + (bar / 2.0) as i32;
                        let at = if horizontal {
                            (b + l.style.size, mid)
                        } else {
                            (mid, b - l.style.size * 12 / 10)
                        };
                        labels.push((
                            label_text(&l, s, i, s.values[i].unwrap_or(0.0)),
                            l.style,
                            at,
                        ));
                    }
                }
            }
        }
        PlotKind::Line { grouping, .. } | PlotKind::Area { grouping } => {
            let area = matches!(p.kind, PlotKind::Area { .. });
            let markers = matches!(p.kind, PlotKind::Line { markers: true, .. }) && !area;
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
                    for w in pts.windows(2) {
                        line(dl, w[0].1, w[1].1, color, 28);
                    }
                    if markers {
                        for (_, q) in &pts {
                            fill(dl, rect_path(q.0 - 60, q.1 - 60, 120, 120), color, None);
                        }
                    }
                }
                let l = s.labels.unwrap_or(p.labels);
                if l.value || l.category || l.series {
                    for (i, q) in &pts {
                        labels.push((
                            label_text(&l, s, *i, s.values[*i].unwrap_or(0.0)),
                            l.style,
                            (q.0, q.1 - l.style.size * 8 / 10),
                        ));
                    }
                }
            }
        }
        PlotKind::Scatter { lines, markers, .. } => {
            // Scatter x values share the category slots in order.
            for (k, s) in p.series.iter().enumerate() {
                let color = series_color(s, k0 + k);
                let pts: Vec<(i32, i32)> = s
                    .values
                    .iter()
                    .enumerate()
                    .filter_map(|(i, v)| Some((cat_at(i), to((*v)?))))
                    .collect();
                if lines {
                    for w in pts.windows(2) {
                        line(dl, w[0], w[1], color, 28);
                    }
                }
                if markers || !lines {
                    for q in &pts {
                        fill(dl, rect_path(q.0 - 60, q.1 - 60, 120, 120), color, None);
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

fn draw_pie(
    dl: &mut DisplayList,
    text: &Text<'_>,
    p: &Plot,
    (x0, y0, x1, y1): (i32, i32, i32, i32),
) {
    let Some(s) = p.series.first() else { return };
    let PlotKind::Pie { hole, first_angle } = p.kind else {
        return;
    };
    let total: f64 = s.values.iter().map(|v| v.unwrap_or(0.0).abs()).sum();
    if total <= 0.0 {
        return;
    }
    let r = ((x1 - x0).min(y1 - y0) / 2) as f64 * 0.9;
    let (cx, cy) = (((x0 + x1) / 2) as f64, ((y0 + y1) / 2) as f64);
    let inner = r * hole.clamp(0, 90) as f64 / 100.0;
    // Angles clockwise from the top.
    let point = |a: f64, rad: f64| {
        let t = a.to_radians();
        ((cx + rad * t.sin()) as i32, (cy - rad * t.cos()) as i32)
    };
    let mut a = first_angle as f64;
    let l = s.labels.unwrap_or(p.labels);
    for (i, v) in s.values.iter().enumerate() {
        let v = v.unwrap_or(0.0).abs();
        let sweep = v / total * 360.0;
        if sweep <= 0.0 {
            continue;
        }
        let color = s
            .point_colors
            .iter()
            .find(|(k, _)| *k == i)
            .map_or(ACCENTS[i % ACCENTS.len()], |p| p.1);
        let mut path = Path::default();
        let steps = ((sweep / 3.0).ceil() as usize).max(2);
        if inner > 0.0 {
            path.move_to(point(a, inner));
        } else {
            path.move_to((cx as i32, cy as i32));
        }
        for k in 0..=steps {
            path.line_to(point(a + sweep * k as f64 / steps as f64, r));
        }
        if inner > 0.0 {
            for k in (0..=steps).rev() {
                path.line_to(point(a + sweep * k as f64 / steps as f64, inner));
            }
        }
        path.close();
        fill(dl, path, color, Some(0xFF_FFFF));
        if l.value || l.percent || l.category || l.series {
            let mut s_text = label_text(&l, s, i, v);
            if l.percent {
                let pct = format!("{}%", (v / total * 100.0).round() as i64);
                s_text = if s_text.is_empty() {
                    pct
                } else {
                    format!("{s_text}, {pct}")
                };
            }
            let (lx, ly) = point(a + sweep / 2.0, (r + inner) / 2.0 + r * 0.15);
            text.centered(dl, &s_text, &l.style, lx, ly);
        }
        a += sweep;
    }
}
