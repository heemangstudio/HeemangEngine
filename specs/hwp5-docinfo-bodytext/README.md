# hwp5-docinfo-bodytext: HWP 5 DocInfo, BodyText and controls

This unit turns framed HWP 5 records into the document model: the DocInfo reference tables (fonts, border fills, character and paragraph shapes, tabs, numbering, bullets, styles and the compatibility records) and every BodyText section (paragraphs, text with control characters, runs, stored line segments, range tags, and all controls with their nested paragraph lists).

## Connections

| Direction | Peer unit | Contract |
| --- | --- | --- |
| In | `hwp5-container` | One decompressed (and for distribution documents decrypted) DocInfo byte stream and one byte stream per section, each already split into records (tag, level, payload). Record framing, tag numbering and the 4095 extended-size rule live there ([records.md](../hwp5-container/records.md)). |
| In | `hwp5-container` | The FileHeader version (needed only to explain optional trailing fields; every optional field here is detected by remaining length, never by version). |
| Out | `document-model` | DocInfo tables, sections, paragraphs, controls. The meaning of each model value (units, defaults, reference bases) is specified in [document-model](../document-model/README.md); this unit states which bytes produce it. |
| Out | `hwp5-container` post-parse passes | Auto-number controls with their stored numbers, note controls, section start numbers, and the document start numbers from DOCUMENT_PROPERTIES. |
| Out | layout, pagination | Stored line segments (preserved or dropped by the rule in [paragraph-records.md](paragraph-records.md)), page definitions, column definitions, master pages, note shapes. |
| Out | HWP 5 writer | Raw payload bytes for every record this unit does not fully model, the unread tail bytes of partly modelled records, and the original stream bytes for unchanged sections. |
| Out | OLE, chart, equation and form units | Raw OLE/chart records, equation scripts, form property strings. |

## Files

| File | Contents |
| --- | --- |
| [docinfo.md](docinfo.md) | Every DocInfo record: document properties, id mappings, BinData entries, face names per language slot, border fills and the fill block, character shapes, tab definitions, numbering, bullets, paragraph shapes, styles, document data, compatibility, layout compatibility, memo shapes, forbidden characters, track changes |
| [paragraph-records.md](paragraph-records.md) | BodyText record tree, paragraph header, paragraph text decoding (control characters, 16-byte control blocks, special spaces, fields), character shape runs, stored line segments, range tags, control-to-record association, cross-paragraph field linking |
| [section-and-page.md](section-and-page.md) | Section definition, page definition, footnote and endnote shape (28-byte form), page border and fill, column definition, master pages (base and extended), headers and footers |
| [controls.md](controls.md) | Non-object controls: footnote, endnote, hidden comment, auto number, new number, page hide, odd/even adjust, page number position, index mark, bookmark, character overlap, dutmal, fields, forms, unknown controls |
| [objects.md](objects.md) | Common object attributes, captions, tables and cells, shape component (matrices, line, fill, shadow), text boxes, line, rectangle, ellipse, arc, polygon, curve, connector, picture, OLE and chart, group, equation |
| [tolerance.md](tolerance.md) | Malformed input: truncated payloads, oversized counts, nesting caps, orphan records, zero-size master lists; the failure granularity |
| [parity-and-gaps.md](parity-and-gaps.md) | Corrections to the public specification found in Hancom files, Hancom-parity rules, old-engine bugs, overfit rules, performance notes, open questions |
| [test-vectors.md](test-vectors.md) | Checkable cases with sample paths, record locations and expected values, labelled Hancom ground truth or old-engine behavior |

## Conventions used in this unit

- All integers are little-endian. `u8/u16/u32` are unsigned, `i8/i16/i32` signed. HWPUNIT is 1/7200 inch; HWPUNIT16 is a signed 16-bit HWPUNIT.
- A string written as "HWP string" is a `u16` character count followed by that many UTF-16LE code units. Lone surrogates decode to U+FFFD and decoding continues.
- COLORREF is `u32` laid out `0x00BBGGRR` (red in the low byte). A nonzero top byte marks "no color" (Hancom uses `0xFFFFFFFF` for an empty fill face and `0xFF000000` for an unset pattern color).
- FourCC control ids are defined big-endian (`'t','b','l',' '` is `0x74626C20`) and stored as a little-endian `u32`, so the bytes in the file read reversed (` lbt`). The same reversed order appears inside 16-byte control blocks in paragraph text.
- "Offset" is a byte offset inside a record payload. For CTRL_HEADER the offsets count from the start of the payload, so the control id occupies bytes 0-3 and the control's own fields start at 4 unless stated otherwise.
- Reference bases: character shape, paragraph shape, style, tab definition and face name ids are 0-based. Border fill, numbering, bullet and BinData ids are 1-based, with 0 meaning none.
- Doubled lengths: paragraph-shape margins, indent, paragraph spacing, non-percent line spacing, tab stop positions and the section default tab are stored at twice the HWPUNIT value (see [document-model/units.md](../document-model/units.md)). This unit reads the stored value unchanged; consumers halve.

## Main findings for builders

1. The public specification is wrong in several record layouts that Hancom actually writes. Each was confirmed against Hancom-saved HWP files and their Hancom-saved HWPX twins; the full list with evidence is in [parity-and-gaps.md](parity-and-gaps.md). The most consequential:
   - BORDER_FILL stores each side as an interleaved `type, width, color` triple, not three arrays; line type 0 is none and 1 is solid.
   - The fill block uses a `u8` gradient type and five `u32` gradient fields, not `i16` fields; image fill stores contrast before brightness.
   - FOOTNOTE_SHAPE is 28 bytes: the separator length is 4 bytes and the three gaps are above-line, below-line and between-notes in that order.
   - Object text wrap codes are 0 square, 1 top-and-bottom, 2 behind, 3 in front.
   - Section definition bits 29, 30 and 31 announce base master pages for both, even and odd pages, and their LIST_HEADERs appear in that order. A positional "both, odd, even" assignment is wrong.
   - Picture and rectangle corner coordinates are stored as interleaved (x, y) pairs.
   - LIST_HEADER is 8 bytes: paragraph count, then the list property word at offset 4 (bits 0-2 text direction, 3-4 line wrap, 5-6 vertical alignment), not 6 bytes.
2. Every 16-byte control block in paragraph text ends with the same code unit it starts with, and every extended control code has exactly one CTRL_HEADER child in the same order (205,809 of 205,809 paragraphs in the corpus).
3. Bit 31 of the paragraph character count marks the last paragraph of its list (46,913 of 46,913 cases), and a paragraph with no PARA_TEXT record always has a count of 1.
4. Optional trailing fields are detected by remaining length. Never branch on the FileHeader version for record layouts.
5. A record that fails to decode must not take its siblings down. One truncated record must not drop its whole section.

## Builder questions

No `design/spec-requests.md` existed when this unit was written; no builder questions are answered here. Open questions are listed at the end of [parity-and-gaps.md](parity-and-gaps.md).
