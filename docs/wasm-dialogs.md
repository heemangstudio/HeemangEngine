# hm-wasm Studio dialogs contract

All calls JSON.stringify their request. `Doc.query` returns JSON. `Doc.transact({ops:[...]})` returns the existing Receipt, one atomic undo group. Coordinates use HU (7200/inch) and wire UTF-16 offsets (extended controls occupy 8). No WASM rebuild until parent coordinates.

```ts
type ListAddress = {section:number; steps?:{para:number;control:number;index:number}[]};
type Address = ListAddress & {para:number;offset:number};
type ControlAddress = ListAddress & {para:number;control:number};
type Side = 'both'|'even'|'odd';
type PageSettingsPatch = {width?:number;height?:number;marginLeft?:number;marginRight?:number;marginTop?:number;marginBottom?:number;marginHeader?:number;marginFooter?:number;gutter?:number;landscape?:boolean;binding?:number};
type SectionSettingsPatch = {attr?:number;columnGap?:number;tabStop?:number;outlineNumbering?:number;pageNumber?:number;figureNumber?:number;tableNumber?:number;equationNumber?:number;language?:number};
type ColumnSettingsPatch = {count?:number;kind?:number;direction?:number;equalWidths?:boolean;gap?:number;columns?:[number,number][];lineKind?:number;lineWidth?:number;lineColor?:number};
type NoteShapePatch = {attr?:number;userChar?:number;prefix?:number;suffix?:number;start?:number;separatorLength?:number;spaceAbove?:number;spaceBelow?:number;spaceBetween?:number;lineKind?:number;lineWidth?:number;lineColor?:number};
type FieldKind = 'clickHere'|'hyperlink'|'bookmark'|'date'|'time'|'fileName'|'title'|'pageNumber'|'pageCount';
type FieldPatch = {command?:string;name?:string;editable?:boolean;value?:string;dirty?:boolean};
```

Queries:
```ts
{kind:'sectionSettings',section:number}
// -> {section,page:PageSettingsPatch|null,settings:SectionSettingsPatch|null,columns:ColumnSettingsPatch|null,noteShapes:{footnote:NoteShapePatch|null,endnote:NoteShapePatch|null}}
{kind:'noteShape',section:number,noteKind:'footnote'|'endnote'}
// -> NoteShapePatch|null
{kind:'controls',list?:ListAddress}
// -> ControlRecord[]; recursively visits the requested list (entire document if omitted)
{kind:'fields',list?:ListAddress}
// -> ControlRecord[] filtered to fields and point bookmarks
{kind:'documentProperties'}
// -> {sectionCount,pageStart,footnoteStart,endnoteStart,figureStart,tableStart,equationStart,caretList,caretPara,caretPos}
// ControlRecord: {target:ControlAddress,offset:number,kind:string,lists:ListAddress[],side?:Side,number?:number,flags?:number,fieldId?:number,fieldKind?:FieldKind,name?:string,command?:string,guide?:string,editable?:boolean,value?:string,end?:Address}
```

Operations:
```ts
{op:'setPageSettings',section:number,patch:PageSettingsPatch}
{op:'setSectionSettings',section:number,patch:SectionSettingsPatch}
{op:'setColumns',at:Address,patch:ColumnSettingsPatch}
{op:'setNoteShape',section:number,kind:'footnote'|'endnote',patch:NoteShapePatch}
{op:'setPageHide',at:Address,flags:number} // 0..63: header,footer,master,border,fill,page-number
{op:'createHeaderFooter',section:number,kind:'header'|'footer',side:Side,at?:Address}
{op:'insertNote',at:Address,kind:'footnote'|'endnote'}
{op:'insertField',at:Address,kind:FieldKind,command?:string,name?:string,editable?:boolean,text?:string}
{op:'updateField',target:ControlAddress,patch:FieldPatch}
{op:'insertBookmark',at:Address,name:string}
{op:'renameBookmark',target:ControlAddress,name:string}
{op:'deleteControl',target:ControlAddress} // field delimiters removed, displayed value preserved (an unfilled click-here guide goes too)
{op:'setDocumentProperties',patch:{pageStart?:number;footnoteStart?:number;endnoteStart?:number;figureStart?:number;tableStart?:number;equationStart?:number;caretList?:number;caretPara?:number;caretPos?:number}}
```

Creation receipts include the caret inside the new HF/note list. Resolve targets after control insertion/deletion by requerying controls. Existing nested-list insertText/delete/split/formatChars/formatParas/applyStyle work for all furniture. Header/footer creation is idempotent for the same section/kind/side.

Metadata title/author/etc are currently preserved container data, absent from the typed model. A separate metadata API is still being investigated; documentProperties above means native DocInfo properties, not summary information. PageCount dynamic fields also require a verified native representation; any unsupported kind will fail explicitly until supported.

