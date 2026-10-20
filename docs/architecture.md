# HeemangEngine architecture

HeemangEngine is the document engine of HamaEditor. It opens, lays out, renders, edits and saves HWP 5.x, HWPX, HML and HWP 3.0 documents. It serves one app, so every decision below optimizes for that app's runtimes, formats and agent workflows over generality.

## 1. Overview

### 1.1 Runtimes

| Runtime | Build | Role |
|---|---|---|
| Studio worker | wasm32, single thread | Owns the live document in a dedicated Web Worker. Parses, lays out, paints pages into `OffscreenCanvas`, serves the agent API, answers the UI with a line-box index. |
| Hub sidecar | native | `heemang serve --stdio` for documents with no open tab. Same API, no canvas. |
| CLI | native | Exports (PNG, PDF, SVG, text, Markdown), `info`, `layout-dump`, the parity tools. |

### 1.2 Principles

1. **Hancom's formatter is the reference algorithm.** Layout runs the stages Hancom runs, in its order, with its integer arithmetic. Each stage is a pure function over integers with test vectors from `specs/`. Hancom Office for Mac is the target; Hancom-exported PDFs and Hancom-saved line segments are ground truth.
2. **Integer HWPUNIT end to end.** Layout is `i32` HU (7200 per inch) with Hancom's MulDiv rounding. Floats appear only after the display list. wasm and native are bit-identical by construction.
3. **Persistent snapshots.** The document is an immutable tree of `Arc` nodes held in persistent B+trees with monoid summaries. A snapshot clone is a few pointer copies, so forks, undo states, versions and background layout share structure without locks. What changed between two snapshots is found by pointer equality, never by dirty flags.
4. **One operation log.** Every change is a typed, invertible op in a transaction. Undo, redo, fork commit, stale-edit rebase, versions and three-way merge read the same log.
5. **Hancom's position space inside, character offsets outside.** The model stores text exactly as HWP 5 positions it (eight-unit inline and extended controls), so stored line segments, character shape runs and edit offsets share one index space. The agent API translates to character offsets at its boundary.
6. **Lossless by default.** Every parsed node remembers its source byte span. Clean nodes are written by copying bytes; only touched nodes are re-encoded. Unknown records, elements and attributes ride along in their original slots.
7. **Bytes stay outside the wasm heap.** The file, BinData, decoded images and font files live in JS memory (or mmap natively). The heap holds the model, metric tables and caches. wasm memory never shrinks, so the heap is managed as a peak.
8. **Pay for what is seen.** Parsing, layout, image decoding and glyph outlines are lazy and prioritized by what the viewport needs.
9. **Parity is a product feature.** The line-segment audit, band scores against Hancom PDFs and PDF glyph geometry are commands in the repo, run by the local gate on every change.

### 1.3 Reference facts

The handbook fixture (`2025 행정업무운영 편람(최종).hwp`): 10.7 MB, 14 sections, 10,514 paragraphs including cells, 13,876 stored line segments, 256,563 UTF-16 text units, 2.1 MB of decompressed body records, 388 table records, 593 shape components, 435 BinData streams totaling 9.6 MB, 382 pages. Text is small and images dominate the bytes.

The previous engine exports all 382 pages of the handbook to SVG natively in 1.56 s, while its in-app open took about 285 s. The in-app cost was pipeline work (whole-document processing before first paint, image and font handling, heap traffic), not layout math. This design removes those costs structurally.

---

## 2. Crates

Rust stable, edition 2024, resolver 3. All crates live in `crates/` with directory name equal to package name. External dependencies are declared in each crate's own manifest; the root manifest declares members, internal path dependencies, lints and profiles.

### 2.1 Crate table

| Layer | Crate | Responsibility | Main external deps | wasm |
|---|---|---|---|:-:|
| L0 | `hm-units` | `Hu`, `mul_div`, wrapping helpers, the float32 half step, `Packed` distances, unit converters, integer geometry | none | yes |
| L0 | `hm-tree` | `SumTree` (persistent B+tree with monoid summaries), `PVec`, cursors, pointer-equality diff, append-only `Interned<T>` tables | `smallvec` | yes |
| L0 | `hm-io` | `ByteSource` and `SourceStore`, `SrcSpan`, shared `Bytes`, little-endian `Cursor` with tail capture, bounded streaming inflate and deflate, CRC32 | `miniz_oxide` | yes |
| L1 | `hm-model` | Node IDs, nodes, style tables, preservation types, `Doc` snapshot, `IdIndex`, `DocBuilder`, invariant checker | | yes |
| L1 | `hm-text` | Seven-slot script classifier, break classes, forbidden-character tables, grid classes, HChar and Johab conversion, grapheme caret stops | generated tables | yes |
| L2 | `hm-dl` | Display list types, binary encoding, band index, canonical hashing | `xxhash-rust` | yes |
| L2 | `hm-container` | CFB reader and deterministic writer; ZIP reader and writer with raw member copy | | yes |
| L2 | `hm-xml` | Pull parser with element byte spans, raw fragment capture, namespace table, span-splicing writer | `quick-xml` | yes |
| L2 | `hm-fonts` | Font catalog and resolution, SFNT access, HFT decoder, `FaceMetrics`, metric packs, advance tables, outlines | `read-fonts`, `skrifa`, `postcard` | yes |
| L2 | `hm-image` | Image sniffing; native decoders (PNG, JPEG, GIF, BMP, TIFF); crop and effect math | `png`, `zune-jpeg`, `gif` | sniffing and `tiff-lite` |
| L2 | `hm-ops` | Ops, transactions, apply and invert, transform, three-way merge, conflicts, structural diff | | yes |
| L3 | `hm-hwp5` | HWP 5.x: FileHeader, record layer, distribution and password transforms, DocInfo, BodyText, writer | `aes`, `des`, `cfb-mode`, `sha1` | yes |
| L3 | `hm-hwpx` | HWPX (OWPML) reader and writer | | yes |
| L3 | `hm-hml` | HML (HWPML 2.x and 3.x) reader and writer | | yes |
| L3 | `hm-hwp3` | HWP 3.0 reader | | yes |
| L3 | `hm-layout` | The formatter: property resolution, measurement, width scan, line metrics, paragraphs, pages, tables, objects, notes, furniture, incremental engine, line-segment audit | | yes |
| L3 | `hm-extract` | Plain text and Markdown extraction from the model | | yes |
| L3 | `hm-metafile` | WMF, EMF and EMF+ playback into display list items | | yes |
| L3 | `hm-eqn` | Hancom equation script parser and box layout | | yes |
| L3 | `hm-chart` | HWP chart and OOXML chart models and rendering | | yes |
| L4 | `hm-paint` | Page layout to display list: glyph runs, decorations, borders, fills, objects, page furniture | | yes |
| L4 | `hm-query` | Selectors, projections, text-quote anchors, API offset translation | | yes |
| L5 | `hm-raster` | tiny-skia backend with glyph mask cache, PNG encoding | `tiny-skia`, `png` | yes |
| L5 | `hm-svg` | SVG writer | | yes |
| L5 | `hm-pdf` | PDF writer with subset fonts | `pdf-writer`, `subsetter` | no |
| L6 | `hm-api` | Sessions, live document, forks, the six operations, validation, versions, open and export entry points, scheduler-agnostic services | `serde`, `serde_json` | yes |
| L7 | `hm-protocol` | Worker and stdio message schema, binary codecs for hot messages, line-box index encoding | | yes |
| L8 | `hm-wasm` | Worker entry, cooperative scheduler, command-buffer backend, JS byte and font providers | `wasm-bindgen`, `js-sys` | only |
| L8 | `hm-cli` | The `heemang` binary | `clap`, `memmap2`, `rayon` | no |
| tool | `hm-parity` | Band scorer, PDF glyph geometry extractor, line-segment audit reports, corpus runner | `lopdf`, `png` | no |
| tool | `hm-testkit` | Corpus discovery through `HEEMANG_CORPUS`, spec vector loader, golden helpers (dev-dependency only) | | no |

Outside `crates/`: `xtask/` (the local gate, wasm build, size report, spec vector codegen, compat coverage), `benches/` (criterion benches as `hm-bench`, wasm heap probe), `web/` (worker bootstrap, canvas command player, line-index module, test page), `specs/`, `docs/`, `parity/` (committed baselines only).

### 2.2 Layering rules

- A crate depends only on crates in lower layers. `hm-layout` never depends on a renderer; it uses object sizes stored in the model and calls `hm-eqn` or `hm-chart` sizing only inside an edit that changes their content.
- Every crate marked for wasm in the table passes `cargo clippy --target wasm32-unknown-unknown -- -D warnings`; `hm-pdf`, `hm-cli`, `hm-parity` and `hm-testkit` are native only.
- Feature flags: `complex-shaping` (harfrust for Arabic, Indic and Thai, outside parity scope), `atlas` (glyph atlas painter), `fallback-calibration` (experimental fitted fallback widths, off), `threads` (rayon natively).
- The wasm build is single-threaded on stable Rust. Shared-memory wasm needs a rebuilt standard library with atomics, which stable does not provide. All shared data is immutable `Arc` or `Send + Sync`, so a threaded worker build is a flag away when stable allows it. Natively, rayon parses sections, measures paragraphs and renders pages in parallel.

### 2.3 Build profiles and size

