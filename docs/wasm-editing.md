# WASM editing contracts

`Doc.transact` applies its operations in order against the preceding operation's result, as one undo step. Invalid operations and stale epochs leave document bytes, derived styles, history and epochs unchanged.

## Tables

The six structural operations use `TableAddress`, a paragraph-list path followed by paragraph and control indexes. Row and column indexes are zero-based. Counts must be positive; the last row or column cannot be deleted. Delete the enclosing control to remove a whole table. Structural edits require a complete, nonoverlapping grid, bounded to one million grid positions.

- Insertion grows spans crossing the insertion boundary and fills new positions with empty cells using the adjacent cell's formatting. The table grows by the inserted bands' sizes.
- Deletion shrinks crossing spans and keeps their content. Cells entirely inside the deleted bands are removed with their content.
- Merge uses inclusive rectangle ends. Every intersected cell must lie wholly inside the rectangle. The top-left cell survives, with all selected paragraphs and controls appended in row order.
- Split addresses a cell origin. `rows` and `columns` are the requested number of pieces. Existing grid boundaries are used when divisible; otherwise boundaries inside the cell are refined, extending neighboring spans without changing those neighbors' contents or sizes. The first piece retains the content; the others are empty. `equalRowHeight` assigns equal stored minimum heights; otherwise the existing row-height distribution is used. Content can require taller rows.

`query({kind: "table", table, page?})` returns row/column counts and every cell's origin, spans, current paragraph-list address and visible `Rect[]`. Rectangles carry page indexes and follow painting placement and clipping, including nested cells, repeated headers and carried fragments. `page` filters rectangles while retaining all cell metadata.

A structural receipt's `tableChanges` entries follow operation order. Each entry maps old cell origins to new origins, or `null` for removed cells. `index` is the cell's new list index, and `paragraphShift` is the number of paragraphs preceding its content after a merge. Cell-list addresses must be resolved again after structural edits and history moves. Nested paths can be updated using the same index and paragraph shift. Tagged cell selections and directed text selections follow these mappings; a selection touching a deleted cell is cleared.

## History selections

`transact` and `paste` accept `selection` for undo and `afterSelection` for redo. Both accept directed text selections and tagged cell selections. Formatting retains the supplied selection when no after-selection is provided. Insertion returns its collapsed caret. Paste infers its before-selection from `replace` or `at` when omitted, and returns its collapsed after-selection by default. Explicit affinities and direction are preserved.

## Page metadata and typing measurements

`Doc.pageInfo(index)` returns the same metadata as `Doc.pages()[index]`, while constructing and serializing only one page. It rejects an invalid index with `NO_SUCH_PAGE`.

`node crates/hm-wasm/js/bench/editing.mjs` measures 36 individual native WASM edits at the Studio investigation's handbook and gate positions. It reports wall times and process CPU time for each boundary over three fresh document instances, and checks the resulting text. Set `HEEMANG_CORPUS`, `HEEMANG_FONTS`, and optionally `HM_WASM_PKG` to compare packages. These measurements exclude Studio event handling and canvas scheduling.
