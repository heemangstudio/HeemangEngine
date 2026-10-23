// The engine through its JS boundary: open, state, pages, locate, select,
// navigate, transact, undo and serialize, on corpus documents
// (HEEMANG_CORPUS, fonts from HEEMANG_FONTS).

import test from 'node:test';
import assert from 'node:assert/strict';
import { decodePacket, decodeOutlines } from '../player.mjs';
import { hm, engine, registerFonts, sample } from './env.mjs';

const HWP = '2022년 국립국어원 업무계획.hwp';
const HWPX = 'hwpx/2024년 1분기 해외직접투자 보도자료 ff.hwpx';

const J = v => JSON.stringify(v);
const paragraphs = (doc, list) => JSON.parse(doc.query(J({ kind: 'paragraphs', list })));
const text = (doc, list, para) => paragraphs(doc, list)[para].text;
const state = doc => JSON.parse(doc.state());

function open(rel) {
  const bytes = sample(rel);
  if (!bytes) return null;
  const e = engine();
  const doc = e.open(bytes);
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
  const e = new hm.Engine();
  assert.equal(code(() => e.open(new Uint8Array([1, 2, 3]))), 'UNSUPPORTED_FORMAT');
  // An HWP 5 compound file signature with nothing behind it.
  const cfb = new Uint8Array(1024);
  cfb.set([0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1]);
  assert.ok(['MALFORMED_INPUT', 'UNSUPPORTED_FORMAT'].includes(code(() => e.open(cfb))));
  const caps = JSON.parse(hm.capabilities());
  assert.equal(caps.contract, 1);
  assert.deepEqual(caps.open, ['hwp5', 'hwpx']);
  const bytes = sample(HWPX);
  if (bytes) assert.ok(state(e.open(bytes)).pageCount > 0);
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
    const again = e.open(saved);
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
    const again = e.open(doc.serialize(false));
    assert.equal(text(again, n.list, n.para), '[편집]' + n.info.text);
    assert.equal(JSON.parse(again.state()).pageCount, JSON.parse(doc.state()).pageCount);
  });
}
