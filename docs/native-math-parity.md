# Native drawing strokes and the September 2023 math witness

The `3-09월_교육_통합_2023` HWP and HWPX twins expose two independent
problems on main `6d4f7b0`. Equations do not need a size adjustment to fix either
one.

## Drawing stroke calculation

Mac Hancom exports drawing-object strokes in 12-HU units, or 600 dpi. It rounds
half up and paints an enabled zero-width stroke as one unit. A fresh 18-page
HWPX probe, with identical rectangles and independently varied source widths,
produced these PDF stroke widths:

| Source HU | PDF HU |
| --- | --- |
| 0, 1, 5, 6, 7, 11, 12, 13, 17 | 12 |
| 18, 19, 24, 25 | 24 |
| 33 | 36 |
| 56 | 60 |
| 70 | 72 |
| 112 | 108 |
| 198 | 204 |

Quantizing in the shared drawing outline constructor gives every display
backend the same native width. The rendering test covers the minimum,
half-up boundary, upward and downward rounding, and distinct adjacent widths.
No object geometry, equation metrics, paragraph membership or pagination
changes.

The page-9 mismatch is a stroke-coverage problem. The second empty green
rectangle's bottom is at 34126 HU in our display list and 34128 HU in the Mac
PDF. Before the fix, its 56-HU stroke splits across two raster rows whose
maximum color channel is exactly 200. Hancom's 60-HU stroke leaves those rows
at 191 and 196. The parity scorer counts channels strictly below 200 as ink,
so it reported 1430 missing pixels, 4.52% of the page and 53.74% of its band.
Both twins become 0.00% on page 9 with the measured stroke width.

## Page 8 needs the paragraph owner's change

The boxed condition belongs to body paragraph 440, object 0, text-frame
paragraph 0. Its content width is 25512 HU, with a 1200-HU left margin and
24312-HU line segment. The long equation starts at wire position 26. Its
calculated width/ascent/descent is 8204/985/135 HU; the inline box uses the
stored 1080-HU height and 950-HU baseline. Mac's equation-to-following-text
origins span 8220 HU on its 12-HU PDF coordinate grid.

Our first line ends at position 36, placing the equation and `이다` on line 1
and leaving the period alone on line 2. Mac starts line 2 at the equation,
position 26, and keeps `이다.` with it. Our changed membership also puts the
1080-HU equation height on line 1 rather than line 2.

Fresh Mac causal exports of copied HWPX packages gave:

| Change to the boxed paragraph | Mac equation line |
| --- | --- |
| Original package | 2 |
| Remove only its line-segment array | 2 |
| Change stored second start from 26 to 24 | 2 |
| Remove array, change declared equation width 9024 to 100 | 2 |
| Remove array, change declared equation width 9024 to 16000 | 2 |
| Remove array and final `이다.` | 1 |
| Remove array and only the final period | 1, with `이다` |

With no suffix, the equation starts 14544 HU after the line's origin; our
natural text prefix measures 14543 HU. Removing only the period leaves a
condensed one-line paragraph. These controls isolate forbidden-start
punctuation and word-boundary selection, rather than an equation size or
imported-line artifact.

`hm-layout/src/para.rs::can_break_before` admits the Latin period after the
non-Latin word as a candidate. On overflow, `correct_forbidden` cannot move
back across `다`: the paragraph keeps Korean words, so the immediately
preceding syllable is not a legal boundary. It stops instead of recovering
the preceding legal word boundary at position 26. This requires coordination
with the grid worker who owns `para.rs`. That file is untouched here.

## Verification

Mac PDFs, probe copies, measurements and logs are local under
`tmp/native-math-r1/` in the objects worktree. Probe source is untracked under
`target/native-math-r1/`; predecessor CLI examples are unchanged.

The scoped corpus round is `h2-objects-native-math-1`, compared with frozen
main round `h2c-7` and old-engine round `r1`. It covers both target twins and
ten sibling math documents, 12 documents total. Ten pages become 0.00%,
including page 9 of both September twins. Their page 5 improves from 1.97% to
0.09%. Improved pages are September 2023 p5/p9 in both formats; October 2022
p2/p3/p4/p8 in both formats; p9 of all six November 2024 variants; and
`exam_science.hwp` p1. `issue-505-equations.hwp` is unchanged. No checked
page's mismatch or worst band increases. Pages worse than
r1 fall from 54 to 44, with all page counts unchanged. Page 8 remains
3.81% page mismatch and 8.31% worst band in both formats.

G1 round `h2-objects-native-math-1` passes on all four gate documents.

All 683 corpus layout dumps are byte-identical before and after the change,
with no command errors. Both target fixture hashes still match the corpus
manifest. Native and wasm clippy pass with warnings denied; the focused
rendering regression fails before the fix and passes afterward.

Release workspace tests pass, 161 passed and 1 ignored, including the
649-document page-count minimum.

The fresh G2 job `g2-objects-native-math-1` was stopped while waiting for
the shared GUI lock, per queue coordination. Only its owned process tree
was terminated; no other lock or job was touched. Final G2/G3 validation
replays frozen real Mac Hancom outputs from `g2-main-70`, after asserting
all four newly generated edited files equal those frozen edited bytes.
Hashes and scores are recorded in
`tmp/native-math-r1/g2-reference-replay/summary.json`.

G2/G3 reference replay passes all four documents. G2 worst bands are
0.00/7.66/5.08/0.03% for FDI/NIKL/Lang/Math; every G3 band is 0.00%.
No page mismatch or worst band regresses against frozen `g2-main-70` G2
renders. The established G2 harness baseline `h2-main-11` also passes.
An additional check against current unedited G1 flags Lang p1 at 2.49%
versus 2.27%; frozen edited main-70 already has the same 2.49% score.
