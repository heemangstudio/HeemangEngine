//! Charts: the series, plots, axes and labels of an embedded chart, as
//! OOXML chart parts describe them (`c:chartSpace`).

/// What an OLE object shows: the picture its application saved, or a
/// chart drawn from its data.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum OleView {
    /// A Windows metafile or a device-independent bitmap.
    Picture(crate::SharedBytes),
    Chart(Box<Chart>),
}

#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Chart {
    pub title: Option<ChartText>,
    pub plots: Vec<Plot>,
    pub category_axis: Option<Axis>,
    pub value_axis: Option<Axis>,
    pub legend: Option<Legend>,
    /// The chart area's outline color, when drawn.
    pub border: Option<u32>,
    /// The chart area's fill, when not transparent.
    pub fill: Option<u32>,
    /// Default text properties of the chart.
    pub text: TextStyle,
    pub three_d: bool,
}

/// A run of chart text with its style.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ChartText {
    pub text: String,
    pub style: TextStyle,
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TextStyle {
    /// Size in HU.
    pub size: i32,
    pub bold: bool,
    /// COLORREF (`0x00BBGGRR`).
    pub color: u32,
}

impl Default for TextStyle {
    fn default() -> Self {
        TextStyle {
            size: 1000,
            bold: false,
            color: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Grouping {
    #[default]
    Clustered,
    Stacked,
    Percent,
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum PlotKind {
    /// Columns, or bars when horizontal. Gap and overlap are percents of a
    /// bar's width.
    Bar {
        horizontal: bool,
        grouping: Grouping,
        gap: i32,
        overlap: i32,
    },
    Line {
        grouping: Grouping,
        markers: bool,
    },
    Area {
        grouping: Grouping,
    },
    /// Pies and doughnuts (a hole percent above 0); the first slice starts
    /// at `first_angle` degrees clockwise from the top.
    Pie {
        hole: i32,
        first_angle: i32,
        /// A pie whose last points split off into a second pie or a bar.
        of_pie: Option<OfPie>,
    },
    Scatter {
        lines: bool,
        markers: bool,
        smooth: bool,
    },
}

/// The second plot of a pie of pie or bar of pie.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct OfPie {
    pub bar: bool,
    /// Size of the second plot, percent of the first.
    pub second: i32,
    /// Gap between them, percent of the second plot's width.
    pub gap: i32,
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Plot {
    pub kind: PlotKind,
    pub series: Vec<Series>,
    pub labels: DataLabels,
    /// Stock charts: a line from each category's lowest value to its
    /// highest, and bars from its first series to its last with their gap
    /// as a percent of a bar's width.
    pub hi_low: bool,
    pub up_down: Option<i32>,
}

#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Series {
    pub name: String,
    pub categories: Vec<String>,
    pub values: Vec<Option<f64>>,
    /// Scatter x values.
    pub x: Vec<Option<f64>>,
    /// Fill (or line, for lines) color when given.
    pub color: Option<u32>,
    /// Point colors that override the series color (pies).
    pub point_colors: Vec<(usize, u32)>,
    pub labels: Option<DataLabels>,
    /// Whether points carry markers, when the series says.
    pub markers: Option<bool>,
    /// Whether the line joining the points is drawn (`a:ln` without
    /// `a:noFill`), and its width in HU when given.
    pub line: bool,
    pub line_width: Option<i32>,
    pub smooth: bool,
    /// Pie slices pulled out from the center, as percents of the radius:
    /// the series' own and any point's.
    pub explosion: i32,
    pub point_explosion: Vec<(usize, i32)>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DataLabels {
    pub value: bool,
    pub category: bool,
    pub series: bool,
    pub percent: bool,
    pub style: TextStyle,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Axis {
    pub deleted: bool,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub major_unit: Option<f64>,
    /// Major gridline color, when drawn.
    pub gridlines: Option<u32>,
    /// The axis line color, when drawn.
    pub line: Option<u32>,
    pub labels: TextStyle,
    /// Major tick marks: none, outside, inside or crossing.
    pub ticks: Ticks,
    /// Labels as percents (`0%`).
    pub percent: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Ticks {
    None,
    #[default]
    Out,
    In,
    Cross,
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Legend {
    /// `b`, `t`, `l`, `r` or `tr`.
    pub position: u8,
    pub style: TextStyle,
}
