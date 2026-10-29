# Compound file (CFB) reading

HWP 5 documents are Microsoft compound files ([MS-CFB], public spec in `public-specs/ms-cfb.pdf`). This file states what the HWP reader needs from a CFB reader, which validations are mandatory, and how real damaged files must still open.

## Header fields used

| Offset | Size | Field | Rule |
| ---: | ---: | --- | --- |
| 0 | 8 | Signature `D0 CF 11 E0 A1 B1 1A E1` | Must match; input must be at least 512 bytes |
| 24 | 2 | Minor version | Not checked (fixtures: 0x3E) |
| 26 | 2 | Major version | 3 (512-byte sectors) or 4 (4096-byte sectors); all fixtures are version 3 |
| 30 | 2 | Sector shift | Must be 9 or 12. Never shift by an unchecked file value. |
| 32 | 2 | Mini sector shift | 6 (64-byte mini sectors) |
| 44 | 4 | Number of FAT sectors | See surplus rule below |
| 48 | 4 | First directory sector | |
| 56 | 4 | Mini stream cutoff | 4096; streams smaller than this live in the mini stream |
| 60 | 4 | First mini FAT sector | |
| 64 | 4 | Number of mini FAT sectors | |
| 68 | 4 | First DIFAT sector | |
| 72 | 4 | Number of DIFAT sectors | At most 128 |
| 76 | 436 | First 109 DIFAT entries | |

- Sector `s` starts at byte (s + 1) x sector size. For 4096-byte sectors the 512-byte header is padded to a whole sector.
- Physical sectors = ceil(input length / sector size) - 1.
- Special sector ids: `FFFFFFFF` free, `FFFFFFFE` end of chain, `FFFFFFFD` FAT sector, `FFFFFFFC` DIFAT sector.

## Directory entries (128 bytes)

| Offset | Size | Field |
| ---: | ---: | --- |
| 0 | 64 | Name, UTF-16LE, NUL-terminated |
| 64 | 2 | Name length in bytes including the terminator |
| 66 | 1 | Object type: 0 unused, 1 storage, 2 stream, 5 root |
| 68 | 4 | Left sibling id |
| 72 | 4 | Right sibling id |
| 76 | 4 | Child id |
| 116 | 4 | Starting sector |
| 120 | 8 | Stream size (only the low 32 bits are meaningful in version 3) |

Entry validation: object type in {0, 1, 2, 5}; for non-empty entries the name length is even, 2 to 64, the last UTF-16 unit before it is NUL, and the name is valid UTF-16. Entry 0 must be the root (type 5).

## Validation before allocation

Run these on the raw bytes before building directory or FAT structures:

1. Declared FAT sectors must not exceed (sectors needed to index every physical sector) + 8. Needed = ceil(physical sectors / (sector size / 4)).
2. Gather FAT sector ids from the 109 header slots and then the DIFAT chain (each DIFAT sector holds sector size / 4 - 1 ids and a next pointer in its last 4 bytes). Skip free and end-of-chain slots. Stop when the declared number of FAT sectors is collected. Every id must be below the physical sector count and unique. The DIFAT chain must not cycle and must not exceed 128 sectors. The collected count must equal the declared count.
3. Walk the directory sector chain through the FAT. Each directory sector id must be in range and visited once. Count entries as sectors x (sector size / 128); stop with an error above 4096.
4. Walk the directory tree from the root's child: follow left, right and (for storages only) child links with an explicit stack. Each id must be in range and visited once; reachable entries must be storages or streams. Track the UTF-8 path length (parent length + 1 + name length) and fail above 4096 bytes.
5. Every stream's declared size must fit its class limit and all declared sizes together must fit 512 MiB ([limits.md](limits.md)).

## Damaged files Hancom opens

Five fixtures fail strict MS-CFB validation and must still open. All five open in Hancom and in olefile.

