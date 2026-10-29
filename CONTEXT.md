# Engine work map

HeemangEngine reads HWP5, HWPX, HML and HWP3 into `hm-model`, edits with `hm-edit`, lays out with `hm-layout`, and paints with `hm-render`. `hm-cli` and `hm-wasm` expose that pipeline. Start with the row for your task; read implementation details on demand.

| Task | Start here | Focused check |
| --- | --- | --- |
| Model, controls, formatting definitions | `crates/hm-model/src/lib.rs` re-exports | `cargo test -p hm-model` |
| HWP5 read/write | `crates/hm-hwp5/src/lib.rs` | `cargo test -p hm-hwp5` |
| HWPX read/write | `crates/hm-hwpx/src/lib.rs`, `write/` | `cargo test -p hm-hwpx` |
| HML read/write | `crates/hm-hml/src/lib.rs`, `head.rs`, `body.rs`, `object.rs` | `cargo test -p hm-hml` |
| HWP3 import | `crates/hm-hwp3/src/lib.rs`, `convert.rs` | `cargo test -p hm-hwp3` |
| Transactions, inverses, undo/redo | `crates/hm-edit/src/lib.rs`, `ops.rs` | `cargo test -p hm-edit` |
| Save conversion and stored line membership | `crates/hm-edit/src/lines.rs`, `export.rs` | `cargo test -p hm-edit` |
| Measurement, wrapping, page breaks | `crates/hm-layout/src/measure.rs`, `para.rs`, `paginate.rs` | `cargo test -p hm-layout --test pagination` |
| Tables, notes, headers/footers | `crates/hm-layout/src/table/`, `furniture.rs`, `notes.rs` | `cargo test -p hm-layout` |
| Fonts and character mapping | `crates/hm-fonts/src/lib.rs` | `cargo test -p hm-fonts` |
| Painting, pictures, PDF/SVG | `crates/hm-render/src/lib.rs` | `cargo test -p hm-render` |
| Studio-facing requests and implementations | [WASM map](crates/hm-wasm/README.md) | `cargo test -p hm-wasm --test <name>` |
| Native CLI commands | `crates/hm-cli/src/main.rs` | `cargo test -p hm-cli` |

For a symbol, use `rg -n 'fn <name>' crates/<crate>/src`, then read that function and its direct callers. Locate files with `rg --files crates/<crate>`. Avoid dumping whole crates, generated bindings, corpus files, or historical reports into context.

## Contracts to preserve

- Coordinates are wire units, not UTF-8 bytes or JS string indexes. Lengths use integer HU, 7200 per inch. See [units](specs/document-model/units.md) and [paragraph text](specs/document-model/paragraph-text.md) when changing coordinates.
- Readers retain source bytes. Edits invalidate the source of changed nodes; writers preserve unchanged content. Read/write work needs save and reopen checks in the affected formats.
- Content mutations go through atomic `hm-edit` transactions with valid inverses. Failed operations leave document content, history and epochs unchanged. See [editing contracts](docs/wasm-editing.md) when touching transactions.
- Add WASM request types and TypeScript declarations in `types.rs`, advertise supported operations in `capabilities()`, and test through `Engine`/`Doc`.
- Engine work is independently implemented. Existing old-engine code may establish behavior, but must not be copied, ported or paraphrased.

## Worktree checks

Use `CARGO_BUILD_JOBS=3` and this worktree's own target directory. Run the smallest relevant test first. Corpus tests can skip when `HEEMANG_CORPUS` or fonts are absent; report skips explicitly.

```sh
cargo fmt --all -- --check
CARGO_BUILD_JOBS=3 cargo test -p hm-wasm --test <name>
CARGO_BUILD_JOBS=3 WASM_BINDGEN=/path/to/wasm-bindgen crates/hm-wasm/build.sh
node --test crates/hm-wasm/js/test/<name>.test.mjs
```

`wasm-bindgen` must match the pinned crate version in `crates/hm-wasm/Cargo.toml`. The build writes `target/hm-wasm-pkg/web`; Node checks can select another build with `HM_WASM_PKG`.

Broader engine design is in [architecture](docs/architecture.md); some historical measurements there describe earlier implementation stages.
