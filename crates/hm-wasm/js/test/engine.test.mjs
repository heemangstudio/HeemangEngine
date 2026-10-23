// The engine through its JS boundary: open, state, pages, locate, select,
// navigate, transact, undo and serialize, on corpus documents
// (HEEMANG_CORPUS, fonts from HEEMANG_FONTS).

import test from 'node:test';
import assert from 'node:assert/strict';
import { decodePacket, decodeOutlines } from '../player.mjs';
import { readFileSync } from 'node:fs';
import { hm, engine, registerFonts, sample, fontFile } from './env.mjs';

const HWP = '2022년 국립국어원 업무계획.hwp';
const HWPX = 'hwpx/2024년 1분기 해외직접투자 보도자료 ff.hwpx';

const J = v => JSON.stringify(v);
const paragraphs = (doc, list) => JSON.parse(doc.query(J({ kind: 'paragraphs', list })));
const text = (doc, list, para) => paragraphs(doc, list)[para].text;
const state = doc => JSON.parse(doc.state());

// Engines hold their fonts in linear memory until freed; free them after
// each test so the suite stays far from the 4 GiB limit.
const live = [];
const own = x => (live.push(x), x);
test.afterEach(() => {
  for (const x of live.splice(0)) x.free();
});

function open(rel) {
  const bytes = sample(rel);
  if (!bytes) return null;
  const e = own(engine());
  const doc = own(e.open(bytes));
  registerFonts(e, doc);
  return { e, doc, bytes };
}

function code(f) {
  try {
    f();
  } catch (err) {
    return err.code;
  }
  return null;
}

test('rejects malformed and foreign input with codes, then opens normally', () => {
  const e = own(new hm.Engine());
  assert.equal(code(() => e.open(new Uint8Array([1, 2, 3]))), 'UNSUPPORTED_FORMAT');
  // An HWP 5 compound file signature with nothing behind it.
  const cfb = new Uint8Array(1024);
  cfb.set([0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1]);
  assert.ok(['MALFORMED_INPUT', 'UNSUPPORTED_FORMAT'].includes(code(() => e.open(cfb))));
  const caps = JSON.parse(hm.capabilities());
  assert.equal(caps.contract, 1);
  assert.deepEqual(caps.open, ['hwp5', 'hwpx', 'hwp3']);
  const bytes = sample(HWPX);
  if (bytes) assert.ok(state(own(e.open(bytes))).pageCount > 0);
});

