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
  | { kind: "point"; page: number; x: number; y: number; body?: boolean; within?: ListAddress }
  | { kind: "caret"; position: Address; affinity?: Affinity; page?: number };
export interface Rect { page: number; x: number; y: number; width: number; height: number }
/** A table control in a paragraph list. */
export interface TableAddress { section: number; steps?: Step[]; para: number; control: number }
export interface CellRef { row: number; column: number }
/**
 * Ends in different lists select by their nearest common list (an end inside
 * a control takes the whole control), or, in two cells of one table, the block
 * of cells between them.
 */
export type SelectRequest =
  | { from: Address; to: Address; page?: number }
  | { kind: "cells"; table: TableAddress; from: CellRef; to: CellRef; page?: number };
export type SelectedRange =
  | { kind: "text"; from: Address; to: Address }
  | { kind: "cells"; table: TableAddress; from: CellRef; to: CellRef; cells: { row: number; column: number; rowSpan: number; columnSpan: number; list: ListAddress }[] };
export interface Selected { range: SelectedRange; rects: Rect[] }
export type Motion = "left" | "right" | "lineStart" | "lineEnd" | "up" | "down";
export interface NavigateRequest { position: Address; affinity?: Affinity; motion: Motion; preferredX?: number; page?: number }
/**
 * \`size\` in HU (1000 = 10 pt); \`color\` a COLORREF; \`fontFace\` a family for
 * all seven script slots, which must draw with a registered face (its own
 * family or one Hancom draws in its place); \`fontFaces\` a family per
 * slot (Hangul, Latin, Hanja, Japanese, other, symbol, user), null keeping the
 * slot's, read back with all seven; \`charSpacing\` a percent of the character
 * size (-50 to 50) for all slots. \`relSizes\` (10 to 250) and \`offsets\`
 * (-100 to 100) are percents per slot; line shapes, \`outline\` (0 to 6),
 * \`shadow\` (0 none, 1 discrete, 2 continuous) and \`emphasis\` (0 to 12) are
 * HWP codes; \`shadeColor\` 0xFFFFFFFF is no shading.
 */
