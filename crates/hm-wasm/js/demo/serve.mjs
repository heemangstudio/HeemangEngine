// Serves the reference player page with a corpus document and the fonts
// it names: node serve.mjs [port]. Needs HEEMANG_CORPUS and HEEMANG_FONTS
// and the web package in target/hm-wasm-pkg/web (build.sh).

import { createServer } from 'node:http';
import { readFileSync, existsSync, statSync } from 'node:fs';
import { join, resolve, extname, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { fontCatalog, corpus } from '../test/env.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, '../../../..');
const port = Number(process.argv[2] ?? 8597);
const types = { '.html': 'text/html', '.mjs': 'text/javascript', '.js': 'text/javascript', '.wasm': 'application/wasm', '.json': 'application/json' };
const catalog = fontCatalog();
const known = new Set([...catalog.fonts.map(f => f.path), ...catalog.tables]);

function send(res, status, body, type = 'application/octet-stream') {
  res.writeHead(status, { 'content-type': type, 'cache-control': 'no-store' });
  res.end(body);
}

createServer((req, res) => {
  const url = new URL(req.url, 'http://x');
  const p = decodeURIComponent(url.pathname);
  if (p === '/catalog.json') return send(res, 200, JSON.stringify(catalog), types['.json']);
  if (p === '/file') {
    const f = url.searchParams.get('path');
    if (!known.has(f)) return send(res, 404, 'unknown font');
    return send(res, 200, readFileSync(f));
  }
  if (p.startsWith('/corpus/')) {
    const f = resolve(corpus, p.slice(8));
    if (!f.startsWith(resolve(corpus)) || !existsSync(f)) return send(res, 404, 'no such document');
    return send(res, 200, readFileSync(f));
  }
  const map = p.startsWith('/pkg/') ? join(root, 'target/hm-wasm-pkg/web', p.slice(5)) : join(here, '..', p);
  const f = resolve(map);
  if (!(f.startsWith(resolve(here, '..')) || f.startsWith(join(root, 'target/hm-wasm-pkg'))) || !existsSync(f) || statSync(f).isDirectory()) {
    return send(res, 404, 'not found', 'text/plain');
  }
  send(res, 200, readFileSync(f), types[extname(f)] ?? 'application/octet-stream');
}).listen(port, '127.0.0.1', () => console.log(`http://127.0.0.1:${port}/demo/index.html?doc=<corpus path>`));
