# Paragraph text and positions

## Paragraph header

| Field | Meaning |
|---|---|
| Character count | number of UTF-16 units in the paragraph text stream, including control blocks and the paragraph end. Bit 31 is set on the last paragraph of a paragraph list (observed in every Hancom-saved section: 82 of 82 last paragraphs set, 17,111 of 17,111 others clear). Mask it off when reading; regenerate it when writing. |
| Control mask | OR of (1 << code) for every control code present (see below) |
| Paragraph shape id | 0-based |
| Style id | 0-based |
| Break type | bit 0 section break, bit 1 multi-column break, bit 2 page break, bit 3 column break |
| Counts | character shape runs, range tags, line segments (regenerated from the arrays on write) |
| Instance id | unique paragraph id; preserved |
| Track-change merge flag | 5.0.3.2 and later; preserved |

A paragraph whose count is 1 has no text record; its text is just the paragraph end.

## Position space

All positions inside a paragraph (character shape run starts, range tags, stored line segment text starts, field and highlight positions, caret positions in the file) are UTF-16 unit offsets in Hancom's text stream. The model keeps text in this space. Character (scalar) offsets are an API concern and are translated at the boundary.

Unit widths:

| Element | Units |
|---|---|
| Character in the Basic Multilingual Plane | 1 |
| Supplementary-plane character (surrogate pair) | 2 |
| Character control (codes 0, 10, 13, 24-31) | 1 |
| Inline control (codes 4-9, 19-20) | 8 |
| Extended control (codes 1-3, 11-12, 14-18, 21-23) | 8 |
| Paragraph end (13) | 1, always last |

Control codes (HWP 5.0 table 6):

| Code | Class | Meaning | Model representation |
|---|---|---|---|
| 0 | char | unusable | dropped |
| 1 | extended | reserved | control slot |
| 2 | extended | section or column definition | control slot |
| 3 | extended | field begin (click-here, hyperlink, bookmark range, formula, summary, user info, date, document date, path, cross reference, mail merge, memo, proofreading mark, private info) | control slot that opens a field range |
| 4 | inline | field end | closes the innermost open field |
| 5-7 | inline | reserved | skipped (8 units) |
| 8 | inline | title mark | skipped (8 units) |
| 9 | inline | tab | tab character with 7 extension units |
| 10 | char | line break | line break character |
| 11 | extended | drawing object or table (also picture, equation, form, OLE, chart) | control slot |
| 12 | extended | reserved | control slot |
| 13 | char | paragraph end | end |
| 14 | extended | reserved | control slot |
| 15 | extended | hidden comment | control slot |
| 16 | extended | header or footer | control slot |
| 17 | extended | footnote or endnote | control slot |
| 18 | extended | auto number or new number | control slot |
| 19-20 | inline | reserved | skipped |
| 21 | extended | page control (page hide, page number position) | control slot |
| 22 | extended | bookmark, index mark | control slot |
| 23 | extended | ruby (덧말) or character overlap (글자 겹침) | control slot |
| 24 | char | hyphen (discretionary) | soft hyphen, no ink or advance unless the line breaks there |
| 25-29 | char | reserved | kept as one character; the old engine shows a space |
| 30 | char | no-break space (묶음 빈칸) | U+00A0 with origin "control 30" |
| 31 | char | fixed-width space (고정폭 빈칸) | U+2007 with origin "control 31" |

An 8-unit block is: the code, 6 data units, and the code repeated. For extended controls the data units hold the 4-character control id (`secd`, `tbl `, `gso `, `%hlk` and so on) and the control's record follows as a CTRL_HEADER child of the paragraph, in the same order as the extended blocks appear. Inline controls have no CTRL_HEADER; field end (4) does not consume a control.

### Length invariant

character count = Σ over text characters of their unit width + 8 × (tabs + inline control blocks + extended control blocks) + 1

Example (Hancom-saved, `samples/3-09월_교육_통합_2022.hwp`, section 0 paragraph 0): text " 의 값은?" (6 BMP characters), 5 extended controls (section definition, column definition, header, endnote, equation): 6 + 40 + 1 = 47 stored. Paragraph 1: "① ⇥⇥② ⇥⇥③ " with 4 tabs, 6 other characters and 3 equations: 6 + 32 + 24 + 1 = 63 stored.

