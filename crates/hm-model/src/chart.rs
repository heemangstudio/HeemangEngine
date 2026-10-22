//! Charts: the series, plots, axes and labels of an embedded chart, as
//! OOXML chart parts describe them (`c:chartSpace`).

/// What an OLE object shows: the picture its application saved, or a
/// chart drawn from its data.
#[derive(Clone, Debug)]
pub enum OleView {
    /// A Windows metafile or a device-independent bitmap.
    Picture(crate::SharedBytes),
    Chart(Box<Chart>),
}

#[derive(Clone, Debug, Default, PartialEq)]
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
pub struct ChartText {
    pub text: String,
    pub style: TextStyle,
}

#[derive(Clone, Copy, Debug, PartialEq)]
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
            color: 0x0040_4040,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Grouping {
    #[default]
    Clustered,
    Stacked,
    Percent,
}

#[derive(Clone, Copy, Debug, PartialEq)]
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
    },
    Scatter {
        lines: bool,
        markers: bool,
        smooth: bool,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Plot {
    pub kind: PlotKind,
    pub series: Vec<Series>,
    pub labels: DataLabels,
}

#[derive(Clone, Debug, Default, PartialEq)]
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
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DataLabels {
    pub value: bool,
    pub category: bool,
    pub series: bool,
    pub percent: bool,
    pub style: TextStyle,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
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
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Legend {
    /// `b`, `t`, `l`, `r` or `tr`.
    pub position: u8,
    pub style: TextStyle,
}
