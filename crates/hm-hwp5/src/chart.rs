//! OOXML chart parts (`c:chartSpace`, ECMA-376 DrawingML charts) read into
//! the chart model: plots with their series caches, axes, titles, legend
//! and the colors of the default Office theme.

use hm_model::chart::{
    Axis, Chart, ChartText, DataLabels, Grouping, Legend, OfPie, Plot, PlotKind, Series, TextStyle,
    Ticks,
};

/// One XML element with its local name, attributes, text and children.
#[derive(Debug, Default)]
struct Node {
    name: String,
    attrs: Vec<(String, String)>,
    text: String,
    children: Vec<Node>,
}

impl Node {
    fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }

    fn children<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }

    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// The `val` attribute of a child element.
    fn val(&self, child: &str) -> Option<&str> {
        self.child(child).and_then(|c| c.attr("val"))
    }

    fn flag(&self, child: &str) -> Option<bool> {
        self.val(child).map(|v| v == "1" || v == "true")
    }

    /// The first descendant with this name, depth first.
    fn find(&self, name: &str) -> Option<&Node> {
        self.children.iter().find_map(|c| {
            if c.name == name {
                Some(c)
            } else {
                c.find(name)
            }
        })
    }
}

fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let Some(end) = tail.find(';') else {
            out.push_str(tail);
            return out;
        };
        let entity = &tail[1..end];
        let ch = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            e if e.starts_with("#x") => u32::from_str_radix(&e[2..], 16)
                .ok()
                .and_then(char::from_u32),
            e if e.starts_with('#') => e[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => out.push(c),
            None => out.push_str(&tail[..=end]),
        }
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Parses an XML document into its root element.
fn parse_xml(src: &str) -> Option<Node> {
    let mut stack: Vec<Node> = vec![Node::default()];
    let mut i = 0;
    let b = src.as_bytes();
    while i < b.len() {
        if b[i] == b'<' {
            if src[i..].starts_with("<!--") {
                i += src[i..].find("-->")? + 3;
                continue;
            }
            if src[i..].starts_with("<?") || src[i..].starts_with("<!") {
                i += src[i..].find('>')? + 1;
                continue;
            }
            let end = i + src[i..].find('>')?;
            let tag = &src[i + 1..end];
            i = end + 1;
            if let Some(name) = tag.strip_prefix('/') {
                let done = stack.pop()?;
                if local(name.trim()) != done.name {
                    return None;
                }
                stack.last_mut()?.children.push(done);
                continue;
            }
            let self_closing = tag.ends_with('/');
            let tag = tag.trim_end_matches('/');
            let (name, rest) = tag.split_once(char::is_whitespace).unwrap_or((tag, ""));
            let mut node = Node {
                name: local(name).to_string(),
                ..Node::default()
            };
            let mut r = rest;
            while let Some(eq) = r.find('=') {
                let key = r[..eq].trim();
                let after = r[eq + 1..].trim_start();
                let quote = after.chars().next()?;
                let close = after[1..].find(quote)? + 1;
                node.attrs
                    .push((local(key).to_string(), unescape(&after[1..close])));
                r = &after[close + 1..];
            }
            if self_closing {
                stack.last_mut()?.children.push(node);
            } else {
                stack.push(node);
            }
        } else {
            let end = src[i..].find('<').map_or(b.len(), |k| i + k);
            if let Some(top) = stack.last_mut() {
                top.text.push_str(&unescape(&src[i..end]));
            }
            i = end;
        }
    }
    let mut root = stack.pop()?;
    root.children.pop()
}

/// The theme colors Hancom draws charts with, as COLORREF. Hancom does not
/// map `bg1` to the light theme color: a chart area filled with `bg1` is
/// black in its output, as `tx1` text is (1790387 p27/p28).
fn scheme(name: &str) -> u32 {
    let rgb = match name {
        "accent1" => 0x6182D6,
        "accent2" => 0xFF843A,
        "accent3" => 0xB2B2B2,
        "accent4" => 0xFFD700,
        "accent5" => 0x289B6E,
        "accent6" => 0x70AD47,
        "lt1" => 0xFFFFFF,
        "bg2" | "lt2" => 0xE7E6E6,
        "tx2" | "dk2" => 0x44546A,
        _ => 0x000000,
    };
    colorref(rgb)
}

