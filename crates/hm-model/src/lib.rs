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
    Align, BinDataEntry, BorderFill, BorderLine, Bullet, CharShape, CompatibleDocument, DocInfo,
    DocProperties, FaceName, IdMappings, InfoItem, InfoSlot, LayoutCompatibility, LineSpacingKind,
    Numbering, NumberingLevel, ParaHead, ParaShape, SolidFill, Style, TabDef, TabStop,
};
pub use document::{Diagnostic, DocMeta, Document, Format, Section, Version};
pub use para::{
    CharRun, ExtraRecord, LineSeg, PARA_END, ParaHeader, Paragraph, RangeTag, Unit, Units,
    break_kind, control_width, is_extended_control,
};
pub use raw::{CtrlId, HwpString, RawRecord};
pub use script::Script;
pub use units::Hu;
