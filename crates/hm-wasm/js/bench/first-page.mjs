// Open to first page in a fresh Node process, with the document's fonts:
// node first-page.mjs <doc> [runs]. Prints one JSON line per run: register
// (font bytes into the engine), open (parse), layout (all pages), page
// (first page packet) and paint-ready (outlines of its glyphs), in ms, and
// the linear memory with and without the font bytes.
import { readFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const [doc, runs = '3', child] = process.argv.slice(2);
if (!child) {
  for (let k = 0; k < Number(runs); k++) {
    process.stdout.write(execFileSync(process.execPath, [fileURLToPath(import.meta.url), doc, '1', 'child'], { env: process.env }));
  }
  process.exit(0);
}
const { engine, fontCatalog, memory: mem } = await import('../test/env.mjs');
const memory = () => mem.buffer.byteLength;
const bytes = readFileSync(doc);
const catalog = fontCatalog();
const e = engine();
const t0 = performance.now();
const d = e.open(bytes);
const t1 = performance.now();
const wanted = new Set(JSON.parse(d.fontNames()));
let fontBytes = 0;
for (const f of catalog.fonts) {
  if (!f.families.some(n => wanted.has(n))) continue;
  const b = readFileSync(f.path);
  fontBytes += b.length;
  e.addFont(f.path, b);
}
const t2 = performance.now();
const pages = JSON.parse(d.state()).pageCount;
const t3 = performance.now();
const packet = d.page(0);
const t4 = performance.now();
const { decodePacket } = await import('../player.mjs');
const p = decodePacket(packet);
e.glyphOutlines(p.glyphs);
const t5 = performance.now();
const r = (a, b) => Math.round((b - a) * 10) / 10;
console.log(JSON.stringify({
  open_ms: r(t0, t1), register_ms: r(t1, t2), layout_ms: r(t2, t3), first_page_ms: r(t3, t4), outlines_ms: r(t4, t5),
  open_to_first_page_ms: r(t0, t4) - r(t1, t2), pages, packet_bytes: packet.length,
  memory_mib: Math.round(memory() / 1048576 * 10) / 10, font_mib: Math.round(fontBytes / 1048576 * 10) / 10,
}));