export interface CharPatch {
  bold?: boolean; italic?: boolean; underline?: boolean; strike?: boolean; size?: number; color?: number;
  fontFace?: string; fontFaces?: (string | null)[]; charSpacing?: number; spacings?: (number | null)[]; ratios?: (number | null)[];
  relSizes?: (number | null)[]; offsets?: (number | null)[]; superscript?: boolean; subscript?: boolean; emboss?: boolean; engrave?: boolean;
  kerning?: boolean; underlineKind?: "none" | "below" | "above"; underlineShape?: number; underlineColor?: number; strikeShape?: number;
  strikeColor?: number; outline?: number; shadow?: number; shadowColor?: number; shadowOffset?: [number, number]; emphasis?: number;
  shadeColor?: number;
}
export type Align = "justify" | "left" | "right" | "center" | "distribute" | "divide";
export interface ParaPatch { align?: Align; spaceBefore?: number; spaceAfter?: number; left?: number; right?: number; indent?: number; lineSpacing?: number; lineSpacingKind?: "percent" | "fixed" | "added" | "atLeast" }
export interface StylePatch { characters?: CharPatch; paragraphs?: ParaPatch; name?: string; englishName?: string; kind?: "paragraph" | "character"; next?: number; charShapeId?: number; paraShapeId?: number }
export type ListDefinition = { kind: "bullet"; bullet: string } | { kind: "numbering"; start?: number; levels: { format: string; numberKind?: number; start?: number }[] };
export interface FontFace { id: number; name: string }
export interface ListSpec { kind: "bullet" | "numbering"; id: number; level?: number; restart?: number }
export interface PageSettingsPatch { width?: number; height?: number; marginLeft?: number; marginRight?: number; marginTop?: number; marginBottom?: number; marginHeader?: number; marginFooter?: number; gutter?: number; landscape?: boolean; binding?: number }
export interface SectionSettingsPatch { attr?: number; columnGap?: number; tabStop?: number; outlineNumbering?: number; pageNumber?: number; figureNumber?: number; tableNumber?: number; equationNumber?: number; language?: number }
export interface ColumnSettingsPatch { count?: number; kind?: number; direction?: number; equalWidths?: boolean; gap?: number; columns?: [number, number][]; lineKind?: number; lineWidth?: number; lineColor?: number }
export interface NoteShapePatch { attr?: number; userChar?: number; prefix?: number; suffix?: number; start?: number; separatorLength?: number; spaceAbove?: number; spaceBelow?: number; spaceBetween?: number; lineKind?: number; lineWidth?: number; lineColor?: number }
export interface FieldPatch { command?: string; name?: string; editable?: boolean; value?: string }
export interface DocumentPropertiesPatch { pageStart?: number; footnoteStart?: number; endnoteStart?: number; figureStart?: number; tableStart?: number; equationStart?: number; caretList?: number; caretPara?: number; caretPos?: number }
export interface BorderLineSpec { kind: number; width: number; color: number }
export type PageFill = { kind: "none" } | { kind: "solid"; color: number; patternColor?: number; pattern?: number } | { kind: "gradient" | "image" };
/** `lines` (left, right, top, bottom) and `fill` derive a border fill from the current one; queries report both. */
export interface PageBorderPatch { attr?: number; offsets?: [number, number, number, number]; borderFill?: number; lines?: [BorderLineSpec, BorderLineSpec, BorderLineSpec, BorderLineSpec]; fill?: PageFill }
export type Side = "both" | "even" | "odd";
export type NoteKind = "footnote" | "endnote";
export type FieldKind = "clickHere" | "hyperlink" | "bookmark" | "date" | "time" | "fileName" | "title" | "pageNumber" | "pageCount";
export type ControlAddress = TableAddress;
export interface SectionSettings {section:number;page:PageSettingsPatch|null;settings:SectionSettingsPatch|null;columns:ColumnSettingsPatch|null;noteShapes:{footnote:NoteShapePatch|null;endnote:NoteShapePatch|null};pageBorders:PageBorderPatch[]}
/** A form object's state. `value` is a check box or radio button's 0 off, 1 on, 2 indeterminate; `text` an edit or combo box's entry. `properties` flattens every property set, first name wins. */
export interface FormRecord {formType:"PushButton"|"CheckBox"|"RadioButton"|"ComboBox"|"ListBox"|"Edit"|"ScrollBar"|"Unknown";name:string;value:number;text:string;caption:string;enabled:boolean;width:number;height:number;foreColor:number;backColor:number;properties:Record<string,string>;items:string[]}
export interface ControlRecord {target:ControlAddress;offset:number;kind:string;lists:ListAddress[];side?:Side;number?:number;flags?:number;fieldId?:number;fieldKind?:FieldKind;name?:string;command?:string;guide?:string;editable?:boolean;value?:string;end?:Address;form?:FormRecord}
/** HU throughout, including natural image dimensions. Patches reject unknown fields. */
export interface ObjectPatch {
  width?:number; height?:number; horzOffset?:number; vertOffset?:number; treatAsChar?:boolean;
  vertRelTo?:"paper"|"page"|"para"; horzRelTo?:"paper"|"page"|"column"|"para";
  vertAlign?:"top"|"center"|"bottom"|"inside"|"outside"; horzAlign?:"left"|"center"|"right"|"inside"|"outside";
  textWrap?:"square"|"topAndBottom"|"behind"|"inFront"; margins?:[number,number,number,number];
  description?:string; rotation?:number; flipHorizontal?:boolean; flipVertical?:boolean; zOrder?:number; sizeProtect?:boolean;
  restrictInPage?:boolean; allowOverlap?:boolean; keepWithAnchor?:boolean;
  border?:{color?:number;width?:number;style?:number}; crop?:[number,number,number,number];
  brightness?:number; contrast?:number; effect?:number; transparency?:number; pictureMargins?:[number,number,number,number];
  fill?:FillPatch; fillAlpha?:number;
}
export interface LinePatch { style:number; width:number; color:number }
export type FillPatch = { kind:"none" } | { kind:"solid"; background:number; patternColor?:number; pattern?:number };
export interface BorderFillPatch {
  left?:LinePatch; right?:LinePatch; top?:LinePatch; bottom?:LinePatch; diagonal?:LinePatch;
  slash?:number; backslash?:number; centerLine?:"none"|"horizontal"|"vertical"|"cross"; fill?:FillPatch;
}
export interface TablePatch { repeatHeader?:boolean; split?:"none"|"row"|"cell"; spacing?:number; padding?:[number,number,number,number]; borderFill?:BorderFillPatch }
export interface CellPatch { width?:number; height?:number; padding?:[number,number,number,number]; verticalAlign?:"top"|"center"|"bottom"; ownPadding?:boolean; protected?:boolean; header?:boolean; textDirection?:number; editableInForm?:boolean; borderFill?:BorderFillPatch }
export interface ObjectProperties extends ObjectPatch {
  kind:string; text?:string; size?:[number,number]; pictureMargins?:[number,number,number,number];
  brightness?:number; contrast?:number; effect?:number; transparency?:number; naturalWidth?:number; naturalHeight?:number;
  fill?:{kind:number;color:number|null;solid:unknown;gradient:unknown;image:unknown;alpha:number[]};
  /** Drawings and equations: null without a caption; `list` is the caption's list index in the object's steps. */
  caption?:{side:"left"|"right"|"top"|"bottom";includeMargin:boolean;width:number;gap:number;maxWidth:number;paragraphs:number;list:number}|null;
}
/** `caption` is null without a caption; `zones` are the table's cell zones, each with the border fill it applies. */
export interface TableProperties extends TablePatch {
  rows:number; columns:number; borderFill:unknown;
  caption:{side:"left"|"right"|"top"|"bottom";includeMargin:boolean;width:number;gap:number;maxWidth:number;paragraphs:number}|null;
  zones:{startRow:number;startColumn:number;endRow:number;endColumn:number;borderFill:unknown}[];
}
export interface CellProperties extends CellPatch { row:number; column:number; rowSpan:number; columnSpan:number; tablePadding:number[]; borderFill:unknown }
export interface PictureData { bytes:number[]; extension:string; mime:string; naturalWidth:number; naturalHeight:number }
/** Equation fields: size in HU (1000 = 10 pt), baseline a percent of the height. */
export interface EquationPatch { script?: string; size?: number; color?: number; baseline?: number; font?: string }
export interface EquationInfo { script: string; size: number; color: number; baseline: number; font: string | null; width: number; height: number }
export interface EquationPreview { svg: string; widthPx: number; heightPx: number; baselinePx: number; warnings: string[] }
export type EditOp =
  | { op: "setCharShape"; from: Address; to: Address; charShapeId: number }
  | { op: "setParaShape"; from: Address; to: Address; paraShapeId: number }
  | { op: "createStyle"; style: StylePatch }
  | { op: "updateStyle"; id: number; style: StylePatch }
  | { op: "deleteStyle"; id: number }
  | { op: "ensureList"; definition: ListDefinition }
  | {op:"setPageSettings";section:number;patch:PageSettingsPatch}
  | {op:"setSectionSettings";section:number;patch:SectionSettingsPatch}
  | {op:"setColumns";at:Address;patch:ColumnSettingsPatch}
  | {op:"setNoteShape";section:number;kind:NoteKind;patch:NoteShapePatch}
  | {op:"setPageBorder";section:number;side:Side;patch:PageBorderPatch}
  | {op:"setPageHide";at:Address;flags:number}
  | {op:"createHeaderFooter";section:number;kind:"header"|"footer";side:Side}
  | {op:"insertNote";at:Address;kind:NoteKind}
  /** Gives every note the number Hancom shows. Runs on its own after edits that add, remove or restart notes. */
  | {op:"renumberNotes"}
  | {op:"insertField";at:Address;kind:FieldKind;to?:Address;command?:string;name?:string;editable?:boolean;text?:string}
  | {op:"updateField";target:ControlAddress;patch:FieldPatch}
  | {op:"insertBookmark";at:Address;name:string}
  | {op:"renameBookmark";target:ControlAddress;name:string}
  | {op:"deleteControl";target:ControlAddress}
  | {op:"setFormValue";target:ControlAddress;value?:number;text?:string}
  | {op:"setDocumentProperties";patch:DocumentPropertiesPatch}
  | {op:"columnBreak";at:Address}
  | {op:"insertNewNumber";at:Address;number:number}
  | { op:"createTable"; at:Address; rows:number; columns:number; columnWidths?:number[]; rowHeights?:number[]; inline?:boolean }
  | { op:"insertPicture"; at:Address; bytes:number[]; extension:string; description?:string; width:number; height:number; naturalWidth?:number; naturalHeight?:number; inline?:boolean; horzOffset?:number; vertOffset?:number }
  | { op:"assignPictureImage"; object:ObjectAddress; bytes:number[]; extension:string; naturalWidth:number; naturalHeight:number }
  | { op:"createShape"; at:Address; kind:"rect"|"ellipse"|"line"|"textbox"; connector?:ConnectorSpec; width:number; height:number; text?:string; inline?:boolean; horzOffset?:number; vertOffset?:number }
  | { op:"deleteObject"; object:ObjectAddress }
  | { op:"relocateObject"; object:ObjectAddress; at:Address }
  | { op:"setLineEnds"; object:ObjectAddress; start:[number,number]; end:[number,number] }
  | { op:"groupObjects"; objects:ObjectAddress[] }
  | { op:"ungroupObject"; object:ObjectAddress }
  | { op:"setObjectProperties"; object:ObjectAddress; patch:ObjectPatch }
  | { op:"setTableProperties"; table:TableAddress; patch:TablePatch }
  | { op:"setCellProperties"; table:TableAddress; row:number; column:number; patch:CellPatch }
  | { op:"setCellZone"; table:TableAddress; startRow:number; startColumn:number; endRow:number; endColumn:number; borderFill:BorderFillPatch }
  | { op: "insertEquation"; at: Address; script: string; size?: number; color?: number; font?: string; baseline?: number }
  | { op: "setEquation"; object: TableAddress; patch: EquationPatch }
  | { op: "setTableCaption"; table: TableAddress; text: string; withNumber?: boolean }
  /** A picture, drawing or equation caption; "그림 N" or "수식 N" first with `withNumber`. Created below the object when missing. */
  | { op: "setObjectCaption"; object: ObjectAddress; text: string; withNumber?: boolean }
  | { op: "insertText"; at: Address; text: string; format?: CharPatch }
  | { op: "delete"; from: Address; to: Address }
  | { op: "split"; at: Address; pageBreak?: boolean }
  | { op: "formatChars"; from: Address; to: Address; patch: CharPatch }
  | { op: "formatParas"; from: Address; to: Address; patch: ParaPatch }
  | { op: "pageBreak"; at: Address; on: boolean }
  | { op: "applyStyle"; from: Address; to: Address; styleId: number }
  | { op: "insertTableRows"; table: TableAddress; beforeRow: number; count: number }
  | { op: "deleteTableRows"; table: TableAddress; startRow: number; count: number }
  | { op: "insertTableColumns"; table: TableAddress; beforeColumn: number; count: number }
  | { op: "deleteTableColumns"; table: TableAddress; startColumn: number; count: number }
  | { op: "mergeTableCells"; table: TableAddress; startRow: number; startColumn: number; endRow: number; endColumn: number }
  | { op: "splitTableCell"; table: TableAddress; row: number; column: number; rows: number; columns: number; equalRowHeight: boolean }
  | { op: "setList"; from: Address; to: Address; list: ListSpec | null }
  | { op: "moveObject"; object: ObjectAddress; dx: number; dy: number }
  | { op: "resizeObject"; object: ObjectAddress; width: number; height: number }
  | { op: "resizeTableCells"; table: TableAddress; cells: CellResize[] };
