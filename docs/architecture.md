# HeemangEngine architecture

HeemangEngine reads, lays out, renders, edits and saves HWP 5.x and HWPX documents for HamaEditor. It targets Hancom Office for Mac: our render matches Hancom page for page, our saved files open in Hancom showing what we showed, and Hancom's saved files open in ours showing what Hancom showed. The plan and its gates live in `heemang-cleanroom/design/plan-v2.md`.

## 1. Principles

1. **Hancom position space.** Paragraph text is stored as HWP 5 wire units: ordinary characters take one UTF-16 unit (two for a surrogate pair), char controls one, inline and extended controls eight, and every paragraph ends with CR. Character shape runs, stored line segments, range tags and edit offsets all index this space. HWPX fills the same representation.
2. **Integer HWPUNIT.** Layout runs in `i32` HU (7200 per inch, 100 per point) with Hancom's MulDiv rounding (round half away from zero, `-1` on a zero divisor or overflow). Floats appear only when a backend maps the display list to device pixels.
3. **Lossless by default.** Every parsed node keeps the source bytes of its record subtree. A clean node is written by copying those bytes; only dirty nodes are re-encoded. Fields we do not interpret ride along as tails and raw records in their original slots.
4. **Our own layout is primary.** The formatter computes every line from fonts and shapes. Hancom's stored line segments are a check: the audit compares them with our lines and reports agreement.
5. **Pay for what is used.** Fonts load lazily from the catalog, glyph outlines are produced only for glyphs that are drawn, and streams the model does not need stay compressed in the container until save.
6. **Hancom facts are data.** Constants, record layouts and measured rules come from the public specifications, the specs in `specs/`, and measurements of Hancom output. Each such rule is written once, next to the code that applies it.

## 2. Crates

Rust stable, edition 2024, resolver 3. Each crate lives in `crates/<name>`. Every crate except `hm-cli` builds for `wasm32-unknown-unknown`.

| Crate | Owns | Depends on |
|---|---|---|
| `hm-model` | Units and MulDiv, `SharedBytes`, the document model, style tables, preservation slots, dirty tracking | none |
| `hm-hwp5` | CFB reader and writer, FileHeader, raw deflate, record layer, DocInfo and BodyText reader and writer | `hm-model`, `miniz_oxide` |
| `hm-hwpx` | ZIP, XML, OWPML reader and writer (phase 2) | `hm-model` |
| `hm-fonts` | Font catalog, face resolution, SFNT metrics, advances and outlines, HFT (phase 2) | `skrifa` |
| `hm-layout` | Style resolution, measurement, line breaking, line metrics, paragraph placement, columns and pages, line segment audit | `hm-model`, `hm-fonts` |
| `hm-render` | Display list and its backends: tiny-skia PNG now, SVG later | `hm-model`, `hm-fonts`, `hm-layout`, `tiny-skia` |
| `hm-edit` | Edit operations, undo and redo, edit scripts (phase 3) | `hm-model`, `hm-layout` |
| `hm-cli` | The `heemang` binary | all of the above, `memmap2` |
| `hm-wasm` | Studio-facing API (phase 4) | |

Layering is strict: the model knows no format, layout knows no renderer, and only `hm-cli` touches the file system. Font bytes and document bytes reach the library crates as byte slices or `SharedBytes`, so the wasm build gets them from JavaScript the same way the CLI gets them from `mmap`.

## 3. Document model (`hm-model`)

### 3.1 Units

`Hu` values are plain `i32`. `units::mul_div(a, b, c)` is Hancom's MulDiv. Paragraph-shape lengths (margins, indent, paragraph spacing, non-percent line spacing, tab positions, the section default tab) are stored doubled in HWP 5; the model keeps the stored value and exposes the effective half value through accessors, so odd values survive a round trip. COLORREF is `0x00BBGGRR`; a non-zero top byte means no color.

### 3.2 Shared bytes