fn colorref(rgb: u32) -> u32 {
    ((rgb & 0xFF) << 16) | (rgb & 0xFF00) | ((rgb >> 16) & 0xFF)
}

/// A DrawingML color element with its luminance and shade modifiers.
fn color(n: &Node) -> Option<u32> {
    let base = match n.name.as_str() {
        "srgbClr" => colorref(u32::from_str_radix(n.attr("val")?, 16).ok()?),
        "schemeClr" => scheme(n.attr("val")?),
        "prstClr" => match n.attr("val")? {
            "white" => 0xFF_FFFF,
            _ => 0,
        },
        "sysClr" => colorref(u32::from_str_radix(n.attr("lastClr").unwrap_or("000000"), 16).ok()?),
        _ => return None,
    };
    let mut c = [base & 0xFF, (base >> 8) & 0xFF, (base >> 16) & 0xFF].map(|v| v as f64 / 255.0);
    let pct = |name: &str| {
        n.child(name)
            .and_then(|m| m.attr("val"))
            .and_then(|v| v.parse::<f64>().ok())
            .map(|v| v / 100_000.0)
    };
    if let Some(m) = pct("lumMod") {
        let off = pct("lumOff").unwrap_or(0.0);
        let (h, s, l) = hsl(c);
        c = rgb(h, s, (l * m + off).clamp(0.0, 1.0));
    }
    if let Some(t) = pct("tint") {
        c = c.map(|v| v + (1.0 - v) * (1.0 - t));
    }
    if let Some(s) = pct("shade") {
        c = c.map(|v| v * s);
    }
    let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    Some(byte(c[0]) | byte(c[1]) << 8 | byte(c[2]) << 16)
}

fn hsl([r, g, b]: [f64; 3]) -> (f64, f64, f64) {
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.0;
    if max == min {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h / 6.0, s, l)
}

fn rgb(h: f64, s: f64, l: f64) -> [f64; 3] {
    if s == 0.0 {
        return [l; 3];
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let f = |mut t: f64| {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    [f(h + 1.0 / 3.0), f(h), f(h - 1.0 / 3.0)]
}

/// The solid fill color of a fill holder (`spPr`, `a:ln`, `a:defRPr`).
fn solid(n: &Node) -> Option<u32> {
    if n.child("noFill").is_some() {
        return None;
    }
    let fill = n.child("solidFill")?;
    fill.children.iter().find_map(color)
}

/// The line color of a shape's properties: `None` when no line is drawn.
fn line_color(sp: Option<&Node>) -> Option<u32> {
    let ln = sp?.child("ln")?;
    solid(ln)
}

/// Gray Hancom draws chart frames, axes and gridlines in when their
/// properties name no line.
const RULE: u32 = 0x0086_8686;

/// The color of a line Hancom draws unless its properties turn it off.
fn rule(sp: Option<&Node>) -> Option<u32> {
    match sp.and_then(|s| s.child("ln")) {
        Some(ln) if ln.child("noFill").is_some() => None,
        Some(ln) => Some(solid(ln).unwrap_or(RULE)),
        None => Some(RULE),
    }
}

/// Text style from a `c:txPr` or `c:rich` body's first run properties.
fn text_style(n: Option<&Node>, base: TextStyle) -> TextStyle {
    let Some(rpr) = n.and_then(|n| n.find("defRPr").or_else(|| n.find("rPr"))) else {
        return base;
    };
    TextStyle {
        size: rpr
            .attr("sz")
            .and_then(|v| v.parse::<i32>().ok())
            .unwrap_or(base.size),
        bold: rpr.attr("b").map_or(base.bold, |v| v == "1"),
        color: solid(rpr).unwrap_or(base.color),
    }
}

fn numbers(n: Option<&Node>) -> Vec<Option<f64>> {
    let Some(cache) = n.and_then(|n| n.find("numCache").or_else(|| n.find("numLit"))) else {
        return Vec::new();
    };
    let count = cache
        .val("ptCount")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);
    let mut out = vec![None; count];
    for pt in cache.children("pt") {
        let idx: usize = pt.attr("idx").and_then(|v| v.parse().ok()).unwrap_or(0);
        let v = pt
            .child("v")
            .and_then(|v| v.text.trim().parse::<f64>().ok());
        if idx >= out.len() {
            out.resize(idx + 1, None);
        }
        out[idx] = v;
    }
    out
}

fn strings(n: Option<&Node>) -> Vec<String> {
    let Some(cache) = n.and_then(|n| {
        n.find("strCache")
            .or_else(|| n.find("strLit"))
            .or_else(|| n.find("numCache"))
    }) else {
        return Vec::new();
    };
    let count = cache
        .val("ptCount")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);
    let mut out = vec![String::new(); count];
    for pt in cache.children("pt") {
        let idx: usize = pt.attr("idx").and_then(|v| v.parse().ok()).unwrap_or(0);
        if idx >= out.len() {
            out.resize(idx + 1, String::new());
        }
        out[idx] = pt.child("v").map(|v| v.text.clone()).unwrap_or_default();
    }
    out
}

