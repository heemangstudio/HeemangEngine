// Reference page for the canvas player: paints a corpus document's pages
// from packets, optionally beside the engine's own raster with a pixel
// comparison (?compare=1), and edits through locate, navigate and transact.
// Query: doc=<corpus path>, dpi=96, pages=<count>, compare=1.

import init, { Engine } from '/pkg/hm_wasm.js';
import { Player, decodePacket, patchImage } from '../player.mjs';

const q = new URLSearchParams(location.search);
const dpi = Number(q.get('dpi') ?? 96);
const compare = q.get('compare') === '1';
const status = document.getElementById('status');
const J = v => JSON.stringify(v);
const hu = px => Math.round((px * 7200) / dpi);
const px = v => (v * dpi) / 7200;

const bytes = async url => new Uint8Array(await (await fetch(url)).arrayBuffer());

await init({ module_or_path: '/pkg/hm_wasm_bg.wasm' });
const catalog = await (await fetch('/catalog.json')).json();
const engine = new Engine();
for (const p of catalog.tables) {
  const b = await bytes(`/file?path=${encodeURIComponent(p)}`);
  if (/fontmap\.dat$/i.test(p)) engine.addFontMap(b);
  else if (/hftinfo\.dat$/i.test(p)) engine.addHftInfo(b);
  else engine.addCodeTable(b);
}
const docBytes = await bytes(`/corpus/${q.get('doc')}`);
const t0 = performance.now();
const doc = engine.open(docBytes);
// The first available candidate of each face, its first stand-in and the
// first available family of each class default list.
const req = JSON.parse(doc.fontRequests());
const have = new Set(catalog.fonts.flatMap(f => f.families));
const first = list => list.find(n => have.has(n));
const wanted = new Set([
  ...req.faces.flatMap(f => [first(f.candidates), first(f.standIns)]),
  ...req.classDefaults.map(first),
].filter(Boolean));
const files = catalog.fonts.filter(f => f.families.some(n => wanted.has(n)));
const tOpen = performance.now() - t0;
const fontBytes = await Promise.all(files.map(f => bytes(`/file?path=${encodeURIComponent(f.path)}`)));
const t1 = performance.now();
files.forEach((f, k) => engine.addFont(f.path, fontBytes[k]));
const player = new Player(engine);
const info = JSON.parse(doc.pages());
const count = Math.min(info.length, Number(q.get('pages') ?? info.length));
const views = [];
const results = { doc: q.get('doc'), dpi, fonts: files.length, pages: [] };

function paintPage(i) {
  const v = views[i];
  const packet = decodePacket(doc.page(i));
  const ctx = v.canvas.getContext('2d');
  ctx.fillStyle = '#fff';
  ctx.fillRect(0, 0, v.canvas.width, v.canvas.height);
  player.paint(ctx, packet, dpi, (item, d) =>
    patchImage(doc.patch(i, item, d), (w, h) => new OffscreenCanvas(w, h)));
  return packet;
}

for (let i = 0; i < count; i++) {
  const page = info[i];
  const w = Math.round(px(page.width));
  const h = Math.round(px(page.height));
  const pair = document.createElement('div');
  pair.className = 'pair';
  const box = document.createElement('div');
  box.className = 'page';
  box.style.width = `${w}px`;
  const canvas = Object.assign(document.createElement('canvas'), { width: w, height: h });
  const overlay = Object.assign(document.createElement('canvas'), { width: w, height: h, className: 'overlay' });
  box.append(canvas, overlay);
  pair.append(box);
  document.getElementById('pages').append(pair);
  views.push({ canvas, overlay, pair });
  const s = performance.now();
  const packet = paintPage(i);
  const ms = performance.now() - s;
  if (i === 0) results.firstPaintMs = performance.now() - t1;
  const r = { page: i, paintMs: Math.round(ms * 10) / 10, ops: packet.ops.length, lines: packet.lines.length };
  if (compare) {
    const png = new Blob([doc.pagePng(i, dpi)], { type: 'image/png' });
    const img = await createImageBitmap(png);
    const ref = Object.assign(document.createElement('canvas'), { width: w, height: h });
    const rc = ref.getContext('2d');
    rc.drawImage(img, 0, 0);
    const refBox = document.createElement('div');
    refBox.className = 'page';
    refBox.append(ref);
    pair.append(refBox);
    const a = canvas.getContext('2d').getImageData(0, 0, w, h).data;
    const b = rc.getImageData(0, 0, w, h).data;
    let ink = 0;
    let off = 0;
    for (let k = 0; k < a.length; k += 4) {
      const darkB = Math.min(b[k], b[k + 1], b[k + 2]) < 200;
      const darkA = Math.min(a[k], a[k + 1], a[k + 2]) < 200;
      if (darkB) ink++;
      const d = Math.max(Math.abs(a[k] - b[k]), Math.abs(a[k + 1] - b[k + 1]), Math.abs(a[k + 2] - b[k + 2]));
      if ((darkA || darkB) && d > 64) off++;
    }
    r.inkPixels = ink;
    r.offPixels = off;
    r.offShare = ink ? Math.round((off / ink) * 10000) / 100 : 0;
  }
  results.pages.push(r);
}
results.openMs = Math.round(tOpen);
status.textContent = `${count} pages, ${files.length} font files, first paint ${Math.round(results.firstPaintMs)} ms`;
const out = document.getElementById('result');
out.textContent = J(results);
window.results = results;
window.doc = doc;

