# HeemangEngine architecture

HeemangEngine reads, lays out, renders, edits and saves HWP 5.x and HWPX documents for HamaEditor. It targets Hancom Office for Mac: our render matches Hancom page for page, our saved files open in Hancom showing what we showed, and Hancom's saved files open in ours showing what Hancom showed. The plan and its gates live in `heemang-cleanroom/design/plan-v2.md`; Hancom layout facts with sources live in `heemang-cleanroom/design/facts/`.

## 1. Principles

1. **Hancom position space.** Paragraph text is stored as HWP 5 wire units: ordinary characters take one UTF-16 unit (two for a surrogate pair), char controls one, inline and extended controls eight, and every paragraph ends with CR. Character shape runs, stored line segments, range tags and edit offsets all index this space. HWPX fills the same representation.
2. **Integer HWPUNIT.** Layout runs in `i32` HU (7200 per inch, 100 per point) with Hancom's MulDiv (round half away from zero, `-1` on a zero divisor or overflow). Floats appear only when a backend maps the display list to device pixels.
3. **Lossless by default.** Every parsed node keeps the source bytes of its record subtree. A clean node is written by copying those bytes; only dirty nodes are re-encoded. Fields we do not interpret ride along as tails and raw records in their original slots.
4. **Our own layout is primary.** The formatter computes every line from fonts and shapes. Hancom's stored line segments are a check: the audit compares them with our lines and reports agreement; layout never reads them.
5. **Pay for what is used.** Fonts are memory-mapped and parsed on first use, outlines are built only for drawn glyphs, and streams the model does not need stay compressed in the container until save.
6. **Hancom facts are data.** Constants, record layouts and measured rules come from the public specifications, `specs/`, the facts map and measurements of Hancom output. Each rule is written once, next to the code that applies it.

## 2. Crates

Rust stable, edition 2024, resolver 3. Each crate lives in `crates/<name>`. Every crate except `hm-cli` builds for `wasm32-unknown-unknown`.

| Crate | Owns | Depends on |
|---|---|---|
| `hm-model` | Units and MulDiv, `SharedBytes`, the document model, style tables, preservation slots, dirty tracking | none |
| `hm-hwp5` | CFB reader and writer, FileHeader, raw deflate, record layer, DocInfo and BodyText reader and writer | `hm-model`, `miniz_oxide` |
| `hm-hwpx` | ZIP, XML, OWPML reader and writer (phase 2) | `hm-model` |
| `hm-fonts` | Font catalog, face resolution, glyph lookup, unhinted advances and outlines; HFT recognized (decoding is phase 2) | `skrifa` |
| `hm-layout` | Style resolution, measurement, line breaking, line metrics, paragraph heads, placement, columns and pages, object bands, the line segment audit | `hm-model`, `hm-fonts` |
| `hm-render` | Display list and the tiny-skia PNG backend; SVG later | `hm-model`, `hm-fonts`, `hm-layout`, `tiny-skia` |
| `hm-edit` | Edit operations, undo and redo, edit scripts (phase 3) | `hm-model`, `hm-layout` |
| `hm-cli` | The `heemang` binary | all of the above, `memmap2` |
| `hm-wasm` | Studio-facing API (phase 4) | |

Layering is strict: the model knows no format, layout knows no renderer, and only `hm-cli` touches the file system. Document and font bytes reach library crates as `SharedBytes` or `Arc<dyn AsRef<[u8]>>`, so wasm gets them from JavaScript the way the CLI gets them from `mmap`.

## 3. Document model (`hm-model`)

### 3.1 Units and bytes

`Hu` is `i32`; `units::mul_div` is Hancom's MulDiv; `units::unpack_distance` decodes packed distances (value shifted left once, bit 0 marking a percentage of a supplied basis). Paragraph margins, indent, paragraph spacing, non-percent line spacing, tab positions and the section tab interval are packed; the model keeps the stored word and decodes at use. `SharedBytes` is an `Arc<[u8]>` plus a range: parsing slices decompressed streams without copying, so a source span, a tail or a raw payload costs a pointer and two integers.

### 3.2 Tree