### Tab extension units

The 7 units after a tab code (units 1-6 data, unit 7 the repeated code 9):

| Unit | Meaning (HWP 5.0 inline tab) |
|---|---|
| 1 | tab advance width as last laid out by Hancom, in HU (low 16 bits) |
| 2 | high 16 bits of the width (observed 0) |
| 3 | high byte: tab kind + 1 (1 left, 2 right, 3 center, 4 decimal; 0 also occurs); low byte: leader (fill) type |
| 4-6 | reserved; observed either all 0x0000 or all 0x0020; preserved |
| 7 | 0x0009 |

Observed over 3,348 tabs in 150 Hancom-saved samples: unit 1 always non-zero, unit 2 always 0, unit 3 values 0x0100 (2,741), 0x0000 (453), 0x0003 (78), 0x0203 (35), 0x0200 (26), 0x0103 (13).

OWPML writes the tab as `hp:tab` with `width`, `leader`, `type`. The old engine reads the OWPML `width` as a tab interval (the tab moves to the next multiple of width from the line start, evidence `samples/hwpx/eq-002.hwpx`: left margin 85 pt + tab width 40 pt puts the content at 125.6 pt) and the HWP 5.0 unit 1 as a resolved advance, and marks the OWPML origin in unit 6 bit 15 internally. How layout uses these values belongs to the text layout unit; the model must keep both the raw units and the source.

A tab whose 6 data units are all zero and unit 7 is 9 carries no Hancom layout information (written by tools that did not lay the line out). Treat it as absent, not as a zero-width tab.

## Control anchoring

Controls are kept in an ordered list per paragraph. Each extended control occupies one 8-unit slot in the text stream, so its anchor position is the UTF-16 position of its slot. For a model that does not keep explicit slots, the slot positions are recovered from gaps: walking the characters in order, a gap of g units between the end of one character and the start of the next holds g / 8 controls (and the gap before the first character holds controls anchored at position 0).

Rules:

- The control list order equals the slot order in the stream.
- A movable control (table, drawing object, picture, equation, footnote, endnote, auto number, character overlap) occupies one logical character position for caret movement and splitting. Bookmarks keep a position but are not movable. Section and column definitions, headers, footers, page controls and fields are not caret stops.
- When a paragraph is split at an object boundary, an object moving to the new paragraph splits before its 8-unit slot.
- Field end and the trailing controls of a paragraph also take 8 units each; joining two paragraphs places the second paragraph's text after the first paragraph's trailing control and field-end slots.

HWP 3.0 input has no gap information. The old engine's HWP 3.0 reader writes consecutive offsets and an object replacement character (U+FFFC) per inline object; slots are then recovered from U+FFFC positions in order. A new HWP 3.0 reader should produce explicit 8-unit slots instead.

## Special spaces

Two Unicode characters have more than one source representation; the model records the origin of each occurrence because width and language slot differ:

| Character | Origins |
|---|---|
| U+00A0 | control 30 (`hp:nbSpace` in OWPML), or a literal U+00A0 in text |
| U+2007 | control 31 (`hp:fwSpace` in OWPML), or a literal U+2007 in text |

The origin list is sorted by character index with no duplicates, and each entry must match the actual character. Copy, paste and undo carry origins with the text. A missing origin means unknown (older data).

Control-origin spaces are non-printing (width only). Their base width rule is in [effective-formatting.md](effective-formatting.md). Literal U+00A0 uses the Latin slot and literal U+2007 the Symbol slot under the macOS metrics policy.

## Character shape runs

The run list is (start position, character shape id) pairs sorted by start. The first run must start at 0 (HWP 5.0 rule). The shape at position p is the last run whose start ≤ p; positions past the last character use the last run. An empty run list means shape 0.

Editing rules that keep runs consistent:

- Inserting at p shifts every run start > p by the inserted width, and a run starting exactly at p (other than at 0) shifts too, so the inserted text takes the shape of the text before it.
- Deleting a range clamps run starts inside it to the range start and drops runs that become empty; adjacent runs with equal ids merge.
- Applying a shape to a range splits runs at the range ends and merges equal neighbors afterward.
- A hyperlink range ends at its last visible character so that the gap between two adjacent links keeps the surrounding shape.

## Range tags and highlights

A range tag is (start, end, tag) with the kind in the high 8 bits of the tag and 24 bits of data. Kind 2 is the highlighter (형광펜) with a COLORREF in the low 24 bits (BGR), end exclusive (Hancom 2022 measurement). Other kinds are preserved.

OWPML writes highlights as empty `hp:markpenBegin color="#RRGGBB"` and `hp:markpenEnd` markers inside `hp:t`. These markers take no position: stored line segment `textpos` values do not count them. Convert them to kind-2 range tags at the UTF-16 position where they occur.

## Fields in text

A field begin (code 3) opens a range that the next unmatched field end (code 4) closes; fields nest. A field end with no open begin in the same paragraph closes a field that began in an earlier paragraph of the same list (multi-paragraph field). HWP 5.0 field ends do not carry the begin id, so pairing is by order: walk the list keeping a stack of open fields; each orphan end pops one. OWPML field ends carry `beginIDRef` and `fieldid`. Splitting a paragraph inside a field keeps the begin on the left and moves the end to the new paragraph as an orphan end; joining re-pairs it.

## Stored line segments

Each paragraph may carry Hancom's last layout of its lines: one record per line segment (a line split around an object has several).

| Field | Meaning |
|---|---|
| Text start | UTF-16 position where the segment starts |
| Vertical position | top of the line from the top of the paragraph list's flow (section body, cell, text box), HU |
| Line height | line box height, HU |
| Text height | height of the text part, HU |
| Baseline distance | from line top to baseline, HU |
| Line spacing | space added after the line, HU (can be negative) |
| Column start | horizontal start of the segment from the column (or cell) content start, HU |
| Segment width | width of the segment, HU |
| Tag | flags below |

Tag bits:

| Bit | Meaning |
|---|---|
| 0 | first line of a page |
| 1 | first line of a column |
| 16 | empty segment |
| 17 | first segment of the line |
| 18 | last segment of the line |
| 19 | auto hyphenation applied |
| 20 | indentation applied |
| 21 | paragraph head (numbering or bullet) present |
| 31 | implementation property |

Observed tag values over 150 Hancom-saved samples (205,809 paragraphs): 0x00060000 (single-segment line) 167,368; 0x00160000 (indented) 14,601; 0x00260000 (with paragraph head) 593; 0x00020000 14; 0x00050000 14; 0x00000000 11. Bits 0 and 1 never appear in saved files. 54,452 paragraphs (26%) had no stored segments at all; layout must not depend on their presence.

A segment is in a wrap zone (beside an object) when column start > 0 or 0 < segment width < column width.

Placeholders used when a paragraph has no stored segment:

| Purpose | Values |
|---|---|
| New empty paragraph in a new document | line height 1000, text height 1000, baseline 850, line spacing 600, tag 0x00060000 |
| Paragraph split from a paragraph with line height 0 | line height 400, text height 400, baseline 320, line spacing 0, tag 0x00060000 |
| "No stored layout" marker | all metrics 0, tag 0x80070000 (single segment + empty + implementation property) |

A new paragraph's first segment vertical position is unknown until layout; the left part of a split keeps the stored vertical position of the original first line.

The stored segments are Hancom ground truth for layout tests (see [test-vectors.md](test-vectors.md)). Whether layout trusts or recomputes them depends on provenance and edit state ([provenance.md](provenance.md)).

## Other paragraph data

- Numbering restart override per paragraph: none (continue the list), continue previous list, or new start n (OWPML and HWP 5.0 carry this in paragraph-level data; see [styles-numbering.md](styles-numbering.md)).
- OWPML run structure: empty `hp:t` children, page-number children and section-prefix children inside a run are kept with their child positions so an unedited paragraph writes back identically. Any edit that changes run boundaries drops this structure.