fn labels(n: Option<&Node>, base: TextStyle) -> Option<DataLabels> {
    let n = n?;
    if n.flag("delete") == Some(true) {
        return Some(DataLabels::default());
    }
    Some(DataLabels {
        value: n.flag("showVal").unwrap_or(false),
        category: n.flag("showCatName").unwrap_or(false),
        series: n.flag("showSerName").unwrap_or(false),
        percent: n.flag("showPercent").unwrap_or(false),
        style: text_style(n.child("txPr"), base),
    })
}

fn grouping(n: &Node) -> Grouping {
    match n.val("grouping") {
        Some("stacked") => Grouping::Stacked,
        Some("percentStacked") => Grouping::Percent,
        _ => Grouping::Clustered,
    }
}

fn axis(n: Option<&Node>, base: TextStyle) -> Option<Axis> {
    let n = n?;
    let scaling = n.child("scaling");
    let num = |name: &str| {
        scaling
            .and_then(|s| s.val(name))
            .and_then(|v| v.parse::<f64>().ok())
    };
    Some(Axis {
        deleted: n.flag("delete").unwrap_or(false),
        min: num("min"),
        max: num("max"),
        major_unit: n.val("majorUnit").and_then(|v| v.parse().ok()),
        gridlines: n
            .child("majorGridlines")
            .and_then(|g| rule(g.child("spPr"))),
        line: rule(n.child("spPr")),
        labels: text_style(n.child("txPr"), base),
        ticks: match n.val("majorTickMark") {
            Some("none") => Ticks::None,
            Some("in") => Ticks::In,
            Some("cross") => Ticks::Cross,
            _ => Ticks::Out,
        },
        percent: false,
    })
}