```text
Document { meta: DocMeta, info: DocInfo, sections: Vec<Section>, source: Option<SharedBytes>, diagnostics }
  DocInfo   properties, id mappings, BinData entries, faces[7], border fills, char shapes, tab defs,
            numberings, bullets, para shapes, styles, compatibility, raw records, record order
  Section   { paras: Vec<Paragraph>, src, source_index }
  Paragraph { header, text: Vec<u16>, runs, line_segs, range_tags, controls: Vec<Control>, extra, src }
  Control   { kind: Section(SectionDef) | Columns(ColumnDef) | Raw(RawControl), src }
```

`Paragraph::units()` walks the text as characters, one-unit controls, inline controls and extended controls (each extended control maps to the next entry of `controls`). `SectionDef` holds its page definition, note shapes and page border fills as typed children; everything else under it stays raw. `RawControl` keeps the control id, the CTRL_HEADER payload and the descendant records with relative levels; `object_common()` reads the leading object fields (anchor, offsets, size, margins, wrap) without changing the control. Phase 2 replaces raw kinds (tables, pictures, shapes, equations, header and footer, notes, fields) with typed ones; whatever stays raw still saves byte for byte.

DocInfo entries are typed structs whose optional trailing fields are `Option`s detected by remaining length, with unread bytes in `tail`. A record that ends early keeps the fields read so far and its table position. References keep Hancom's bases: char shape, para shape, style, tab definition and face ids are 0-based; border fill, numbering, bullet and BinData ids are 1-based.

### 3.3 Preservation and dirty tracking

- **Source spans.** Paragraphs, controls and DocInfo entries carry `src: Option<SharedBytes>`, the exact bytes of their record subtree. `None` marks a created or changed node. A section's `src` is its decompressed stream, cleared when the paragraph list itself changes.
- **Clean means copyable.** A node is clean when its own `src` is present and its children are clean. Writers copy clean nodes and re-encode dirty ones; a dirty paragraph with clean controls re-encodes its own records and copies the controls.
- **Marking.** Every mutation goes through `hm-edit`, which clears `src` on the node it changes. `mark_all_dirty` forces full re-encoding for tests.
- **Order and slots.** DocInfo keeps the source record order as typed slots; new entries are inserted after the last entry of their kind. Unknown paragraph child records keep their slot among the known children.
- **Derived fields.** Header counts, `ID_MAPPINGS` (stored counts plus entries added since reading), the control mask and the last-paragraph bit are regenerated when a record is re-encoded. Hancom leaves the paragraph end out of the control mask and sets the soft-hyphen bit only sometimes, so that bit follows the stored mask.

## 4. HWP 5 reading and writing (`hm-hwp5`)

- **CFB.** One tolerant reader: bounded FAT, DIFAT and mini FAT chains with revisit guards, surplus FAT sectors, sector reuse, case-insensitive full-path lookup. The writer emits version 3 files that keep the source directory (names, tree links, colors, CLSIDs, state bits, timestamps) and lay sectors out deterministically.
- **FileHeader** gives version and flags. Password documents fail with `Encrypted`; distribution documents open their stub body with a diagnostic (decryption is phase 2).
- **Streams** are raw deflate when compressed; writers append Hancom's CRC-32 and length trailer.
- **Records**: tag 10 bits, level 10 bits, size 12 bits with the `0xFFF` extension. Framing yields `SharedBytes` payloads; truncation ends the stream with a diagnostic and keeps what came before.
- **DocInfo** parses eagerly into typed tables; **BodyText** parses each section into paragraphs, typed section and column definitions, raw controls and extra records. Malformed input never panics.
- **Writing.** A clean document returns its source bytes. Otherwise the container is rebuilt: DocInfo and dirty sections are re-encoded (clean nodes copied, record levels shifted when a subtree moves), deflated, and every other stream (BinData, previews, summary, scripts, history, unknown) is copied as stored. Adding or removing sections is not supported yet.

## 5. Fonts (`hm-fonts`)

