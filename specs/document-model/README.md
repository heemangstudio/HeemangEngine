# Document model

What a document is after parsing, independent of the source format (HWP 5.x, HWPX, HML, HWP 3.0), and how a run's effective formatting is resolved from the shared tables.

## Scope

In scope:

- The document tree: document header, document-level tables (DocInfo in HWP 5.0 terms, `head` in OWPML terms), sections, paragraphs, controls, binary data.
- Text: UTF-16 position space, control characters and their widths in that space, inline control anchoring, special spaces, fields, highlight ranges.
- Character and paragraph shapes: field meanings, bit layouts, defaults, unit scales.
- Effective formatting of a character: language slot selection, per-slot font, size, ratio, spacing, offset, font substitution.
- Styles, paragraph numbering and bullets, auto numbers.
- Tables, cells, merges, cell margins.
- Objects: common object attributes, shape components, pictures, equations, forms, captions, binary data references.
- Page definitions, page areas, binding and gutter, columns, page borders, headers, footers, master pages, footnote and endnote shapes.
- Fields, hyperlinks, bookmarks, memos.
- Provenance: which source format a document came from and which layout behaviors change with it.
- Units and conversions.

Out of scope (other units): record and XML decoding (parser units), serialization, line breaking and text measurement, pagination and page-number placement, painting, editing commands. This unit states what the parsed values mean and which defaults apply; consumers state how they use them.

## Inputs and outputs

- Input: parser output for every format. Every parser fills the same model. Where a format has no field for a model value, the default listed here applies.
- Output: the model consumed by layout (shapes, stored line segments, page areas, object attributes), painting (resolved colors, fills, borders), editing (positions, runs), and writers (raw preservation).

## Files

| File | Contents |
|---|---|
| [units.md](units.md) | HWPUNIT, signed and unsigned forms, doubled paragraph-shape lengths, color format, percent fields, pixel and point conversions, rounding |
| [document-structure.md](document-structure.md) | Document tree, id tables and reference bases, section definition, raw preservation contract, binary data references |
| [paragraph-text.md](paragraph-text.md) | Position space, control character classes and widths, paragraph length formula, control anchoring, special spaces, fields in text, range tags and highlights, stored line segments, character shape runs |
| [char-shape.md](char-shape.md) | Character shape fields and attribute bits, defaults, per-language values, font face table |
| [effective-formatting.md](effective-formatting.md) | How a character's effective font, size, ratio, spacing and offset are resolved; language slot classification; font substitution and fallback; weight policies; script and offset constants |
| [para-shape.md](para-shape.md) | Paragraph shape fields and attribute bits, line spacing (legacy and current fields), tab definitions, resolved defaults |
| [styles-numbering.md](styles-numbering.md) | Styles, paragraph numbering and bullets, numbering state, number formats, auto numbers |
| [tables.md](tables.md) | Table and cell model, grid and spans, cell margin selection, page break codes, header rows, defaults |
| [objects.md](objects.md) | Common object attributes (with the stored text-wrap mapping), shape component, pictures, equations, forms, captions, border and fill |
| [page-and-notes.md](page-and-notes.md) | Page definition, page areas, binding, columns, page border, headers and footers, master pages, page hide, footnote and endnote shapes |
| [fields-hyperlinks.md](fields-hyperlinks.md) | Field controls, command grammars (click-here, hyperlink, cross reference, formula, memo), properties bits, bookmarks |
| [provenance.md](provenance.md) | Source format, conversion lineage, layout compatibility flags and what each changes, font metrics policy, HWPX compatibility switches |
| [known-gaps.md](known-gaps.md) | Bugs, overfit rules and unverified assumptions in the old engine |
| [test-vectors.md](test-vectors.md) | Checkable cases with sample paths, locations and expected values |

## Key rules at a glance

1. All lengths are HWPUNIT (1/7200 inch, 100 per point) except paragraph-shape lengths (margins, indent, paragraph spacing, non-percent line spacing, tab positions, section default tab), which are stored at twice the HWPUNIT value in HWP 5.0 and in un-switched OWPML. Hancom ground truth: stored line segment column start equals half the paragraph left margin in 5,221 of 5,230 sampled paragraphs.
2. Text positions are UTF-16 code units in Hancom's stream: ordinary characters take 1 (2 for supplementary-plane characters), tab and every inline or extended control take 8, the paragraph end takes 1.
3. A run's formatting comes only from the paragraph's own paragraph shape id and the character shape id active at the position. Styles are templates for editing and for page-number formatting; they are not re-applied at layout time.
4. Each character uses one of seven language slots (Hangul, Latin, Hanja, Japanese, Other, Symbol, User). The slot selects font, ratio, spacing, relative size and offset from the character shape. Classification is by code point with Hancom-specific exceptions (punctuation goes to the Latin slot, spaces inherit the previous character's slot).
5. Stored HWP 5.0 object text-wrap codes are 0 Square, 1 Top and bottom, 2 Behind text, 3 In front of text. The public specification table (with Tight and Through at 1 and 2) does not match Hancom-saved files.
6. Stored HWP 5.0 table page-break codes: 0 no split, 1 split anywhere (OWPML `TABLE`), 2 split at cell boundaries (OWPML `CELL`).
7. From document format version 5.0.2.5, the paragraph line spacing type and value come from the third attribute word and the trailing line-spacing field; the older fields hold a fallback for old readers.
8. The source format and conversion lineage are fixed at parse time and select a small set of layout compatibility behaviors listed in [provenance.md](provenance.md).

## Builder questions

No `design/spec-requests.md` existed when this unit was written, so no builder questions are answered here. Open questions found during this work are listed at the end of [known-gaps.md](known-gaps.md).