/** An object control in a paragraph list (a table's address names it too). */
export type ObjectAddress = TableAddress;
/** HU added to a cell's laid-out width and height; `index` is the cell's in the table. */
export interface CellResize { index: number; widthDelta?: number; heightDelta?: number }
/**
 * An object or table a page draws (`Doc.pageObjects`), bottom to top. The box
 * excludes outer margins; a table split over pages gives this page's part.
 * \`line\` holds a line's start and end on the page.
 */
export interface PageObject {
  section: number; steps?: Step[]; para: number; control: number;
  kind: "table" | "picture" | "shape" | "line" | "connector" | "group" | "ole" | "equation" | "form" | "object";
  page: number; x: number; y: number; width: number; height: number;
  inline: boolean; layer: "behind" | "text" | "front"; z: number;
  furniture: boolean; locked: boolean; text: boolean; missing?: boolean; rotation?: number; line?: [number, number, number, number];
}
/** A directed selection the host keeps with an undo step. */
export type Selection =
  | { anchor: Address; focus: Address; anchorAffinity: Affinity; focusAffinity: Affinity }
  | { kind: "cells"; table: TableAddress; from: CellRef; to: CellRef };
export interface TransactRequest { expectedEpoch?: number; ops: EditOp[]; selection?: Selection; afterSelection?: Selection }
/**
 * \`changedPages\`: pages (zero-based, as now laid out, plus pages that no
 * longer exist) to paint again; null for all. \`selection\`: after a
 * transaction, the collapsed caret; after undo, the selection from before the
 * step; after redo, the one after it. \`historyId\` names the history position.
 */