| Fixture | Anomaly |
| --- | --- |
| `pic2.hwp` | Declares 27 FAT sectors; 18 can index the 1,179,136-byte file. DIFAT lists all 27 (ids 3, 129, 257, ...). |
| `task2097/18095317_eogu_geumji.hwp` | Declares 62 FAT sectors; 31 needed. |
| `issue2063_huge_cellbreak_table.hwp` | Declares 21 FAT sectors; 11 needed. |
| `mix-shape-01.hwp` | Two FAT chains reference sector 0 (a strict reader reports "sector 0 pointed to twice"). |
| `tac-case-001.hwp` | Same as `mix-shape-01.hwp`. |

Required recovery behavior (a second, lenient reader used only when strict opening fails):

- Surplus FAT sectors: accept a declared count up to the physical sector count; read only the prefix of FAT sectors needed to index the file. Skip out-of-range or duplicate FAT sector ids instead of failing.
- DIFAT chain: follow at most the declared number of DIFAT sectors; stop silently on a revisit, an out-of-range sector, or end of chain.
- Directory chain: read at most 4096 x 128 bytes.
- Name length field: clamp to 64 before slicing.
- Sector chains (FAT and mini FAT): stop on a revisited sector, an out-of-range sector, or end of chain; never read past the stream's declared size. A chain that reuses a sector (the sector 0 case) is read as written.
- Mini sectors are 64 bytes; the mini stream is the root entry's chain, capped at 512 MiB.
- Accept sector shifts 9 to 12 in recovery mode (the old recovery path also accepted 10 and 11).

Path lookup in recovery mode walks the red-black tree from the root by name. If the walk meets damage (cycle, id out of range, an unused or root entry where a child is expected), fall back to a search over all entries for a unique entry with that name and an acceptable type; two matches mean not found. A stream can never act as a parent storage. Same-named streams in different storages (`BodyText/Section0` and `ViewText/Section0`) must resolve by full path.

Recommendation: build one reader with these tolerant rules and a strict switch. The strict switch is for re-parsing our own saved output: a file that only opens through recovery must not be written to disk, because Hancom may reject it.

## Name matching

[MS-CFB] compares names case-insensitively using simple uppercase mapping per UTF-16 unit, shorter names first. Use the same rule for every lookup and for comparing paths (for example `BinData/bin0001.StrAße` matches `BINDATA/BIN0001.STRAßE`; `ß` does not expand to `SS`). The old recovery path compared names case-sensitively; no fixture depends on that.

## HWP 5 stream inventory

Counts are from the 383 HWP 5 fixtures.

| Path | Fixtures | Handling |
| --- | ---: | --- |
| `FileHeader` | 383 | [file-header.md](file-header.md); never compressed |
| `DocInfo` | 383 | Record stream; compressed when the header says so |
| `BodyText/SectionN` | 383 (494 streams) | Record stream per section; compressed when the header says so |
| `ViewText/SectionN` | 7 (21 streams) | Distribution body, encrypted then compressed ([distribution.md](distribution.md)) |
| `SectionN` at the root | 0 | Legacy location; read when `BodyText/SectionN` is absent |
| `BinData/BINxxxx.ext` | 2002 streams | [bindata.md](bindata.md) |
| `PrvText` | 383 | Preview text |
| `PrvImage` | 383 | Preview image |
| `\005HwpSummaryInformation` | 380 | OLE property set; 2 fixtures name it `HwpSummaryInformation` without the `\005` prefix |
| `DocOptions/_LinkDoc` | 383 | Preserved |
| `Scripts/DefaultJScript`, `Scripts/JScriptVersion` | 383 | Preserved (encrypted in distribution documents) |
| `DocHistory/HistoryLastDoc`, `DocHistory/VersionLogN` | 1 (`basic/treatise sample.hwp`) | Preserved |
| `RhwpHwpxOrigin` | 1 | Root stream written by an earlier editor, content `1`; sets HWPX lineage ([post-parse.md](post-parse.md)) |
| `RhwpHyperlinkFormat` | 0 | Root stream written by an earlier editor: JSON (version 1) with original hyperlink colors and underlines per link id; decoded when at most 16 MiB and removed from the preserved set. Owned by the hyperlink feature. |

