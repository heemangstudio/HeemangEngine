//! Document model: HWP units, paragraphs, controls, style tables and
//! preservation slots.
//!
//! The model is format-neutral. Text lives in Hancom's wire position space,
//! lengths are integer HWPUNIT, and every parsed node keeps the bytes it was
//! read from so writers copy what did not change.

mod bytes;
mod control;
mod docinfo;
mod document;
mod fill;
mod para;
mod raw;
mod script;
pub mod units;

pub use bytes::SharedBytes;
pub use control::{
    ColumnDef, Control, ControlKind, NoteShape, ObjectCommon, PageBorderFill, PageDef, RawControl,
    SectionChild, SectionDef,
};
pub use docinfo::{
    Align, BinDataEntry, Bullet, CharShape, CompatibleDocument, DocInfo, DocProperties, FaceName,
    IdMappings, InfoItem, InfoSlot, LayoutCompatibility, LineSpacingKind, Numbering,
    NumberingLevel, ParaHead, ParaShape, Style, TabDef, TabStop,
};
pub use document::{Diagnostic, DocMeta, Document, Format, Section, Version};
pub use fill::{BORDER_WIDTHS, BorderFill, BorderLine, Fill, Gradient, ImageFill, SolidFill};
pub use para::{
    CharRun, ExtraRecord, LineSeg, PARA_END, ParaHeader, Paragraph, RangeTag, Unit, Units,
    break_kind, control_width, is_extended_control,
};
pub use raw::{CtrlId, HwpString, RawRecord};
pub use script::Script;
pub use units::Hu;