`SharedBytes` is an `Arc<[u8]>` plus a range. Parsing slices the decompressed stream into `SharedBytes` without copying, so a source span, an unread tail or a raw record payload costs one pointer and two integers. Cloning a document clones `Arc`s, not bytes.

### 3.3 Tree

```text
Document
  meta: DocMeta            format, version, file flags, the raw FileHeader
  info: DocInfo            properties, BinData entries, style tables, compatibility, raw records, record order
  sections: Vec<Section>
    paras: Vec<Paragraph>
      header: ParaHeader   shape id, style id, break kind, instance id, raw extras
      text: Vec<u16>       wire units, ends with CR
      runs: Vec<CharRun>   (position, char shape id), first at 0
      line_segs: Vec<LineSeg>   Hancom's stored lines, nine fields each
      range_tags: Vec<RangeTag>
      controls: Vec<Control>    one per extended control slot, in slot order
      extra: Vec<RawRecord>     unknown child records with their slot
  source: Option<SharedBytes>   the original file, for copying untouched container parts
```

`Control` is `{ kind: ControlKind, src }`. Typed kinds today are `SectionDef` (with its page definition, note shapes and page border fills as typed children, other children raw) and `ColumnDef`. Every other control is `ControlKind::Raw`: its control id, its CTRL_HEADER payload and its whole child record tree with relative levels. Phase 2 builders replace `Raw` with typed kinds (tables, pictures, shapes, equations, headers and footers, notes, fields) one at a time; anything they leave raw still saves byte for byte.

Style tables (`DocInfo`) hold face names per script slot (Hangul, Latin, Hanja, Japanese, Other, Symbol, User), border fills, char shapes, tab definitions, numberings, bullets, para shapes and styles. Each entry is a typed struct whose optional trailing fields are detected by remaining length and whose unread bytes are kept as a tail. References keep Hancom's bases: char shape, para shape, style, tab definition and face ids are 0-based; border fill, numbering, bullet and BinData ids are 1-based with 0 meaning none.

### 3.4 Preservation and dirty tracking

- **Source spans.** Each paragraph, control and DocInfo record carries `src: Option<SharedBytes>`: the exact bytes of its record subtree (headers included) in the decompressed stream. `None` means the node was created or modified.
- **Clean means copyable.** A node is clean when its own `src` is present and all its children are clean. Writers copy clean nodes and re-encode dirty ones; a dirty paragraph whose controls are clean re-encodes its own records and copies the controls.
- **Marking.** Every mutation goes through `hm-edit`, which clears `src` on the node it changes. Parents stay clean in their own fields; the deep check finds the dirty child.
- **Record order.** DocInfo keeps the original record sequence as a list of typed slots, so a re-encoded DocInfo emits records in the original order and appends new entries after the last entry of their kind. Unknown paragraph child records keep their index among the paragraph's children.
- **Derived fields.** Counts in headers (char shape runs, range tags, line segments, `ID_MAPPINGS`), the control mask and the last-paragraph bit are regenerated from the model when a record is re-encoded.

### 3.5 Invariants

Runs start at 0 and are sorted; each extended control slot in the text has one control; text ends with CR; style references are in range or resolve to defaults. Out-of-range references never panic.

## 4. HWP 5 reading (`hm-hwp5`)

1. **Detection** by content: CFB magic `D0 CF 11 E0` (HWP 5), ZIP (HWPX), the HWP 3 signature. The extension is ignored.
2. **CFB.** One tolerant reader: validates header fields before allocating, follows FAT, DIFAT and mini FAT chains with revisit and range guards, accepts surplus FAT sectors and chains that reuse a sector (files Hancom opens), and resolves paths case-insensitively by full path. It reads stream data lazily.
3. **FileHeader.** Signature, version, property flags (all bits kept), the raw 256 bytes.
4. **Streams.** Raw deflate when the compressed bit is set; Hancom's 8-byte CRC trailer is ignored on read. Distribution and password documents are detected and reported; decryption is phase 2 work.
5. **Records.** Header DWORD: tag 10 bits, level 10 bits, size 12 bits; size `0xFFF` takes the next DWORD. The reader yields `(tag, level, payload)` as `SharedBytes` without copying and turns truncation into an error for that record only.
6. **DocInfo** parses eagerly into `DocInfo`. **BodyText** parses each `BodyText/SectionN` into paragraphs: header, text, runs, stored line segments, range tags, controls (typed or raw) and unknown records.
7. **Tolerance.** Malformed input never panics. A damaged record keeps its bytes raw and becomes a diagnostic.

