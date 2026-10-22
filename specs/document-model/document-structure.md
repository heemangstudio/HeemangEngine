# Document structure

## Tree

A parsed document has:

| Part | Contents |
|---|---|
| File header | format version (major, minor, build, revision), property flags (compressed, password, distribution-only, and others), raw 256-byte header for round trip |
| Document properties | section count, start numbers (page, footnote, endnote, picture, table, equation), caret position (list id, paragraph id, character position) |
| Document tables | binary data list, font faces per language (7 lists), border fills, character shapes, tab definitions, numberings, bullets, paragraph shapes, styles, memo shapes, unmodeled records, compatibility settings |
| Sections | ordered list; each has a section definition and a paragraph list |
| Preview | thumbnail image (PNG, BMP or GIF) and preview text |
| Binary payloads | image, OLE and font bytes keyed by storage id, loaded lazily |
| Preserved extras | unmodeled container streams (HWP 5.0) and unmodeled package entries (OWPML: `version.xml`, `settings.xml`, `Preview/*`, scripts, unknown entries) |
| Provenance | source format, conversion lineage, see [provenance.md](provenance.md) |

Paragraph lists also appear inside controls: table cells, text boxes of drawing objects, captions, headers, footers, footnotes, endnotes, master pages, hidden comments, memo field bodies. Every paragraph list obeys the same rules as the section body.

## File header flags (HWP 5.0)

| Bit | Meaning |
|---|---|
| 0 | streams are raw-deflate compressed |
| 1 | password protected |
| 2 | distribution-only document (body in `ViewText`, encrypted) |
| 3 | script stored |
| 4 | DRM |
| 5 | XML template stored |
| 6 | document history stored |
| 7 | electronic signature |
| 8 | certificate encryption |
| 9 | signature pre-save |
| 10 | certificate DRM |
| 11 | CCL document |
| 12 | mobile optimized |
| 13 | personal-information security document |
| 14 | track changes document |
| 15 | KOGL (공공누리) license document |
| 16 | contains video controls |
| 17 | contains table-of-contents field controls |

Converting a distribution-only document to an editable one clears bit 2, regenerates the header, and drops the distribution data record (tag 0x01C, `HWPTAG_DISTRIBUTE_DOC_DATA`) from the document tables. The body text is already decrypted at parse time.

## Document table references

Indices that controls, paragraphs and shapes use to refer into the document tables. Getting the base wrong shifts every lookup by one.

| Reference | Base | 0 means |
|---|---|---|
| Paragraph shape id (paragraph header) | 0 | first paragraph shape |
| Character shape id (run list, numbering head, bullet, style) | 0 | first character shape |
| Style id (paragraph header) | 0 | first style (Normal) |
| Font id per language (character shape) | 0, within that language's face list | first face of that language |
| Tab definition id (paragraph shape) | 0 | first tab definition (usually empty) |
| Border fill id (paragraph shape, cell, table, character shape, page border, zone) | 1 | none |
| Numbering id (paragraph shape, head type number or outline) | 1 | none (for outline, see section outline numbering) |
| Bullet id (paragraph shape, head type bullet) | 1 | none |
| Section outline numbering id | 1 | none |
| Binary data reference (picture, image fill, image bullet) | 1, position in the binary data record list | none |
| Memo shape id | 1 | none |