for (const rel of [HWP, HWPX]) {
  test(`views, locates and selects: ${rel}`, t => {
    const o = open(rel);
    if (!o) return t.skip('corpus not set');
    const { e, doc } = o;
    const s = state(doc);
    assert.equal(s.format, rel.endsWith('x') ? 'hwpx' : 'hwp5');
    assert.equal(s.contentEpoch, 0);
    const pages = JSON.parse(doc.pages());
    assert.equal(pages.length, s.pageCount);
    assert.ok(pages[0].width > 50000 && pages[0].height > 50000);
    // The body sits inside the margins, the header above it, the footer
    // below, and the columns inside the body.
    const { margins: m, body, header, footer, columns } = pages[0];
    assert.equal(body.x, m.left + m.gutter);
    assert.equal(pages[0].width - body.x - body.width, m.right);
    assert.ok(header.y + header.height <= body.y && footer.y >= body.y + body.height);
    assert.ok(columns.length > 0 && columns.every(c => c.x >= body.x && c.x + c.width <= body.x + body.width));

    const svg = doc.pageSvg(0, 96);
    assert.match(svg, /^<svg xmlns="http:\/\/www.w3.org\/2000\/svg"/);
    assert.ok(svg.includes('<use href="#g0"'));
    const pdf = new TextDecoder().decode(doc.pdf(0, 2, 96));
    assert.ok(pdf.startsWith('%PDF-1.4') && pdf.includes(`/Count ${Math.min(2, s.pageCount)}`));
    assert.equal(code(() => doc.pdf(5000, 5001, 96)), 'NO_SUCH_PAGE');

    const p = decodePacket(doc.page(0));
    assert.equal(p.width, pages[0].width);
    assert.ok(p.ops.some(o => o.op === 'glyphs'));
    assert.ok(p.lines.length > 0);
    const outlines = decodeOutlines(e.glyphOutlines(p.glyphs));
    assert.equal(outlines.size, p.glyphs.length / 2);
    const drawn = [...outlines.values()].filter(o => o.cmds.length > 0).length;
    assert.ok(drawn > outlines.size * 0.9, `${drawn} of ${outlines.size} glyphs have outlines`);

    // Every stop of a body line hits back to itself.
    // (A stop sharing its x with a neighbor, before a zero-width control,
    // hits the first of them.)
    const unique = (l, k) => l.stops[k - 1][1] < l.stops[k][1] && l.stops[k][1] < l.stops[k + 1][1];
    const line = p.lines.find(l => l.list.steps.length === 0 && l.stops.length > 4 && unique(l, 2));
    const [pos, x] = line.stops[2];
    const hit = JSON.parse(doc.locate(J({ kind: 'point', page: 0, x: x + 10, y: line.y + line.height / 2 })));
    assert.equal(hit.position.para, line.para);
    assert.equal(hit.position.offset, pos);
    assert.deepEqual([hit.caret.x, hit.caret.y], [x, line.y]);
    const caret = JSON.parse(doc.locate(J({ kind: 'caret', position: hit.position })));
    assert.deepEqual(caret, hit.caret);

    // Wrapped line ends are upstream, and the caret stays on that line.
    const wrapped = p.lines.find((l, i) =>
      p.lines[i + 1]?.para === l.para && l.list.steps.length === 0 && l.stops.length > 3 &&
      l.stops.at(-2)[1] < l.stops.at(-1)[1]);
    if (wrapped) {
      const [endPos, endX] = wrapped.stops.at(-1);
      const end = JSON.parse(doc.locate(J({ kind: 'point', page: 0, x: endX + 500, y: wrapped.y + 10 })));
      assert.equal(end.position.offset, endPos);
      assert.equal(end.affinity, 'upstream');
      const c = JSON.parse(doc.locate(J({ kind: 'caret', position: end.position, affinity: 'upstream' })));
      assert.equal(c.y, wrapped.y);
      const down = JSON.parse(doc.locate(J({ kind: 'caret', position: end.position })));
      assert.notEqual(down.y, wrapped.y);
    }

    // A selection across a paragraph covers each of its lines.
    const list = { section: 0, steps: [] };
    const info = paragraphs(doc, list)[line.para];
    const rects = JSON.parse(doc.select(J({
      from: { section: 0, para: line.para, offset: info.caret[0] },
      to: { section: 0, para: line.para, offset: info.caret[1] },
    })));
    const shown = p.lines.filter(l => l.para === line.para && l.list.steps.length === 0).length;
    assert.ok(rects.length >= shown, `${rects.length} rects for ${shown} lines`);
    assert.ok(rects.every(r => r.width >= 0 && r.height > 0));

    // Movement: right then left returns; down then up returns to the line.
    const nav = (position, motion, extra = {}) =>
      JSON.parse(doc.navigate(J({ position, motion, ...extra })));
    const right = nav(hit.position, 'right');
    assert.ok(right.position.offset > hit.position.offset);
    assert.deepEqual(nav(right.position, 'left').position, hit.position);
    const down = nav(hit.position, 'down', { preferredX: hit.caret.x });
    assert.ok(down.caret.y > hit.caret.y || down.caret.page > 0);
    const up = nav(down.position, 'up', { preferredX: hit.caret.x, page: down.caret.page });
    assert.equal(up.caret.y, hit.caret.y);
    const home = nav(hit.position, 'lineStart');
    assert.equal(home.caret.x, line.stops[0][1]);

    // A save with no edits gives back the input.
    assert.deepEqual(Buffer.from(doc.serialize(false)), o.bytes);
    doc.close();
    assert.equal(code(() => doc.state()), 'CLOSED');
  });

  test(`edits atomically with undo and saves: ${rel}`, t => {
    const o = open(rel);
    if (!o) return t.skip('corpus not set');
    const { e, doc } = o;
    const list = { section: 0, steps: [] };
    const paras = paragraphs(doc, list);
    const pi = paras.findIndex(p => p.text.length > 10);
    const at = { section: 0, para: pi, offset: paras[pi].caret[0] };
    const before = paras[pi].text;

    const r = JSON.parse(doc.transact(J({ expectedEpoch: 0, ops: [{ op: 'insertText', at, text: '가나다' }] })));
    assert.equal(r.contentEpoch, 1);
    assert.deepEqual(r.caret, { ...at, offset: at.offset + 3 });
    assert.equal(text(doc, list, pi), '가나다' + before);
    assert.ok(decodePacket(doc.page(0)).layoutEpoch > 0);

    // A stale epoch changes nothing; a failing op rolls back the ones before it.
    assert.equal(code(() => doc.transact(J({ expectedEpoch: 0, ops: [{ op: 'insertText', at, text: 'x' }] }))), 'STALE_EPOCH');
    const bad = { section: 0, para: 99999, offset: 0 };
    assert.equal(code(() => doc.transact(J({ ops: [
      { op: 'insertText', at, text: 'zz' },
      { op: 'delete', from: bad, to: bad },
    ] }))), 'BAD_ADDRESS');
    assert.equal(text(doc, list, pi), '가나다' + before);
    assert.equal(state(doc).contentEpoch, 1);

    // Formatting and a split in one transaction undo together.
    const r2 = JSON.parse(doc.transact(J({ ops: [
      { op: 'formatChars', from: at, to: { ...at, offset: at.offset + 3 }, patch: { bold: true, color: 0x79_4e_1f } },
      { op: 'split', at: { ...at, offset: at.offset + 3 } },
    ] })));
    assert.equal(r2.undo, 2);
    assert.equal(text(doc, list, pi + 1), before);
    JSON.parse(doc.undo());
    assert.equal(text(doc, list, pi), '가나다' + before);
    JSON.parse(doc.redo());
    assert.equal(text(doc, list, pi + 1), before);

    const saved = doc.serialize(true);
    assert.equal(state(doc).dirty, false);
    const again = own(e.open(saved));
    assert.equal(text(again, list, pi), '가나다');
    assert.equal(text(again, list, pi + 1), before);
    JSON.parse(doc.undo());
    JSON.parse(doc.undo());
    assert.equal(text(doc, list, pi), before);
    assert.equal(doc.undo(), 'null');
  });
}

