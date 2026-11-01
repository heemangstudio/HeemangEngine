// Shared setup for the Node tests: the generated package, corpus files and
// a font catalog built with `familyNames` (cached outside the repository).

import { readFileSync, readdirSync, statSync, writeFileSync, existsSync } from 'node:fs';
import { join, dirname, resolve, delimiter } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { tmpdir } from 'node:os';

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, '../../../..');
const pkg = process.env.HM_WASM_PKG ?? join(root, 'target/hm-wasm-pkg/web');

export const hm = await import(pathToFileURL(join(pkg, 'hm_wasm.js')));
/** The instance's linear memory. */
export const memory = hm.initSync({ module: readFileSync(join(pkg, 'hm_wasm_bg.wasm')) }).memory;
export const corpus = process.env.HEEMANG_CORPUS;
// `:`-separated on macOS and Linux, `;`-separated on Windows, as PATH is.
export const fontDirs = (process.env.HEEMANG_FONTS ?? '').split(delimiter).filter(Boolean);

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
      catalog.fonts.push({ path: f.path, size: f.size, families: cache[key] });
    } else if (/[\\/]u300_[^\\/]*\.dat$/.test(lower) || /(fontmap|hftinfo)\.dat$/.test(lower)) {
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

/** A `FontFile` over bytes the host holds. */
export const fontFile = bytes => ({ size: bytes.length, read: (offset, length) => bytes.subarray(offset, offset + length) });

/**
 * Registers the catalog fonts a document draws with (`fontRequests`): the
 * first available candidate of each face and Hancom's default face; then
 * the fallbacks for characters those lack; after a layout, what the layout
 * asked for beyond that (rare), and the outlines it measured that the font
 * copies lack. Returns the bytes read.
 */
export function registerFonts(e, doc, loaded = new Set()) {
  const fonts = fontCatalog().fonts;
  // The engine matches family names without case or spaces.
  const norm = n => n.toLowerCase().replace(/\s/g, '');
  const have = new Set(fonts.flatMap(f => f.families.map(norm)));
  const first = list => list.map(norm).find(n => have.has(n));
  // Hancom ships some files twice; one copy is enough.
  const copy = f => `${f.path.split(/[\\/]/).pop().toLowerCase()}:${f.size}`;
  const copies = new Set([...loaded].map(p => copy(fonts.find(f => f.path === p) ?? { path: p })));
  const add = families => {
    let bytes = 0;
    for (const f of fonts) {
      if (loaded.has(f.path) || copies.has(copy(f)) || !f.families.some(n => families.has(norm(n)))) continue;
      copies.add(copy(f));
      const b = readFileSync(f.path);
      e.addFontFile(f.path, fontFile(b));
      loaded.add(f.path);
      bytes += b.length;
    }
    return bytes;
  };
  const wanted = req => {
    const families = new Set();
    for (const f of req.faces) {
      const n = first(f.candidates);
      if (n) families.add(n);
    }
    // Hancom's default face, else the first serif and sans families.
    const defaults = first(req.classDefaults[0]);
    for (const n of defaults ? [defaults] : req.classDefaults.slice(1).map(first)) {
      if (n) families.add(n);
    }
    for (const n of req.fallbacks.map(norm)) if (have.has(n)) families.add(n);
    return families;
  };
  let bytes = add(wanted(JSON.parse(doc.fontRequests())));
  // Fallbacks for characters those faces lack.
  bytes += add(wanted(JSON.parse(doc.fontRequests())));
  doc.state();
  // Anything only the layout found (rare): registering it lays out again.
  const more = add(wanted(JSON.parse(doc.fontRequests())));
  registerFonts.second = more;
  bytes += more;
  // Outlines the layout asked for that the font copies lack.
  doc.state();
  for (const path of JSON.parse(e.outlineRequests())) e.addFontFile(path, fontFile(readFileSync(path)));
  return bytes;
}