## Implementation additions / clarifications

- `insertField` now accepts optional `to: Address`. With `to`, it wraps existing text in one paragraph and preserves char runs, range marks and nested controls. Do not also supply `text`. Ordered range in the same list/paragraph is required.
- `setPageHide` with `flags:0` removes an existing hide control; absent + 0 is a no-op.
- Added `{op:'columnBreak',at:Address}` (splits and marks new paragraph), `{op:'insertNewNumber',at:Address,number:number}` (page restart), and `{op:'setPageBorder',section:number,side:Side,patch:{attr?:number,offsets?:[number,number,number,number],borderFill?:number}}`.
- `sectionSettings` also returns `pageBorders:PageBorderPatch[]`, ordered both/even/odd.
- `tabStop` is unpacked HU on query and edit. `SectionSettingsPatch.attr` is the native section attribute bitword; patch it from current query to preserve unknown bits.
- `setActiveField`/`clearActiveField` have no engine session API. The engine can insert at the interior wire offset (`offset+8` even for an empty field); bridge must explicitly choose that address rather than the outside logical offset. Keep legacy active-session wrappers unsupported if the bridge cannot carry that address.
- Clickhere command metadata uses the existing verified `hm_model::Params` text representation. Bridge may encode Direction/memo Params into `command`.

## Verified support boundary

Page-number insertion uses native `AutoNumber` and is supported in header/footer/body/note lists. `pageCount` returns `UNSUPPORTED`. Date/time/fileName/title require an explicit caller-supplied verified native `command`; omission returns `UNSUPPORTED`. The engine preserves those native field commands and displayed value text; it does not calculate dates, file names or document summary values. No speculative default command grammar is emitted.

Title/author/subject/keywords summary metadata and border-fill style editing remain gaps. Note insertion assigns a stored number from the configured start and existing count; renumbering existing notes after deletion/reordering and full restart-policy handling remain gaps. `setNoteShape` stores the native shape and supports native layout/save roundtrip.

## Page border styles and form objects

- `setPageBorder.patch` also takes `lines` (left, right, top, bottom `{kind,width,color}`: HWP 5 line kind, width index, COLORREF) and `fill` (`{kind:'none'}` or `{kind:'solid',color,patternColor?,pattern?}`). The engine copies the current border fill (or an empty one), replaces those parts, reuses an equal DocInfo entry or appends one, and points the page border at it. `sectionSettings.pageBorders[]` reports `lines` and `fill` (`gradient`/`image` are reported, not edited). An appended entry stays in DocInfo after undo, like derived char shapes.
- `controls` records for form objects carry `form: FormRecord` (`formType`, `name`, `value`, `text`, `caption`, `enabled`, `width`, `height`, `foreColor`, `backColor`, flattened `properties`, combo `items`), in any list.
- `{op:'setFormValue',target,value?,text?}` sets a check box or radio button's `ButtonSet.Value` (0..2) or an edit/combo/list box's `Text`; other combinations are `BAD_REQUEST`.

## Hancom editing parity

- Click-here (누름틀) fields: a field record reports `dirty`. An unfilled field (not dirty, its text its guide) shows the guide on the editing screen: `pages`, `page` and `pageSvg` lay it out in the room it takes, as Hancom's window does. Print leaves it out and closes up the line, as Hancom's print and PDF do: `pdf`, `printPageSvg(i, dpi)` and `printPageCount()` use that layout (capability `printLayout`). `updateField` with `value` marks the field changed unless the patch sets `dirty`; without `value` the mark stays. Typing into an unfilled field is `{value}`; rewriting its guide (누름틀 고치기) is `{command, value: guide, dirty: false}`. `deleteControl` of an unfilled field takes its guide text with it, as 누름틀 지우기 does. HWPX spells a click-here command's guide and memo out as `Direction` and `HelpState` parameters, as Hancom writes them.
- `createHeaderFooter` puts a new header or footer after the header and footer controls already in its paragraph, so they apply in creation order as in Hancom (a 짝수 쪽 header made after a 양쪽 one wins on even pages). `at` (the caret) anchors it in that body paragraph, so it starts on the caret's page; the default stays the section's first paragraph. Its paragraph takes the document's 머리말 style (footers too, as Hancom gives them), and a footer sits at the bottom of its area.
- `createTable` fills the list it goes in between 283 HU outside margins: the column of the section at that paragraph (the first column of unequal ones), the cell inside its margins, the text box inside its margins, or the body width. New tables repeat their title rows, stay inside the page, are placed from the paragraph and the column and are numbered as tables; rows store 282 HU and grow to their text, in the text or not. A table has up to 65536 cells.
