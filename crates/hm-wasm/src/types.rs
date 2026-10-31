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
  kerning?: boolean; useFontSpace?: boolean; underlineKind?: "none" | "below" | "center" | "above"; underlineShape?: number; underlineColor?: number; strikeShape?: number;
  strikeColor?: number; outline?: number; shadow?: number; shadowColor?: number; shadowOffset?: [number, number]; emphasis?: number;
  shadeColor?: number; borderFillId?: number; borderFill?: BorderFillPatch;
}
export type Align = "justify" | "left" | "right" | "center" | "distribute" | "divide";
/**
 * Paragraph shape fields; unknown keys are rejected. Distances are HU within the
 * document's largest page side; \`lineSpacing\` is 0 to 500 percent, or HU up to that side.
 * The flags and codes are the paragraph shape's own (HWPX names): \`breakLatinWord\` 0 keep
 * words, 1 hyphenate, 2 break words; \`breakNonLatinWord\` 0 by word (어절, HWPX BREAK_WORD),
 * 1 by syllable (글자, HWPX KEEP_WORD, Hancom's default); \`condense\` (minimum space) 0 to 75
 * percent; \`verticalAlign\` 0 baseline, 1 top, 2 center, 3 bottom; \`lineWrap\` 0 break,
 * 1 squeeze, 2 keep on one line; \`eAsianEng\` and \`eAsianNum\` space Korean from Latin letters and digits. \`borderFillId\` is the paragraph
 * border fill (0 none), \`borderOffsets\` [left, right, top, bottom] HU. \`autoTabLeft\` and
 * \`autoTabRight\` are the tab definition's automatic tabs, kept when only \`tabStops\` change.
 */
export interface ParaPatch {
  align?: Align; spaceBefore?: number; spaceAfter?: number; left?: number; right?: number; indent?: number; lineSpacing?: number;
  lineSpacingKind?: "percent" | "fixed" | "added" | "atLeast"; tabStops?:{position:number;type:number;fill:number}[];
  keepWithNext?: boolean; keepLines?: boolean; widowOrphan?: boolean; pageBreakBefore?: boolean; breakLatinWord?: number;
  breakNonLatinWord?: number; snapToGrid?: boolean; condense?: number; fontLineHeight?: boolean; verticalAlign?: number;
  lineWrap?: number; eAsianEng?: boolean; eAsianNum?: boolean; borderFillId?: number; borderOffsets?: [number, number, number, number];
  borderConnect?: boolean; borderIgnoreMargin?: boolean; autoTabLeft?: boolean; autoTabRight?: boolean;
  /** The paragraph head the shape carries (a style's 문단 종류): definition id 1-based, 0 with an outline for the section's; level 0 to 6. `none` keeps the id and level (up to 7) it is sent, so a reported head goes back unchanged. */
  heading?: { kind: "none" | "outline" | "numbering" | "bullet"; id?: number; level?: number };
}
export interface StylePatch { characters?: CharPatch; paragraphs?: ParaPatch; name?: string; englishName?: string; kind?: "paragraph" | "character"; next?: number; charShapeId?: number; paraShapeId?: number }
export type ListDefinition = { kind: "bullet"; bullet: string } | { kind: "numbering"; start?: number; levels: { format: string; numberKind?: number; start?: number }[] };
export interface FontFace { id: number; name: string }
export type ListRestart = { mode: "continue" } | { mode: "restart"; start: number };
export interface ListSpec { kind: "bullet" | "numbering" | "outline"; id: number; level?: number; restart?: number }
export interface PageSettingsPatch { width?: number; height?: number; marginLeft?: number; marginRight?: number; marginTop?: number; marginBottom?: number; marginHeader?: number; marginFooter?: number; gutter?: number; landscape?: boolean; binding?: number }
export interface SectionSettingsPatch { attr?: number; columnGap?: number; tabStop?: number; outlineNumbering?: number; pageNumber?: number; figureNumber?: number; tableNumber?: number; equationNumber?: number; language?: number }
export interface ColumnSettingsPatch { count?: number; kind?: number; direction?: number; equalWidths?: boolean; gap?: number; columns?: [number, number][]; lineKind?: number; lineWidth?: number; lineColor?: number }
/**
 * A section's footnote or endnote shape (HWP 5.0 FOOTNOTE_SHAPE). `attr` bits:
 * - 0-7 number format: 0 1,2,3; 1 circled digits; 2 I,II,III; 3 i,ii,iii; 4 A,B,C; 5 a,b,c;
 *   6 circled A,B,C; 7 circled a,b,c; 8 가,나,다; 9 circled 가,나,다; 10 ㄱ,ㄴ,ㄷ; 11 circled ㄱ,ㄴ,ㄷ;
 *   12 일,이,삼; 13 一,二,三; 14 circled 一,二,三; 15 갑,을,병; 16 甲,乙,丙;
 *   0x80 (128) four symbols *,†,‡,§ repeated; 0x81 (129) `userChar` repeated.
 * - 8-9 placement. Footnote: 0 each column, 1 merged under all columns, 2 rightmost column.
 *   Endnote: 0 end of document, 1 end of section.
 * - 10-11 numbering: 0 continue from the previous section, 1 restart each section from `start`
 *   (`start` is stored but unused otherwise), 2 restart every page (footnotes only; an endnote
 *   shape with 2 keeps counting and saves without a numbering type, as Hancom does).
 * - 12 note-list numbers as superscript. 13 footnotes right below the text (beneath text).
 * `userChar`, `prefix` and `suffix` are UTF-16 code units, 0 for none (Hancom's default suffix is
 * 41, `)`). `separatorLength` is HU: -1 Hancom's 5 cm default, 0 no line, 14692344 the full column.
 * `spaceAbove`/`spaceBelow`/`spaceBetween` are HU. `lineKind` is the border line kind
 * (0 none, 1 solid, 2 dot, 3 dash, 8 double…), `lineWidth` the border width index
 * (0 0.1 mm, 1 0.12 mm, 4 0.25 mm, 8 0.6 mm), `lineColor` a COLORREF (0x00BBGGRR).
 * `setNoteShape` rewrites every note of that kind in the section (number format, characters,
 * superscript, numbering) in the same transaction.
 */
