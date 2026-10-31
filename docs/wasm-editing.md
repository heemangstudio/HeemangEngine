# WASM editing contracts

`Doc.transact` applies its operations in order against the preceding operation's result, as one undo step. Invalid operations and stale epochs leave document bytes, derived styles, history and epochs unchanged.

## Tables

The seven structural operations use `TableAddress`, a paragraph-list path followed by paragraph and control indexes. Row and column indexes are zero-based. Counts must be positive; the last row or column cannot be deleted. Delete the enclosing control to remove a whole table. Structural edits require a complete, nonoverlapping grid, bounded to one million grid positions.

- Insertion copies a template band: `copyRow` (`copyColumn`), or the band at the insertion point (the last band when appending). Each new cell takes the cross-axis span and size of the template cell beside it and the template band's size, filling positions not covered by spans crossing the boundary. Crossing spans grow; a crossing cell widens with inserted columns but keeps its stored height, as Hancom does. The table grows by the inserted bands' sizes.
- Deletion shrinks crossing spans and keeps their content. Cells entirely inside the deleted bands are removed with their content.
- Merge uses inclusive rectangle ends. Every intersected cell must lie wholly inside the rectangle. The top-left cell survives with the paragraphs and controls of every non-empty selected cell in row order; cells holding one empty paragraph are dropped, and an all-empty merge keeps one empty paragraph. A caret in a dropped cell maps to the last kept paragraph before it.
- Split addresses a cell origin. `rows` and `columns` are the requested number of pieces. A span the pieces divide keeps its grid lines, each piece taking its bands' drawn row heights or stored column widths. Otherwise new grid lines are added only where needed, extending neighboring spans without changing their contents or sizes: column pieces share the cell's width evenly; row pieces divide the cell's drawn height, the first keeping its content height (lines plus vertical padding) and the others sharing the rest, or all sharing it equally with `equalRowHeight`. Pieces store those heights, which may be zero. The first piece retains the content; the others are empty.

- `moveTableCellEdge` moves the `right` or `bottom` edge of the cell at an origin by `delta` HU, as Hancom resizes one cell of a cell block: only that cell and the cells across the edge change size (widths from the stored ones, heights from the rows as drawn), a grid line is added where the edge lands and lines no cell uses are dropped. The cells across the edge must lie within the cell's rows (columns); each side keeps at least 200 HU, and an edge on the table border or one that cannot move is rejected.

Saving writes an edited table's laid-out height as its stored height; cell heights keep their stored minimums. Hancom does the same when typing grows or shrinks a row.

`query({kind: "table", table, page?})` returns row/column counts and every cell's origin, spans, current paragraph-list address and visible `Rect[]`. Rectangles carry page indexes and follow painting placement and clipping, including nested cells, repeated headers and carried fragments. `page` filters rectangles while retaining all cell metadata.

A structural receipt's `tableChanges` entries follow operation order. Each entry maps old cell origins to new origins, or `null` for removed cells. `index` is the cell's new list index, and `paragraphShift` is the number of paragraphs preceding its content after a merge. Cell-list addresses must be resolved again after structural edits and history moves. Nested paths can be updated using the same index and paragraph shift. Tagged cell selections and directed text selections follow these mappings; a selection touching a deleted cell is cleared.

## History selections

`transact` and `paste` accept `selection` for undo and `afterSelection` for redo. Both accept directed text selections and tagged cell selections. Formatting retains the supplied selection when no after-selection is provided. Insertion returns its collapsed caret. Paste infers its before-selection from `replace` or `at` when omitted, and returns its collapsed after-selection by default. Explicit affinities and direction are preserved.

## Page metadata and typing measurements

`Doc.pageInfo(index)` returns the same metadata as `Doc.pages()[index]`, while constructing and serializing only one page. It rejects an invalid index with `NO_SUCH_PAGE`.

`node crates/hm-wasm/js/bench/editing.mjs` measures 36 individual native WASM edits at the Studio investigation's handbook and gate positions. It reports wall times and process CPU time for each boundary over three fresh document instances, and checks the resulting text. Set `HEEMANG_CORPUS`, `HEEMANG_FONTS`, and optionally `HM_WASM_PKG` to compare packages. These measurements exclude Studio event handling and canvas scheduling.

## Formatting definitions

`query({kind:"fonts"})` returns seven arrays of `{id,name}` document faces, in Hangul, Latin, Hanja, Japanese, other, symbol and user slot order. IDs are local to each slot. These are document face names. Editing `fontFace` or `fontFaces` by name needs a family that draws with a registered face: its own, one Hancom draws in its place, or a font-map stand-in. `query({kind:"fontCandidates",names})` returns, per name, `{name,candidates,standIns,resolves}`: the families a host can register for it and whether it already resolves.