## 5. Fonts (`hm-fonts`)

- **Catalog.** The host registers font files (TTF, OTF, TTC; HFT recognized and set aside). The CLI's `--font-path` accepts files and directories read one level deep and maps them with `mmap`. Each face entry records every family name in its `name` table (Korean names included), the PostScript name, weight, italic and collection index. Faces load on first use.
- **Resolution.** For a face name and script slot: exact case-insensitive family match, then the face record's substitute name, then a small alias table, then the slot default chain (serif or sans by the name's class), then any face covering the character. Each result records whether it was exact.
- **Metrics.** `skrifa` supplies units per em, ascent, descent, the charmap and unhinted advances. Outlines come from `skrifa` as path commands in font units.
- **Measurement.** Per character, Hancom's Mac rule: with logical height `h` (base size times relative size), `q = h / 4` truncated, the advance is `MulDiv(adv, q, upem) * 4` at ratio 100 and `(ratio * q * adv) / (upem * 100) * 4` truncated otherwise. Character spacing then adds `MulDiv(advance / 4, spacing, 100) * 4`. A space is half the size unit (`(q / 2) * 4`), independent of the font.

## 6. Layout (`hm-layout`)

### 6.1 Stages

1. **Resolve.** For each run, the char shape gives per-slot face, ratio, spacing, relative size and offset, plus base height and attributes. Faces resolve once per (face id, slot) through `hm-fonts`.
2. **Measure.** Each wire unit gets a script slot and an advance in HU, stored in one `Vec<i32>` per paragraph. Control slots and surrogate tails get 0; tabs are resolved during breaking.
3. **Break.** A sequential width scan per line. Available width is the column width minus margins and indent (first line) or hanging indent. Break opportunities follow the paragraph's Korean unit (word or syllable) and Latin unit (word, hyphen or character); trailing spaces hang; a word wider than the line breaks by character. Line-start and line-end forbidden characters move the break back.
4. **Line metrics.** Each run contributes its logical height and a baseline at 85 percent of it; the line takes the maxima. Spacing per kind: percent adds `MulDiv(h / 4, pct - 100, 100) * 4`; fixed sets the advance to the value; added adds the value; at-least adds `max(value - h, 0)`.
5. **Place.** Spacing before on the first line and after on the last; horizontal alignment inside the line: left, right, center, justify (extra width to spaces, last line left), distribute and divide (extra width across characters, every line).
6. **Paginate.** Sections start new pages. A section's page definition gives the body area; column definitions split it into columns. Lines flow top to bottom; a line that does not fit moves to the next column, then the next page. Page and column breaks on paragraphs are honored; a column definition in a later paragraph opens a new column region below the current one.

### 6.2 Output

`DocLayout` holds per-paragraph `ParaLayout` (lines with start, end, x, width, height, baseline, spacing, the advance array and the per-unit style index) and `Vec<Page>`, where a page lists placed lines with page coordinates. `layout-dump` prints both.

### 6.3 Stored line segment audit

For each paragraph with stored segments, the audit compares line count and break positions, then `vertsize`, `textheight`, `baseline`, `spacing` and `horzpos`/`horzsize` per line. It reports per-document agreement rates and the first diverging paragraphs. It reads stored values only; it never feeds them into layout.

## 7. Rendering (`hm-render`)