// Interaction: click places the caret, shift-click selects, keys edit.
let caret = null;
let anchor = null;
let preferredX = null;

let picked = null;

function drawOverlay(rects = []) {
  for (const v of views) v.overlay.getContext('2d').clearRect(0, 0, v.overlay.width, v.overlay.height);
  if (picked && views[picked.page]) {
    const c = views[picked.page].overlay.getContext('2d');
    c.strokeStyle = '#3b82f6';
    c.lineWidth = 1.5;
    c.strokeRect(px(picked.x), px(picked.y), px(picked.width), px(picked.height));
  }
  for (const r of rects) {
    const c = views[r.page]?.overlay.getContext('2d');
    if (!c) continue;
    c.fillStyle = '#3b82f640';
    c.fillRect(px(r.x), px(r.y), px(r.width), px(r.height));
  }
  if (caret && views[caret.caret.page]) {
    const c = views[caret.caret.page].overlay.getContext('2d');
    c.fillStyle = '#000';
    c.fillRect(px(caret.caret.x), px(caret.caret.y), 1.5, px(caret.caret.height));
  }
}

views.forEach((v, page) => {
  v.overlay.style.pointerEvents = 'auto';
  v.overlay.addEventListener('mousedown', ev => {
    const hit = JSON.parse(doc.locate(J({ kind: 'point', page, x: hu(ev.offsetX), y: hu(ev.offsetY) })));
    if (!hit) return;
    picked = hit.object ?? null;
    if (ev.shiftKey && caret) {
      anchor ??= caret;
      caret = hit;
      drawOverlay(JSON.parse(doc.select(J({ from: anchor.position, to: caret.position }))));
    } else {
      anchor = null;
      caret = hit;
      drawOverlay();
    }
    preferredX = null;
  });
});

function repaint() {
  for (let i = 0; i < views.length; i++) paintPage(i);
}

function moveTo(position, affinity = 'downstream') {
  const c = JSON.parse(doc.locate(J({ kind: 'caret', position, affinity, page: caret?.caret.page })));
  if (c) caret = { position, affinity, caret: c };
  drawOverlay();
}

// Keys go through a hidden textarea so IME composition reaches the engine:
// preedit text is laid out in place and committed as one undo step.
const input = Object.assign(document.createElement('textarea'), { autocomplete: 'off' });
input.style.cssText = 'position:fixed;left:-1000px;top:0;width:10px;height:10px;opacity:0';
document.body.append(input);
addEventListener('mouseup', () => caret && input.focus());

function after(r, fallback) {
  repaint();
  moveTo(r?.caret ?? fallback ?? caret.position);
}

input.addEventListener('compositionupdate', ev => {
  if (!caret) return;
  const r = JSON.parse(doc.compose(J(composing ? { text: ev.data } : { at: caret.position, text: ev.data })));
  composing ??= caret.position;
  repaint();
  const c = JSON.parse(doc.locate(J({ kind: 'caret', position: r.caret, page: caret.caret.page })));
  if (c) caret = { position: r.caret, affinity: 'downstream', caret: c };
  drawOverlay();
});
input.addEventListener('compositionend', ev => {
  if (!composing) return;
  const r = JSON.parse(doc.commitComposition(J({ text: ev.data })));
  composing = null;
  input.value = '';
  after(r);
});
input.addEventListener('input', ev => {
  if (ev.isComposing || composing || !caret || !ev.data) return;
  const r = JSON.parse(doc.transact(J({ ops: [{ op: 'insertText', at: caret.position, text: ev.data }] })));
  input.value = '';
  after(r);
});
let composing = null;

input.addEventListener('keydown', ev => {
  if (!caret || ev.isComposing || composing) return;
  const motions = { ArrowLeft: 'left', ArrowRight: 'right', ArrowUp: 'up', ArrowDown: 'down', Home: 'lineStart', End: 'lineEnd' };
  if (motions[ev.key]) {
    const m = motions[ev.key];
    if (m === 'up' || m === 'down') preferredX ??= caret.caret.x;
    else preferredX = null;
    const next = JSON.parse(doc.navigate(J({ position: caret.position, affinity: caret.affinity, motion: m, preferredX, page: caret.caret.page })));
    if (next) caret = next;
    anchor = null;
    drawOverlay();
    ev.preventDefault();
    return;
  }
  if (ev.key === 'Enter') {
    ev.preventDefault();
    after(JSON.parse(doc.transact(J({ ops: [{ op: 'split', at: caret.position }] }))));
  } else if (ev.key === 'Backspace') {
    ev.preventDefault();
    const prev = JSON.parse(doc.navigate(J({ position: caret.position, motion: 'left' })));
    if (!prev) return;
    after(JSON.parse(doc.transact(J({ ops: [{ op: 'delete', from: prev.position, to: caret.position }] }))), prev.position);
  } else if ((ev.metaKey || ev.ctrlKey) && ev.key === 'z') {
    ev.preventDefault();
    doc.undo();
    repaint();
    drawOverlay();
  }
});
