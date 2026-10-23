//! Requests and results crossing the boundary as JSON, and errors.
//!
//! The TypeScript declarations in [`TS`] describe the same shapes and are
//! emitted into the generated `.d.ts`.

use hm_edit::{ListPath, Pos, Step};
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

#[wasm_bindgen(typescript_custom_section)]
pub const TS: &str = r#"
/** One step from a paragraph into a list one of its controls holds. */
export interface Step { para: number; control: number; index: number }
/** A wire position: UTF-16 units, controls take 8. */
export interface Address { section: number; steps?: Step[]; para: number; offset: number }
export type Affinity = "downstream" | "upstream";
/** Page HU (7200 per inch). */
export interface Caret { page: number; x: number; y: number; height: number }
/** A floating object or table of a page; `control` indexes its paragraph's controls. */
export interface ObjectTarget {
  section: number; steps?: Step[]; para: number; control: number; kind: string;
  layer: "behind" | "text" | "front"; page: number; x: number; y: number; width: number; height: number;
}
/** `object` is set when the point picks a floating object rather than text. */
export interface Located { position: Address; affinity: Affinity; caret: Caret; object?: ObjectTarget }
export type LocateRequest =
  | { kind: "point"; page: number; x: number; y: number }
  | { kind: "caret"; position: Address; affinity?: Affinity; page?: number };
export interface Rect { page: number; x: number; y: number; width: number; height: number }
export interface SelectRequest { from: Address; to: Address; page?: number }
export type Motion = "left" | "right" | "lineStart" | "lineEnd" | "up" | "down";
export interface NavigateRequest { position: Address; affinity?: Affinity; motion: Motion; preferredX?: number; page?: number }
export interface CharPatch { bold?: boolean; italic?: boolean; underline?: boolean; size?: number; color?: number }
export type Align = "justify" | "left" | "right" | "center" | "distribute" | "divide";
export interface ParaPatch { align?: Align; spaceBefore?: number; spaceAfter?: number; left?: number; indent?: number; lineSpacing?: number }
export type EditOp =
  | { op: "insertText"; at: Address; text: string }
  | { op: "delete"; from: Address; to: Address }
  | { op: "split"; at: Address; pageBreak?: boolean }
  | { op: "formatChars"; from: Address; to: Address; patch: CharPatch }
  | { op: "formatParas"; from: Address; to: Address; patch: ParaPatch }
  | { op: "pageBreak"; at: Address; on: boolean };
export interface TransactRequest { expectedEpoch?: number; ops: EditOp[] }
export interface Receipt { contentEpoch: number; layoutEpoch: number; pageCount: number; caret?: Address; undo: number; redo: number }
export interface DocState {
  instance: number; contentEpoch: number; layoutEpoch: number; fontGeneration: number;
  format: "hwp5" | "hwpx"; pageCount: number; dirty: boolean; undo: number; redo: number;
}
export interface PageInfo { index: number; section: number; width: number; height: number; number: number }
export interface Capabilities {
  contract: number; packet: number; open: string[]; serialize: string[];
  scopes: string[]; ops: string[]; locate: string[]; motions: string[]; query: string[]; missing: string[];
}
export interface ListAddress { section: number; steps?: Step[] }
export interface QueryRequest { kind: "paragraphs"; list: ListAddress; from?: number; count?: number }
export interface ControlInfo { index: number; offset: number; kind: string; lists: number }
export interface ParagraphInfo { index: number; text: string; length: number; caret: [number, number]; controls: ControlInfo[] }
export interface SearchRequest {
  text: string; caseSensitive?: boolean; list?: ListAddress; limit?: number;
  /** With `from`, the one match after it (before it with `backward`), wrapping around. */
  from?: Address; backward?: boolean;
}
export interface Match { from: Address; to: Address }
export interface SearchResult { contentEpoch: number; matches: Match[] }
export interface FaceRequest { name: string; candidates: string[]; standIns: string[] }
export interface FontRequests { faces: FaceRequest[]; classDefaults: string[][] }
export interface ComposeRequest { at?: Address; text?: string }
export interface CopyRequest { from: Address; to: Address }
export interface Copied { text: string; html: string }
export interface PasteRequest { at: Address; text?: string; expectedEpoch?: number }
export type ErrorCode =
  | "MALFORMED_INPUT" | "UNSUPPORTED_FORMAT" | "BAD_REQUEST" | "BAD_ADDRESS"
  | "STALE_EPOCH" | "EDIT_FAILED" | "NO_SUCH_PAGE" | "CLOSED" | "SAVE_FAILED"
  | "COMPOSING" | "STALE_HANDLE";
"#;

/// A failure JS can tell apart by `code`.
#[wasm_bindgen]
#[derive(Debug)]
pub struct EngineError {
    code: &'static str,
    message: String,
}

#[wasm_bindgen]
impl EngineError {
    #[wasm_bindgen(getter)]
    pub fn code(&self) -> String {
        self.code.to_string()
    }

    #[wasm_bindgen(getter)]
    pub fn message(&self) -> String {
        self.message.clone()
    }

    #[wasm_bindgen(js_name = toString)]
    pub fn to_js_string(&self) -> String {
        format!("{}: {}", self.code, self.message)
    }
}

impl EngineError {
    pub fn new(code: &'static str, message: impl std::fmt::Display) -> EngineError {
        EngineError {
            code,
            message: message.to_string(),
        }
    }

    pub fn code_str(&self) -> &'static str {
        self.code
    }
}

pub type Result<T> = std::result::Result<T, EngineError>;

pub fn parse<'a, T: Deserialize<'a>>(json: &'a str) -> Result<T> {
    serde_json::from_str(json).map_err(|e| EngineError::new("BAD_REQUEST", e))
}