export interface Receipt {
  contentEpoch: number; layoutEpoch: number; pageCount: number; caret?: Address; undo: number; redo: number;
  changedPages: number[] | null; selection?: Selection | null; historyId: number; tableChanges?: TableChange[];
}
/** Ordered per operation. Removed cells map to null; merged cells to their survivor.
 * All cell list addresses in the table expire after structural edits or undo/redo.
 * Requery the table to resolve a mapped origin to its new list. */
export interface TableChange { table: TableAddress; cells: { from: CellRef; to: CellRef | null; index: number | null; paragraphShift: number }[] }
export interface TableCell extends CellRef { rowSpan: number; columnSpan: number; list: ListAddress; rects: Rect[] }
export interface TableInfo { table: TableAddress; rows: number; columns: number; cells: TableCell[]; rects: Rect[] }
export interface DocState {
  instance: number; contentEpoch: number; layoutEpoch: number; fontGeneration: number;
  format: "hwp5" | "hwpx" | "hwp3" | "hml"; pageCount: number; dirty: boolean; undo: number; redo: number;
}
export interface Area { x: number; y: number; width: number; height: number }
/** A picture to decode with the browser and draw in place of a patch. */
export interface PictureSource {
  key: string; mime: string; width: number; height: number;
  matrix: [number, number, number, number, number, number];
  clip: [number, number][] | null;
}
export interface Margins { left: number; right: number; top: number; bottom: number; header: number; footer: number; gutter: number }
export interface PageInfo {
  index: number; section: number; width: number; height: number; number: number; landscape: boolean;
  margins: Margins; body: Area; header: Area; footer: Area; columns: Area[];
}
export interface Capabilities {
  contract: number; packet: number; pageInfo: boolean; equationScripts: boolean; paragraphCaptures: boolean; tableTranspose: boolean; connectors: boolean; open: string[]; serialize: string[];
  scopes: string[]; ops: string[]; paste: string[]; locate: string[]; motions: string[]; query: string[]; missing: string[];
}
export interface ListAddress { section: number; steps?: Step[] }
export type QueryRequest =
  | { kind: "fonts" }
  | { kind: "fontCandidates"; names: string[] }
  | { kind: "charShape"; id: number }
  | { kind: "paraShape"; id: number }
  | {kind:"sectionSettings";section:number}
  | {kind:"noteShape";section:number;noteKind:NoteKind}
  | {kind:"documentProperties"}
  | {kind:"controls"|"fields";list?:ListAddress}
  | {kind:"objectProperties";object:ObjectAddress}
  | {kind:"tableProperties";table:TableAddress}
  | {kind:"cellProperties";table:TableAddress;row:number;column:number}
  | {kind:"pictureData";object:ObjectAddress}
  | { kind: "equation"; object: TableAddress }
  | { kind: "equationPreview"; script: string; size: number; color?: number; font?: string }
  | { kind: "table"; table: TableAddress; page?: number }
  | { kind: "paragraphs"; list: ListAddress; from?: number; count?: number }
  | { kind: "formatContext"; at: Address; to?: Address }
  | { kind: "styles"; from?: number; count?: number }
  | { kind: "style"; id: number }
  | { kind: "lists" }
  | { kind: "list"; listKind: "bullet" | "numbering"; id: number }
  | { kind: "count"; list?: ListAddress; from?: Address; to?: Address }
  | { kind: "formatRuns"; list: ListAddress; from?: number; count?: number }
  | { kind: "lines"; list: ListAddress; para: number };
/**
 * Characters as Hancom's status bar counts them (\`count\`): one per code
 * point, spaces, tabs and fixed spaces included, line breaks, numbers and
 * objects not; with each field's end, and the text of the tables, text
 * boxes, captions, headers, footers and notes inside. The whole document's
 * body with no \`list\` or range, one list with all it holds, or \`[from, to)\`
 * with what the controls in it hold.
 */
