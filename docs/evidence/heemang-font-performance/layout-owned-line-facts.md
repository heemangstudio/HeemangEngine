# Layout-owned line facts trial

The unchanged seven-run research gate still fails ordinary typing p50 after table-query reuse. Existing CPU samples use this branch's native source and WASM. Restricting samples to 15 synchronous ordinary-character intervals gives relayout 22.37/23.93 ms and LineFacts reset inclusive 4.576/4.580 ms. Facts are rebuilt after memoized paragraph clones, including unchanged paragraphs. Other named groups are pagination direct self 5.51/6.41 ms, paragraph clone 1.13/2.97 ms, old path destruction 1.09/1.39 ms and object bands 0.79/0.93 ms.

The trial attaches an `Arc<OnceLock<LineFacts>>` to each `ParaLayout`. Formatting alone does not compute facts. The body flow's first use initializes the same cell held by the earlier memo clone. Later memo hits share it. Fresh formats get an empty cell; `para::reflow` replaces it when line membership changes. The four reset points in `Flow` disappear. Note and object frame visibility lists do not change `ParaLayout.lines`.

This adds a public `ParaLayout.line_facts` field and makes `table::LineFacts` a public opaque Rust type. Facts' data and methods stay internal. Existing struct literals with `..Default::default()`, `Default` and derived `Clone` remain supported. No WASM request, packet, document model or serialized format changes. Each fresh paragraph layout gets one shared lazy cell, including nested paragraphs whose facts are never computed; cold/runtime qualification must account for that allocation.

The permanent native test proves that a memo clone created before initialization shares the eventual result, and compares it with direct wire scans. Dirty controls change inline-content classification; edited text gets new facts; old layouts keep their earlier content. Existing wire-scan controls cover Hangul, surrogates, tabs, floating/inline controls, unordered/overlapping ranges and empty layouts.

Two real negatives remove only the reflow invalidation line. The strengthened paragraph reflow test fails with cached length 1/index 1. Existing square-gap regression `preceding_square_bands_reflow_only_the_stored_spans_they_block` fails with length 2/index 2. Restoring invalidation passes the complete hm-layout suite: 131 passed, one existing ignored test. Logs remain under `/private/tmp/heemang-perf-resume/layout-facts-*.log`.

```sh
CARGO_BUILD_JOBS=3 cargo test -p hm-layout
```

Full native/corpus equality against frozen `rc-performance-de9d79e` is pending. No new performance measurement or acceptance claim is made.