- `release`: `lto = "fat"`, `codegen-units = 1`.
- `wasm-release`: inherits `release`, `panic = "abort"`, `opt-level = 3` for hot crates (`hm-units`, `hm-tree`, `hm-io`, `hm-text`, `hm-fonts`, `hm-layout`, `hm-dl`, `hm-paint`) and `opt-level = "z"` for cold ones (`hm-hwp3`, `hm-hml`, `hm-chart`, `hm-metafile`, `hm-eqn`, `hm-extract`), then `wasm-opt -O3`.
- `parity`: `release` without LTO for fast parity rounds.
- Worker module budget: 1.6 MB raw, 550 KB brotli, tracked with `twiggy` by the gate.
- No serde, regex or `format!` on hot paths in wasm. Hot worker messages and the line index use hand-written fixed layouts. The agent API and CLI JSON use serde (cold path).

---

## 3. Document model

### 3.1 Positions

`TextPos(u32)` indexes UTF-16 units in HWP 5 wire position space: ordinary characters take one unit, surrogate pairs two, char controls (line break, hyphen, non-breaking and fixed-width spaces) one, inline controls (tab, field end and the like) and extended controls (tables, objects, fields, notes, section and column definitions) eight, and every paragraph ends with CR. Character shape runs, stored line `textpos`, range tags, edit offsets and line-index caret stops all use this space. HWPX and HML readers build the same representation, giving each inline control element its eight-unit slot.

The agent API speaks **API offsets**: UTF-16 units of the paragraph's plain-text projection, where each control is one U+FFFC unit. `hm-query` translates per paragraph through the sorted control table, in O(controls). The UI treats `TextPos` values from the line index as opaque and sends them back unchanged.

### 3.2 Node IDs

```rust
/// block:16 | serial:48. Never reused within a session.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct NodeId(NonZeroU64);
```

- **Blocks.** Content of section stream or part `k` gets block `1 + k`; auxiliary parts (HWPX master pages and similar) get blocks from `0x8000`; nodes created during the session get block `0xFFFF` from one counter shared by the live document and all forks.
- **Serials** follow document order within a block at parse time. Lazy, partial and parallel section parsing therefore assign the same IDs as a full sequential parse, and reopening the same bytes reproduces every ID. A persisted op log replays against the reopened file.
- **Kinds** live in the node, not the ID. API form: kind letters, block in base36 (`x` for the session block), a dot, serial in base36: `p3.1kq`, `tc3.2a0`, `px.7`. Kinds: `s` section, `p` paragraph, `r` run, `t` table, `tr` row, `tc` cell, `o` object, `f` field, `n` note, `h` header, footer or master page, `b` bookmark, `m` memo.
- **Lifecycle.** A split keeps the ID on the left part and allocates one for the right part. A join keeps the first ID and retires the second. A move keeps the ID. Copy and paste allocate new IDs. Undo restores original IDs (restoring is not reuse). Redo replays the IDs recorded in its ops. Retired IDs are recorded in a persistent `retired -> survivor` map, so stale anchors still resolve.

### 3.3 Snapshot

```rust
#[derive(Clone)]                       // O(1)
pub struct Doc {
    pub meta: Arc<DocMeta>,            // format, version, file flags, properties, summary
    pub styles: Arc<StyleTables>,
    pub sections: SumTree<SectionItem>,
    pub bins: Arc<BinTable>,           // BinData: source location, kind, compression, size if known
    pub raw: Arc<RawContainer>,        // streams, storages and parts the model does not interpret
    pub ids: IdIndex,                  // NodeId -> parent, persistent
    pub source: Arc<SourceStore>,      // original bytes, outside the wasm heap
}
```

`SumTree<T>` is a persistent B+tree (fan-out 16 to 32) whose nodes cache a monoid summary of their subtree. Updates copy one root-to-leaf path. It holds section lists, every paragraph list (section bodies, cells, text boxes, headers, footers, notes, captions), table rows, text ropes above 4,096 units, the layout mirrors and the page list.

`IdIndex` is a two-level `PVec`: one per block, indexed by serial, storing the parent `NodeId`. Position within a large body comes from a per-body position table built on first use and shared until that body changes.

### 3.4 Paragraphs and text

```rust
pub struct Body { pub id: NodeId, pub paras: SumTree<ParaItem> }   // ParaItem(Arc<Para>)

pub struct ParaSummary {
    pub paras: u32,
    pub units: u64,
    pub controls: u32,
    pub numbering: CounterXform,         // composed list, outline and note counters (5.11)
}

pub struct Para {
    pub id: NodeId,
    pub shape: ParaShapeId,
    pub style: StyleId,
    pub head: ParaHeadRaw,               // break flags, instance id, list-end bit, unknown bits
    pub text: Text,                      // wire position space, ends with CR
    pub runs: Runs,                      // SoA: starts, shapes, run ids; covers the text
    pub controls: SmallVec<[(TextPos, Arc<Control>); 1]>,
    pub ranges: Option<Box<[RangeTag]>>,
    pub stored: Option<Arc<StoredLines>>,// saved line segments as loaded
    pub origin: Origin,
    pub extra: Option<Box<Extra>>,
}

pub enum Text { Small(SmallVec<[u16; 30]>), Shared(Arc<[u16]>), Rope(SumTree<TextChunk>) }
```

- Runs carry IDs. Typing inside a run keeps its ID; formatting part of a run splits it with the left part keeping the ID. Adjacent equal runs are not merged in memory (that would retire IDs); writers merge them on output.
- `StoredLines` keeps the nine stored fields per segment (`textpos`, `vertpos`, `vertsize`, `textheight`, `baseline`, `spacing`, `horzpos`, `horzsize`, `flags`) in compact form. An edited paragraph is a new node without stored lines; the layout engine keeps the frozen prefix from the paragraph's previous layout (5.5).

### 3.5 Controls, objects and tables

```rust
pub struct Control { pub id: NodeId, pub kind: Ctrl, pub obj: Option<Box<ObjectCommon>>,
                     pub origin: Origin, pub extra: Option<Box<Extra>> }

pub enum Ctrl {
    SectionDef(Arc<SectionDef>), ColumnDef(ColumnDef),
    Header(Furniture), Footer(Furniture), MasterPage(Furniture), Footnote(Note), Endnote(Note),
    AutoNumber(AutoNumber), NewNumber(NewNumber), PageNumberPos(PageNumberPos), PageHide(PageHide),
    Bookmark(Bookmark), FieldBegin(Field), FieldEnd { field: NodeId },
    Table(Arc<Table>), Picture(Picture), Shape(Arc<Shape>), Equation(Equation),
    Ole(Ole), Chart(Chart), TextArt(TextArt), Form(FormControl), Memo(Memo),
    Dutmal(Dutmal), CharOverlap(CharOverlap), HiddenComment(Body), IndexMark(IndexMark),
    Unknown(UnknownCtrl),                // ctrl id, raw header bytes, raw child record tree
}

pub struct ObjectCommon {
    pub anchor: AnchorRel, pub inline_as_char: bool,
    pub x: Packed, pub y: Packed, pub w: i32, pub h: i32, pub size_rel: SizeRel,
    pub wrap: Wrap, pub wrap_side: WrapSide, pub z: i32, pub margins: [i16; 4],
    pub allow_overlap: bool, pub keep_in_page: bool, pub caption: Option<Caption>,
    pub matrix: Option<Box<ObjMatrix>>, pub instance_id: u32, pub tail: Tail,
}

pub struct Table { pub id: NodeId, pub props: TableProps, pub rows: SumTree<RowItem> }
pub struct Row   { pub id: NodeId, pub cells: SmallVec<[Arc<Cell>; 8]> }
pub struct Cell  { pub id: NodeId, pub addr: (u16, u16), pub span: (u16, u16), pub size: (i32, i32),
                   pub padding: [i16; 4], pub border_fill: BorderFillId, pub list: ListHeadRaw,
                   pub body: Body, pub origin: Origin, pub extra: Option<Box<Extra>> }
```

Section-level controls (section and column definitions, headers, footers, master pages, numbering, page hiding) stay inline in their paragraphs, exactly where the formats put them. A column change in the middle of a section is a normal inline control. The API presents them as section properties by locating the governing control. Shapes form their own tree (`Shape { geom, line, fill, effects, children, text: Option<TextBox> }`).

### 3.6 Styles

```rust
pub struct StyleTables {
    pub faces: [Interned<FaceName>; 7],      // Hangul, Latin, Hanja, Japanese, Other, Symbol, User
    pub char_hot: Interned<CharShapeHot>,    // what measurement reads
    pub char_cold: Interned<CharShapeCold>,  // colors, decoration detail, tail bytes
    pub para_hot: Interned<ParaShapeHot>,
    pub para_cold: Interned<ParaShapeCold>,
    pub border_fills: Interned<BorderFill>, pub tabs: Interned<TabDef>,
    pub numberings: Interned<Numbering>, pub bullets: Interned<Bullet>,
    pub styles: Interned<Style>, pub memo_shapes: Interned<MemoShape>,
    pub track_changes: Interned<TrackChangeInfo>, pub forbidden: Option<Arc<ForbiddenChars>>,
    pub compat: CompatProfile,               // target program, five flag words, unknown bits kept
    pub extra: Extra,                        // uninterpreted DocInfo or header records, in order
    pub file_counts: FileCounts,             // entries [0, n) are the file's, in file order
}

#[repr(C)]
pub struct CharShapeHot {                    // about 50 bytes
    pub face: [u16; 7], pub ratio: [u8; 7], pub spacing: [i8; 7],
    pub rel_size: [u8; 7], pub offset: [i8; 7], pub height: i32, pub attr: u32,
}
```

