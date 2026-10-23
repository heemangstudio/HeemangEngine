# Picture sampling measurements

Measured on 2026-10-23 with Hancom Office HWP for Mac and the parity harness at
200 dpi using PyMuPDF 2.0. Probes and Hancom PDFs are in
`heemang-cleanroom/parity/notes/picture-sampling/` outside the repository.

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
