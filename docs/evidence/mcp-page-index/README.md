# MCP page selection

The renderer now collects cell boxes with the line index in one walk. Point selection first checks those boxes and builds model cell paths only when a cell contains the point. Table lookup reuses the same geometry. Painted and metadata indexes carry identical cells, and 262 saved point/within requests across six documents match the prior package exactly.

A failed transaction can replace model addresses while preserving epochs and history. All six transaction entry points clear derived pages, indexes and paths on error. The original failed-table test exposed the earlier stale-address version; the corrected version passes that test and the complete real-WASM suite, 59/59 with no skips. The native corrected WASM suite passes 88/88. Existing HML capability and script-spacing expectations were corrected separately, with a real text-box save/reopen proof.

The frozen package is `/Users/pyu/projects/hobbies/Rau/heemang-cleanroom/parity/bins/perf-mcp-rollback-9ab3c752`, WASM SHA256 `9ab3c75261e404db6f80d1b493942d999efc9b97a4c7a90539cf82721cc47b1d`. Its `source.patch` preserves the compiled source before test-only corrections.

The report's authenticated MCP gate passes only together with Studio's paragraph-format snapshots and identical-state JSON decoding reuse. Seven measured pairs, after one excluded warmup, give baseline 11.2 ms and candidate 10.7 ms medians, six paired wins and zero errors. Earlier failed trials remain preserved. Full six-document cold-open/typing qualification and repeated MCP on the other five documents remain pending.

Studio evidence at `docs/evidence/heemang-performance/first-edit/mcp-structure.json` records every gate, raw and manifest hashes, diagnostics, source hashes and correctness logs. Raw artifacts remain under `/private/tmp/heemang-perf-resume/`; the current passing gate is `report-mcp-seven-state-decode`.