- `Interned<T>` is append-only: chunks of 256 entries behind `Arc` plus a hash index for deduplication. Adding a shape copies the last chunk. Entries are never mutated, removed or reordered, so indices in clean paragraphs stay valid and clean paragraphs copy byte for byte.
- Ops carry style **values**, never indices. Applying an op interns the value into the target's tables, so a fork and the live document can both add shapes and still merge.
- Raw distance words (paragraph margins, indents, spacing, tab positions, object offsets) stay `Packed`: value shifted left one, bit 0 meaning percent of a basis supplied at use. Layout decodes them; HWPX and HML readers encode their XML quantities into this form, so the formatter has one code path.

### 3.7 Preservation

```rust
pub enum Origin {
    None,                                                  // created or modified in this session
    Hwp5 { stream: StreamRef, span: Span32, level: u8 },   // span in the decompressed stream
    Xml  { part: PartRef, span: Span32 },                  // span in the inflated part
}

pub struct Extra {
    pub records: SmallVec<[RawRecord; 1]>,   // unmodeled HWP5 child records: tag, relative level, slot, payload
    pub attrs: SmallVec<[RawAttr; 0]>,       // unmodeled XML attributes, original order
    pub children: SmallVec<[RawXml; 0]>,     // unmodeled XML elements, with slot among known children
    pub tail: Tail,                          // payload bytes beyond the known fields
}
```

- **Cleared by construction.** Every edit path-copies, and the copy constructor sets `origin = None` on each copied node. A node whose `Arc` came from the parser is clean by definition. Undo restores the old `Arc`s, so clean nodes become byte-copyable again.
- **Granularity.** Sections, paragraphs, controls, tables, rows and cells each carry an origin. A change deep in a table re-encodes the cell, its row, the table and the containing paragraph; every sibling is copied.
- **Spans, not buffers.** Origins point into the source; writers re-inflate the original stream once at save time. Unknown records and attributes that must survive re-encoding are small and live in `Extra`.
- **Patch on copy.** A few positional fields change while a node stays clean (the HWP 5 last-paragraph bit when a paragraph is appended after it, `ID_MAPPINGS` counts, `itemCnt` attributes). Writers copy the bytes and patch those fields in place.
- **Unmodeled containers.** `PrvImage`, `Scripts`, `DocHistory`, `DocOptions`, `XMLTemplate`, summary information, OLE storages, `settings.xml`, `version.xml`, previews and unknown parts live in `RawContainer` as references and are copied without decompression.

### 3.8 Invariants

`Doc::check()` verifies: runs cover the text; each eight-unit control slot has exactly one control; table grids cover every slot exactly once; IDs are unique and match `IdIndex`; style indices are in range; stored line `textpos` values are monotonic. Debug builds and tests run it after every transaction; release builds run per-op local checks.

---

## 4. Parsing

### 4.1 Shared machinery

```rust
pub trait ByteSource: Send + Sync {
    fn len(&self) -> u64;
    fn read(&self, offset: u64, dst: &mut [u8]) -> Result<()>;   // native: mmap; wasm: copy from a JS ArrayBuffer
    fn slice(&self, range: Range<u64>) -> Option<&[u8]> { None } // zero copy when resident
}

pub struct Opened { pub doc: Doc, pub diagnostics: Vec<Diag>, pub pending: Vec<LazySection> }
pub fn open(src: Arc<dyn ByteSource>, opts: &OpenOptions) -> Result<Opened, OpenError>;
```

- Detection by signature: CFB magic (HWP 5), ZIP whose `mimetype` is `application/hwp+zip` (HWPX), XML root `HWPML` (HML), the HWP 3.0 signature string.
- Inflate streams through a 64 KiB window into the record reader; a record larger than the window is assembled in one reusable buffer. Peak transient memory per stream is the window plus the largest record.
- `RecordReader<'a>` yields `(tag, level, payload, span)` without allocating. `Cursor<'a>` reads little-endian fields with bounds checks that return `Err(Truncated)`; every typed reader keeps unconsumed bytes as `tail`.
- Parse scratch uses a per-section bump arena reset after the section's nodes are built.
- Malformed input never panics. Damaged records become `Diag` entries and keep their bytes raw so a save does not lose them. Every parser has a fuzz target.
- `DocBuilder` assigns IDs per block and records origins.

### 4.2 HWP 5.x

1. **Container.** `hm-container::cfb` reads the header, FAT, DIFAT, mini FAT and directory and resolves streams lazily by name. No stream data is read at this step.
2. **FileHeader.** Signature, version, flag word (compressed, password, distribution, script, DRM, XML template, history, signature, certificate; all bits kept raw), encryption version.
3. **Transforms.** Compressed streams are raw deflate. Distribution documents carry a 256-byte seed record at the start of each protected stream (`ViewText/SectionN`, `Scripts/*`, `DocHistory/*`); the AES-128 ECB key is recovered from it without a secret and the rest is decrypted, then inflated. Password documents use the algorithm the encryption version selects (DES ECB variants, or AES-128 CFB with a SHA-1 derived key) and ask the host for a password. DRM documents fail with `OpenError::Drm`.
4. **Record layer.** Header DWORD: tag 10 bits, level 10 bits, size 12 bits; size `0xFFF` means a following DWORD carries the true size.
5. **DocInfo** is parsed eagerly into `StyleTables` and `DocMeta`, including `COMPATIBLE_DOCUMENT` and `LAYOUT_COMPATIBILITY`. Unknown records go to `StyleTables.extra` in order.
6. **BodyText** is parsed lazily per section: paragraph header, text, char shape runs, line segments, range tags, control headers and their nested records. Unknown controls become `Ctrl::Unknown` with their record tree.
7. **BinData** stays as references (storage path, compression). `PrvText`, `PrvImage` and other storages stay in `RawContainer`.

### 4.3 HWPX

1. `hm-container::zip` reads the central directory; entries inflate on demand and keep compressed bytes plus CRC as source references for raw copy.
2. `META-INF/container.xml` leads to `Contents/content.hpf` (manifest and spine: section order, BinData items). `version.xml`, `settings.xml` and `Contents/header.xml` parse eagerly into the same `StyleTables`.
3. Section parts parse lazily with the visible-first policy. `quick-xml` reads from the streaming inflater with a reused buffer, so a large section part is never resident.
4. Namespaces match by URI (2011, 2016 and 2021 families) with the original prefix table kept for output. `hp:switch` keeps every branch; the reader takes the first `hp:case` whose namespace is supported, else `hp:default`, and the writer emits the original switch.
5. Elements map to the same model types as HWP 5. Inline controls get eight-unit slots; quantities become wire forms; `hp:linesegarray` becomes `StoredLines`. Unknown elements and attributes go to `Extra` with their slot.

### 4.4 HML and HWP 3.0

HML is one XML document with upper-case tags and base64 BinData, read with the `hm-xml` span machinery and its own element mapping, written by span splicing. HWP 3.0 is a read-only converter (HChar text through `hm-text`, no stored line segments) whose result saves as HWP 5 or HWPX.

### 4.5 Laziness tiers

| Tier | When | Work |
|---|---|---|
| 0 | open | container index, FileHeader, DocInfo or `header.xml`, section count |
| 1 | first paint | prefix of the section to show, parsed only until page 1 (or the restored scroll page) is filled; fonts used by it; layout of that page |
| 2 | background | rest of that section, other sections in order, full pagination |
| 3 | on demand | images, OLE, chart data, glyph outlines, metafiles |

A query or transaction that needs later-tier content promotes that work to the front of the scheduler. Until pagination completes, the scrollbar page count comes from stored line segments whose flag bit 0 marks the first line of a page; the real count replaces it as pagination proceeds.

---

## 5. Layout

### 5.1 Arithmetic

```rust
#[repr(transparent)] pub struct Hu(pub i32);   // 7200 per inch, 100 per point

/// 64-bit product, round half away from zero; -1 on zero divisor or i32 overflow.
pub fn mul_div(a: i32, b: i32, c: i32) -> i32;

impl Packed { pub fn resolve(self, basis: impl FnOnce() -> i32) -> i32; }  // >> 1, bit 0 = percent
```

- Additions and subtractions in ported stages use `wrapping_*` where Hancom wraps. Truncating division is `/` on `i32`, annotated. Arithmetic shifts are `>>` on signed values.
- The only float in layout is the float32 half of a metric in relative line metrics, `(x as f32 * 0.5) as i32`, which is IEEE-exact on every target.
- Constants: 7200 HU per inch, mm to HU `mm * 283465 / 1000`, 75 HU per pixel at 96 dpi, default tab 8000 HU. Device conversion happens only in backends.
- Stored line fields keep Hancom's meaning (`vertpos` relative to the flow origin the spec defines, `horzpos` relative to the column start), so the audit compares like with like; page coordinates are derived.

### 5.2 Layout environment

```rust
pub struct LayoutEnv {
    pub compat: CompatProfile,       // five flag words, target program, runtime markers
    pub platform: Platform,          // Mac (default, parity target) or Windows
    pub producer: ProducerFlags,     // creator version bits
    pub fonts: Arc<FontEnv>,         // resolved faces and FaceMetrics for this document
    pub options: LayoutOptions,      // view options that change layout (hidden text shown, revisions)
    pub epoch: u64,                  // part of every cache key
}
```