Out-of-range references resolve to defaults (see each table's defaults) and never panic.

### Binary data records and payloads

Each binary data record has a type (link, embedding, OLE storage), compression mode (storage default, always compressed, never compressed), access state, link paths (absolute and relative, link type only), storage id, and extension.

- A picture's binary reference is the 1-based position of the record in the document-table list, not the storage id. Example: `samples/hwpspec.hwp` references binary item 1, whose record has storage id 0x000C (`BIN000C.png`).
- The payload stream name is `BinData/BIN%04X.<ext>` from the storage id (upper-case hex, 4 digits).
- Stream compression: record "storage default" follows file header bit 0; the other two modes override it.
- An OLE storage payload may carry a 4-byte size prefix before the compound file signature; strip it when bytes 4..12 are the signature and bytes 0..8 are not.
- Link records have no payload. A picture that references a link record shows the linked file if available beside the document, else a placeholder.
- OWPML: binary items come from the package manifest; `binaryItemIDRef` names a manifest item. The old engine also gives OWPML chart parts sparse ids from 60001 upward; lookup falls back from position to id equality only when the position is out of range.
- New embedded items get storage id = max(existing storage ids, existing payload ids) + 1. Positional numbering (count + 1) collides with sparse ids and swaps images on save.

## Section definition

Fields of the section definition control (`secd`), in HWP 5.0 order:

| Field | Type | Meaning |
|---|---|---|
| Attributes | UINT32 | bits below |
| Column gap | HWPUNIT16 | gap between columns on the same page (also in the column definition) |
| Vertical line grid | HWPUNIT16 | 0 off, n = grid pitch in HU |
| Horizontal character grid | HWPUNIT16 | 0 off, n = grid pitch in HU |
| Default tab interval | HWPUNIT | doubled scale (8000 stored = 40 pt) |
| Outline numbering id | UINT16 | 1-based numbering used by outline-head paragraphs whose own numbering id is 0 |
| Page number | UINT16 | 0 continue from previous section, n restart at n |
| Picture, table, equation start numbers | UINT16 × 3 | 0 continue, n restart |
| Representative language | UINT16 | 5.0.1.5 and later |
| Trailing bytes | | preserved |

Child records: page definition, footnote shape, endnote shape (in that order; the first note shape record is footnotes, the second endnotes), page border fill (first one; further ones are preserved), master page lists.

Attribute bits:

| Bits | Meaning |
|---|---|
| 0 | hide header on the first page |
| 1 | hide footer on the first page |
| 2 | hide master page on the first page |
| 3 | hide page border on the first page |
| 4 | hide page background on the first page |
| 5 | hide page number position on the first page |
| 8 | page border on the first page only |
| 9 | page background on the first page only |
| 16-18 | text direction: 0 horizontal, 1 vertical |
| 19 | hide empty lines (at most two empty lines at the top of a page get zero height) |
| 20-21 | page number rule when a section break starts a new page: 0 continue, 1 odd, 2 even |
| 22 | manuscript-paper (원고지) orthography |

Observed attribute values in Hancom files: 0x00000000 (most), 0x20000000, 0xC0000000, 0xC0000004, 0x00080000. Bits 29-31 have no public meaning; preserve them.

A section's first paragraph carries the section definition and column definition as its first controls. Later paragraphs may carry further column definitions (column changes mid-section). The old engine also treats a section-definition control met inside the paragraph list as authoritative for that section's grid checks.

## Raw preservation contract

The model distinguishes values it understands from bytes it carries.

- Every document-table record, control header, and list header keeps its unparsed tail bytes.
- Unknown records (document tables and body) are kept with tag, level and payload in their original order.
- Unknown controls keep their control id, payload and child records with relative levels (direct child = 1).
- Unknown OWPML package entries are kept byte for byte at their path. `settings.xml` is read for the print crop-mark setting (`PrintCropMark` non-zero draws 1 cm corner crop marks when printing or exporting PDF, Hancom behavior) and otherwise preserved.
- The OWPML head tail between `</hh:refList>` and `</hh:head>` (compatibility document, document options, track-change configuration) is preserved verbatim; the compatibility switches inside it are also parsed (see [provenance.md](provenance.md)).
- OWPML memo properties (`hh:memoProperties`), numbering and bullet `paraHead` children (10 levels, alignment flags), field `hp:parameters`, and other structures the model only partly represents are kept as raw fragments and spliced back.
- Equality of two shapes (for interning on edit) ignores raw bytes.

## Style interning on edit

Formatting edits never mutate a shared shape in place. The editor clones the base shape, applies the change, and searches the table for an equal shape (raw bytes ignored); it reuses that id or appends a new one. The search is linear; see the performance note in [known-gaps.md](known-gaps.md).