/** The first list reached from body controls of `kind`, with a text paragraph. */
function nested(doc, kind) {
  for (const p of paragraphs(doc, { section: 0 })) {
    for (const c of p.controls) {
      if (c.kind !== kind) continue;
      for (let index = 0; index < c.lists; index++) {
        const list = { section: 0, steps: [{ para: p.index, control: c.index, index }] };
        const inner = paragraphs(doc, list);
        const k = inner.findIndex(q => q.text.length > 0);
        if (k >= 0) return { list, para: k, info: inner[k] };
      }
    }
  }
  return null;
}

for (const [rel, kind] of [
  ['footnote-tbox-01.hwp', 'object'],
  ['hwpx/footnote-tbox-01.hwpx', 'object'],
  ['footnote-tbox-01.hwp', 'footnote'],
  ['143E433F503322BD33.hwp', 'header'],
  ['hwpx/143E433F503322BD33.hwpx', 'header'],
  ['multi-table-001.hwp', 'table'],
  ['hwp3-sample19-hwp5.hwp', 'footer'],
]) {
  test(`edits a ${kind} list and finds it on the page: ${rel}`, t => {
    const o = open(rel);
    if (!o) return t.skip('corpus not set');
    const { e, doc } = o;
    const n = nested(doc, kind);
    assert.ok(n, `no ${kind} with text`);
    const at = { ...n.list, para: n.para, offset: n.info.caret[0] };
    const caret = JSON.parse(doc.locate(J({ kind: 'caret', position: at })));
    assert.ok(caret, 'the list shows on a page');
    const hit = JSON.parse(doc.locate(J({ kind: 'point', page: caret.page, x: caret.x + 5, y: caret.y + caret.height / 2 })));
    assert.deepEqual(hit.position.steps, n.list.steps);
    doc.transact(J({ ops: [{ op: 'insertText', at, text: '[편집]' }] }));
    assert.equal(text(doc, n.list, n.para), '[편집]' + n.info.text);
    const again = own(e.open(doc.serialize(false)));
    assert.equal(text(again, n.list, n.para), '[편집]' + n.info.text);
    assert.equal(JSON.parse(again.state()).pageCount, JSON.parse(doc.state()).pageCount);
  });
}