pub fn json<T: Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_default()
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct StepJs {
    pub para: u32,
    pub control: u32,
    pub index: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Address {
    pub section: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<StepJs>,
    pub para: u32,
    pub offset: u32,
}

impl Address {
    pub fn pos(&self) -> Pos {
        Pos {
            list: ListPath {
                section: self.section,
                steps: self
                    .steps
                    .iter()
                    .map(|s| Step {
                        para: s.para,
                        control: s.control,
                        index: s.index,
                    })
                    .collect(),
            },
            para: self.para,
            offset: self.offset,
        }
    }

    pub fn of(p: &Pos) -> Address {
        Address {
            section: p.list.section,
            steps: p
                .list
                .steps
                .iter()
                .map(|s| StepJs {
                    para: s.para,
                    control: s.control,
                    index: s.index,
                })
                .collect(),
            para: p.para,
            offset: p.offset,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Affinity {
    #[default]
    Downstream,
    Upstream,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub struct Caret {
    pub page: u32,
    pub x: i32,
    pub y: i32,
    pub height: i32,
}

#[derive(Clone, Debug, Serialize)]
pub struct ObjectTarget {
    pub section: u32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<StepJs>,
    pub para: u32,
    pub control: u32,
    pub kind: &'static str,
    pub layer: &'static str,
    pub page: u32,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Serialize)]
pub struct Located {
    pub position: Address,
    pub affinity: Affinity,
    pub caret: Caret,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object: Option<ObjectTarget>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum LocateRequest {
    Point {
        page: u32,
        x: f64,
        y: f64,
    },
    Caret {
        position: Address,
        #[serde(default)]
        affinity: Affinity,
        page: Option<u32>,
    },
}

#[derive(Debug, Serialize)]
pub struct Rect {
    pub page: u32,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Deserialize)]
pub struct SelectRequest {
    pub from: Address,
    pub to: Address,
    pub page: Option<u32>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Motion {
    Left,
    Right,
    LineStart,
    LineEnd,
    Up,
    Down,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NavigateRequest {
    pub position: Address,
    #[serde(default)]
    pub affinity: Affinity,
    pub motion: Motion,
    pub preferred_x: Option<f64>,
    pub page: Option<u32>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
pub struct CharPatchJs {
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub size: Option<i32>,
    pub color: Option<u32>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParaPatchJs {
    pub align: Option<String>,
    pub space_before: Option<i32>,
    pub space_after: Option<i32>,
    pub left: Option<i32>,
    pub indent: Option<i32>,
    pub line_spacing: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum EditOp {
    InsertText {
        at: Address,
        text: String,
    },
    Delete {
        from: Address,
        to: Address,
    },
    #[serde(rename_all = "camelCase")]
    Split {
        at: Address,
        #[serde(default)]
        page_break: bool,
    },
    FormatChars {
        from: Address,
        to: Address,
        patch: CharPatchJs,
    },
    FormatParas {
        from: Address,
        to: Address,
        patch: ParaPatchJs,
    },
    PageBreak {
        at: Address,
        on: bool,
    },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactRequest {
    pub expected_epoch: Option<u32>,
    pub ops: Vec<EditOp>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Receipt {
    pub content_epoch: u32,
    pub layout_epoch: u32,
    pub page_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caret: Option<Address>,
    pub undo: usize,
    pub redo: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocState {
    pub instance: u32,
    pub content_epoch: u32,
    pub layout_epoch: u32,
    pub font_generation: u32,
    pub format: &'static str,
    pub page_count: usize,
    pub dirty: bool,
    pub undo: usize,
    pub redo: usize,
}

#[derive(Debug, Serialize)]
pub struct PageInfo {
    pub index: usize,
    pub section: u32,
    pub width: i32,
    pub height: i32,
    pub number: u32,
}

#[derive(Debug, Deserialize)]
pub struct ListAddress {
    pub section: u32,
    #[serde(default)]
    pub steps: Vec<StepJs>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum QueryRequest {
    Paragraphs {
        list: ListAddress,
        #[serde(default)]
        from: u32,
        count: Option<u32>,
    },
}

#[derive(Debug, Serialize)]
pub struct ControlInfo {
    pub index: u32,
    pub offset: u32,
    pub kind: &'static str,
    pub lists: usize,
}

#[derive(Debug, Serialize)]
pub struct ParagraphInfo {
    pub index: u32,
    pub text: String,
    pub length: u32,
    pub caret: [u32; 2],
    pub controls: Vec<ControlInfo>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchRequest {
    pub text: String,
    #[serde(default)]
    pub case_sensitive: bool,
    pub list: Option<ListAddress>,
    pub limit: Option<usize>,
    pub from: Option<Address>,
    #[serde(default)]
    pub backward: bool,
}

#[derive(Debug, Serialize)]
pub struct Match {
    pub from: Address,
    pub to: Address,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub content_epoch: u32,
    pub matches: Vec<Match>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FaceRequest {
    pub name: String,
    pub candidates: Vec<String>,
    pub stand_ins: Vec<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FontRequests {
    pub faces: Vec<FaceRequest>,
    pub class_defaults: Vec<Vec<String>>,
    /// Families not registered that the last layout looked in for
    /// characters a face lacked, in the order it tried them.
    #[serde(default)]
    pub fallbacks: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct ComposeRequest {
    pub at: Option<Address>,
    #[serde(default)]
    pub text: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CopyRequest {
    pub from: Address,
    pub to: Address,
}

#[derive(Debug, Serialize)]
pub struct Copied {
    pub text: String,
    pub html: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PasteRequest {
    pub at: Address,
    pub text: Option<String>,
    pub expected_epoch: Option<u32>,
}