export interface NoteShapePatch { attr?: number; userChar?: number; prefix?: number; suffix?: number; start?: number; separatorLength?: number; spaceAbove?: number; spaceBelow?: number; spaceBetween?: number; lineKind?: number; lineWidth?: number; lineColor?: number }
/**
 * `value` replaces the field's text, in the shape of the text it replaces, and marks the field
 * changed (`dirty`) unless `dirty` says otherwise; other changes leave the mark. An unfilled
 * click-here field (not dirty, its text its guide) shows the guide on the editing screen and not
 * in print: typing into it deletes the guide, inserts the text and sets `{dirty: true}` (the text
 * takes the shape before the field); rewriting its guide is `{command, value: guide, dirty: false}`,
 * as Hancom does.
 */
export interface FieldPatch { command?: string; name?: string; editable?: boolean; value?: string; dirty?: boolean }
export interface DocumentPropertiesPatch { pageStart?: number; footnoteStart?: number; endnoteStart?: number; figureStart?: number; tableStart?: number; equationStart?: number; caretList?: number; caretPara?: number; caretPos?: number }
export interface BorderLineSpec { kind: number; width: number; color: number }
export type PageFill = { kind: "none" } | { kind: "solid"; color: number; patternColor?: number; pattern?: number } | { kind: "gradient" | "image" };
/** `lines` (left, right, top, bottom) and `fill` derive a border fill from the current one; queries report both. */
export interface PageBorderPatch { attr?: number; offsets?: [number, number, number, number]; borderFill?: number; lines?: [BorderLineSpec, BorderLineSpec, BorderLineSpec, BorderLineSpec]; fill?: PageFill }
export type Side = "both" | "even" | "odd";
export type NoteKind = "footnote" | "endnote";
export type FieldKind = "clickHere" | "hyperlink" | "bookmark" | "date" | "time" | "fileName" | "title" | "pageNumber" | "pageCount" | "formula";
export type ControlAddress = TableAddress;
export interface SectionSettings {section:number;page:PageSettingsPatch|null;settings:SectionSettingsPatch|null;columns:ColumnSettingsPatch|null;noteShapes:{footnote:NoteShapePatch|null;endnote:NoteShapePatch|null};pageBorders:PageBorderPatch[]}
/** A form object's state. `value` is a check box or radio button's 0 off, 1 on, 2 indeterminate; `text` an edit or combo box's entry. `properties` flattens every property set, first name wins. */
export interface FormRecord {formType:"PushButton"|"CheckBox"|"RadioButton"|"ComboBox"|"ListBox"|"Edit"|"ScrollBar"|"Unknown";name:string;value:number;text:string;caption:string;enabled:boolean;width:number;height:number;foreColor:number;backColor:number;properties:Record<string,string>;items:string[]}
export interface ControlRecord {target:ControlAddress;offset:number;kind:string;lists:ListAddress[];side?:Side;number?:number;numberKind?:"page"|"footnote"|"endnote"|"figure"|"table"|"equation"|"other";userChar?:number;prefix?:number;suffix?:number;noteFlag?:number;flags?:number;fieldId?:number;fieldKind?:FieldKind;name?:string;command?:string;guide?:string;editable?:boolean;dirty?:boolean;value?:string;end?:Address;form?:FormRecord;formula?:string;resultFormat?:string;position?:number;format?:number;sideChar?:number}
/** HU throughout, including natural image dimensions. Patches reject unknown fields. */
export interface ObjectPatch {
  width?:number; height?:number; horzOffset?:number; vertOffset?:number; treatAsChar?:boolean;
  vertRelTo?:"paper"|"page"|"para"; horzRelTo?:"paper"|"page"|"column"|"para";
  vertAlign?:"top"|"center"|"bottom"|"inside"|"outside"; horzAlign?:"left"|"center"|"right"|"inside"|"outside";
  textWrap?:"square"|"topAndBottom"|"behind"|"inFront"; margins?:[number,number,number,number];
  description?:string; rotation?:number; flipHorizontal?:boolean; flipVertical?:boolean; zOrder?:number; sizeProtect?:boolean;
  restrictInPage?:boolean; allowOverlap?:boolean; keepWithAnchor?:boolean;
  border?:{color?:number;width?:number;style?:number;endCap?:number;head?:number;tail?:number;headSize?:number;tailSize?:number}; crop?:[number,number,number,number];
  brightness?:number; contrast?:number; effect?:number; transparency?:number; pictureMargins?:[number,number,number,number];
  fill?:FillPatch; fillAlpha?:number;
  caption?:CaptionPatch; textMargins?:[number,number,number,number];textVerticalAlign?:"top"|"center"|"bottom";round?:number;shadow?:{kind?:number;color?:number;x?:number;y?:number};
}
export interface CaptionPatch {
  enabled?:boolean; side?:"left"|"right"|"top"|"bottom"; verticalAlign?:"top"|"center"|"bottom";
  width?:number;gap?:number;maxWidth?:number;includeMargin?:boolean;
}
export interface LinePatch { style:number; width:number; color:number }
export type FillPatch = { kind:"none"|"image" } | { kind:"solid"; background:number; patternColor?:number; pattern?:number } | {kind:"gradient";gradientType?:number;angle?:number;centerX?:number;centerY?:number;step?:number};
export interface BorderFillPatch {
  left?:LinePatch; right?:LinePatch; top?:LinePatch; bottom?:LinePatch; diagonal?:LinePatch;
  slash?:number; backslash?:number; centerLine?:"none"|"horizontal"|"vertical"|"cross"; fill?:FillPatch;
}
/** `lock` 개체 보호하기, `sizeProtect` 크기 고정. */
export interface TablePatch { lock?:boolean; sizeProtect?:boolean; repeatHeader?:boolean; split?:"none"|"row"|"cell"; spacing?:number; padding?:[number,number,number,number]; borderFill?:BorderFillPatch; borderFillId?:number }
export interface CellPatch { width?:number; height?:number; padding?:[number,number,number,number]; verticalAlign?:"top"|"center"|"bottom"; ownPadding?:boolean; protected?:boolean; header?:boolean; textDirection?:number; editableInForm?:boolean; lineWrap?:"break"|"squeeze"|"keep"; borderFill?:BorderFillPatch; borderFillId?:number;fieldName?:string }
export interface ObjectProperties extends Omit<ObjectPatch,"fill"|"caption"> {
  kind:string; text?:string; size?:[number,number]; pictureMargins?:[number,number,number,number];
  brightness?:number; contrast?:number; effect?:number; transparency?:number; naturalWidth?:number; naturalHeight?:number;
  fill?:{kind:number;color:number|null;solid:unknown;gradient:unknown;image:unknown;alpha:number[]};
  /** Drawings and equations: null without a caption; `list` is the caption's list index in the object's steps. */
  caption?:{side:"left"|"right"|"top"|"bottom";verticalAlign:"top"|"center"|"bottom";includeMargin:boolean;width:number;gap:number;maxWidth:number;paragraphs:number;list:number}|null;
}
/** `caption` is null without a caption; `zones` are the table's cell zones, each with the border fill it applies. */
export interface TableProperties extends Omit<TablePatch,"borderFill"> {
  rows:number; columns:number; borderFill:unknown;
  caption:{side:"left"|"right"|"top"|"bottom";verticalAlign:"top"|"center"|"bottom";includeMargin:boolean;width:number;gap:number;maxWidth:number;paragraphs:number}|null;
  zones:{startRow:number;startColumn:number;endRow:number;endColumn:number;borderFill:unknown}[];
}
export interface CellProperties extends Omit<CellPatch,"borderFill"> { row:number; column:number; rowSpan:number; columnSpan:number; tablePadding:number[]; borderFill:unknown }
export interface PictureData { bytes:number[]; extension:string; mime:string; naturalWidth:number; naturalHeight:number }
/** Equation fields: size in HU (1000 = 10 pt), baseline a percent of the height. */
export interface EquationPatch { script?: string; size?: number; color?: number; baseline?: number; font?: string }
export interface EquationInfo { script: string; size: number; color: number; baseline: number; font: string | null; width: number; height: number }
export interface EquationPreview { svg: string; widthPx: number; heightPx: number; baselinePx: number; warnings: string[] }
export type EditOp =
  | { op: "setCharShape"; from: Address; to: Address; charShapeId: number }
  | { op: "setParaShape"; from: Address; to: Address; paraShapeId: number }
  | { op: "createStyle"; style: StylePatch }
  /** Paragraphs in a paragraph style follow its new shapes attribute by attribute in the same step; what they set themselves stays. */
  | { op: "updateStyle"; id: number; style: StylePatch }
  /** Paragraphs in the style take `replacement` (an id before the deletion), following it attribute by attribute; a style in use needs one. Styles naming the deleted one as next name themselves. */
  | { op: "deleteStyle"; id: number; replacement?: number }
  | { op: "ensureList"; definition: ListDefinition }
  | {op:"setPageSettings";section:number;patch:PageSettingsPatch}
  | {op:"setSectionSettings";section:number;patch:SectionSettingsPatch}
  | {op:"setColumns";at:Address;patch:ColumnSettingsPatch}
  | {op:"setNoteShape";section:number;kind:NoteKind;patch:NoteShapePatch}
  | {op:"setPageBorder";section:number;side:Side;patch:PageBorderPatch}
  | {op:"setPageHide";at:Address;flags:number}
  /**
   * A header or footer for `side` pages, or the one the section already has. A new one goes in the
   * body paragraph of `at` (the caret's, so it starts on the caret's page; default the section's
   * first paragraph), after the header and footer controls already there, as Hancom orders them.
   * Its paragraph takes the document's 머리말 style, as Hancom gives headers and footers.
   */
  | {op:"createHeaderFooter";section:number;kind:"header"|"footer";side:Side;at?:Address}
  /** A note at `at` (body, cell or text box; not inside a note): one paragraph holding the note's own number and a space, in the 각주/미주 style's shapes (the shapes at the mark when the document lacks the style). The receipt caret is after the space (offset 9). */
  | {op:"insertNote";at:Address;kind:NoteKind}
  /** Gives every note the number and mirrored note shape Hancom shows. Runs on its own after edits that add, remove, paste or restart notes or change a note shape. */
  | {op:"renumberNotes"}
  /**
   * A field around `text` (or the text up to `to`). A `formula` field (표 계산식) takes the formula
   * as `command` (`=SUM(ABOVE)`), its result as `text` and the result format as `resultFormat`
   * (default `%g,`); it stores Hancom's command `formula??format;;result`. Recalculating is
   * `updateField` with the new result as `value` (and `dirty: false`), which rewrites the stored
   * result too.
   */
  | {op:"insertField";at:Address;kind:FieldKind;to?:Address;command?:string;name?:string;editable?:boolean;text?:string;resultFormat?:string}
  | {op:"updateField";target:ControlAddress;patch:FieldPatch}
  | {op:"insertBookmark";at:Address;name:string}
  | {op:"renameBookmark";target:ControlAddress;name:string}
  | {op:"deleteControl";target:ControlAddress}
  | {op:"setFormValue";target:ControlAddress;value?:number;text?:string}
  | {op:"setDocumentProperties";patch:DocumentPropertiesPatch}
  /** Hancom's 단 나누기: at a paragraph's start or in an empty paragraph it starts the column there, at its end the next paragraph does, and only in the middle (or at the end of the last paragraph) is the paragraph split; the caret goes to where the column starts. `on:false` takes away the page or column break the paragraph starts with, like `pageBreak` `on:false`. */
  | {op:"columnBreak";at:Address;on?:boolean}
  /** 새 번호로 시작: the counter of `kind` (page by default) goes on from `number` after `at`. */
  | {op:"insertNewNumber";at:Address;number:number;kind?:"page"|"footnote"|"endnote"|"figure"|"table"|"equation"}
  /**
   * 쪽 번호 매기기: where page numbers print (0 none, 1 top left, 2 top center, 3 top right,
   * 4 bottom left, 5 bottom center, 6 bottom right, 7 outside top, 8 outside bottom, 9 inside top,
   * 10 inside bottom), their number shape (`NoteShapePatch` format codes, default 0 = 1,2,3) and the
   * side character around them (a UTF-16 code unit, 0 none; Hancom's 줄표 `-`, 45, by default).
   * Replaces the section's page number position, or adds one to its first paragraph as Hancom
   * does; `controls` reports it as `pageNumber` with `position`, `format` and `sideChar`.
   */
  | {op:"setPageNumberPos";section:number;position:number;format?:number;sideChar?:number}
  | { op:"createTable"; at:Address; rows:number; columns:number; columnWidths?:number[]; rowHeights?:number[]; inline?:boolean }
  /** Numbered as a picture, drawn above the section's objects; in the text it keeps the text above and below it and flows with it. */
  | { op:"insertPicture"; at:Address; bytes:number[]; extension:string; description?:string; width:number; height:number; naturalWidth?:number; naturalHeight?:number; inline?:boolean; horzOffset?:number; vertOffset?:number }
  | { op:"assignPictureImage"; object:ObjectAddress; bytes:number[]; extension:string; naturalWidth:number; naturalHeight:number }
  /** Hancom's new drawing: in front of the text, overlap allowed, drawn above the section's objects, a 0.12 mm black line with flat ends, a white fill (not lines), its kind's description ("사각형입니다." ...), text inside 1 mm margins centred vertically. A line may have no extent across it. */
  | { op:"createShape"; at:Address; kind:"rect"|"ellipse"|"line"|"textbox"|"polygon"|"arc"; connector?:ConnectorSpec; width:number; height:number; points?:[number,number][]; text?:string; inline?:boolean; horzOffset?:number; vertOffset?:number }
  /** Gives a rectangle, ellipse, polygon, arc or curve without text Hancom's empty text part (1 mm margins, centred); the receipt caret is at the start of that text. A drawing with text is left as it is (no history step); lines, pictures, groups and other objects are rejected. */
  | { op:"ensureShapeText"; object:ObjectAddress }
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
  | { op: "promoteOleEquation"; object: TableAddress }
  | { op: "setEquation"; object: TableAddress; patch: EquationPatch }
  | { op: "setTableCaption"; table: TableAddress; text: string; withNumber?: boolean }
  /** A picture, drawing or equation caption; "그림 N" or "수식 N" first with `withNumber`. Created below the object when missing. */
  | { op: "setObjectCaption"; object: ObjectAddress; text: string; withNumber?: boolean }
  | { op: "insertText"; at: Address; text: string; format?: CharPatch }
  | { op: "delete"; from: Address; to: Address }
  /** Breaks the paragraph (Enter). The line left above keeps the shape before the cut for its end mark. At the end of a paragraph whose style names another next style, the new paragraph takes that style and its shapes, as in Hancom; `nextStyle:false` keeps the style (to restore a merged paragraph). */
  | { op: "split"; at: Address; pageBreak?: boolean; nextStyle?: boolean }
  | { op: "formatChars"; from: Address; to: Address; patch: CharPatch }
  | { op: "formatParas"; from: Address; to: Address; patch: ParaPatch }
  /** Sets the page break before the paragraph at `at`; `on:false` takes away the page or column break it starts with. */
  | { op: "pageBreak"; at: Address; on: boolean }
  /** A paragraph style over a range replaces the paragraphs' shapes and characters with the style's; at a caret (`from` equal to `to`) the paragraph follows the style attribute by attribute and keeps what it sets itself, as in Hancom. A character style formats a range and does nothing at a caret. */
  | { op: "applyStyle"; from: Address; to: Address; styleId: number }
  | { op: "insertTableRows"; table: TableAddress; beforeRow: number; count: number; copyRow?: number }
  | { op: "deleteTableRows"; table: TableAddress; startRow: number; count: number }
  | { op: "insertTableColumns"; table: TableAddress; beforeColumn: number; count: number; copyColumn?: number }
  | { op: "deleteTableColumns"; table: TableAddress; startColumn: number; count: number }
  | { op: "mergeTableCells"; table: TableAddress; startRow: number; startColumn: number; endRow: number; endColumn: number }
  | { op: "splitTableCell"; table: TableAddress; row: number; column: number; rows: number; columns: number; equalRowHeight: boolean }
  | { op: "moveTableCellEdge"; table: TableAddress; row: number; column: number; side: "right" | "bottom"; delta: number }
  | { op: "setListRestart"; at: Address; restart: ListRestart | null }
  | { op: "setList"; from: Address; to: Address; list: ListSpec | null }
  | { op: "moveObject"; object: ObjectAddress; dx: number; dy: number }
  | { op: "resizeObject"; object: ObjectAddress; width: number; height: number }
  | { op: "resizeTableCells"; table: TableAddress; cells: CellResize[] };