## Section discovery

- Count sections by probing index 0, 1, 2, ... while any of `BodyText/SectionN`, `ViewText/SectionN` or root `SectionN` exists.
- In all 383 fixtures this count equals the DocumentProperties section count, taking `ViewText` into account. Distribution documents carry a 1-section stub in `BodyText` (`BodyText/Section0` only) and the real sections in `ViewText` (`hwpspec.hwp`: 6 `ViewText` sections, 1 `BodyText` section, DocumentProperties says 6).
- Which stream holds section N: if the header's distribution bit is set and `ViewText/SectionN` exists, that stream (decrypt, then inflate). Otherwise `BodyText/SectionN`, else root `SectionN` (inflate when compressed).
- Non-distribution files can carry leftover `ViewText` streams (`20250130-hongbo.hwp`, `task2070/1130000-201900011_D0150004-1-002_2017년기준 시장구조조사.hwp`). Ignore them when the distribution bit is clear.
- The distribution stub paragraph in `BodyText/Section0` (Hancom text) reads: "이 문서는 상위 버전의 배포용 문서입니다. 문서를 읽으려면 최신 버전의 한글 또는 한글 전용 뷰어가 필요합니다. (주의! 현재 상태에서 문서를 저장하는 경우 원래 문서의 내용이 사라집니다.)" It is never shown when decryption succeeds.

## Preview streams

- `PrvImage`: observed PNG (216 fixtures), GIF (163), JPEG (3), empty (1). The public document says BMP or GIF; current Hancom writes PNG. Kind detection as in [format-detection.md](format-detection.md).
- `PrvText`: UTF-16LE text, at most 2046 bytes in the fixtures (about 1023 units). Hancom formats it as `<...>` chunks per paragraph and `\r\n`. Decode with U+FFFD for unpaired surrogates; fewer than 2 bytes or an empty result means no preview text.
- A missing preview stream is not an error.

## Summary information

`\005HwpSummaryInformation` is an OLE property set (FMTID section starting at the offset stored at byte 44). Observed properties: 2 title, 3 subject, 4 author, 5 keywords, 6 comments, 8 last saved by, 9 application version string (for example `13, 0, 0, 3457 WIN32LEWindows_10`), 20 date string in Korean (`1998년 2월 26일 목요일, 11시 2분`), plus integer properties 11 to 14 and 21. Strings are type 0x1F (UTF-16 with a unit count). Only property 20 is used by this unit (HWP 3 lineage, [post-parse.md](post-parse.md)); the stream is preserved verbatim.

## Streams preserved for round trip

Keep every stream that is not regenerated by the writer, with its full path and raw bytes:
- Excluded: `FileHeader`, `DocInfo`, every `BodyText/*`, every `ViewText/*`, `PrvImage`, `PrvText`, and each `BinData` stream that corresponds to a loaded BIN_DATA item (path computed with the naming rule of [bindata.md](bindata.md) and compared with CFB name matching).
- Included: summary information, `DocOptions/*`, `Scripts/*`, `DocHistory/*`, `XMLTemplate/*`, unknown root streams, and orphan `BinData` streams that no BIN_DATA item names (for example `img-start-001.hwp` has 20 `BinData` streams and 0 BIN_DATA items; `exam_social.hwp` has orphans `BIN0005.emf` and `BIN0006.emf`).
- Each preserved stream is read with the binary member limit clamped by the remaining container budget; a read failure skips that stream.
- Order: directory walk order.

Old-engine gap: when the file only opened through recovery, it preserved nothing and read no previews, so saving such a document dropped summary information, scripts, history and orphan images.
