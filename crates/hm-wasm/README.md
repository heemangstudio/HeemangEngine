# WASM code map

`Engine` and `Doc` are exported at the crate root. Their implementations are grouped below. `lib.rs` owns shared state, capability advertising and closed-document/layout access. Private modules keep the JS/Rust interface unchanged.

| Task | Implementation | Existing integration tests |
| --- | --- | --- |
| Fonts, open/create, font requests | `src/engine.rs` | `js/test/engine.test.mjs` |
| Layout cache, page packets, patches, PDF/SVG/PNG | `src/rendering.rs`, `packet.rs`, `index.rs` | `tests/index.rs`, `packet.rs` |
| Hit testing, selection, navigation, table geometry | `src/selection.rs`, `range.rs` | `tests/index.rs`, `js/test/engine.test.mjs` |
| Convert edit requests to model operations | `src/operations.rs` | `tests/agent_api.rs`, `dialogs.rs`, `js/test/engine.test.mjs` |
| Apply transactions and IME composition | `src/editing.rs` | `js/test/engine.test.mjs` |
| Table-range clipboard, transposition, paragraph captures | `src/studio_bridge.rs` | `tests/studio_bridge.rs` |
| Connector attachments and routing | `src/connectors.rs` | `tests/studio_bridge.rs` |
| Copy/paste document fragments | `src/clipboard.rs`, `clip.rs` | `js/test/engine.test.mjs` |
| Undo/redo, checkpoints, receipts, object transforms | `src/history.rs`, `agent_api.rs` | `tests/adopt.rs`, `objects.rs`, `js/test/engine.test.mjs` |
| Bounded reads and search | `src/queries.rs` | `tests/equation_query.rs`, `js/test/engine.test.mjs` |
| Structural version merges | `src/merge.rs` (exports), `merge_core.rs` (three-way properties), `merge_document.rs` (review/model/validation), `merge_text.rs` (wire-unit alignment), `merge_resources.rs` (logical definitions), `merge_container.rs` (native opaque members) | `tests/structural_merge.rs`, `merge_model.rs`, `cargo test -p hm-wasm --lib merge_` |
| Save, convert, mark saved | `src/saving.rs` | `js/test/conversion.test.mjs`, `engine.test.mjs` |
| Wire schema and TypeScript declarations | `src/types.rs` | Callers in `tests/`, `js/test/` |
| Table edits and properties | `src/tables.rs`, `dialogs.rs` | `tests/dialogs.rs`, `js/test/engine.test.mjs` |
| Objects, equations, grouping | `src/object_api.rs`, `objects.rs`, `grouping.rs` | `tests/objects.rs`, `grouping.rs`, `equation_query.rs` |
| Legacy OLE equation promotion | `src/agent_api.rs`, `hm-hwp5/src/ole.rs` | `tests/ole_promotion.rs` |
| Formatting, styles, lists | `src/format.rs`, `definitions.rs` | `js/test/studio-widgets.test.mjs`, `engine.test.mjs` |
| Headers/footers, notes, fields and pages | `src/notes.rs`, `dialogs.rs` | `tests/dialogs.rs` |

Find a method without reading every implementation:

```sh
rg -n 'fn (serialize_as|transact|query)' crates/hm-wasm/src
rg -n 'enum (EditOp|QueryRequest)' crates/hm-wasm/src/types.rs
rg --files crates/hm-wasm/tests
```

Use the existing `Engine`/`Doc` integration seam for regression coverage. Run from the repository root with `cargo test -p hm-wasm --test <actual-file-stem>`; select the test name from `rg --files` before running it. Source modules share crate-private state, while the external interface stays at `Engine`/`Doc`. Do not add a second public adapter for a feature.

Read [editing contracts](../../docs/wasm-editing.md) for transaction/history changes, [dialog contracts](../../docs/wasm-dialogs.md) for property patches, or [picture sampling](../../docs/picture-sampling.md) for decoding/resampling. Unrelated task work does not need all three.
