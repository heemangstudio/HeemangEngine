# Host HFT source retention

The interactive host retains an approved HFT source after its first full read. Unseen characters can then draw synchronously. Repeated labels reuse that payload, the existing family face is refreshed when another bank joins, and unregistering releases the source. `FontSystem::add_compact` keeps its previous memory and refill contract; host TrueType registration delegates to it.

The red controls fail on unseen compatibility jamo `ㅆ` with compact HFT loading and on Latin `A` when a cached family's later bank is hidden. The fixed native suites pass 106 tests. Coverage includes other unseen jamo and syllables, the actual TESSMHG/TESSMEN banks, compact-source promotion with owed relayout, stable face IDs and labels, duplicate registration, source release and unchanged compact TrueType behavior. Studio font/player/transaction checks pass 31 tests and TypeScript checks pass.

Real headless Studio at `http://127.0.0.1:7733/?engine=heemang`, with a fixture Pi hub, replays the research report's 41 IME/Latin strokes through the production input handler. There are zero outline refills, zero late completed bitmaps, zero missed edited frames and zero page errors. Full-redraw and exact cached/uncached packet comparisons both match 41/41. Reopening replaces document instance 1 with 2 at the same font generation 54; exact-packet comparison still matches. Native font bytes remain 29,886,155 across typing, about 18.35 MB above the earlier compact trial.

The temporary WASM uses accepted engine base `e2e0d75` plus this source-retention change and the separate LineFacts trial. It is 7,544,291 bytes, SHA-256 `be2b5cb9d31bef505bdd22faa192791996560fce126a02baff08389c2a822b3d`. Studio uses its separate guide-redraw and glyph-run cache commits, plus unqualified fused-IME/query-guard trials. The manifest records the exact sources and document. Generated WASM is excluded from the commit.

Latency fields in the correctness JSONL are excluded from performance acceptance because the run overlapped other work and pixel/trace instrumentation. Cold first-visible and completed-result typing p50/p95 still require at least seven quiet runs against the old app on all six agreed documents.

Native command, with `HEEMANG_FONTS` pointing to the installed Hancom Fonts and Bin/Code directories and the TrueType probes:

```sh
CARGO_BUILD_JOBS=3 cargo test -p hm-fonts -p hm-wasm
```

Live command from the separate Studio worktree, with `HV_BASELINE=/private/tmp/heemang-perf-resume/baseline-main`:

```sh
HV_CANDIDATE_VITE_PORT=7733 HV_BASELINE_VITE_PORT=7743 node rhwp/rhwp-studio/e2e/heemang-verify/cold-open-bench.mjs --mode=typing --docs=report --apps=candidate --runs=1 --warmup=0 --trace-glyphs --verify --verify-packets --out=/private/tmp/heemang-perf-resume/research-host-hft-correctness
```