/** An object control in a paragraph list (a table's address names it too). */
export type ObjectAddress = TableAddress;
/**
 * HU added to a cell's laid-out width and height; `index` is the cell's in the table.
 * `width` and `height` set the laid-out size instead and win over the deltas.
 */
export interface CellResize { index: number; widthDelta?: number; heightDelta?: number; width?: number; height?: number }
/**
 * An object or table a page draws (`Doc.pageObjects`), bottom to top. The box
 * excludes outer margins; a table split over pages gives this page's part.
 * \`line\` holds a line's start and end on the page.
 */
export interface PageObject {
  section: number; steps?: Step[]; para: number; control: number;
  kind: "table" | "picture" | "shape" | "line" | "connector" | "group" | "ole" | "equation" | "form" | "object";
  page: number; x: number; y: number; width: number; height: number;
  inline: boolean; layer: "behind" | "text" | "front"; wrap: "square" | "topAndBottom" | "behindText" | "inFrontOfText"; rows?: number; cols?: number; z: number;
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
  /** `printPageCount`, `printPageSvg` and `pdf` give Hancom's print layout; the pages are the editing screen's. */
  printLayout?: boolean;
  scopes: string[]; ops: string[]; paste: string[]; locate: string[]; motions: string[]; query: string[]; missing: string[];
  /**
   * Paragraph edits that follow Hancom:
   * \`emptyMergeKeepsLower\`: deleting across paragraphs from one keeping no characters keeps the last one's style and shapes in the first one's place (its breaks and definitions).
   * \`columnBreakRule\`: \`columnBreak\` marks rather than splits at a paragraph's start or end or in an empty one; \`on:false\` (also \`pageBreak\`) takes away either break.
   * \`styleFollow\`: \`updateStyle\` restyles the paragraphs in the style and \`applyStyle\` at a caret follows the style, attribute by attribute; \`deleteStyle\` takes \`replacement\`.
   * \`paraFlags\`: \`ParaPatch\` flags, break units, borders and automatic tabs.
   * \`splitShape\`: the line \`split\` leaves keeps the shape before the cut.
   * \`nextStyle\`: \`split\` at a paragraph's end starts its style's next style.
   * \`newNumberKinds\`: \`insertNewNumber\` takes \`kind\`.
   * \`paraHeading\`: \`ParaPatch.heading\`.
   */
  paragraphRules?: string[];
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
  index: number; styleId: number; paraShapeId: number; paragraphs: ParaPatch; list: ListRef | null; listRestart: ListRestart | null;
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
export interface FormatContext { characters: CharPatch; paragraphs: ParaPatch; styleId: number; list: ListRef | null; listRestart: ListRestart | null; mixed: string[] }
export interface StyleInfo {
  id: number; name: string; englishName: string; kind: "paragraph" | "character"; next: number;
  paraShape: number; charShape: number; characters: CharPatch; paragraphs: ParaPatch;
}
export interface ListLevel { format: string; numberKind: number; start: number }
export interface ListInfo { kind: "bullet" | "numbering" | "outline"; id: number; bullet?: string; start?: number; levels?: ListLevel[] }
export interface ControlInfo {
  index: number; offset: number; kind: string; lists: number;
  /** Authored equation source, including an empty string; absent on other controls. */
  script?: string;
  /** Objects, tables and form objects: whether it sits in the text like a character. */
  inline?: boolean;
  /** Headers and footers: the pages they apply to. */
  side?: "both" | "even" | "odd";
  /** Notes and automatic numbers: the number Hancom last assigned. */
  number?: number;
}
export interface ParagraphInfo {
  index: number;
  /** Characters, tabs, line breaks (\n), the two fixed spaces (U+00A0, U+2007) and soft hyphens (U+00AD). */
  text: string;
  /** The wire offset of each UTF-16 unit of `text`, then the end mark's. */
  offsets: number[];
  length: number; caret: [number, number]; controls: ControlInfo[];
}
export interface SearchRequest {
  text: string; caseSensitive?: boolean; list?: ListAddress; limit?: number;
  /** With `from`, the one match after it (before it with `backward`), wrapping around. */
  from?: Address; backward?: boolean;
  /** Match text as if controls (notes, fields, objects) were absent; a match may then span them. */
  acrossControls?: boolean;
}
export interface Match { from: Address; to: Address }
export interface SearchResult { contentEpoch: number; matches: Match[] }
export interface FaceRequest { name: string; candidates: string[]; standIns: string[]; text?: boolean }
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

/// A request read from JSON. `JSON.stringify` writes a lone UTF-16
/// surrogate (half of a pasted pair) as an escape serde_json refuses; such
/// an escape reads as U+FFFD, as the host's string conversion makes of a
/// raw one.
pub fn parse<T: serde::de::DeserializeOwned>(json: &str) -> Result<T> {
    serde_json::from_str(json)
        .or_else(|e| match lone_surrogates(json) {
            Some(fixed) => serde_json::from_str(&fixed),
            None => Err(e),
        })
        .map_err(|e| EngineError::new("BAD_REQUEST", e))
}

/// `json` with each `\uD800`..`\uDFFF` escape that is not half of a pair
/// replaced by `\uFFFD`; None when it has none.
fn lone_surrogates(json: &str) -> Option<String> {
    let b = json.as_bytes();
    let unit = |i: usize| {
        (b.get(i) == Some(&b'\\') && b.get(i + 1) == Some(&b'u'))
            .then(|| json.get(i + 2..i + 6))
            .flatten()
            .and_then(|h| u16::from_str_radix(h, 16).ok())
    };
    let low = |u: u16| (0xDC00..0xE000).contains(&u);
    let (mut out, mut last, mut i) = (String::new(), 0, 0);
    while i < b.len() {
        if b[i] != b'\\' {
            i += 1;
            continue;
        }
        let Some(u) = unit(i) else {
            // Any other escape is two bytes: skipping both keeps `\\u` text.
            i += 2;
            continue;
        };
        if (0xD800..0xDC00).contains(&u) && unit(i + 6).is_some_and(low) {
            i += 12;
            continue;
        }
        if (0xD800..0xE000).contains(&u) {
            out.push_str(&json[last..i]);
            out.push_str("\\uFFFD");
            last = i + 6;
        }
        i += 6;
    }
    (last > 0).then(|| out + &json[last..])
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
    /// Blanks measured with the font's own width (Hancom's 글꼴에 어울리는 빈칸).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_font_space: Option<bool>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border_fill_id: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border_fill: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TabStopJs {
    pub position: i32,
    #[serde(rename = "type")]
    pub kind: u8,
    pub fill: u8,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tab_stops: Option<Vec<TabStopJs>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_with_next: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_lines: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub widow_orphan: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_break_before: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub break_latin_word: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub break_non_latin_word: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snap_to_grid: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condense: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_line_height: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vertical_align: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_wrap: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub e_asian_eng: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub e_asian_num: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border_fill_id: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border_offsets: Option<[i32; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border_connect: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border_ignore_margin: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_tab_left: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_tab_right: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heading: Option<HeadingJs>,
}

/// A paragraph shape's head: `none`, `outline`, `numbering` or `bullet`, the definition
/// id (1-based; 0 with an outline is the section's) and the level (0 to 6).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HeadingJs {
    pub kind: String,
    #[serde(default)]
    pub id: u16,
    #[serde(default)]
    pub level: u32,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dirty: Option<bool>,
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
    Formula,
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
        at: Option<Address>,
    },
    InsertNote {
        at: Address,
        kind: NoteKind,
    },
    RenumberNotes {},
    /// Internal: removes field markers a deletion in this transaction left without their
    /// partner, so a cut field disappears whole (see `Doc::transact`).
    #[serde(skip)]
    BalanceFields {},
    InsertField {
        at: Address,
        kind: FieldKindJs,
        to: Option<Address>,
        command: Option<String>,
        name: Option<String>,
        editable: Option<bool>,
        text: Option<String>,
        #[serde(rename = "resultFormat")]
        result_format: Option<String>,
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
        on: Option<bool>,
    },
    InsertNewNumber {
        at: Address,
        number: u16,
        kind: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    SetPageNumberPos {
        section: u32,
        position: u32,
        format: Option<u32>,
        side_char: Option<u16>,
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
        replacement: Option<u32>,
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
        /// Polygon vertices relative to the shape box.
        points: Option<Vec<(i32, i32)>>,
    },
    EnsureShapeText {
        object: TableAddress,
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
    PromoteOleEquation {
        object: TableAddress,
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
        /// The row whose cells, column spans and height the new rows copy;
        /// the row at the insertion point (or the last row) when absent.
        copy_row: Option<u16>,
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
        /// The column whose cells, row spans and width the new columns copy;
        /// the column at the insertion point (or the last column) when absent.
        copy_column: Option<u16>,
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
    /// Moves the right or bottom edge of the cell at `row`, `column` by
    /// `delta` HU, resizing that cell and the cells across the edge only.
    #[serde(rename_all = "camelCase")]
    MoveTableCellEdge {
        table: TableAddress,
        row: u16,
        column: u16,
        /// `right` or `bottom`.
        side: String,
        delta: i32,
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
        next_style: Option<bool>,
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
    SetListRestart {
        at: Address,
        restart: Option<ListRestartSpec>,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "camelCase", deny_unknown_fields)]
pub enum ListRestartSpec {
    Continue,
    Restart { start: u32 },
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
    /// For notes and automatic numbers: the number Hancom last assigned.
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
    #[serde(default)]
    pub across_controls: bool,
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
    /// Whether actual text uses this family, rather than only empty/control-run metrics.
    #[serde(default = "font_text_default")]
    pub text: bool,
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

fn font_text_default() -> bool {
    true
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FontRequests {
    pub faces: Vec<FaceRequest>,
    pub class_defaults: Vec<Vec<String>>,
    /// Families not registered that the last layout looked in for
    /// characters a face lacked, in the order it tried them, and Wingdings
    /// for symbol-font characters (U+F020..U+F0FF) of runs whose face is
    /// not a symbol font.
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