/// Reads a chart part.
pub fn parse(xml: &str) -> Option<Chart> {
    let root = parse_xml(xml)?;
    if root.name != "chartSpace" {
        return None;
    }
    let chart = root.child("chart")?;
    let text = text_style(root.child("txPr"), TextStyle::default());
    let mut out = Chart {
        text,
        ..Chart::default()
    };
    if let Some(sp) = root.child("spPr") {
        out.fill = solid(sp);
        out.border = rule(Some(sp));
    } else {
        out.fill = Some(0xFF_FFFF);
        out.border = Some(RULE);
    }
    let auto_deleted = chart.flag("autoTitleDeleted").unwrap_or(false);
    if let Some(t) = chart.child("title") {
        let rich = t.find("rich");
        let words: String = rich.map_or(String::new(), |r| {
            let mut s = String::new();
            collect_text(r, &mut s);
            s
        });
        let style = text_style(rich.or(t.child("txPr")), TextStyle { size: 1400, ..text });
        if !words.is_empty() || !auto_deleted {
            out.title = Some(ChartText {
                text: words,
                style: TextStyle {
                    bold: rich.is_none() || style.bold,
                    ..style
                },
            });
        }
    }
    let plot_area = chart.child("plotArea")?;
    out.three_d = chart.child("view3D").is_some();
    for p in &plot_area.children {
        let kind = match p.name.as_str() {
            "barChart" | "bar3DChart" => PlotKind::Bar {
                horizontal: p.val("barDir") == Some("bar"),
                grouping: grouping(p),
                gap: p
                    .val("gapWidth")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(150),
                overlap: p.val("overlap").and_then(|v| v.parse().ok()).unwrap_or(0),
            },
            "lineChart" | "line3DChart" | "stockChart" | "radarChart" => PlotKind::Line {
                grouping: grouping(p),
                markers: p.flag("marker").unwrap_or(true),
            },
            "areaChart" | "area3DChart" => PlotKind::Area {
                grouping: grouping(p),
            },
            "pieChart" | "pie3DChart" | "doughnutChart" | "ofPieChart" => PlotKind::Pie {
                hole: p.val("holeSize").and_then(|v| v.parse().ok()).unwrap_or(0),
                first_angle: p
                    .val("firstSliceAng")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0),
                of_pie: (p.name == "ofPieChart").then(|| OfPie {
                    bar: p.val("ofPieType") == Some("bar"),
                    second: p
                        .val("secondPieSize")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(75),
                    gap: p
                        .val("gapWidth")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(150),
                }),
            },
            "scatterChart" | "bubbleChart" => {
                let style = p.val("scatterStyle").unwrap_or("marker");
                PlotKind::Scatter {
                    lines: style.contains("line") || style.contains("smooth"),
                    markers: style.contains("marker") || style == "lineMarker",
                    smooth: style.contains("smooth"),
                }
            }
            _ => continue,
        };
        let plot_labels = labels(p.child("dLbls"), text).unwrap_or_default();
        let series = p
            .children("ser")
            .map(|s| {
                let sp = s.child("spPr");
                let fill = sp
                    .and_then(solid)
                    .or_else(|| sp.and_then(|x| line_color(Some(x))));
                Series {
                    name: strings(s.child("tx"))
                        .into_iter()
                        .next()
                        .unwrap_or_else(|| {
                            s.child("tx")
                                .and_then(|t| t.child("v"))
                                .map(|v| v.text.clone())
                                .unwrap_or_default()
                        }),
                    categories: strings(s.child("cat")),
                    values: numbers(s.child("val").or_else(|| s.child("yVal"))),
                    x: numbers(s.child("xVal")),
                    color: fill,
                    point_colors: s
                        .children("dPt")
                        .filter_map(|d| {
                            let idx = d.val("idx")?.parse().ok()?;
                            Some((idx, solid(d.child("spPr")?)?))
                        })
                        .collect(),
                    labels: labels(s.child("dLbls"), text),
                    markers: s
                        .child("marker")
                        .and_then(|m| m.val("symbol"))
                        .map(|v| v != "none"),
                    line: sp
                        .and_then(|x| x.child("ln"))
                        .is_none_or(|ln| ln.child("noFill").is_none()),
                    line_width: sp
                        .and_then(|x| x.child("ln"))
                        .and_then(|ln| ln.attr("w"))
                        .and_then(|w| w.parse::<i32>().ok())
                        .map(|emu| emu / 127),
                    smooth: s.flag("smooth").unwrap_or(false),
                    explosion: s.val("explosion").and_then(|v| v.parse().ok()).unwrap_or(0),
                    point_explosion: s
                        .children("dPt")
                        .filter_map(|d| {
                            Some((
                                d.val("idx")?.parse().ok()?,
                                d.val("explosion")?.parse().ok()?,
                            ))
                        })
                        .collect(),
                }
            })
            .collect();
        out.plots.push(Plot {
            kind,
            series,
            labels: plot_labels,
            hi_low: p.child("hiLowLines").is_some(),
            up_down: p.child("upDownBars").map(|u| {
                u.val("gapWidth")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(150)
            }),
        });
    }
    let axis_text = text;
    // A scatter chart's x axis is its value axis along the bottom or top.
    let across = |n: &&Node| matches!(n.val("axPos"), Some("b" | "t"));
    let scatter = out
        .plots
        .iter()
        .any(|p| matches!(p.kind, PlotKind::Scatter { .. }));
    let (cat, val) = if scatter {
        (
            plot_area.children("valAx").find(across),
            plot_area.children("valAx").find(|n| !across(n)),
        )
    } else {
        (
            plot_area
                .child("catAx")
                .or_else(|| plot_area.child("dateAx")),
            plot_area.child("valAx"),
        )
    };
    out.category_axis = axis(cat, axis_text);
    out.value_axis = axis(val, axis_text);
    // Hancom titles a chart without title text after its only series, or
    // 차트 제목.
    if let Some(t) = out.title.as_mut().filter(|t| t.text.is_empty()) {
        let mut series = out.plots.iter().flat_map(|p| p.series.iter());
        t.text = match (series.next(), series.next()) {
            (Some(s), None) if !s.name.is_empty() => s.name.clone(),
            _ => "차트 제목".to_string(),
        };
    }
    let percent = out.plots.iter().any(|p| {
        matches!(
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
        )
    });
    if let Some(a) = out.value_axis.as_mut() {
        a.percent = percent;
    }
    if let Some(l) = chart.child("legend") {
        out.legend = Some(Legend {
            position: l
                .val("legendPos")
                .and_then(|v| v.bytes().next())
                .unwrap_or(b'r'),
            style: text_style(l.child("txPr"), axis_text),
        });
    }
    Some(out)
}