- Compatibility flags are declared once in `crates/hm-layout/compat/flags.toml` (name, word, bit, spec reference, consumer module) and expanded into named `const fn` accessors. Unnamed bits get raw accessors and are preserved. `cargo xtask compat-coverage` lists flags without a consumer.
- Presets (None, Hwp70, Word, Hunmin, ODT) are constants. Selection follows the specified catalog rule; documents without compatibility records use the specified default per format.
- `Platform::Mac` selects unhinted design-advance measurement. `Platform::Windows` exists for later calibration and is not a parity target.

### 5.3 The formatter stages

Each stage is a pure integer function with spec vectors in `specs/`; `cargo xtask spec-vectors` turns vector tables into generated tests.

- **A. Property resolution.** Paragraph shape (margins, indent, spacing kind and value, alignment, line-break policy bits, heading kind, baseline alignment, keep and widow flags, border fill and offsets, tab definition, numbering, grid use) and each run's character shape (per slot face, ratio, spacing, relative size, offset; base height; decoration, emphasis, super and subscript, font-space attributes). Numbering and bullet prefixes become a virtual prefix run measured in the line.
- **B. Advance cache.** Per paragraph, a contiguous `Box<[i32]>` indexed by `TextPos`. Each unit is classified into a script slot, measured through the slot face's metric pack with `normalized_advance` (quarter-height MulDiv, separate ratio branch, four-HU quantization), then passed through the measured-advance adjustment stage (East Asian and Latin or number spacing, font-space attribute, quotation handling). Missing glyphs fall back through the face's chain and record a fallback event. Surrogate second units and control slots store 0; tabs and inline objects are placeholders resolved by the scan. Edits re-measure the changed units plus the neighbors the adjustment stage reads.
- **C. Width scan and breaking.** A sequential scan per line, as Hancom runs it: accumulate width with character spacing applied in quarter units, character-grid snapping, tab stops resolved at the current x, inline object widths and border transitions. Available width comes from `available_segments(y, h)`, which subtracts indents and exclusion zones and can return several segments (Hancom's continuation segments). On overflow, backtrack in Hancom's policy order: Latin word, hyphenation branch or character per paragraph bits; Korean word or syllable per paragraph bit; forbidden start and end characters with neighbor backtracking; permitted punctuation overrun; trailing whitespace handling. Output per line: start, end, used width, segment index.
- **D. Line metrics.** Per run, the line metric helpers produce height, ascent and the extra above and below from the face's `FaceMetrics` scaled by run height over 1024, with position offsets and the relative-size tables. Inline objects contribute their own height and baseline. The aggregation reducer computes Hancom's seven quantities (line height, eligible height, baseline, eligible baseline, percent reference, extent before, extent after) over the eligible subset, and the spacing tail turns them into `vertsize`, `textheight`, `baseline` and `spacing` per spacing kind (percent, fixed, added, at least). Compatibility flags apply at their specified points.
- **E. Paragraph placement.** Spacing before on line 0 and after on the last line, decoded from `Packed` with the character-unit basis. Line-grid snapping with the native rounding expression. Paragraph border and fill offsets. Horizontal alignment (left, right, center, justify, distribute, divide) inside each segment; justification distributes in HU with the remainder assigned left to right.
- **F. Pages.** The page formatter state machine (5.6).

```rust
pub struct Line {                          // about 40 bytes
    pub start: TextPos, pub end: TextPos,
    pub y: i32,                            // paragraph-relative top
    pub height: i32, pub text_height: i32, pub baseline: i32, pub spacing: i32,
    pub x: i32, pub width: i32, pub used: i32,
    pub extra_segs: Option<Box<[Seg]>>,    // continuation segments around floats
    pub flags: LineFlags,
}

pub struct ParaLayout {
    pub key: ParaKey,                      // content hash, width signature, env epoch
    pub lines: SmallVec<[Line; 2]>,
    pub height: i32,
    pub source: LineSource,                // Computed | Replayed | Reflowed { from_line }
    pub boxes: SmallVec<[InlineBox; 0]>,   // inline objects, equations, dutmal
    pub fallbacks: FallbackSet,
    pub audit: Option<SegAudit>,
}
```

Glyph positions are not stored per line. Paint and the line index materialize them from the advance cache for lines they touch.

### 5.4 Fonts in layout

Layout reads fonts through `FontEnv`: `FaceMetrics` per resolved face, advances by `(face, char)` from dense cmap pages, and the exact-or-substituted flag per slot. Face resolution for a page completes before that page is first painted, so a paragraph never flips between replay and own math after the user has seen it.

### 5.5 Stored line segments: audit, replay and reflow

Every paragraph laid out while still unedited is compared with its stored segments:

```rust
pub struct SegAudit {
    pub breaks: AuditBreaks,          // Match | Diverge { first_line: u16 }
    pub metrics: u16,                 // bitmask: vertsize, textheight, baseline, spacing, horzpos, horzsize
    pub vertpos_delta: i32,           // first segment, after page placement
    pub flags_match: bool,
}
```

| Faces resolved exactly | Stored segments | Untouched paragraph shows | After an edit |
|---|---|---|---|
| yes | agree | own result (identical) | own result |
| yes | disagree | own result; audit recorded | own result |
| no | present | **replay**: stored breaks and vertical metrics, own horizontal placement inside each stored segment (justified lines scaled to the stored width) | **reflow**: lines before the edited line keep their breaks and metrics; from the edited line on, own math with vertical metrics borrowed from stored lines of the same character shape, so nothing above moves and line height does not jump |
| any | absent | own result | own result |

`vertpos` is always recomputed by the page formatter; stored `vertpos` is audited only. The audit feeds `info`, `layout-dump`, validation reports and `heemang parity lineseg`.

### 5.6 Page formatter

```rust
pub enum Step { Continue, NextPage, Restart(Phase), Complete, Error(LayoutError) }

pub struct PageCursor {
    pub section: u32, pub flow: FlowPos,          // paragraph id, line index, cell path inside split rows
    pub column_set: u16, pub column: u8,
    pub carry: Arc<Carry>,                        // notes continued, floats still occupying area, counters
}

pub trait Block {
    fn place(&self, req: &PlaceRequest, cx: &mut PlaceCx) -> PlaceResult;
}

pub struct PlaceRequest<'a> {
    pub region: RectHu, pub y: i32, pub resume: BlockCursor,
    pub exclusions: &'a ExclusionMap, pub first_on_page: bool, pub env: &'a LayoutEnv,
}

pub enum PlaceResult {
    Done { height: i32, placed: Placed },
    Split { height: i32, placed: Placed, resume: BlockCursor },
    Defer,                                        // move to the next column or page
    Restart(Phase),                               // a placed object changes earlier lines on this page
}
```

- The paginator core owns the page loop, flow and columns, section starts, page and column breaks, keep-with-next, keep-lines-together, widow and orphan control, hidden empty lines at page top and blank-page rules. Paragraphs, tables and block objects implement `Block`. Floats register into the page's `ExclusionMap`. Notes and furniture reserve areas through `PageHooks` before body placement.
- A newly placed object that overlaps lines already placed returns `Restart`; the page reruns from its start cursor with the object fixed. The cap is 16 restarts per page; a cap hit is accepted and reported in `layout-dump` and validation.
- Columns: a page is a stack of column regions; a column definition mid-section closes the current region and opens one below it. Distributed regions balance by binary search over cached line heights. Separators follow the column definition and compatibility flags.

```rust
pub struct PageEntry {
    pub start: PageCursor, pub end: PageCursor,
    pub fingerprint: u64,                 // hash(start cursor, carry, page geometry)
    pub content: Arc<PageContent>,        // regions, placed blocks, floats, notes, furniture refs
    pub render_hash: Option<u128>,
}
pub struct Pagination { pub pages: SumTree<PageEntry>, pub complete: bool, pub rev: Revision }
```

### 5.7 Tables

Column boundaries come from stored cell widths; auto-fit recomputes from content minimums only when requested. Row heights take the larger of content height plus padding and the declared height, with merged cells pushing excess into their last spanned row. Edge borders resolve by the specified precedence among cell, zone, table and adjacent fills; diagonals and fills are cell decorations. The table's split mode selects keep whole, split between rows or split inside rows; inside-row splits cut each cell body at a line boundary. Repeated header rows repeat on continuations. Nested tables recurse through cell bodies. A cell body has a layout mirror `SumTree<LayoutItem>` summarizing `{ paras, lines, height }`, so typing in a cell with thousands of paragraphs relays one paragraph, updates one tree path and finds row split points in O(log n).

### 5.8 Objects and wrap

Each anchored object resolves its frame from anchor (paper, page, column, paragraph), offsets, size relations, alignment and `keep_in_page`. `ExclusionMap` keeps y-bands with blocked x-intervals per column: square uses the frame plus margins, tight and through use the contour polygon converted to per-band intervals, top-and-bottom blocks the column width, behind and in front of text block nothing. Objects anchored to paper or page are pre-registered from paragraphs expected on the page; paragraph-anchored objects register as their anchor line is placed. Inline-as-character objects are `InlineBox`es in the line with baseline rules from the profile. Captions are bodies laid out beside their object. Text boxes and groups lay out their bodies with the same paragraph formatter in their frames.

### 5.9 Notes and furniture

- **Footnotes.** Placing a line with a footnote reference reserves the note body height plus separator and spacing from the footnote shape at the bottom of the column. When it does not fit, the line moves on, or the note continues on the next page as specified. **Endnotes** collect per section or document and flow after the body with their own separator.
- **Headers, footers, master pages** are selected per section and page parity (both, odd, even, first) with `PageHide` suppression, placed in the header and footer areas per the section's rules, and laid out once per variant. Page-dependent fields are measured with their actual value; the cache key includes the field values, and a total-page change relays only furniture holding a total-pages field.

### 5.10 Vertical writing

Paragraph and line layout run in logical axes (inline, block). Sections, cells and text boxes with vertical direction map logical to physical in `hm-paint`, and glyph orientation rules apply at run materialization.

### 5.11 Numbering

Each paragraph contributes a `CounterXform` (per level: keep, increment, reset to n; deeper levels reset on increment). Composition is associative, so the body summary holds the composed transform of each subtree and the counter state before any paragraph is an O(log n) query. Inserting a numbered paragraph updates one path. Note and page numbering use the same mechanism in the pagination carry.

### 5.12 Incremental layout and caches

| Cache | Key | Budget (wasm) |
|---|---|---|
| Resolved run styles | char shape id, compat, font epoch | small, never evicted (append-only shapes) |
| Dense cmap pages | face, 256-code-point page: `(glyph, advance)` | inside the font budget |
| Advance cache | paragraph content hash, style epoch, font epoch, compat | 4 MiB LRU |
| Paragraph layout | advance key, para shape, width signature (column width, indent, exclusions relative to the paragraph top) | 3 MiB LRU |
| Layout mirrors | body id | resident (heights for pagination) |
| Cell and table layouts | content hash, width | 2 MiB LRU |
| Furniture | body hash, variant, field values | 1 MiB |
| Pagination | per section | resident |
| Display lists | page fingerprint plus chunk hashes | visible pages plus 2 |

- Content hashes are xxh3-64, computed lazily per `Arc` and memoized, so the live document, forks and undo states hit the same entries.
- **Change set.** After a transaction, a pointer diff of the old and new trees yields the replaced paragraphs in O(changes x log n).
- **Line-local relayout.** A text edit relays from one line before the edited line and stops when a recomputed break lands on an old break shifted by the edit delta with equal carried state.
- **Height-stable fast path.** If the paragraph height and segment widths are unchanged and no float or note moved, pagination is untouched and only the damaged chunk repaints.
- **Pagination convergence.** Otherwise pagination resumes at the page holding the first change and stops at the first page whose start fingerprint equals the old page's at the same cursor; later pages are spliced in. A cascade beyond the viewport finishes in background slices; a transaction asking for validation finishes it synchronously.
- **Sections paginate independently** except for page numbering and first-page parity; natively they run in parallel and a section whose assumed parity was wrong repaginates.
- **Differential guarantee.** After any edit sequence, incremental layout equals from-scratch layout of the same snapshot (page fingerprints, line boxes, display-list hashes). The gate runs randomized edit scripts to enforce it.

---

## 6. Rendering

### 6.1 Display list

```rust
pub struct PageDl {
    pub size: SizeHu,
    pub chunks: Vec<Chunk>,             // one per block: paragraph lines, table slice, object, furniture, note area
    pub ops: OpBuf,                     // SoA: tags, i32 operands, glyph ids
    pub paths: PathArena,               // deduplicated paths in HU
    pub bands: BandIndex,               // 64 horizontal bands -> chunk bitsets
    pub hash: u128,                     // canonical xxh3-128, identical in wasm and native
}
pub struct Chunk { pub owner: NodeId, pub bbox: RectHu, pub hash: u64, pub ops: Range<u32> }

pub enum Op {
    PushClip(ClipRef), PopClip, PushTransform(Affine), PopTransform,
    PushGroup { opacity: u8, blend: Blend }, PopGroup,
    FillRect { rect: RectHu, paint: PaintId },
    FillPath { path: PathId, paint: PaintId, rule: FillRule }, StrokePath { path: PathId, stroke: StrokeId },
    Glyphs { face: FaceKey, size: Hu, ratio: u16, synth: Synth, color: Rgba, origin: PointHu,
             glyphs: Range<u32>, xs: Range<u32>, text: Option<Range<TextPos>> },
    Image { bin: BinId, src: RectPx, dst: Quad, effects: EffectsId },
    Embedded { kind: EmbedKind, id: u32, dst: RectHu },   // equation, chart, metafile sub-lists
    Placeholder { rect: RectHu, kind: PlaceholderKind },
    Mark { node: NodeId, range: (TextPos, TextPos), kind: MarkKind },   // links, bookmarks; paints nothing
}
```

- Coordinates stay in HU; backends apply one affine transform. Glyph x positions are `i32` HU computed at build time from the advance cache plus justification.
- Paints: solid, linear, radial, rect and path gradients, the 48 hatch patterns, image tiles. Strokes carry width, dash, caps, joins, arrows and inset or outset alignment.
- Draw order: paper, page border and fill, master page, behind-text objects, body text and inline objects, in-front objects, header and footer, notes. Per shape: fill, outline, effects, children.
- `hm-paint` emits decorations (underline, strike, emphasis marks, highlight, character borders, outline, shadow, emboss, engrave, ruby) as glyph runs, rects and strokes using the compatibility rules for underline width and position.
- **Damage** is the union of the old and new bounding boxes of replaced chunks, extended to the column bottom when a block's height changed. Backends replay only chunks the band index finds inside the damage.

### 6.2 Worker backend: OffscreenCanvas 2D

- The UI transfers canvases once with `transferControlToOffscreen`: one per visible page up to 200 percent zoom, 1024-pixel tiles above it. Recycled tiles are revealed only after the worker reports their new content.
- **Command buffer.** For a tile and damage rect, wasm writes the intersecting ops, resolved to device space, into a scratch buffer in linear memory. One call into the JS player in `web/` replays it on the 2D context. One wasm-to-JS crossing per tile, never per glyph.
- **Glyphs as `Path2D`.** The first time a glyph is drawn, the buffer carries a `DefineGlyph` op with its outline in font units; the player caches a `Path2D` per `(face, glyph)`. Each line's glyphs combine into one path per color with `addPath` and fill once. Zoom never invalidates glyph paths. Screen text therefore uses the same outlines and positions as the parity PNGs and supports HFT faces without web fonts. A glyph atlas rasterized by `hm-raster` sits behind the `atlas` feature with the same display list, chosen only if worker benchmarks show `Path2D` fills are the bottleneck.
- **Images.** BinData bytes never enter the wasm heap. wasm resolves the source byte ranges (CFB sector runs or ZIP entry range); JS builds a `Blob` from `ArrayBuffer` subarrays, inflates through `DecompressionStream('deflate-raw')` when needed and calls `createImageBitmap` at display size times device pixel ratio. Bitmaps live in a JS LRU (96 MiB). Until ready, the page shows a light placeholder and completion repaints exactly the image rect. TIFF decodes through `hm-image` (`tiff-lite`) at display size. Effects map to `ctx.filter` where exact, otherwise to a cached derived bitmap.
- **Thumbnails** for validation render into a small `OffscreenCanvas` and `convertToBlob`.

### 6.3 Native raster

`hm-raster` renders a `PageDl` into a tiny-skia pixmap at any dpi: paths, gradients, patterns, clips, groups, images through `hm-image`, glyphs from an A8 mask cache keyed by `(face, glyph, ppem bucket, x phase in quarter pixels)`. Outlines are unhinted with linear coverage. Pages render in parallel. This is the backend the parity harness scores.

### 6.4 PDF and SVG

- `hm-pdf` (pdf-writer and subsetter): SFNT faces subset and embedded; HFT glyphs as Type 3 fonts whose procedures fill the decoded outlines; each glyph run one positioned text operator; `ToUnicode` maps; JPEG passthrough, other images Flate; gradients and hatches as shadings and tiling patterns.
- `hm-svg`: glyph outlines in `<defs>` once per glyph, placed with `<use>`; paths, gradients, patterns, clips; images as data URIs or sidecar files; one file per page.

### 6.5 Metafiles, equations, charts, OLE

- WMF and EMF play back into display list items (paths, text through the same font system, embedded DIBs) through `hm-metafile`, cached per BinData as a vector sub-list. They stay vector in PDF and SVG and render identically everywhere.
- `hm-eqn` parses Hancom equation script and lays out boxes in HU with the equation fonts, size, color and baseline of the control. Layout uses the stored object size unless the equation was edited.
- OLE objects and unedited charts render their cached presentation, which is Hancom's own drawing. `hm-chart` renders from data (HWP chart object tree, OOXML chart XML) when no presentation exists or after an edit.

### 6.6 Render hashes

A page's render hash is the canonical xxh3-128 of its display list: integer coordinates, glyph ids, colors, image content hashes, quantized transforms. It is identical in wasm and native and needs no rasterization, so validation reports, the hub and the worker agree.

---

## 7. Fonts

- **Catalog.** Entries carry every family name in the `name` table, PostScript name, weight, width, italic, PANOSE, kind (TTF, OTF, TTC with index, HFT), size and mtime. Natively, `--font-path` accepts files and directories read one level deep, including `.hft`. The app builds the same catalog with `heemang fonts scan --json` over system fonts, user font folders, Hancom's bundled directories when installed and the shipped fallbacks (Noto Sans KR and others), caches it on disk and hands it to the worker. Entries sort by a stable key, so resolution is deterministic for the same files.
- **Resolution.** Per script slot: exact case-insensitive family match with Hancom's case table, HFT-to-TTF substitution, the face record's substitute name and type, alias table, PANOSE nearest, slot default. Tables are data files from `specs/`. Each result records exact or substituted, which drives the replay policy and the validation report's `font_fallbacks`.
- **FaceMetrics.** Five values at a 1024 logical em (ascent, descent, internal leading, external leading, em) plus kind (outline or HFT, which selects the descent sign rule and the 80/20 split) and a CJK-support predicate, computed with integer arithmetic from the tables the spec names.
- **Metric packs.** Per face: cmap as dense 256-code-point pages of `(glyph, advance)`, `FaceMetrics`, and kerning absent. Packs are `postcard`-encoded and cached by file hash (IndexedDB in the app, a cache directory natively), so a face activates in milliseconds.
- **HFT.** The decoder reads the 512-byte header and checksum, width blocks indexed by HChar, records and element chains, the two rolling-XOR stream ciphers and the cubic outline byte code; bitmap strikes are decoded for diagnostics only. HFT glyphs become the same `Outline` type as SFNT glyphs.
- **Outlines.** `skrifa` without hinting for SFNT; the HFT interpreter for HFT. Produced lazily per glyph.
- **Worker residency.** Font files live as JS `ArrayBuffer`s in the worker; wasm copies byte ranges synchronously through `ByteSource`, so a new glyph never waits on a round trip. Only metric packs enter the heap (100 to 300 KB per large CJK face instead of 5 to 20 MB).
- **Measurement model.** Hangul, Latin and CJK are measured per character with explicit positions: no run shaping, no ligatures, no kerning. `complex-shaping` runs harfrust only for scripts that cannot render without it.
- **Identity.** Same bytes, same Rust code, integer arithmetic: `layout-dump` of the corpus is byte-identical between native and wasm, and the gate checks it.

---

## 8. Editing and the agent API

### 8.1 Session

```rust
pub struct Session {
    ids: Arc<IdSpace>,                 // session block counter, shared by live and forks
    live: LiveDoc,
    forks: FxHashMap<ForkId, Fork>,
    versions: VersionStore,
    layout: LayoutService,             // caches shared by live and forks
    fonts: Arc<FontSystem>,
}

pub struct LiveDoc {
    head: Doc, rev: Revision,
    log: Vec<CommittedTx>,
    undo: UndoStack,                   // (Doc before, Doc after, tx) per step
    recent: RevisionRing,              // (rev, Doc) for the last 64 revisions
}

pub struct Fork { id: ForkId, owner: AgentRef, base_rev: Revision, base: Doc, head: Doc, txs: Vec<AppliedTx> }
```

Creating a fork copies two `Doc` values: O(1), under a microsecond. Its layout starts from the live caches and lays out only what it changes. Discarding drops its `Arc`s.

### 8.2 Ops and transactions

```rust
pub enum Op {
    InsertText   { at: ParaPos, text: Utf16, run: RunAssign },               // existing run, or new run with a shape value
    DeleteText   { range: ParaRange, removed: Option<Box<Removed>> },
    FormatChars  { range: ParaRange, patch: CharPatch, prior: Option<Box<RunsSnapshot>> },
    FormatParas  { paras: SmallVec<[NodeId; 4]>, patch: ParaPatch, prior: Option<Box<[ParaShapeVal]>> },
    SetStyle     { paras: SmallVec<[NodeId; 4]>, style: StyleRef, prior: Option<Box<[StyleRef]>> },
    SplitPara    { at: ParaPos, new: NodeId },
    JoinParas    { first: NodeId, second: NodeId, joined_at: Option<TextPos> },
    InsertParas  { body: NodeId, after: Option<NodeId>, paras: Vec<ParaSnapshot> },
    RemoveParas  { ids: Vec<NodeId>, removed: Option<Box<RemovedParas>> },
    MoveParas    { ids: Vec<NodeId>, to_body: NodeId, after: Option<NodeId>, from: Option<Box<Slots>> },
    InsertControl { at: ParaPos, ctrl: ControlSnapshot },
    RemoveControl { id: NodeId, removed: Option<Box<ControlSnapshot>> },
    SetProps      { id: NodeId, patch: PropPatch, prior: Option<PropPatch> },  // object, table, cell, section, field
    InsertRows { table: NodeId, at: u32, rows: Vec<RowSnapshot> },
    RemoveRows { table: NodeId, ids: Vec<NodeId>, removed: Option<Box<[RowSnapshot]>> },
    InsertCols { table: NodeId, at: u32, cells: Vec<CellSnapshot> },
    RemoveCols { table: NodeId, at: u32, n: u32, removed: Option<Box<[CellSnapshot]>> },
    MergeCells { table: NodeId, keep: NodeId, absorb: Vec<NodeId>, removed: Option<Box<[CellSnapshot]>> },
    SplitCell  { table: NodeId, cell: NodeId, rows: u16, cols: u16, new: Vec<NodeId> },
    DefineStyle { style: StyleVal },
    SetDocProps { patch: DocPatch, prior: Option<DocPatch> },
}
pub struct ParaPos { pub para: NodeId, pub at: TextPos }
pub struct ParaRange { pub para: NodeId, pub start: TextPos, pub end: TextPos }

pub struct Transaction { pub author: Author, pub label: Option<SmolStr>, pub ops: Vec<Op>, pub base: Revision }
pub struct AppliedTx  { pub tx: Transaction, pub inverse: Vec<Op>, pub created: Vec<NodeId>,
                        pub touches: TouchSet, pub changed: ChangeSummary }
```

- Ops address nodes by ID plus wire offsets inside one paragraph; never by tree position or style index.
- `apply(doc, op)` fills the inverse data (removed text, prior values) and the touch set (nodes and property keys read and written).
- A transaction applies all ops to a scratch snapshot or none.
- Cross-paragraph replace compiles to delete within the first paragraph, `RemoveParas` for the middle, delete within the last, `JoinParas`, insert.
- Studio typing produces the same ops. Consecutive single-character inserts in one paragraph within one second form one undo step.

### 8.3 Undo and redo

The live document has one linear undo stack shared by the user and direct agent edits. Undo of the top step restores the previous snapshot root in O(1); redo restores the later root. Untouched nodes therefore return pointer-equal and byte-copyable. Reverting a step that is not on top (one agent batch among later user edits) rebases its inverse ops over the later steps with the commit transform and returns conflicts instead of applying them.

### 8.4 Commit, rebase and stale edits

`commit(fork)` transforms the fork's ops over every live op committed since `base_rev`, in order, and applies the result as one transaction (one undo step). If live has not moved, the fork head becomes live directly.

| fork op \ concurrent live op | Insert at j (n units) | Delete [a, b) | Split at s (new Q) | Join P+Q (P had k units) | Remove subtree X | SetProps same node |
|---|---|---|---|---|---|---|
| Insert at i | `i > j`, or `i == j` with live first: `i + n` | `i <= a` same; `i >= b`: `i - (b - a)`; inside: **conflict** | `i > s`: retarget to Q at `i - s` | in Q: retarget to P at `i + k` | inside X: **conflict** | n/a |
| Delete [c, d) | shift; live insert strictly inside: **conflict** | overlap shrinks; disjoint shifts | split into P and Q parts | retarget the Q part | inside X: **conflict** | n/a |
| FormatChars | shift and split like delete | shrink | split | retarget | **conflict** | same attribute with different value: **conflict** |
| SetProps | n/a | n/a | n/a | props of Q: **conflict** | **conflict** | same key: equal is a no-op, different is **conflict**; other keys merge |
| Structural (rows, cols, merge) | n/a | n/a | n/a | n/a | **conflict** | merged-away cell edited by the other side: **conflict** |

```rust
pub struct Conflict {
    pub fork_op: OpRef, pub live_tx: Revision, pub live_author: Author,
    pub node: NodeId, pub kind: ConflictKind,    // DeletedTarget, OverlappingEdit, PropertyClash, StructureClash
    pub fork_text: Option<String>, pub live_text: Option<String>,
}
```

- Commit is atomic by default: any conflict aborts and returns the list; user edits are never overwritten. `on_conflict: "skip"` applies the non-conflicting ops and returns the skipped ones.
- The same transform remaps the user's caret and selection through agent edits on live, and agent-held positions through user edits.
- A `transact` on live whose `expect_revision` is older than head is not rejected: anchors resolve against the snapshot in `recent`, ops rebase over the intervening transactions and apply, or conflicts return. Beyond 64 revisions the call returns `STALE` with the current revision.

### 8.5 Diff and three-way merge

- `diff(a, b)` walks both snapshots with pointer equality, so cost follows what changed. Output: inserted, deleted, modified (text hunks in API offsets, format changes), moved, with IDs, paths and current page locations; `"visual": true` adds before and after thumbnails of changed pages.
- `merge(base, ours, theirs)`. Same lineage (forks, in-session versions, autosave of this session): rebase theirs over ours with the commit transform. Different lineage (file changed on disk, two saved files): align by content first (paragraphs by patience diff over text hash and shape, tables by order, size and first-row text, cells by grid address, objects by kind, size and instance id), adopt base IDs for matched nodes, run Myers diff inside matched paragraphs, turn both sides into op logs and merge them with the same transform. Conflicting regions return both versions; the merged document takes ours by default.

### 8.6 The six operations

| Operation | Purpose |
|---|---|
| `query` | Selector reads with projections |
| `transact` | Atomic typed batch on `live`, a fork id, or `new-fork`; returns created IDs and a validation report |
| `render` | Page or region image (PNG or WebP) at a dpi, for any target |
| `diff` | Structural and textual diff between targets or versions |
| `commit` | Rebase a fork onto live as one undo step, or return conflicts |
| `discard` | Drop a fork |

`transact` takes friendly commands that compile to the closed op set: `replace_text` (by quote or range), `insert_paragraphs`, `delete`, `format`, `set_style`, `table.insert_rows`, `table.remove_rows`, `table.insert_cols`, `table.merge`, `set_cell_text`, `insert_picture`, `set_props`, `set_section`. Every anchor resolves to `(paragraph id, range)` before ops are built.

```rust
pub struct ValidationReport {
    pub revision: Revision,
    pub pages_affected: Vec<Range<u32>>,
    pub page_count: (u32, u32),                 // before, after
    pub overflow: Vec<Overflow>,                // clipped cell text, object off page, row taller than page, unbreakable line
    pub table_splits: Vec<TableSplitIssue>,     // forced split, lost header repeat, cut in a protected row
    pub font_fallbacks: Vec<FallbackUse>,       // requested face, used face, sample chars, nodes
    pub render_hashes: Vec<(u32, u128)>,        // changed pages
    pub thumbnails: Vec<(u32, Bytes)>,          // optional
    pub warnings: Vec<Warning>,                 // fuzzy anchor, unknown style name, restart cap, numbering restarted
    pub timing_us: u32,
}
```

Validation reuses the incremental engine: relayout of changed paragraphs, pagination from the first changed page to convergence, display lists and hashes for affected pages only, thumbnails through `hm-raster` at low dpi.

### 8.7 Query

```
selector    := compound (combinator compound)*
compound    := type? ('#' id)? attr* pseudo*
type        := section | para | run | table | row | cell | object | picture | shape | equation
             | field | header | footer | footnote | endnote | bookmark | memo | heading
attr        := '[' name (op value)? ']'          op: = != ^= $= *= ~= < <= > >=
pseudo      := :nth(n) | :first | :last | :contains("text") | :matches(/re/) | :page(n | a-b)
             | :in(#id) | :has(selector) | :empty | :overflowing | :fallback-font | :level(n)
combinator  := ' ' | '>' | '+' | '~'
```

Projections pick fields (`id`, `kind`, `text`, `style`, `shape.*`, `runs[]`, `page`, `box`, `lines[].box`, `children`, `path`, `revision`, `cells[].{...}`) with `text.max_chars`, `limit` and `cursor`. Layout fields force layout only up to the pages involved. Selectors also accept a JSON AST form. Text-quote anchors (`quote`, optional `prefix`, `suffix`, `within`) resolve against a per-body plain-text projection cached per body version: exact match, then prefix and suffix disambiguation, then whitespace and NFC normalization. Ambiguous anchors return candidates with IDs instead of guessing.

### 8.8 Modes and capabilities

| Mode | Capabilities, enforced in `hm-api` |
|---|---|
| 채팅 | `query`, `render`, `diff`; plans may be approved into another mode |
| 수정 | plus `transact` on `live`; each batch one undo step with instant feedback |
| 에이전트 | plus `transact` on forks, `commit`, `discard`; review at turn end |
| 전체 | plus direct `transact` on `live` without review, save |

### 8.9 Versions

A version is `(label, revision, Doc)` in memory, O(1). Persisted versions and autosave store `(source file hash, engine major version, op log)` in postcard; load-time IDs are deterministic, so the log replays after reopen. Logs refuse to replay across incompatible engine majors and fall back to content-aligned merge. Restoring a version is a snapshot swap recorded as one undo step.

---

## 9. Worker protocol

### 9.1 Ownership

The worker owns the live document whenever a Studio tab has it open. For documents with no tab, the hub runs `heemang serve --stdio`, which speaks the same `hm-protocol` API messages without canvas or line index and serves text and Markdown extraction for reference files. When a tab opens a document the sidecar is editing, the sidecar hands over (save or version handoff) and the worker becomes the owner. There is exactly one owner at any time.

The worker opens its own WebSocket to the local hub, so MCP tool calls never cross the UI thread. The UI receives `changed` events to remap the caret, update review state and label undo steps.

### 9.2 Messages

Hot-path messages use fixed binary layouts in transferred `ArrayBuffer`s; API calls carry JSON strings.

```ts
type ToWorker =
  | { t: "open"; doc: number; bytes: ArrayBuffer; name: string; opts: OpenOptions }   // bytes stay in worker JS
  | { t: "canvas"; slot: number; canvas: OffscreenCanvas } | { t: "release"; slot: number }
  | { t: "viewport"; seq: number; zoom: number; dpr: number; visible: Int32Array }    // (page, rect) pairs
  | { t: "input"; seq: number; edit: Uint8Array }          // insert, delete(dir, unit), enter, tab, format, paste
  | { t: "preedit"; seq: number; at: Uint8Array; text: string; caret: number } | { t: "preedit_end"; seq: number }
  | { t: "select"; seq: number; anchor: Uint8Array; focus: Uint8Array }
  | { t: "undo" | "redo"; seq: number }
  | { t: "api"; id: number; call: string; json: string }
  | { t: "save"; id: number; format: "hwp" | "hwpx" | "hml"; opts: SaveOptions }
  | { t: "font"; req: number; face: number; bytes: ArrayBuffer };                   // answer to need_font

type FromWorker =
  | { t: "opened"; info: string; estimatedPages: number }
  | { t: "pages"; count: number; sizes: Int32Array; complete: boolean }
  | { t: "ack"; seq: number; rev: number; caret: Int32Array; index: ArrayBuffer | null }   // edit plus line-index delta, atomic
  | { t: "index"; page: number; data: ArrayBuffer }
  | { t: "painted"; slot: number; page: number; rev: number; damage: Int32Array }
  | { t: "need_font"; req: number; face: string; style: number }
  | { t: "changed"; rev: number; author: string; ranges: Int32Array }
  | { t: "api_result"; id: number; json: string }
  | { t: "saved"; id: number; parts: ArrayBuffer[] };                                     // streamed Blob parts
```

Every input carries a sequence number. The UI shows its caret optimistically and reconciles on `ack`. The edit and its line-index delta arrive in one message, so caret and text never disagree for a frame. Paint runs in the same worker task right after `ack` and commits with the next frame.

### 9.3 Line-box index

One little-endian `ArrayBuffer` per page, sent for visible pages plus two on each side and patched with deltas on edits:

```
header : magic "HMLI", version, page, revision, n_nodes, n_lines, n_stops, n_boxes, n_styles   (u32)
nodes  : n_nodes x u64        NodeId table (paragraphs, cells)
lines  : n_lines x 16 x i32   node_ix, start, end, stop_ix, x0, x1, top, height, baseline,
                              col_x0, col_x1, clip_ix, cell_ix, style_ix, flags (rtl, vertical, last), reserved
stops  : n_stops x i32        caret x at every valid caret position in the line (page HU)
stopp  : n_stops x u32        TextPos of each stop
stopfl : n_stops x u8         word start, grapheme boundary, hidden
boxes  : n_boxes x 8 x i32    node_ix, kind (picture, shape, table, cell, equation), x, y, w, h, z, parent_ix
styles : n_styles x 6 x i32   family id, size HU, ratio, color, flags, baseline offset (caret-run styling for IME)
```

A dense page is about 8 KB. `web/line-index.ts` provides `hitTest(x, y)`, `caretRect(pos)`, `rangeRects(range)`, `moveVertical(pos, dir, goalX)`, `wordAt(pos)` and `objectAt(x, y)`, all local and synchronous.

### 9.4 IME

The UI thread owns composition. On each composition update it draws the preedit text at the caret (styled from the line's `styles` entry, with underline and its own caret) and sends `preedit`. The worker lays out the line with a provisional gap of the preedit's width (no op, no undo entry), so following text shifts and wraps while the overlay fills the gap. `compositionend` sends the committed text as a normal `input`; its `ack` tells the UI to remove the overlay in the same frame the real glyphs paint. Hangul composition never waits for the worker, and the document never holds half-composed syllables.

### 9.5 Scheduler

Cooperative, with priorities: input and acks; paint of visible damage; layout of the viewport; near pages; background parse and pagination; validation thumbnails and prefetch. Background work runs in 4 ms quanta on immutable snapshots and checks a revision counter between quanta: content-addressed results from a stale revision are kept, page assignments are dropped. When the page is cross-origin isolated, a `SharedArrayBuffer` flag set by the UI with `Atomics.store` preempts background work between units.

---

## 10. Serialization

### 10.1 Strategy

- No-op save returns the original bytes.
- Untouched streams and parts are copied in compressed (and encrypted) form without re-inflation.
- In a touched stream, clean nodes copy their origin spans from the re-inflated source with patch on copy; dirty nodes re-encode from the model and re-emit `Extra` records, attributes, children and tails in their original slots.
- Saving streams: sections serialize one at a time into chunks transferred to JS as `Blob` parts; the container layout is computed from chunk sizes, so the heap never holds the whole output.

### 10.2 HWP 5.x

- `hm-container::cfb` rebuilds the container deterministically with the original directory order, CLSIDs, timestamps and state bits; mini stream for streams under 4,096 bytes; unmodeled storages copied raw.
- DocInfo is copied unless style tables grew; then file records copy, new entries append and `ID_MAPPINGS` counts update.
- A section with no dirty node copies compressed. A dirty section re-encodes paragraph by paragraph and deflates (level 9, raw).
- Edited paragraphs are written with a zero line-segment count and no `PARA_LINE_SEG` (Hancom lays them out on open). `SaveOptions::emit_line_segs` writes computed segments; it becomes the default once corpus break agreement reaches 99.5 percent and Hancom reopen checks pass.
- `PrvText` regenerates from the document text; `PrvImage` is kept or regenerated from page 1 through `hm-raster` on request.
- Distribution documents re-encrypt with a fresh distribution header (deterministic seed in tests). Password documents re-encrypt with AES-128 CFB when the password is known.
- The time-seeded random block some records carry is excluded from byte comparisons.

### 10.3 HWPX

`hm-container::zip` writes `mimetype` first and stored, then raw-copies untouched entries (same compressed bytes, CRC, sizes and order). `header.xml` splices new style entries into their lists and updates `itemCnt`. Section parts are written by walking the original bytes and splicing fresh XML only for dirty paragraphs and dirty controls inside clean paragraphs, with original prefixes, declarations and whitespace elsewhere. Line segment arrays follow the HWP 5 policy. The manifest updates when BinData items change; the preview text regenerates.

### 10.4 HML and cross-format

HML uses the same span-splicing writer over its single document, with base64 BinData spans copied raw. Saving across formats emits every node from the model; content with no counterpart in the target format is listed in a `ConversionReport` returned with the save.

### 10.5 Round-trip levels

1. No-op save is byte-identical for every corpus file.
2. Touch save (every paragraph forced dirty, content unchanged) reparses to an equal model digest, and every untouched node's bytes equal the original.
3. Edited saves reopen in Hancom Office on the reference Mac (scripted export to PDF) and match the engine's own layout of the same edit.

---

## 11. Performance techniques

### 11.1 Aims and levers

| Aim | Required | Design target | Levers |
|---|---|---|---|
| Handbook first page painted | under 2 s | under 0.3 s with warm font cache, under 0.6 s cold | prespawned worker, tier 0 and 1 only, partial section parse, metric packs only, images deferred |
| Full pagination in background | under 15 s | about 1 s wasm single thread | integer formatter, advance and paragraph caches, sections parsed in background, 4 ms quanta |
| Typing p95, body text | under 8 ms | about 1 ms worker, one frame end to end | O(1) op apply, incremental re-measure, line-local relayout, height-stable fast path, chunk damage, index delta in the ack |
| Typing p95, giant cell | under 10 ms | about 2 ms worker | cell `SumTree` with height summaries, O(log n) row height and split update |
| Engine heap, handbook | at most 38.5 MiB | about 29 MiB peak | bytes outside the heap, bounded transients, compact model, LRU caches |
| Fork creation | O(1) | under 1 microsecond | `Arc` snapshot |
| 10-op transact with validation, 100 pages | under 50 ms | under 10 ms | shared content-addressed caches, convergence, hash-only display lists |

### 11.2 First-paint budget (handbook, wasm)

| Step | Estimate |
|---|---|
| Worker and module ready (prespawned, compiled module cached) | 0 ms on the open path |
| CFB directory and FileHeader | under 1 ms |
| DocInfo (HWP) or `header.xml` (HWPX) | 15 ms or 40 ms |
| Inflate and parse the first 200 KB of the visible section | 3 ms |
| Face activation for page 1 (cached metric packs; cold reads in parallel) | 2 ms warm, 50 to 250 ms cold |
| Measure and lay out page 1 | 1 ms |
| Display list and paint of visible tiles | 5 to 10 ms |

### 11.3 Heap budget (handbook, wasm)

| Component | Budget |
|---|---|
| Model: paragraphs, text, runs, controls, tables, stored lines, ID index | 10 MiB |
| Style tables | 1.5 MiB |
| Op log and undo snapshots (shared structure) | 2 MiB |
| Font metric packs (about 20 faces) | 3 MiB |
| Advance, paragraph and table caches (LRU) | 9 MiB |
| Layout mirrors and pagination | 1 MiB |
| Display lists for visible pages plus 2, tile command buffer | 1.5 MiB |
| Transient: inflate window, XML buffer, largest record | 1 MiB |
| Allocator slack | 2 MiB |

Outside the heap: source bytes, decoded images, glyph `Path2D`s, font files, canvases.

### 11.4 Techniques

- **Data layout:** SoA runs, hot and cold style split, 40-byte lines, `i32` HU everywhere, contiguous advance arrays, dense cmap pages, `SmallVec` for short lists, `Box<[T]>` for frozen data.
- **Allocation:** bump arenas for parse scratch, reused scratch buffers in layout and painting, no allocation in the width-scan inner loop, no per-glyph allocation.
- **Algorithms:** sequential width scan (lines are short, so a linear scan is cheap and exact), `SumTree` summaries for heights, counts and numbering, fingerprint convergence, band index for damage, pointer-equality diff.
- **Hashing:** xxh3 with fixed seeds; `rustc-hash` maps in hot loops.
- **Laziness:** the four tiers, outline and image deferral, layout projections on demand.

### 11.5 Measurement

- `benches/` (criterion, native): `open_first_page`, `paginate_all`, `parse_handbook`, `measure_paragraph`, `break_paragraph`, `typing_body` (1,000 scripted inserts with relayout and damage), `typing_giant_cell` (5,000-paragraph cell), `fork_transact_validate_100p`, `rebase_1000_ops`, `save_noop`, `save_dirty`, `inflate_section`.
- The wasm heap probe runs the wasm build in Node and reports `WebAssembly.Memory` high water after open and after full pagination.
- The worker bench (headless Chrome) measures open to first paint, full pagination, typing latency through the Event Timing API plus worker marks, and heap high water.
- Results append to `benches/history.jsonl`; the gate flags regressions above 10 percent. Every performance claim cites a bench diff.

---

## 12. Testing and parity

### 12.1 The gate

There is no hosted CI. `cargo xtask check` is the gate every change passes before merge: `cargo fmt --check`; `cargo clippy -- -D warnings` natively and for wasm32 over the worker crates; unit and integration tests; the spec-vector tests; the line-segment ratchet; the native and wasm `layout-dump` identity check on the identity corpus; the module size budget. `cargo xtask parity` runs band scores and is required for layout and paint changes.

### 12.2 Test layers

- **Unit tests** next to code: `mul_div` vectors including overflow and negative divisors, `Packed` decoding, record framing, CFB sector chains, ZIP edge cases, crypto vectors, HChar conversion for all 11,172 modern syllables, HFT decoding, advance normalization, line metric and spacing vectors, break rules, `SumTree` operations, every op's apply and invert.
- **Property tests** (`proptest`): `invert(apply(op))` restores the document; rebase convergence for non-conflicting pairs; merge symmetry; IDs unique and never reused; invariants after random transactions; selection remapping stays in bounds.
- **Integration tests** per crate in `tests/`: every corpus file opens without panic; HWP and HWPX twins produce equal model digests (listed representation differences aside); layout invariants per page; round-trip levels 1 and 2.
- **Differential layout:** incremental equals from-scratch after randomized edit scripts.
- **Fuzzing** (`cargo fuzz`): record reader, CFB, ZIP, XML readers, HFT decoder, equation parser, metafile interpreter, op applier.

### 12.3 Parity instruments

- **Line-segment ratchet.** `heemang parity lineseg <corpus>` lays out every paragraph with stored segments and reports exact break agreement, per-field metric agreement and vertpos agreement, by feature (plain text, tables, floats, notes) with the worst paragraphs. Current rates live in `parity/lineseg-baseline.json`; the gate fails when any rate drops and raises the baseline when rates rise. The corpus holds about 200,000 stored segments, so this signal needs no Hancom at test time.
- **Band scores.** `heemang parity score --doc <file> --ref <pdf | png dir> --dpi 200` renders through `hm-raster` and scores each page per horizontal band against Hancom's PDF rendered to PNG, reporting the worst band per page. Target: under 1 percent mismatch in the worst band of every page. Results are JSON per round; a regression on any page blocks the change.
- **Glyph geometry.** `heemang parity geom --doc <file> --ref <pdf>` extracts glyph origins, sizes and baselines from Hancom's PDF content streams (`lopdf`) and matches them to the display list in reading order, reporting per-line dx and dy in HU. It pins a defect to a line and a quantity.
- **Corpus.** `HEEMANG_CORPUS` points at the fixtures; private documents never enter the repo. The repo commits only self-authored probe documents and their Hancom PDFs.
- **Second opinion.** The previous engine's text export and structure counts are a cross-check for extraction and parsing; Hancom PDFs and Hancom-saved line segments decide disagreements.

### 12.4 CLI contract

```
heemang export-png <doc> -o <dir> --dpi <n> [--font-path <file-or-dir>]...    page_001.png, page_002.png, ...
heemang export-pdf <doc> -o <file> [--font-path ...]
heemang export-svg <doc> -o <dir> [-p <0-based page>] [--font-path ...]
heemang export-text <doc> [-o <file>]
heemang export-markdown <doc> [-o <file>]
heemang info <doc> [--audit] [--font-path ...]     JSON: pages, sections, fonts used, fallbacks, compat profile, audit rates
heemang layout-dump <doc> [-p <page>] [-o <file>] [--font-path ...]   JSON: pages, regions, lines, glyph boxes, objects, audit
heemang fonts scan [--font-path ...] --json
heemang serve --stdio
heemang parity score | geom | lineseg | corpus
```

`--font-path` accepts TTF, OTF, TTC and HFT files and directories read one level deep. Exit codes: 0 success, 1 usage, 2 open or parse failure, 3 layout or render failure, 4 write failure. Contract tests check names, counts, codes and JSON schemas.

### 12.5 Worker tests

Headless Chrome runs open, scroll, type, IME composition and agent transact flows against the worker with timing and memory capture.