export interface CharacterCount { characters: number }
/** A paragraph's formatting (\`formatRuns\`): runs start at a wire offset. */
export interface ParagraphFormat {
  index: number; styleId: number; paraShapeId: number; paragraphs: ParaPatch; list: ListRef | null;
  runs: { offset: number; charShapeId: number; characters: CharPatch }[];
  /** The paragraph starts a new page or column. */
  pageBreak: boolean; columnBreak: boolean;
}
/** A paragraph's laid-out line (\`lines\` query): page HU and the wire range it shows. */
export interface LineInfo { line: number; page: number; y: number; height: number; baseline: number; start: number; end: number; left: number; right: number }

export interface ListRef { kind: "bullet" | "numbering" | "outline"; id: number; level: number }
/**
 * Values at \`at\` (for a caret, what typing there takes: the shape of the
 * unit before it, or the paragraph's first at its start); \`mixed\` names the
 * properties (\`characters.bold\`, \`styleId\`, \`list\`, ...) that differ over
 * the range. \`lineSpacingKind\` identifies the spacing units.
 */
export interface FormatContext { characters: CharPatch; paragraphs: ParaPatch; styleId: number; list: ListRef | null; mixed: string[] }
export interface StyleInfo {
  id: number; name: string; englishName: string; kind: "paragraph" | "character"; next: number;
  paraShape: number; charShape: number; characters: CharPatch; paragraphs: ParaPatch;
}
export interface ListLevel { format: string; numberKind: number; start: number }
export interface ListInfo { kind: "bullet" | "numbering"; id: number; bullet?: string; start?: number; levels?: ListLevel[] }
export interface ControlInfo {
  index: number; offset: number; kind: string; lists: number;
  /** Authored equation source, including an empty string; absent on other controls. */
  script?: string;
  /** Objects, tables and form objects: whether it sits in the text like a character. */
  inline?: boolean;
  /** Headers and footers: the pages they apply to. */
  side?: "both" | "even" | "odd";
  /** Notes: the number Hancom last assigned. */
  number?: number;
}
export interface ParagraphInfo {
  index: number;
  /** Characters, tabs, line breaks (\n) and the two fixed spaces (U+00A0, U+2007). */
  text: string;
  /** The wire offset of each UTF-16 unit of `text`, then the end mark's. */
  offsets: number[];
  length: number; caret: [number, number]; controls: ControlInfo[];
}
export interface SearchRequest {
  text: string; caseSensitive?: boolean; list?: ListAddress; limit?: number;
  /** With `from`, the one match after it (before it with `backward`), wrapping around. */
  from?: Address; backward?: boolean;
}
export interface Match { from: Address; to: Address }
export interface SearchResult { contentEpoch: number; matches: Match[] }
export interface FaceRequest { name: string; candidates: string[]; standIns: string[] }
/** A family a host may set (\`fontCandidates\`): what to register for it, and whether text set in it already draws with a registered face. */
export interface FaceCandidates extends FaceRequest { resolves: boolean }
export interface FontRequests { faces: FaceRequest[]; classDefaults: string[][]; fallbacks: string[] }
/** A font file the host holds, read in pieces (`Engine.addFontFile`). */
export interface FontFileSource { readonly size: number; read(offset: number, length: number): Uint8Array }
export interface ComposeRequest { at?: Address; text?: string }
export interface ConnectorSpec { kind: number; start: [number,number]; end: [number,number] }
export type TableTransposeRequest =
  | {action:"copy";transpose?:boolean;table:TableAddress;startRow:number;startColumn:number;endRow:number;endColumn:number}
  | {action:"paste";transpose?:boolean;table:TableAddress;startRow:number;startColumn:number}
  | {action:"inPlace";table:TableAddress}
  | {action:"insert";at:Address};
