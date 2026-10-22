# Format detection

Detection looks only at content bytes. The file name extension is ignored: `task1768/distribution_doc.hwpx` is an HWP 5 compound file and must open as HWP 5.

## Precedence

Rules are evaluated in this order; the first match wins.

| Order | Condition | Result |
| ---: | --- | --- |
| 1 | Input length is 0 | Empty file |
| 2 | Input starts with bytes `9B 20 44 52 4D 4F 4E 45` (`0x9B` then ` DRMONE`, Fasoo) | DRM-protected container |
| 3 | Input starts with ASCII `SCDSA` (SoftCamp; observed `SCDSA002`, `SCDSA004`) | DRM-protected container |
| 4 | Length at least 8 and bytes 0..3 are `D0 CF 11 E0` | HWP 5 (compound file) |
| 5 | Length at least 8 and bytes 0..3 are `50 4B 03 04` | HWPX (ZIP) |
| 6 | Length at least 17 and bytes 0..16 equal ASCII `HWP Document File` | HWP 3.0 |
| 7 | HWPML root probe succeeds (below) | HML |
| 8 | Otherwise | Unknown |

Notes:
- Rules 4 and 5 check only 4 bytes but require 8 bytes of input. A 2-byte input `50 4B` is Unknown.
- Rule 4 does not check the remaining CFB magic bytes `A1 B1 1A E1`; the CFB opener does (see [cfb.md](cfb.md)), so a truncated or fake header becomes an open error, not Unknown.
- Rule 6 accepts every HWP 3.x and HWP 2.x era file that starts with the 17-byte prefix (the full HWP 3 signature is 30 bytes, `HWP Document File V3.00` followed by `1A 01 02 03 04 05`). The HWP 3 reader decides which versions it supports. `samples/issue_265.hwp` is detected as HWP 3.0.
- A ZIP that is not an HWPX package (for example a DOCX) is still classified HWPX and fails inside the HWPX reader. A better engine checks the `mimetype` member (`application/hwp+zip`) before committing; that check belongs to the HWPX unit.
- DRM checks run first so that a protected file produces a specific message instead of "unknown format".

## HWPML root probe

- Probe only the first 64 KiB.
- Encoding by byte-order mark: `EF BB BF` UTF-8, `FF FE` UTF-16LE, `FE FF` UTF-16BE; no BOM means UTF-8.
- UTF-8 prefix: if the 64 KiB cut falls inside a multi-byte sequence, decode up to the last complete character. Any other invalid UTF-8 fails the probe.
- UTF-16 prefix: drop a trailing odd byte; invalid UTF-16 (unpaired surrogate) fails the probe.
- Parse XML events from the start. Skip the XML declaration, comments, processing instructions and whitespace-only text. The first element (start or empty) must be named exactly `HWPML` (case-sensitive) and carry a non-empty `Version` attribute. Anything else, including a DOCTYPE, other text, or an XML error, fails the probe.
- Version support is decided by the HML reader. A legacy root with `Version="2.1"` is detected as HML and then rejected by the reader as an unsupported version.

## Error classes

Detection failures map to a single "unsupported format" error with one of three classes. The message names the class and lists the supported formats. Lower-level details (for example a CFB parse error message) must not leak into the Unknown class message.

| Class | When | Vendor detail |
| --- | --- | --- |
| Empty file | Rule 1 | none |
| DRM-protected | Rules 2 and 3 | Fasoo or SoftCamp SCDSA, from the matched prefix |
| Unknown format | Rule 8 | none |

## Dispatch

| Detected | Next step |
| --- | --- |
| HWP 5 | Open the compound file through the tolerant reader path in [cfb.md](cfb.md). |
| HWPX | HWPX reader. |
| HWP 3.0 | HWP 3 reader. |
| HML | HML reader; the result also carries HML import metadata (HWPML version, sub-version, style, encoding, resource count, warnings, preserved fragments). |
| DRM, Empty, Unknown | Error, no parse attempted. |

The acquisition-policy size check ([limits.md](limits.md)) runs before detection for every format.

## Thumbnail-only path

A separate lightweight entry extracts the preview image without parsing:
- HWPX input: the first ZIP member whose name starts with `Preview/PrvImage`, capped at 64 MiB; empty means no thumbnail.
- Anything else: open as a compound file and read the root `PrvImage` stream.
- Image kind from magic: PNG (8-byte PNG signature), BMP (`BM`), GIF (`GIF`). Other bytes, including JPEG (seen in 3 fixtures), are reported as unknown kind with size 0 by 0.
- Pixel size: PNG width and height are big-endian u32 at offsets 16 and 20 (needs 24 bytes); BMP width and height are little-endian i32 at 18 and 22, absolute value (needs 26 bytes); GIF width and height are little-endian u16 at 6 and 8 (needs 10 bytes).
- Test vector: `exam_kor.hwp` thumbnail is PNG, 724 by 1024.

Improvement: recognize JPEG (`FF D8 FF`) and read its SOF size; fall back to the tolerant CFB reader so damaged files still produce thumbnails.

Vectors: [vectors/detection.tsv](vectors/detection.tsv).