test('moves by grapheme and searches in nested lists', t => {
  const o = open('multi-table-001.hwp');
  if (!o) return t.skip('corpus not set');
  const { doc } = o;
  const list = { section: 0, steps: [] };
  const paras = paragraphs(doc, list);
  const pi = paras.findIndex(p => p.text.length > 3);
  const at = { section: 0, para: pi, offset: paras[pi].caret[0] };
  // A skin-toned emoji (four units), e with a combining acute (two), and
  // old Hangul jamo (three) are one step each.
  doc.transact(J({ ops: [{ op: 'insertText', at, text: '👍🏽é́ᄒᆞᆫZ' }] }));
  const nav = (position, motion) => JSON.parse(doc.navigate(J({ position, motion }))).position.offset;
  const a = nav(at, 'right');
  assert.equal(a, at.offset + 4);
  const b = nav({ ...at, offset: a }, 'right');
  assert.equal(b, a + 2);
  const c = nav({ ...at, offset: b }, 'right');
  assert.equal(c, b + 3);
  assert.equal(nav({ ...at, offset: c }, 'left'), b);

  const cell = nested(doc, 'table');
  const word = cell.info.text.trim().split(/\s+/)[0];
  const found = JSON.parse(doc.search(J({ text: word })));
  assert.ok(found.matches.some(m => J(m.from.steps) === J(cell.list.steps)), `"${word}" found in the cell`);
  const inCell = JSON.parse(doc.search(J({ text: word, list: cell.list })));
  assert.ok(inCell.matches.length >= 1 && inCell.matches.every(m => J(m.from.steps) === J(cell.list.steps)));
  const z = JSON.parse(doc.search(J({ text: 'z' })));
  assert.ok(z.matches.some(m => m.from.para === pi && m.from.offset === c));
  assert.equal(JSON.parse(doc.search(J({ text: 'z', caseSensitive: true }))).matches.length,
    JSON.parse(doc.search(J({ text: 'z', caseSensitive: true }))).matches.filter(m => m.from.offset !== c || m.from.para !== pi).length);
  // Next from a match finds a later one, wrapping to the first.
  const all = JSON.parse(doc.search(J({ text: word }))).matches;
  if (all.length > 1) {
    const next = JSON.parse(doc.search(J({ text: word, from: all[0].from }))).matches[0];
    assert.deepEqual(next, all[1]);
    const wrap = JSON.parse(doc.search(J({ text: word, from: all.at(-1).from }))).matches[0];
    assert.deepEqual(wrap, all[0]);
    const back = JSON.parse(doc.search(J({ text: word, from: all[1].from, backward: true }))).matches[0];
    assert.deepEqual(back, all[0]);
  }
});

test('lists floating objects and picks them by point', t => {
  const o = open('143E433F503322BD33.hwp');
  if (!o) return t.skip('corpus not set');
  const { doc } = o;
  const objects = JSON.parse(doc.objects(0));
  assert.ok(objects.length > 0);
  const top = objects.at(-1);
  const hit = JSON.parse(doc.locate(J({ kind: 'point', page: 0, x: top.x + top.width / 2, y: top.y + top.height / 2 })));
  if (top.layer !== 'behind') assert.deepEqual(hit.object, top);
  const info = paragraphs(doc, { section: 0 })[top.para];
  assert.ok(info.controls.some(c => c.index === top.control && c.kind === top.kind));
  // Far from every object, a point gives text only.
  const away = JSON.parse(doc.locate(J({ kind: 'point', page: 0, x: 1, y: 1 })));
  assert.ok(!away?.object);
});

test('composes IME text outside the history, then commits it as one step', t => {
  const o = open(HWPX);
  if (!o) return t.skip('corpus not set');
  const { doc } = o;
  const list = { section: 0 };
  const pi = paragraphs(doc, list).findIndex(p => p.text.length > 10);
  const before = text(doc, list, pi);
  const at = { section: 0, para: pi, offset: paragraphs(doc, list)[pi].caret[0] };
  let r = JSON.parse(doc.compose(J({ at, text: 'ㅎ' })));
  assert.equal(r.caret.offset, at.offset + 1);
  doc.compose(J({ text: '하' }));
  r = JSON.parse(doc.compose(J({ text: '한' })));
  assert.equal(text(doc, list, pi), '한' + before);
  assert.equal(r.undo, 0);
  assert.equal(code(() => doc.transact(J({ ops: [{ op: 'insertText', at, text: 'x' }] }))), 'COMPOSING');
  r = JSON.parse(doc.commitComposition('{}'));
  assert.equal(r.undo, 1);
  assert.equal(text(doc, list, pi), '한' + before);
  // A cancelled composition leaves nothing behind.
  doc.compose(J({ at: { ...at, offset: at.offset + 1 }, text: '글' }));
  JSON.parse(doc.cancelComposition());
  assert.equal(text(doc, list, pi), '한' + before);
  assert.equal(doc.cancelComposition(), 'null');
  JSON.parse(doc.undo());
  assert.equal(text(doc, list, pi), before);
});