- **Catalog.** `FontSystem::add(label, bytes)` registers TTF, OTF and TTC files (each collection face is an entry) with every family name of the `name` table in all languages, the PostScript name, weight and italic. HFT files are recorded and skipped. The CLI maps `--font-path` files and directories (one level deep).
- **Faces** load on first use. A face caches charmap and advance lookups per character and outlines per glyph (`skrifa`, unhinted, font units).
- **Resolution.** A face record resolves by exact family (case and space insensitive), PostScript name, the record's substitute and default names, a short alias table, then the class default of its name (serif or sans chains starting with 함초롬바탕 and 함초롬돋움). Characters a face lacks fall back to the first class default face that has them. Results record whether the requested name matched; the audit uses that to separate exact paragraphs.

## 6. Layout (`hm-layout`)

### 6.1 Measurement

- **Script slot** per character (Hangul, Latin, Hanja, Japanese, Other, Symbol, User) follows Hancom for Mac: ASCII and Latin-1 punctuation and curly quotes are Latin; CJK punctuation, arrows, shapes, letterlike symbols, enclosed alphanumerics and Greek are Symbol; spaces take the previous slot.
- **Styles** resolve once per (char shape, slot, face): logical size (base times relative size, times 16/25 for super- and subscript), ratio, spacing, offset, color, decorations.
- **Glyph advance**: `q = |size / 4|`; ratio 100 gives `4 * MulDiv(adv, q, upem)`, other ratios `4 * trunc(ratio * q * adv / (upem * 100))`. Character spacing adds `4 * MulDiv(cache / 4, spacing, 100)`.
- **Blanks**: U+0020 measures `4 * (h / 8)` from the Hangul slot (relative size, then ratio), the fixed-width space `4 * (h / 16)`, both with the Latin slot's spacing; the no-break space takes no spacing. Char shape bit 25 measures U+0020 with the font instead.
- **Automatic spacing** (paragraph flags 2 bits 4 and 5, unless compatibility word 1 bits 9 and 10) adds a quarter of the base size between Latin letters or digits and CJK.
- **Inline objects** (treat-as-character) take their width and height plus outer margins; an equation sits on its stored baseline percentage, other objects at 85 percent.
- **Paragraph heads**: bullets, numbers and outline numbers come from the numbering and bullet tables with document-order counters and Hancom's 15 number shapes. The head is measured with its own char shape, takes its width plus text distance from the first line and seeds the first line's metrics.

### 6.2 Breaking

A sequential width scan per line. Available width is the column width (quantized down to a multiple of 4 HU) less margins and the first-line indent or hanging indent. A character fits when its advance before character spacing fits; equality fits. Condensing (paragraph bits 9-15) lets a candidate borrow that percentage of the line's blank widths. Lines can start after blanks and at every boundary between the Latin run class and other units; inside a run only when that run's policy breaks by character (Latin mode 2, Korean bit 7). In character-break paragraphs the break moves back at most twice past Hancom's forbidden line-end and line-start lists. A word wider than the line breaks by character. Tabs take explicit stops in stored order with per-kind thresholds (left after the position, right after the following run's end, center and decimal after their midpoint), automatic hanging-indent and right-margin stops, then the section interval, all measured from the column's left edge.

### 6.3 Line metrics and alignment

Each run, the paragraph end included, contributes its logical size as height and 85 percent as baseline; the line takes independent maxima. Percent spacing adds `4 * MulDiv(ref / 4, pct - 100, 100)` over the tallest text run; fixed sets the advance, added adds the value, at-least adds what the value exceeds the height by. Placement gives whole quotients of positive slack: justify fills eligible blanks (blanks after visible content and after the last tab), or character gaps when there are none, except on a paragraph's last line and lines ending in a line break; distribute fills blanks and gaps on every line; divide fills blanks; right and center offset the line. Negative slack shrinks blanks by at most a fifth of the narrowest blank and character gaps by the rest. The last visible glyph counts without its spacing in the used width.

### 6.4 Pages and columns