- **Display list.** Per page: size in HU and a flat `Vec<Item>`. `Item::Glyphs` carries face, size in HU, horizontal ratio, color, baseline y and `(glyph id, x)` pairs; `Item::Rect` and `Item::Line` cover decorations. Coordinates stay integer HU.
- **Raster.** tiny-skia fills real glyph outlines: each outline is built once per `(face, glyph)` in font units and transformed per draw with one affine (scale `size / upem * dpi / 7200`, the ratio on x, y flipped). Pages render in parallel threads in the CLI. PNG encoding uses tiny-skia's encoder.
- **Order.** Paper, then body text. Objects, borders, fills, images, headers, footers and notes arrive in phase 2 with their own items.

## 8. Saving

### 8.1 Strategy

- A document with no dirty node saves to its original bytes.
- Otherwise the container is rebuilt: untouched streams (BinData, previews, summary, scripts, history, any unknown stream) are copied exactly as stored; DocInfo and touched sections are re-encoded from the model, copying clean nodes from their source spans, then deflated.
- Re-encoding never invents data: a clean node re-encodes to its source bytes, and a test holds the whole corpus to that.

### 8.2 HWP 5 specifics

- The CFB writer emits version 3 (512-byte sectors), keeps directory entry names, CLSIDs, state bits and timestamps, uses the mini stream below 4,096 bytes, and lays out sectors deterministically.
- Edited paragraphs drop stored line segments only when their text changes; Hancom recomputes lines on open.
- Hancom writes an 8-byte trailer (CRC-32 and length) after each deflate stream; our writer emits it too.

### 8.3 Round-trip levels

1. No-op save is byte-identical for every fixture.
2. Touch save (every paragraph and DocInfo record forced dirty) produces decompressed streams identical to the source.
3. Edited saves open in Hancom for Mac without a repair prompt and match our layout of the edit (phase 3 gate G2), and Hancom's re-save renders the same in ours (G3).

## 9. Editing (phase 3)

Edits are typed operations on the model that clear `src` on the nodes they change and record their inverse for undo. Offsets inside a paragraph are wire units. Layout is incremental per paragraph: an edit re-measures and re-breaks the changed paragraph, and pagination resumes from the page holding it until page starts match the previous layout again.

## 10. Performance

- Parse without copying: records and tails are `SharedBytes` into the decompressed stream.
- One contiguous advance array per paragraph; no allocation per glyph in breaking or painting.
- Faces resolve once per (face id, slot); fonts are memory-mapped and parsed on first use; outlines are cached per glyph.
- Rendering parallelizes over pages.
- Reference document: the 382-page handbook (`2025 행정업무운영 편람(최종).hwp`). Open plus full pagination and full PNG export are measured against the previous engine (`parity/bins/r1`); speed claims cite those numbers.

## 11. Testing and parity

- Unit tests next to code (MulDiv, record framing, CFB chains, measurement arithmetic, breaking rules, line metrics).
- Integration tests per crate in `tests/` read the corpus from `HEEMANG_CORPUS` and skip with a message when it is unset: every HWP fixture opens without panic, no-op save is byte-identical, touch save reproduces every decompressed stream, and the line segment audit reports its rates.
- Parity rounds: `round.sh <parity dir> h<n>-<label> --bin <heemang> --prev r1` scores our PNGs against Hancom PDFs per page band. Gate G1: per page worst band no worse than the old engine, same page count.

### CLI contract

```text
heemang info <doc> [--font-path P]...                    summary: format, version, sections, styles, pages
heemang export-png <doc> -o <dir> --dpi N [--font-path P]...   page_001.png, page_002.png, ...
heemang layout-dump <doc> [-p <0-based page>] [--audit] [--font-path P]...
heemang save <doc> -o <out> [--touch]
```

`--font-path` takes TTF, OTF, TTC and HFT files or directories read one level deep, and may repeat. Exit codes: 0 success, 1 usage, 2 open or parse failure, 3 layout or render failure, 4 write failure.