test('copies and pastes with formatting within and across documents', t => {
  const a = open(HWP);
  const b = open(HWPX);
  if (!a || !b) return t.skip('corpus not set');
  const list = { section: 0 };
  const pa = paragraphs(a.doc, list).findIndex(p => p.text.length > 20);
  const src = { from: { section: 0, para: pa, offset: paragraphs(a.doc, list)[pa].caret[0] } };
  src.to = { ...src.from, offset: src.from.offset + 12 };
  const copied = JSON.parse(a.doc.copy(J(src)));
  assert.equal(copied.text.length, 12);
  assert.match(copied.html, /^<p><span style="font-size:/);

  // Within the document: the pasted text copies back the same.
  const pz = paragraphs(a.doc, list).findIndex((p, i) => i > pa && p.text.length > 5);
  const at = { section: 0, para: pz, offset: paragraphs(a.doc, list)[pz].caret[0] };
  const r = JSON.parse(a.doc.paste(J({ at, text: copied.text })));
  assert.equal(r.caret.offset, at.offset + 12);
  assert.deepEqual(JSON.parse(a.doc.copy(J({ from: at, to: r.caret }))), copied);

  // Into another document of the same engine: faces come along by name.
  a.doc.copy(J(src));
  const pb = paragraphs(b.doc, list).findIndex(p => p.text.length > 5);
  const atB = { section: 0, para: pb, offset: paragraphs(b.doc, list)[pb].caret[0] };
  // Documents of one engine share its clipboard: reopen both on one.
  const e = own(engine());
  const da = own(e.open(a.bytes));
  const db = own(e.open(b.bytes));
  da.copy(J(src));
  const rb = JSON.parse(db.paste(J({ at: atB })));
  assert.deepEqual(JSON.parse(db.copy(J({ from: atB, to: rb.caret }))), copied);
  const again = own(e.open(db.serialize(false)));
  assert.deepEqual(JSON.parse(again.copy(J({ from: atB, to: rb.caret }))), copied);

  // Text that is not the fragment's pastes as typed.
  const plain = JSON.parse(db.paste(J({ at: atB, text: 'plain\ttext' })));
  assert.equal(plain.caret.offset, atB.offset + 5 + 8 + 4);
  assert.equal(text(db, list, pb).slice(0, 10), 'plain\ttext');
});

test('restores checkpoints with their history', t => {
  const o = open(HWP);
  if (!o) return t.skip('corpus not set');
  const { doc } = o;
  const list = { section: 0 };
  const pi = paragraphs(doc, list).findIndex(p => p.text.length > 10);
  const at = { section: 0, para: pi, offset: paragraphs(doc, list)[pi].caret[0] };
  const before = text(doc, list, pi);
  doc.transact(J({ ops: [{ op: 'insertText', at, text: 'A' }] }));
  const cp = doc.checkpoint();
  doc.transact(J({ ops: [{ op: 'insertText', at, text: 'B' }, { op: 'split', at: { ...at, offset: at.offset + 1 } }] }));
  assert.equal(text(doc, list, pi), 'B');
  const r = JSON.parse(doc.restore(cp));
  assert.equal(r.undo, 1);
  assert.equal(text(doc, list, pi), 'A' + before);
  JSON.parse(doc.undo());
  assert.equal(text(doc, list, pi), before);
  assert.equal(doc.releaseCheckpoint(cp), true);
  assert.equal(code(() => doc.restore(cp)), 'STALE_HANDLE');
});

test('creates a document from a template', t => {
  const bytes = sample('footnote-tbox-01.hwp');
  if (!bytes) return t.skip('corpus not set');
  const e = own(engine());
  for (const tpl of [bytes, sample('hwpx/footnote-tbox-01.hwpx')]) {
    const doc = own(e.create(tpl));
    const list = { section: 0 };
    const paras = paragraphs(doc, list);
    assert.equal(paras.length, 1);
    assert.equal(paras[0].text, '');
    assert.equal(JSON.parse(doc.state()).pageCount, 1);
    doc.transact(J({ ops: [{ op: 'insertText', at: { section: 0, para: 0, offset: paras[0].caret[0] }, text: '새 문서' }] }));
    const again = own(e.open(doc.serialize(false)));
    assert.deepEqual(paragraphs(again, list).map(p => p.text), ['새 문서']);
  }
});

test('keeps compact fonts and fills in outlines typed text needs', t => {
  const o = open(HWP);
  if (!o) return t.skip('corpus not set');
  const { e, doc } = o;
  const read = registerFonts(e, doc, new Set());
  assert.ok(e.fontBytes < read / 4, `${e.fontBytes} of ${read} bytes kept`);
  // The font requests asked for everything the layout used.
  const req = JSON.parse(doc.fontRequests());
  assert.ok(req.faces.length > 0);
  assert.deepEqual(JSON.parse(e.outlineRequests()), []);

  // Characters the document never had come without outlines first.
  const list = { section: 0 };
  const pi = paragraphs(doc, list).findIndex(p => p.text.length > 10);
  const at = { section: 0, para: pi, offset: paragraphs(doc, list)[pi].caret[0] };
  doc.transact(J({ ops: [{ op: 'insertText', at, text: '鬱鬱' }] }));
  const page = JSON.parse(doc.locate(J({ kind: 'caret', position: at }))).page;
  const glyphs = () => decodePacket(doc.page(page)).glyphs;
  const generation = e.fontGeneration;
  const missing = [...decodeOutlines(e.glyphOutlines(glyphs())).values()].filter(x => x.em === 0).length;
  assert.ok(missing > 0);
  const wanted = JSON.parse(e.outlineRequests());
  assert.ok(wanted.length > 0);
  for (const path of wanted) e.addFontFile(path, fontFile(readFileSync(path)));
  assert.ok(e.fontGeneration > generation);
  assert.equal(state(doc).fontGeneration, e.fontGeneration);
  const after = [...decodeOutlines(e.glyphOutlines(glyphs())).values()].filter(x => x.em === 0).length;
  assert.equal(after, 0);
  assert.deepEqual(JSON.parse(e.outlineRequests()), []);
});

test('unregisters fonts and lays out without them', t => {
  const o = open(HWP);
  if (!o) return t.skip('corpus not set');
  const { e, doc } = o;
  const loaded = new Set();
  registerFonts(e, doc, loaded);
  const before = e.fontBytes;
  const epoch = state(doc).layoutEpoch;
  const removed = e.unregisterFonts(J([...loaded]));
  assert.ok(removed > 0);
  assert.ok(e.fontBytes < before / 10);
  assert.ok(state(doc).layoutEpoch > epoch);
  assert.equal(code(() => e.unregisterFonts('not json')), 'BAD_REQUEST');
});

test('hashes with BLAKE3 in pieces', () => {
  const h = own(new hm.Blake3Hasher());
  assert.equal(h.finalizeHex(), 'af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262');
  const bytes = new TextEncoder().encode('abc'.repeat(1000));
  const whole = own(new hm.Blake3Hasher());
  whole.update(bytes);
  const parts = own(new hm.Blake3Hasher());
  parts.update(bytes.subarray(0, 1001));
  parts.update(bytes.subarray(1001));
  assert.equal(parts.finalizeHex(), whole.finalizeHex());
});

test('hands pictures to the host to decode', t => {
  const o = open('2025 행정업무운영 편람(최종).hwpx');
  if (!o) return t.skip('corpus not set');
  const { doc } = o;
  const p = decodePacket(doc.page(0));
  const op = p.ops.find(x => x.op === 'patch');
  const src = JSON.parse(doc.pictureSource(0, op.item));
  assert.equal(src.mime, 'image/jpeg');
  const bytes = doc.pictureBytes(0, op.item);
  assert.deepEqual([...bytes.subarray(0, 2)], [0xff, 0xd8]);
  // The whole picture fills the patch's box.
  assert.equal(src.clip, null);
  assert.deepEqual([src.matrix[0], src.matrix[2], src.matrix[4], src.matrix[5]], [op.w, op.x, op.h, op.y]);
  // A page's text items are not pictures.
  assert.equal(code(() => doc.pictureBytes(0, p.ops.length + 5)), 'BAD_REQUEST');
});

/** Presses Enter `n` times at `at`; checks every new line advances and the caret sits on its line. */
function enters(doc, at, n) {
  const lineOf = (pos, caret) =>
    decodePacket(doc.page(caret.page)).lines.find(
      l => l.para === pos.para && J(l.list.steps) === J(pos.steps ?? []) && l.list.section === pos.section,
    );
  let prev = null;
  for (let k = 0; k < n; k++) {
    at = JSON.parse(doc.transact(J({ ops: [{ op: 'split', at }] }))).caret;
    const caret = JSON.parse(doc.locate(J({ kind: 'caret', position: at })));
    const line = lineOf(at, caret);
    assert.ok(line, `Enter ${k + 1}: the new paragraph's line is on the caret's page`);
    assert.equal(caret.y, line.y, `Enter ${k + 1}: caret and line agree`);
    if (prev && prev.page === caret.page) assert.ok(caret.y > prev.y, `Enter ${k + 1}: the line advances`);
    prev = caret;
  }
  return at;
}

test('repeated Enter at the end of the body adds lines and pages', t => {
  const o = open(HWPX);
  if (!o) return t.skip('corpus not set');
  const { doc } = o;
  const section = state(doc).pageCount > 0 ? 1 : 0;
  const paras = paragraphs(doc, { section });
  const last = paras.length - 1;
  const before = state(doc).pageCount;
  enters(doc, { section, para: last, offset: paras[last].caret.at(-1) }, 100);
  assert.ok(state(doc).pageCount > before, 'a hundred empty lines need more pages');
});

test('repeated Enter in a table cell grows the cell', t => {
  const o = open('multi-table-001.hwp');
  if (!o) return t.skip('corpus not set');
  const { doc } = o;
  const n = nested(doc, 'table');
  const before = state(doc).pageCount;
  enters(doc, { ...n.list, para: n.para, offset: n.info.caret.at(-1) }, 80);
  assert.ok(state(doc).pageCount > before, 'eighty empty lines push the table on');
});

test('reports the pages an edit changed', t => {
  const o = open(HWP);
  if (!o) return t.skip('corpus not set');
  const { doc } = o;
  const pages = state(doc).pageCount;
  // A paragraph in the middle of the document: typing into it repaints its page.
  const list = { section: 0 };
  const paras = paragraphs(doc, list);
  const pi = paras.findIndex((p, i) => i > paras.length / 2 && p.text.length > 10);
  const at = { section: 0, para: pi, offset: paras[pi].caret[0] };
  const page = JSON.parse(doc.locate(J({ kind: 'caret', position: at }))).page;
  const r = JSON.parse(doc.transact(J({ ops: [{ op: 'insertText', at, text: '가' }] })));
  assert.ok(r.changedPages.includes(page));
  assert.ok(r.changedPages.length < pages, `${r.changedPages.length} of ${pages} pages`);
  const u = JSON.parse(doc.undo());
  assert.ok(u.changedPages.includes(page));
  // Edits the engine cannot place report every page.
  const cp = doc.checkpoint();
  assert.equal(JSON.parse(doc.restore(cp)).changedPages, null);
});

test('keeps selections and the saved position with the history', t => {
  const o = open(HWP);
  if (!o) return t.skip('corpus not set');
  const { doc } = o;
  const list = { section: 0 };
  const pi = paragraphs(doc, list).findIndex(p => p.text.length > 10);
  const at = { section: 0, para: pi, offset: paragraphs(doc, list)[pi].caret[0] };
  let end = at;
  for (let k = 0; k < 3; k++) end = JSON.parse(doc.navigate(J({ position: end, motion: 'right' }))).position;
  const selection = {
    anchor: { ...at, steps: [] },
    focus: { ...end, steps: [] },
    anchorAffinity: 'downstream',
    focusAffinity: 'upstream',
  };
  assert.equal(state(doc).dirty, false);
  const r = JSON.parse(doc.transact(J({ ops: [{ op: 'delete', from: at, to: end }], selection })));
  assert.deepEqual(r.selection.anchor, r.caret);
  assert.equal(state(doc).dirty, true);
  const u = JSON.parse(doc.undo());
  assert.deepEqual(u.selection, selection);
  assert.equal(u.historyId, 0);
  // Back at the saved position: clean again.
  assert.equal(state(doc).dirty, false);
  const re = JSON.parse(doc.redo());
  assert.equal(re.historyId, r.historyId);
  assert.deepEqual(re.selection, r.selection);
  // Saving acknowledges an epoch; a stale one is refused.
  const epoch = state(doc).contentEpoch;
  assert.equal(code(() => doc.markSaved(epoch - 1)), 'STALE_EPOCH');
  doc.markSaved(epoch);
  assert.equal(state(doc).dirty, false);
  JSON.parse(doc.undo());
  assert.equal(state(doc).dirty, true);
});

/** The first body table of a document: its paragraph, control, and cell lists with text. */
function bodyTable(doc) {
  const paras = paragraphs(doc, { section: 0 });
  for (const p of paras) {
    const c = p.controls.find(x => x.kind === 'table' && x.lists >= 2);
    if (!c) continue;
    const cell = index => ({ section: 0, steps: [{ para: p.index, control: c.index, index }] });
    const cells = [...Array(c.lists).keys()].filter(i => paragraphs(doc, cell(i)).some(q => q.text.length > 0));
    if (cells.length >= 2 && p.index > 0) return { para: p.index, control: c.index, cell, cells, lists: c.lists };
  }
  return null;
}

test('selects from the body into a table as the whole table', t => {
  const o = open('multi-table-001.hwp');
  if (!o) return t.skip('corpus not set');
  const { doc } = o;
  const tb = bodyTable(doc);
  const before = paragraphs(doc, { section: 0 })[tb.para - 1];
  const from = { section: 0, para: tb.para - 1, offset: before.caret[0] };
  const inCell = { ...tb.cell(tb.cells[0]), para: 0, offset: 0 };
  const sel = JSON.parse(doc.selectRange(J({ from, to: inCell })));
  assert.equal(sel.range.kind, 'text');
  assert.deepEqual(sel.range.from, from);
  assert.equal(sel.range.to.para, tb.para);
  assert.ok(sel.range.to.offset >= 8, 'the range ends after the table control');
  // Its rectangles cover the table.
  const target = JSON.parse(doc.objects(JSON.parse(doc.locate(J({ kind: 'caret', position: from }))).page))
    .find(x => x.kind === 'table' && x.para === tb.para);
  if (target) assert.ok(sel.rects.some(r => r.width === target.width && r.y === target.y));
  // The same ends given the other way round select the same.
  assert.deepEqual(JSON.parse(doc.selectRange(J({ from: inCell, to: from }))).range, sel.range);
  // Deleting the range takes the table out; undo puts it back.
  const count = () => paragraphs(doc, { section: 0 }).flatMap(p => p.controls).filter(c => c.kind === 'table').length;
  const n = count();
  doc.transact(J({ ops: [{ op: 'delete', from: inCell, to: from }] }));
  assert.equal(count(), n - 1);
  JSON.parse(doc.undo());
  assert.equal(count(), n);
});

test('selects, copies and clears a block of cells', t => {
  const o = open('multi-table-001.hwp');
  if (!o) return t.skip('corpus not set');
  const { doc } = o;
  const tb = bodyTable(doc);
  const [a, b] = [tb.cells[0], tb.cells.at(-1)];
  const sel = JSON.parse(doc.selectRange(J({
    from: { ...tb.cell(a), para: 0, offset: 0 },
    to: { ...tb.cell(b), para: 0, offset: 0 },
  })));
  assert.equal(sel.range.kind, 'cells');
  assert.ok(sel.range.cells.length >= 2);
  assert.ok(sel.rects.length >= sel.range.cells.length, 'every selected cell shows');
  // The tagged request selects the same block.
  const tagged = JSON.parse(doc.selectRange(J({ kind: 'cells', table: sel.range.table, from: sel.range.from, to: sel.range.to })));
  assert.deepEqual(tagged.range, sel.range);
  const copied = JSON.parse(doc.copy(J({ from: { ...tb.cell(a), para: 0, offset: 0 }, to: { ...tb.cell(b), para: 0, offset: 0 } })));
  assert.match(copied.html, /^<table><tr><td>/);
  const texts = () => sel.range.cells.map(c => paragraphs(doc, c.list).map(p => p.text).join('\n'));
  const was = texts();
  doc.transact(J({ ops: [{ op: 'delete', from: { ...tb.cell(a), para: 0, offset: 0 }, to: { ...tb.cell(b), para: 0, offset: 0 } }] }));
  assert.ok(texts().every(x => x === ''), 'the cells are empty');
  JSON.parse(doc.undo());
  assert.deepEqual(texts(), was);
test('saves in the other format', t => {
  const o = open(HWP);
  if (!o) return t.skip('corpus not set');
  const { e, doc } = o;
  const before = state(doc);
  const hwpx = own(e.open(doc.serializeAs('hwpx', false)));
  assert.equal(state(hwpx).format, 'hwpx');
  assert.equal(state(hwpx).pageCount, before.pageCount);
  const back = own(e.open(hwpx.serializeAs('hwp5', false)));
  assert.equal(state(back).format, 'hwp5');
  assert.equal(state(back).pageCount, before.pageCount);
  assert.deepEqual(paragraphs(back, { section: 0 }).map(p => p.text), paragraphs(doc, { section: 0 }).map(p => p.text));
  assert.equal(code(() => doc.serializeAs('pdf', false)), 'UNSUPPORTED_FORMAT');
});