Each section starts a page; the body is the paper less margins, header and footer areas. Column definitions split the body into columns; a column definition in a later paragraph opens a region below the current one. A line is admitted when its top plus height stays strictly inside the region; paragraph spacing before is applied on the first line, after on the last. Page and column breaks, page-break-before, keep-lines-together, keep-with-next (with the whole next paragraph when it keeps its lines) and one-line widow and orphan control are honored. Floating top-and-bottom objects anchored to the paper, page or paragraph block their band of the page; lines move below them.

### 6.5 Output and audit

`DocLayout` holds per paragraph a `ParaLayout` (lines with positions, segment and content boxes, metrics, the per-unit advance and style arrays, the head) and the pages with placed lines (`x`, `y`, and `vpos` as Hancom stores it). `place()` turns a line into positioned items. `audit()` compares line starts, metrics, segment geometry, `vpos` and tab widths with the stored segments and splits paragraphs measured with the document's own faces from substituted ones. Over the 383 HWP 5 fixtures with Hancom's bundled TTF fonts, 97.9 percent of paragraphs measured with exact faces break exactly as stored (92.4 percent overall).

## 7. Rendering (`hm-render`)

`build_page` turns a page into a `DisplayList` in HU: glyph runs (face, size, ratio, color, synthetic bold and italic, baseline, glyph ids with x) and rectangles for underline and strikeout. `raster::render` fills real glyph outlines with tiny-skia at any dpi, one affine per glyph (scale `size / upem * dpi / 7200`, ratio on x, y flipped, italic shear); synthetic bold strokes the outline. The CLI renders pages on parallel threads and writes PNG.

## 8. Saving

Round-trip levels: (1) a no-op save is byte-identical; (2) a touch save that re-encodes every record reproduces every decompressed stream; (3) edited saves open in Hancom without repair and match our layout (gate G2), and Hancom's re-save renders the same in ours (G3). Levels 1 and 2 hold for every HWP 5 fixture (DocInfo records that other writers cut short are exempt from level 2). Edited paragraphs keep their stored line segments until `hm-edit` decides otherwise; Hancom recomputes lines on open.

## 9. Editing (phase 3)

Edits are typed operations on the model that clear `src` on the nodes they change and record their inverse for undo. Offsets inside a paragraph are wire units. Layout is incremental per paragraph: `para::format` is pure over a paragraph, the tables and a column width, so an edit re-formats the changed paragraph and pagination resumes from its page until page starts match again.

## 10. Performance

Parsing copies no record payloads; measurement keeps one advance array per paragraph; faces resolve once per (char shape, slot); outlines are cached per glyph; pages render in parallel. Reference document: the handbook `2025 행정업무운영 편람(최종).hwp` (382 pages in Hancom). With Hancom's fonts, open takes about 11 ms and full layout 14 ms; a 96 dpi PNG export of every page takes 0.23 s wall at 104 MB peak. These numbers cover body text only: table contents, images and shapes are not laid out or drawn yet, so the layout yields 366 pages. The previous engine takes 0.29 s to open and paginate and 38 s (no font paths) to 100 s (Hancom's fonts) to export the same document to PNG with everything drawn.

## 11. Testing and parity

- Unit tests next to code: MulDiv, packed distances, unit walking, record framing, CFB, deflate, advance and blank arithmetic, script slots, number shapes.
- Corpus tests read `HEEMANG_CORPUS` (and `HEEMANG_FONTS` for layout) and skip when unset: `hm-hwp5/tests/corpus.rs` (every fixture opens, no-op and touch saves) and `hm-layout/tests/lineseg.rs` (break agreement ratchet).
- Parity rounds: `round.sh <parity dir> h<n>-<label> --bin <heemang> --prev r1` scores PNGs against Hancom PDFs per page band. Gate G1: per page worst band no worse than the old engine, same page count.

### CLI

```text
heemang info <doc> [--font-path P]...
heemang export-png <doc> -o <dir> [--dpi N] [-p <page>] [--timing] [--font-path P]...   page_001.png, ...
heemang layout-dump <doc> [-p <page>] [--audit] [--timing] [--font-path P]...           JSON
heemang save <doc> -o <out> [--touch]
```

`--font-path` takes font files or directories read one level deep and may repeat. Exit codes: 0 success, 1 usage, 2 open or parse failure, 3 render failure, 4 write failure.
