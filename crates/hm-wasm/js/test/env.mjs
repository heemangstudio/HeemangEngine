// Shared setup for the Node tests: the generated package, corpus files and
// a font catalog built with `familyNames` (cached outside the repository).

import { readFileSync, readdirSync, statSync, writeFileSync, existsSync } from 'node:fs';
import { join, dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { tmpdir } from 'node:os';

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, '../../../..');
const pkg = process.env.HM_WASM_PKG ?? join(root, 'target/hm-wasm-pkg/web');

export const hm = await import(pathToFileURL(join(pkg, 'hm_wasm.js')));
/** The instance's linear memory. */
export const memory = hm.initSync({ module: readFileSync(join(pkg, 'hm_wasm_bg.wasm')) }).memory;
export const corpus = process.env.HEEMANG_CORPUS;
export const fontDirs = (process.env.HEEMANG_FONTS ?? '').split(':').filter(Boolean);

export const fixture = name => readFileSync(join(here, '../../tests/fixtures', name));
export const sample = rel => (corpus && existsSync(join(corpus, rel)) ? readFileSync(join(corpus, rel)) : null);

function walk(dir, out) {
  let names;
  try { names = readdirSync(dir); } catch { return out; }
  for (const n of names) {
    const p = join(dir, n);
    const s = statSync(p);
    if (s.isDirectory()) walk(p, out);
    else out.push({ path: p, size: s.size, mtime: s.mtimeMs });
  }
  return out;
}

let catalog = null;

/** Font files under HEEMANG_FONTS with their family names. */
export function fontCatalog() {
  if (catalog) return catalog;
  const cachePath = join(tmpdir(), 'hm-wasm-font-catalog.json');
  const cache = existsSync(cachePath) ? JSON.parse(readFileSync(cachePath, 'utf8')) : {};
  const files = fontDirs.flatMap(d => walk(d, []));
  catalog = { fonts: [], tables: [] };
  let changed = false;
  for (const f of files) {
    const lower = f.path.toLowerCase();
    if (/\.(ttf|otf|ttc|otc|hft)$/.test(lower)) {
      const key = `${f.path}:${f.size}:${f.mtime}`;
      if (!cache[key]) {
        cache[key] = JSON.parse(hm.familyNames(readFileSync(f.path)));
        changed = true;
      }
      catalog.fonts.push({ path: f.path, families: cache[key] });
    } else if (/\/u300_[^/]*\.dat$/.test(lower) || /(fontmap|hftinfo)\.dat$/.test(lower)) {
      catalog.tables.push(f.path);
    }
  }
  if (changed) writeFileSync(cachePath, JSON.stringify(cache));
  return catalog;
}

/** An engine with Hancom's code tables, font map and HFT catalog. */
export function engine() {
  const e = new hm.Engine();
  const c = fontCatalog();
  for (const p of c.tables) {
    const b = readFileSync(p);
    if (/fontmap\.dat$/i.test(p)) e.addFontMap(b);
    else if (/hftinfo\.dat$/i.test(p)) e.addHftInfo(b);
    else e.addCodeTable(b);
  }
  return e;
}

/** Registers the catalog fonts a document may draw with; returns bytes added. */
export function registerFonts(e, doc, loaded = new Set()) {
  const wanted = new Set(JSON.parse(doc.fontNames()));
  let bytes = 0;
  for (const f of fontCatalog().fonts) {
    if (loaded.has(f.path) || !f.families.some(n => wanted.has(n))) continue;
    const b = readFileSync(f.path);
    e.addFont(f.path, b);
    loaded.add(f.path);
    bytes += b.length;
  }
  return bytes;
}
