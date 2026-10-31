//! Document model: HWP units, paragraphs, controls, style tables and
//! preservation slots.
//!
//! The model is format-neutral. Text lives in Hancom's wire position space,
//! lengths are integer HWPUNIT, and every parsed node keeps the bytes it was
//! read from so writers copy what did not change.

mod annot;
mod bytes;
pub mod chart;
mod control;
mod convert;
mod docinfo;
mod document;
mod fill;
mod furniture;
pub mod notes;
mod object;
mod para;
mod raw;
mod script;
mod table;
pub mod units;

pub use annot::{
    Bookmark, Compose, ComposeFrame, Dutmal, Field, FieldKind, FormKind, FormObject, IndexMark,
    PROOFREADING, ParamValue, Params, TrackKind, TrackMark, deleted_ranges,
};
pub use bytes::SharedBytes;
pub use control::{
    ColumnDef, Control, ControlKind, LineNumbers, NoteShape, ObjectCommon, PageBorderFill, PageDef,
    RawControl, SectionChild, SectionDef,
};
pub use docinfo::{
    Align, BinDataEntry, Bullet, CharShape, CompatibleDocument, DocInfo, DocProperties, FaceName,
    IdMappings, ImportedLineMembership, InfoItem, InfoSlot, LayoutCompatibility, LineSpacingKind,
    Numbering, NumberingLevel, ParaHead, ParaShape, Style, TabDef, TabStop,
};
pub use document::{BinReader, BinStore, Diagnostic, DocMeta, Document, Format, Section, Version};
pub use fill::{BORDER_WIDTHS, BorderFill, BorderLine, Fill, Gradient, ImageFill, SolidFill};
pub use furniture::{
    AutoNumber, HeaderFooter, NewNumber, Note, NumberKind, PageHide, PageNumberPos, PageParity,
    PageSide, ParaList, hide,
};
pub use object::{
    ArcGeom, Arrow, Caption, Connector, CurveGeom, EllipseGeom, Equation, ImageRef, LineGeom,
    LineStyle, Matrix, ObjectControl, ObjectHeader, ObjectPart, Ole, Picture, PictureEffects,
    PictureGlow, PictureReflection, PictureShadow, PolyGeom, RectGeom, Shadow, Shape,
    ShapeComponent, ShapePart, ShapeStyle, TextBox,
};
pub use para::{
    CharRun, ExtraRecord, LineSeg, ListRestart, PARA_END, ParaHeader, Paragraph, RangeTag, Unit,
    Units, break_kind, control_width, is_extended_control,
};
pub use raw::{CtrlId, HwpString, RawRecord};
pub use script::Script;
pub use table::{Cell, CellZone, Table};
pub use units::Hu;