`query({kind:"charShape",id})` and `query({kind:"paraShape",id})` return a full supported `CharPatch` or `ParaPatch`, or `null` for a missing ID. Paragraph shape queries resolve character-relative distances against 1000 HU; `formatContext` and `formatRuns` use the paragraph's character size. `formatRuns` includes `paraShapeId` per paragraph and `charShapeId` per run. Patches expose supported formatting fields; restore every model field exactly with `{op:"setCharShape",from,to,charShapeId}` or `{op:"setParaShape",from,to,paraShapeId}`. Character ranges are half-open and a collapsed character range is unchanged; paragraph operations include both endpoint paragraphs.

`CharPatch.spacings` and `.ratios` accept up to seven numeric-or-null slots, with null preserving a slot. Spacings are -50..50 percent and ratios 50..200 percent. `charSpacing` sets all slots first, then `spacings` overrides individual slots. Queries return all seven values. `ParaPatch.lineSpacingKind` accepts `percent`, `fixed`, `added` or `atLeast`; `lineSpacing` is a percent for `percent` and HU otherwise. A spacing value without a kind keeps the original API's percent meaning. Setting only the kind preserves the stored value.

`CharPatch` also sets character effects: `superscript`/`subscript` and `emboss`/`engrave` (setting one of a pair clears the other), `kerning`, `underlineKind` (`none`, `below`, `center`, `above`; a centered underline is reported only without a strikeout, which HWP 5 stores as one), `underlineShape`, `underlineColor`, `strikeShape` and `strikeColor` (HWP line shape codes 0..15 and COLORREFs), `outline` (0..6), `shadow` (0 none, 1 discrete, 2 continuous) with `shadowColor` and `shadowOffset:[x,y]` (-100..100 percent), `emphasis` (0..12), `shadeColor` (0xFFFFFFFF for none), and per-slot `relSizes` (10..250) and `offsets` (-100..100) like `ratios`. Queries return all of them. HWP 5 files older than 5.0.3 store no strikeout color, so it reads back empty after saving such a file.

Style edits use ordinary atomic transaction receipts and history:

- `{op:"createStyle",style:{name,englishName?,kind?,next?,charShapeId?,paraShapeId?,characters?,paragraphs?}}` adds a style. `kind` is `paragraph` by default or `character`; `next` defaults to its own ID. Missing shape IDs use style 0's shapes, or shape 0 when no styles exist.
- `{op:"updateStyle",id,style:{...}}` changes supplied fields of an existing style. Shape IDs default to the existing style's IDs. Optional `characters:CharPatch` and `paragraphs:ParaPatch` derive shapes from these base IDs and preserve unmodified model fields. Updates change definitions; `applyStyle` applies a definition to content.
- `{op:"deleteStyle",id}` rejects style zero, missing styles, styles used by any paragraph including nested lists, and styles named by another style's `next`. Deleting an unused style compacts IDs and remaps larger paragraph/style-next references. Requery IDs after deletion or history movement.

Style IDs are zero-based and bounded to 256 styles. Read `styles` after a successful create transaction to obtain the new style's ID. Names need not be unique.

`{op:"ensureList",definition}` creates a definition only when an equivalent supported definition is absent. Numbering uses `{kind:"numbering",start?:1,levels:[{format:"^1.",numberKind?:0,start?:1}]}` with one to seven levels. Missing levels receive `^n.` formats. `numberKind` is the model's 0..15 code; `^n` substitutes level n. Bullet uses `{kind:"bullet",bullet:"•"}` with one non-control BMP character. New definitions use default paragraph-head metrics and character shape zero. Equivalence includes those head fields, so authored head formatting is retained. Repeating the same ensure is a no-op with no epoch/history change. Read `lists` after the transaction to obtain the 1-based definition ID, then pass it to `setList`. `setList` supports `numbering`, `bullet` and `outline`. Outline numbering uses the section definition. Definition creation, style edits and range edits may be combined atomically in one transaction; subsequent operations see newly created IDs.

`{op:"setListRestart",at,restart}` stores a paragraph-local session override, including on a plain paragraph before it becomes a list. `restart` is `null` for file/default rules, `{mode:"continue"}` to continue an already-used definition, or `{mode:"restart",start}` with an unsigned start, zero normalized to one. It affects outline and numbered heads, uses ordinary atomic history/checkpoints, and appears as `listRestart` in `formatContext`/`formatRuns`. Writers and merge property snapshots omit this editor-only override; reopening uses the numbering encoded in the file.

`Doc.pasteFrom(source, range, request)` imports a foreign `CopyRequest` range using the ordinary `PasteRequest` transaction and history contract. It copies into a local fragment, imports only referenced resources, and leaves the Engine's shared user clipboard unchanged on success or failure. Programmatic transfers such as template insertion use this path.
