# Resource limits and budgets

All limits are safety caps against hostile or corrupt input, not format rules. Real Hancom files stay far below them. Every check must happen before the allocation it protects: compare declared sizes and counts first, then read with a hard cap of limit + 1 bytes so a lying size field cannot slip through.

## Acquisition policies

The caller states how it obtained the bytes. The raw input limit depends on that policy.

| Policy | Meaning | Raw input limit |
| --- | --- | --- |
| Untrusted | Network, IPC, clipboard, drag and drop, anything without a fresh approval of one exact file | 128 MiB |
| Approved local file | One exact file just chosen in a native file picker (or named on the CLI command line) | 512 MiB |
| Regenerated | Bytes our own bounded serializer produced in this process (save integrity re-parse) | 512 MiB |

- The regenerated policy is internal only and must never be reachable from outside callers.
- Reading a local file: open the handle first, check it is a regular file, check its metadata length, then read at most limit + 1 bytes and fail if more arrived (guards against a file growing during the read).
- Exceeding the limit is an "input too large" error that reports actual size, limit and the policy.

## Byte budgets

| Budget | Value | Applies to |
| --- | --- | --- |
| Container total | 512 MiB | Sum of all expanded bytes taken out of one document: decompressed structural streams, preview image and text, preserved extra streams, decoded BinData. Also the sum of declared stream sizes in the CFB directory. Also the raw input size of an HWP 5 file. |
| Structural stream | 256 MiB | One expanded `FileHeader`, `DocInfo`, `PrvText`, `BodyText/*`, `ViewText/*`, root-level `Section*`; also one XML part in other formats; also the decrypted distribution section |
| Binary stream | 512 MiB | One BinData payload or any other stream |
| Thumbnail | 64 MiB | `PrvImage` (and HWPX `Preview/PrvImage*`) |
| Embedded OLE presentation stream | 64 MiB | One `OlePres000` stream inside an OLE storage |
| Embedded OOXML chart | 256 MiB | One `OOXMLChartContents` stream |
| Private hyperlink-format stream | 16 MiB | Decoded only when at most this size |

Stream class by path (paths are case-insensitive, leading slashes normalized): `/PrvImage` and anything starting `/Preview/PrvImage` are thumbnail class; `/FileHeader`, `/DocInfo`, `/PrvText`, and anything starting `/BodyText/`, `/ViewText/` or `/Section` are structural; everything else is binary.

## Accounting rules

- A running total of expanded bytes starts at 0 for each document.
- Before reading or inflating a member, compute its member limit = min(class limit, container total limit minus running total). A member limit of 0 is legal: an empty member is accepted and the first byte fails.
- After reading, add the actual size to the running total; failure if the sum exceeds 512 MiB or overflows.
- Lazily resolved BinData charges the budget when it is decoded. A key that is decoded again with an equal or smaller result does not change the total; a larger result charges only the difference. Shared duplicates (the same payload referenced twice) are charged once while alive. The budget and the read must be updated under one lock so two threads cannot both see the same remaining budget.
- When the CFB directory is read, every stream's declared size must not exceed its class limit and the sum of declared sizes must not exceed 512 MiB.

Improvement over the old engine: an oversized optional stream (for example a 70 MiB `PrvImage`) made the whole open fail at directory validation. Skip the optional stream with a warning instead and keep the document.

## Count, length and depth caps

| Cap | Value | Reason |
| --- | --- | --- |
| CFB directory entries | 4096 | Counted as directory sectors times entries per sector, before any entry is materialized |
| CFB DIFAT sectors | 128 | A full 512 MiB version 3 file needs fewer than 65 |
| CFB path length | 4096 bytes of UTF-8, separators included | Walk the directory tree iteratively, tracking parent path length; never build path strings first |
| CFB surplus FAT sectors (strict validation) | 8 beyond the number needed to index the file | Hancom files sometimes keep a few unused FAT sectors |
| OLE relevant streams examined | 16 | Inside one embedded OLE storage |
| HWP 5 paragraph-list nesting | 64 | Deeper lists (cells in cells, text boxes in cells, notes) parse as empty; owned by the BodyText unit |
| HWPX entries | 4096 | HWPX unit |
| HWPX central directory | 32 MiB | HWPX unit |
| HWPX entry name | 4096 bytes | HWPX unit |
| XML depth (HWPX, HML) | 256 | HWPX and HML units |
| HWP 3 nesting | 16 | HWP 3 unit |

## Arithmetic safety

- Every offset plus length computed from file fields uses checked arithmetic. On 32-bit targets (wasm32) an unchecked record end can wrap and turn a bounds check into a 4 GiB allocation.
- Shift amounts read from files (sector size powers) are validated before shifting.
- A panic in the wasm build kills every open document, so every malformed-input path must return an error or a recovered value, never panic.
