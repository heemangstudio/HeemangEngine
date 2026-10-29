# Picture sampling measurements

Measured on 2026-10-23 with Hancom Office HWP for Mac and the parity harness at
200 dpi using PyMuPDF 2.0.

The probes replace the two embedded letterhead PNGs in a copy of
`task2279/36394733_gyeoljae.hwpx` with opaque line grids or deterministic RGB
noise. A standalone section places the pictures at paper coordinates, avoiding
cell clipping. Source images are 196 x 196 and 118 x 118 pixels, both drawn at
5669 x 5669 HU. The first covers 159 x 159 device pixels after the existing
600 dpi drawing-grid and device-edge fitting rules.

For reductions below 2:1, the source-pixel distance from each output pixel's center is
scaled by the reduction ratio. The measured kernel within distance 1 is
`1 - 3d² + 2d³`. Its weights are divided by the ratio, rounded to multiples of
1/256, and corrected to sum to 1 by adjusting the largest weight, choosing the
first on ties. Horizontal samples round to bytes before the vertical pass.
The standalone noise probe's interior matches the Hancom PDF raster exactly.
A line-grid sample and a translucent extreme-reduction check protect the
renderer behavior in `raster.rs`.

For enlargement, linear interpolation matches the probe within 2 channel
levels. Device-size reduction followed by nearest sampling avoids a second
filtering pass. Both changes to device-size handling apply to upright pictures
whose crop fractions span the whole source. Cropped and transformed pictures
keep their previous size selection and shader filter.

The probe with fractional source crop edges shows that Hancom rounds those
edges to source pixels. Applying that rule also needs the cropped frame and
sample origins to agree; it remains separate work.

The 784-pixel and 392-pixel noise probes drawn at the same size show a
separate decoder-subsampling stage for stronger reductions. They keep the
previous tent-filter path for this milestone. Reproducing that stage and
fractional crop placement are follow-ups.

For the 784 x 784 PNG reduced to 159 x 159, a single 4 x 4 box average
with truncation followed by the measured cubic pass matches the probe's
interior exactly. Direct cubic reduction has mean channel error 4.97.
The stronger-reduction follow-up must also check JPEG decoder behavior.

Validation: formatting, workspace and wasm Clippy, and all 150 release tests
pass. Pagination remains 649/661. G1 passes on all four gate documents in
`h2-objects-52`; G2/G3 pass on all four in `g2-objects-51`.

All 683 corpus layout dumps match main 4627d0a. Of 676 scored documents,
226 have changed pixels on 1597 pages. Pages worse than r1 fall from 360 to
341, with 24 cleared pages and five newly worse pages. The new worst bands
are 0.01%, 0.04% twice and 0.51% twice. The latter are thin colored rules
inside the scaled map screenshot in the two `tac-img-02` documents.

Eleven of the twelve selected letterhead documents now pass r1 page by page.
`field-multipara-clickhere` remains at 0.10%; its mixed reduction/enlargement
keeps the previous shader. Full results and per-page scores are in the
external probe directory's `report.json` and `corpus-scoped-summary.json`.

## Enlargement and mixed axes

The next measurements use the same line-grid probe and the six corpus pages
listed below. Enlargement samples use a source-coordinate step truncated to
14 fractional bits. Each axis interpolates linearly and truncates its channel
results before the next axis. When one axis enlarges and the other reduces,
both use this linear path. The source-step precision also agrees with
[MuPDF's image painting arithmetic](https://github.com/ArtifexSoftware/mupdf/blob/master/source/fitz/draw-affine.c).

The 118-pixel grid covers 158 by 159 device pixels. Its measured samples are
`167, 22, 118, 0`; the previous shader produced `168, 23, 119, 1`. A render
test keeps those Hancom samples. Sampling retains only two horizontally
interpolated source rows while building the cached device-size picture.
Frames beyond the cache's 128 MiB entry limit keep shader sampling. A
repeated-draw test checks that fallback after the source has been decoded.

| Corpus document | Page | Previous worst band | New worst band |
| --- | --- | --- | --- |
| `2025-e9a7` | 7 | 0.01% | 0.00% |
| `aift`, HWP and HWPX | 71 | 0.04% | 0.01% |
| `tac-img-02`, HWP and HWPX | 12 | 0.51% | 0.00% |
| `field-multipara-clickhere` | 1 | 0.10% | 0.00% |

All five newly worse pages from the previous milestone meet r1 again. The
last letterhead now also meets r1, bringing the selected letterheads to
12/12. Cropped pictures and stronger reductions remain separate work.

Validation: formatting and workspace/wasm Clippy pass. Release workspace
tests pass with 154 passed and one ignored; the pagination test meets its
649-document minimum. G1 passes in `h2-objects-55`, and G2/G3 pass on all four
documents in `g2-objects-54`, with zero G3 difference. All 683 corpus layout
dumps are identical to merged main `f30ac16`.

The changed pages in 204 corpus documents total 1,160. Pages worse than r1
fall from 284 to 272: 13 clear and one crosses the threshold. The latter is
the HWPX handbook's page 67, whose worst band changes from 1.22% to 1.23%.
The screenshot's embedded BMP and Hancom PDF have slightly different RGB
values.