fn collect_text(n: &Node, out: &mut String) {
    if n.name == "t" {
        out.push_str(&n.text);
    }
    if n.name == "p" && !out.is_empty() {
        out.push('\n');
    }
    for c in &n.children {
        collect_text(c, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_one_is_black_and_light_one_white() {
        let fill = |val: &str| {
            let xml = format!("<a:solidFill><a:schemeClr val=\"{val}\"/></a:solidFill>");
            let node = parse_xml(&xml).unwrap();
            color(node.children.first().unwrap())
        };
        assert_eq!(fill("bg1"), Some(0));
        assert_eq!(fill("lt1"), Some(0xFF_FFFF));
    }

    #[test]
    fn a_column_chart_reads_its_series_and_axis() {
        let xml = r#"<?xml version="1.0"?><c:chartSpace xmlns:c="c" xmlns:a="a"><c:chart>
            <c:plotArea><c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>
            <c:ser><c:idx val="0"/><c:tx><c:strRef><c:strCache><c:ptCount val="1"/><c:pt idx="0"><c:v>A &amp; B</c:v></c:pt></c:strCache></c:strRef></c:tx>
            <c:spPr><a:solidFill><a:schemeClr val="accent2"/></a:solidFill></c:spPr>
            <c:val><c:numRef><c:numCache><c:ptCount val="2"/><c:pt idx="1"><c:v>60</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser>
            <c:gapWidth val="219"/></c:barChart>
            <c:valAx><c:scaling><c:min val="30"/></c:scaling><c:delete val="0"/><c:majorGridlines/></c:valAx></c:plotArea>
            <c:legend><c:legendPos val="b"/></c:legend></c:chart></c:chartSpace>"#;
        let c = parse(xml).unwrap();
        let s = &c.plots[0].series[0];
        assert_eq!(s.name, "A & B");
        assert_eq!(s.values, vec![None, Some(60.0)]);
        assert_eq!(s.color, Some(0x3A84FF));
        assert!(matches!(
            c.plots[0].kind,
            PlotKind::Bar {
                gap: 219,
                horizontal: false,
                ..
            }
        ));
        assert_eq!(c.value_axis.unwrap().min, Some(30.0));
        assert_eq!(c.legend.unwrap().position, b'b');
    }
}