export interface TableTransposeResult { ok:boolean; sourceRows:number; sourceCols:number; targetRows:number; targetCols:number; receipt:Receipt|null; paraIdx?:number; controlIdx?:number; text?:string; html?:string }
export interface CopyRequest { from: Address; to: Address }
/** `objects`: tables and objects carried; they paste back only into the document copied from. */
export interface Copied { text: string; html: string; objects: number }
/** Exactly one of `at` and `replace`. */
export interface PasteRequest {
  at?: Address; replace?: { from: Address; to: Address };
  text?: string; expectedEpoch?: number; selection?: Selection; afterSelection?: Selection;
}
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
        /// Only body text, its cells and its text boxes: no headers,
        /// footers, notes or master pages.
        #[serde(default)]
        body: bool,
        /// Only this list and the lists inside it.
        #[serde(default)]
        within: Option<ListAddress>,
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
pub struct TextSelect {
    pub from: Address,
    pub to: Address,
    pub page: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TableAddress {
    pub section: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<StepJs>,
    pub para: u32,
    pub control: u32,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CellRef {
    pub row: u16,
    pub column: u16,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CellsSelect {
    pub kind: String,
    pub table: TableAddress,
    pub from: CellRef,
    pub to: CellRef,
    pub page: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum SelectRequest {
    Cells(CellsSelect),
    Text(TextSelect),
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectedCell {
    pub row: u16,
    pub column: u16,
    pub row_span: u16,
    pub column_span: u16,
    pub list: ListAddressOut,
}

#[derive(Debug, Serialize)]
pub struct ListAddressOut {
    pub section: u32,
    pub steps: Vec<StepJs>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SelectedRange {
    Text {
        from: Address,
        to: Address,
    },
    Cells {
        table: TableAddress,
        from: CellRef,
        to: CellRef,
        cells: Vec<SelectedCell>,
    },
}

#[derive(Debug, Serialize)]
pub struct Selected {
    pub range: SelectedRange,
    pub rects: Vec<Rect>,
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

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CharPatchJs {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub underline: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strike: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_face: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_faces: Option<Vec<Option<String>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub char_spacing: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spacings: Option<Vec<Option<i32>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ratios: Option<Vec<Option<i32>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rel_sizes: Option<Vec<Option<i32>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offsets: Option<Vec<Option<i32>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub superscript: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscript: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emboss: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engrave: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kerning: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub underline_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub underline_shape: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub underline_color: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strike_shape: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strike_color: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outline: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shadow: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shadow_color: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shadow_offset: Option<Vec<i32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emphasis: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shade_color: Option<u32>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ParaPatchJs {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub align: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub space_before: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub space_after: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indent: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_spacing: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_spacing_kind: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PageSettingsPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub margin_left: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub margin_right: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub margin_top: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub margin_bottom: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub margin_header: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub margin_footer: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gutter: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub landscape: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binding: Option<u32>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SectionSettingsPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attr: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column_gap: Option<i16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tab_stop: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outline_numbering: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_number: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub figure_number: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table_number: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub equation_number: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<u16>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ColumnSettingsPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direction: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub equal_widths: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gap: Option<i16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub columns: Option<Vec<(i16, i16)>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_kind: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_width: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_color: Option<u32>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NoteShapePatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attr: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_char: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefix: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suffix: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub separator_length: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub space_above: Option<i16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub space_below: Option<i16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub space_between: Option<i16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_kind: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_width: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_color: Option<u32>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FieldPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DocumentPropertiesPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_start: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub footnote_start: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endnote_start: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub figure_start: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table_start: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub equation_start: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caret_list: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caret_para: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caret_pos: Option<u32>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PageBorderPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attr: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offsets: Option<[i16; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border_fill: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lines: Option<[BorderLineSpec; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill: Option<PageFill>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BorderLineSpec {
    pub kind: u8,
    pub width: u8,
    pub color: u32,
}

/// A page background. Gradient and image fills are reported only; setting
/// one keeps the current fill.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum PageFill {
    None,
    Solid {
        color: u32,
        #[serde(default, rename = "patternColor")]
        pattern_color: u32,
        #[serde(default)]
        pattern: i32,
    },
    Gradient,
    Image,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FurnitureKind {
    Header,
    Footer,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum NoteKind {
    Footnote,
    Endnote,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Side {
    Both,
    Even,
    Odd,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FieldKindJs {
    ClickHere,
    Hyperlink,
    Bookmark,
    Date,
    Time,
    FileName,
    Title,
    PageNumber,
    PageCount,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", deny_unknown_fields)]
pub enum EditOp {
    SetPageSettings {
        section: u32,
        patch: PageSettingsPatch,
    },
    SetSectionSettings {
        section: u32,
        patch: SectionSettingsPatch,
    },
    SetColumns {
        at: Address,
        patch: ColumnSettingsPatch,
    },
    SetNoteShape {
        section: u32,
        kind: NoteKind,
        patch: NoteShapePatch,
    },
    SetPageBorder {
        section: u32,
        side: Side,
        patch: PageBorderPatch,
    },
    SetPageHide {
        at: Address,
        flags: u32,
    },
    CreateHeaderFooter {
        section: u32,
        kind: FurnitureKind,
        side: Side,
    },
    InsertNote {
        at: Address,
        kind: NoteKind,
    },
    RenumberNotes {},
    InsertField {
        at: Address,
        kind: FieldKindJs,
        to: Option<Address>,
        command: Option<String>,
        name: Option<String>,
        editable: Option<bool>,
        text: Option<String>,
    },
    UpdateField {
        target: TableAddress,
        patch: FieldPatch,
    },
    InsertBookmark {
        at: Address,
        name: String,
    },
    RenameBookmark {
        target: TableAddress,
        name: String,
    },
    DeleteControl {
        target: TableAddress,
    },
    SetFormValue {
        target: TableAddress,
        value: Option<i64>,
        text: Option<String>,
    },
    SetDocumentProperties {
        patch: DocumentPropertiesPatch,
    },
    ColumnBreak {
        at: Address,
    },
    InsertNewNumber {
        at: Address,
        number: u16,
    },

    #[serde(rename_all = "camelCase")]
    SetCharShape {
        from: Address,
        to: Address,
        char_shape_id: u32,
    },
    #[serde(rename_all = "camelCase")]
    SetParaShape {
        from: Address,
        to: Address,
        para_shape_id: u16,
    },
    CreateStyle {
        style: StylePatch,
    },
    UpdateStyle {
        id: u32,
        style: StylePatch,
    },
    DeleteStyle {
        id: u32,
    },
    EnsureList {
        definition: ListDefinition,
    },
    #[serde(rename_all = "camelCase")]
    CreateTable {
        at: Address,
        rows: u16,
        columns: u16,
        column_widths: Option<Vec<i32>>,
        row_heights: Option<Vec<i32>>,
        inline: Option<bool>,
    },
    #[serde(rename_all = "camelCase")]
    InsertPicture {
        at: Address,
        bytes: Vec<u8>,
        extension: String,
        description: Option<String>,
        width: i32,
        height: i32,
        natural_width: Option<i32>,
        natural_height: Option<i32>,
        inline: Option<bool>,
        horz_offset: Option<i32>,
        vert_offset: Option<i32>,
    },
    #[serde(rename_all = "camelCase")]
    AssignPictureImage {
        object: TableAddress,
        bytes: Vec<u8>,
        extension: String,
        natural_width: i32,
        natural_height: i32,
    },
    #[serde(rename_all = "camelCase")]
    CreateShape {
        at: Address,
        kind: String,
        width: i32,
        height: i32,
        text: Option<String>,
        inline: Option<bool>,
        horz_offset: Option<i32>,
        vert_offset: Option<i32>,
        connector: Option<crate::connectors::ConnectorSpec>,
    },
    DeleteObject {
        object: TableAddress,
    },
    RelocateObject {
        object: TableAddress,
        at: Address,
    },
    SetObjectProperties {
        object: TableAddress,
        patch: crate::object_api::ObjectPatch,
    },
    SetTableProperties {
        table: TableAddress,
        patch: crate::object_api::TablePatch,
    },
    SetCellProperties {
        table: TableAddress,
        row: u16,
        column: u16,
        patch: crate::object_api::CellPatch,
    },
    /// Applies a border fill to a block of cells as one cell.
    #[serde(rename_all = "camelCase")]
    SetCellZone {
        table: TableAddress,
        start_row: u16,
        start_column: u16,
        end_row: u16,
        end_column: u16,
        border_fill: crate::object_api::BorderFillPatch,
    },
    InsertEquation {
        at: Address,
        script: String,
        size: Option<u32>,
        color: Option<u32>,
        font: Option<String>,
        baseline: Option<i16>,
    },
    SetEquation {
        object: TableAddress,
        patch: crate::agent_api::EquationPatch,
    },
    #[serde(rename_all = "camelCase")]
    SetTableCaption {
        table: TableAddress,
        text: String,
        with_number: Option<bool>,
    },
    #[serde(rename_all = "camelCase")]
    SetObjectCaption {
        object: TableAddress,
        text: String,
        with_number: Option<bool>,
    },
    #[serde(rename_all = "camelCase")]
    InsertTableRows {
        table: TableAddress,
        before_row: u16,
        count: u16,
    },
    #[serde(rename_all = "camelCase")]
    DeleteTableRows {
        table: TableAddress,
        start_row: u16,
        count: u16,
    },
    #[serde(rename_all = "camelCase")]
    InsertTableColumns {
        table: TableAddress,
        before_column: u16,
        count: u16,
    },
    #[serde(rename_all = "camelCase")]
    DeleteTableColumns {
        table: TableAddress,
        start_column: u16,
        count: u16,
    },
    #[serde(rename_all = "camelCase")]
    MergeTableCells {
        table: TableAddress,
        start_row: u16,
        start_column: u16,
        end_row: u16,
        end_column: u16,
    },
    #[serde(rename_all = "camelCase")]
    SplitTableCell {
        table: TableAddress,
        row: u16,
        column: u16,
        rows: u16,
        columns: u16,
        equal_row_height: bool,
    },

    InsertText {
        at: Address,
        text: String,
        format: Option<CharPatchJs>,
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
    #[serde(rename_all = "camelCase")]
    ApplyStyle {
        from: Address,
        to: Address,
        style_id: u32,
    },
    SetList {
        from: Address,
        to: Address,
        list: Option<ListSpec>,
    },
    MoveObject {
        object: TableAddress,
        dx: i32,
        dy: i32,
    },
    /// A line's end points relative to the top left of its box as laid out.
    SetLineEnds {
        object: TableAddress,
        start: [i32; 2],
        end: [i32; 2],
    },
    GroupObjects {
        objects: Vec<TableAddress>,
    },
    UngroupObject {
        object: TableAddress,
    },
    ResizeObject {
        object: TableAddress,
        width: i32,
        height: i32,
    },
    ResizeTableCells {
        table: TableAddress,
        cells: Vec<crate::objects::CellSize>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StylePatch {
    pub characters: Option<CharPatchJs>,
    pub paragraphs: Option<ParaPatchJs>,
    pub name: Option<String>,
    pub english_name: Option<String>,
    pub kind: Option<String>,
    pub next: Option<u8>,
    pub char_shape_id: Option<u16>,
    pub para_shape_id: Option<u16>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum ListDefinition {
    Numbering {
        start: Option<u16>,
        levels: Vec<NumberLevel>,
    },
    Bullet {
        bullet: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NumberLevel {
    pub format: String,
    pub number_kind: Option<u32>,
    pub start: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct ListSpec {
    pub kind: String,
    pub id: u32,
    #[serde(default)]
    pub level: u32,
    pub restart: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactRequest {
    pub expected_epoch: Option<u32>,
    pub ops: Vec<EditOp>,
    /// The host's directed selection before the edit, kept for undo.
    #[serde(default)]
    pub selection: Option<serde_json::Value>,
    pub after_selection: Option<serde_json::Value>,
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
    /// Pages to paint again; `None` (null) for all.
    pub changed_pages: Option<Vec<usize>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection: Option<serde_json::Value>,
    pub history_id: u64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub table_changes: Vec<TableChange>,
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
    pub landscape: bool,
    /// The section's page margins as set, HU.
    pub margins: Margins,
    /// Page boxes in HU: the body, the header and footer areas and the
    /// columns of every region.
    pub body: Area,
    pub header: Area,
    pub footer: Area,
    pub columns: Vec<Area>,
}

/// A picture the host can decode itself in place of a patch.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PictureSource {
    /// Names the picture's bytes while its document is open.
    pub key: String,
    pub mime: &'static str,
    pub width: u32,
    pub height: u32,
    /// The picture's unit square, `(s, t)` from its top left, to page HU:
    /// `(a s + b t + c, d s + e t + f)`.
    pub matrix: [f32; 6],
    /// The frame corners to clip to, when the picture is cropped.
    pub clip: Option<[(i32, i32); 4]>,
}

#[derive(Debug, Default, Serialize)]
pub struct Margins {
    pub left: i32,
    pub right: i32,
    pub top: i32,
    pub bottom: i32,
    pub header: i32,
    pub footer: i32,
    pub gutter: i32,
}

#[derive(Debug, Default, Serialize)]
pub struct Area {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl From<hm_layout::Rect> for Area {
    fn from(r: hm_layout::Rect) -> Area {
        Area {
            x: r.x,
            y: r.y,
            width: r.w,
            height: r.h,
        }
    }
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
    Fonts,
    FontCandidates {
        names: Vec<String>,
    },
    CharShape {
        id: u32,
    },
    ParaShape {
        id: u32,
    },
    SectionSettings {
        section: u32,
    },
    #[serde(rename_all = "camelCase")]
    NoteShape {
        section: u32,
        note_kind: NoteKind,
    },
    DocumentProperties,
    Controls {
        list: Option<ListAddress>,
    },
    Fields {
        list: Option<ListAddress>,
    },
    ObjectProperties {
        object: TableAddress,
    },
    TableProperties {
        table: TableAddress,
    },
    CellProperties {
        table: TableAddress,
        row: u16,
        column: u16,
    },
    PictureData {
        object: TableAddress,
    },
    Equation {
        object: TableAddress,
    },
    EquationPreview {
        script: String,
        size: u32,
        #[serde(default)]
        color: u32,
        font: Option<String>,
    },
    Table {
        table: TableAddress,
        page: Option<u32>,
    },
    Paragraphs {
        list: ListAddress,
        #[serde(default)]
        from: u32,
        count: Option<u32>,
    },
    FormatContext {
        at: Address,
        to: Option<Address>,
    },
    Styles {
        #[serde(default)]
        from: u32,
        count: Option<u32>,
    },
    Style {
        id: u32,
    },
    Lists,
    #[serde(rename_all = "camelCase")]
    List {
        list_kind: String,
        id: u32,
    },
    Count {
        list: Option<ListAddress>,
        from: Option<Address>,
        to: Option<Address>,
    },
    FormatRuns {
        list: ListAddress,
        #[serde(default)]
        from: u32,
        count: Option<u32>,
    },
    Lines {
        list: ListAddress,
        para: u32,
    },
}

/// One laid-out line of a paragraph: where it is drawn and the wire range
/// it shows. Coordinates are page HU.
#[derive(Debug, Serialize)]
pub struct LineInfo {
    pub line: u32,
    pub page: u32,
    pub y: i32,
    pub height: i32,
    pub baseline: i32,
    pub start: u32,
    pub end: u32,
    pub left: i32,
    pub right: i32,
}

#[derive(Debug, Serialize)]
pub struct ControlInfo {
    pub index: u32,
    pub offset: u32,
    pub kind: &'static str,
    pub lists: usize,
    /// Authored equation source, using the model's UTF-16 text conversion.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script: Option<String>,
    /// For objects, tables and form objects: whether it sits in the text
    /// like a character.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inline: Option<bool>,
    /// For headers and footers: the pages they apply to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub side: Option<&'static str>,
    /// For notes: the number Hancom last assigned.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct ParagraphInfo {
    pub index: u32,
    pub text: String,
    pub offsets: Vec<u32>,
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FaceCandidates {
    pub name: String,
    pub candidates: Vec<String>,
    pub stand_ins: Vec<String>,
    pub resolves: bool,
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
    /// Tables and objects the fragment carries (they paste back only into
    /// the document copied from).
    pub objects: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PasteRequest {
    pub at: Option<Address>,
    pub replace: Option<RangeRequest>,
    pub text: Option<String>,
    pub expected_epoch: Option<u32>,
    /// The host's directed selection before the paste, kept for undo.
    #[serde(default)]
    pub selection: Option<serde_json::Value>,
    pub after_selection: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct RangeRequest {
    pub from: Address,
    pub to: Address,
}

/// Old cell origins map to surviving origins, or null when deleted. Requery
/// all list addresses for this table after any structural edit or history move.
#[derive(Debug, Serialize)]
pub struct TableChange {
    pub table: TableAddress,
    pub cells: Vec<CellRemap>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CellRemap {
    pub from: CellRef,
    pub to: Option<CellRef>,
    pub index: Option<u32>,
    pub paragraph_shift: u32,
    #[serde(skip)]
    pub(crate) span: (u16, u16),
}
