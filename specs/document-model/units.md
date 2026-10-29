# Units and conversions

## HWPUNIT

| Quantity | Value |
|---|---|
| 1 inch | 7200 HU |
| 1 point | 100 HU |
| 1 mm | 7200 / 25.4 = 283.4646 HU |
| 1 px at 96 dpi | 75 HU |
| 1 HU at 96 dpi | 1/75 px = 0.013333 px |

Conversions:

- px = HU × dpi / 7200. At 96 dpi, px = HU / 75.
- HU from px truncates toward zero for geometry derived from layout, and rounds half away from zero for imported values (pasted HTML sizes, user-entered values). Layout state stays in integer HU and never round-trips through px.
- pt = HU / 100. Character base size 1000 HU = 10 pt.
- mm = HU × 25.4 / 7200. Paper sizes are stored rounded to the nearest HU: A4 210 × 297 mm = 59528 × 84188 HU. Some files store 84186 for A4 height (`samples/equation-lim.hwp`, `samples/pua-test.hwp`).

Integer forms used in the formats:

| Form | Width | Signedness | Use |
|---|---|---|---|
| HWPUNIT | 32 bit | unsigned in the spec | sizes, page lengths, offsets |
| SHWPUNIT | 32 bit | signed | positions that may be negative |
| HWPUNIT16 | 16 bit | signed | margins, gaps, paddings |

Object offsets (`vertical offset`, `horizontal offset` of the common object attributes) are declared unsigned but carry negative values in two's complement. Read them as signed 32-bit. Page-definition lengths are unsigned; sums of page margins must saturate (damaged files store huge values).

## Doubled paragraph-shape lengths

HWP 5.0 stores these paragraph-related lengths at twice their HWPUNIT value (one stored unit = 1/14400 inch):

| Field | Where |
|---|---|
| Left margin, right margin, indent (first-line indent or hanging) | paragraph shape |
| Space before, space after | paragraph shape |
| Line spacing value when the type is not percent (fixed, space-only, at-least) | paragraph shape, both the legacy and the current field |
| Tab stop positions | tab definition |
| Default tab interval | section definition (typical stored value 8000 = 40 pt effective) |

Effective HU = stored / 2, kept as a half-unit value (odd stored values exist and mean x.5 HU). Paragraph border spacing (four INT16 values) is not doubled.

OWPML carries both scales:

- Inside `hp:switch` / `hp:case` with required namespace `http://www.hancom.co.kr/hwpml/2016/HwpUnitChar`, margin and line spacing values are effective HU.
- Inside `hp:default`, and in packages that write `hh:margin` without a switch, values use the doubled scale.
- An odd doubled value is written in the case branch as its half (truncated) with `unit="CHAR"`. Example: stored 1 is written as case `value="0" unit="CHAR"`, default `value="1"` (`samples/2025 행정업무운영 편람(최종).hwpx`, paragraph shape 1339 and others). So effective = case value + 0.5 when `unit="CHAR"`.
- Percent line spacing is never doubled.

Hancom ground truth for the doubling: in Hancom-saved HWP 5.0 files the first stored line segment's column start equals half the paragraph left margin (5,221 of 5,230 sampled unindented paragraphs over the first 200 samples; the 9 others are lines inside an object wrap zone). In OWPML packages with `xmlVersion` 1.2 and an un-switched `hh:margin`, line segment `horzpos` equals half the stored left margin (`samples/issue1858_paper_anchor_float_stack.hwpx`: 114 of 130 paragraphs; `samples/task2097/3248363_upmu_bunjang.hwpx`: 69 of 69).

The model should store the doubled value (to keep odd values and raw round trip) and expose effective half-unit values to layout. A model that stores effective HU must keep a half-unit bit.

## Color (COLORREF)

32-bit little-endian value 0x00BBGGRR: red in the low byte.

- Upper byte non-zero means "no color". 0xFFFFFFFF is the common "none" value (character shade color, fill background).
- A solid fill with a non-zero upper byte is no fill.
- Character shade color 0x00FFFFFF (white) means no shading in practice.
- Highlight range tags (see [paragraph-text.md](paragraph-text.md)) carry a 24-bit COLORREF in BGR order.

## Percent and ratio fields

| Field | Stored range | Meaning |
|---|---|---|
| Character ratio (width scale) per language | 50 to 200 (UINT8) | glyph width percent |
| Character spacing per language | -50 to 50 (INT8) | tracking percent of the per-language font size |
| Relative size per language | 10 to 250 (UINT8) | percent of base size; 0 is treated as 100 |
| Character offset per language | -100 to 100 (INT8) | vertical shift percent of base size, positive moves down |
| Shadow offset x, y | -100 to 100 (INT8) | percent |
| Picture transparency (UI) | 0 to 100 | stored alpha byte = floor(t × 255 / 100); UI percent = round(a × 100 / 255), clamped to 100 |
| Fill alpha (OWPML float) | 0.0 to 1.0 | alpha byte = floor(f × 255), f clamped |

## Hancom integer percent rule

Where Hancom scales an integer by a percent, it multiplies in 64 bits and rounds half away from zero:

MulDivPercent(v, p) = sign(v·p) × floor((|v·p| + 50) / 100), and v itself when p = 100.

On overflow of 32 bits the result is −1. This rule is used for control-space widths and character-offset descriptors (see [effective-formatting.md](effective-formatting.md)).

## Line spacing value encoding

- Percent type: value is a percent (160 = 160%). Not doubled.
- Fixed, space-only, at-least types: value is a doubled length (3120 stored = 1560 HU = 15.6 pt).

Example: `samples/2025 행정업무운영 편람(최종).hwpx` writes `FIXED` 1560 in the case branch and 3120 in the default branch; the HWP 5.0 sibling stores 3120.

## Border line widths

Border line width is an index 0 to 15 into a fixed table (mm):

| Index | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 | 12 | 13 | 14 | 15 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| mm | 0.1 | 0.12 | 0.15 | 0.2 | 0.25 | 0.3 | 0.4 | 0.5 | 0.6 | 0.7 | 1.0 | 1.5 | 2.0 | 3.0 | 4.0 | 5.0 |

OWPML writes the width as a mm string ("0.4 mm"). Reading maps to the nearest table entry; out-of-range indices write as 0.1 mm. The table must be a bijection: a coarser mapping turns 0.4 mm into 0.15 mm and 0.6 mm into 0.2 mm.

Shape outline width (drawing objects) is a plain HWPUNIT INT32, not an index.
