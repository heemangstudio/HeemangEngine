# Paragraph line facts

Pagination repeatedly scanned a paragraph's wire units for each line's content and extended controls. The inherited cache still compared all line ranges at every access. This change builds line facts at the outer paragraph's initial format, both format replacements and the in-place square-gap reflow. Per-line queries borrow the recorded control slices. Unordered/overlapping line ranges retain the direct wire-scan fallback.

Nested table/caption stacks and footnote frames do not share the outer section's Flow. Pending table continuation does not access or reset line facts; endnotes use it after the body loop. The paragraph remains immutable while placed. All four format/reflow boundaries reset its facts before line access.

The initial three-boundary trial omitted square-gap reflow. Existing behavioral test `preceding_square_bands_reflow_only_the_stored_spans_they_block` caught a two-line cache indexed by a newly reflowed third line. Adding the fourth reset fixes that case. The permanent wire-equivalence test covers floating/inline tables, Hangul, a surrogate pair, tabs, paragraph ends, overlapping/reversed/empty line ranges and reuse after layout/paragraph replacement.

Fresh `CARGO_BUILD_JOBS=3 cargo test -p hm-layout` on this engine base passes 130 tests, with one existing ignored test. The logs preserve the missing-boundary red control and current focused green suite.

Runtime CPU evidence is diagnostic. Inherited per-access validation consumed 17.21 ms; the lifecycle-reset profile consumed 10.98 ms. Later host-HFT/cache/fused-IME profile records 10.64 ms reset and 56.97 ms total relayout over the same 41 research strokes. Strict repeated medians, native same-packet output comparison and full corpus layout equivalence remain pending. The browser's cached/uncached Player oracle compares painting of the same packet; it does not substitute for the native LineFacts on/off output gate.
