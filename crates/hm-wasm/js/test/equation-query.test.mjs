// Equation source through the actual generated JS/WASM boundary.
import test from 'node:test';
import assert from 'node:assert/strict';
import { decodePacket } from '../player.mjs';
import { hm, fixture, sample } from './env.mjs';

const J = JSON.stringify;
const paragraphs = (doc, list) => JSON.parse(doc.query(J({ kind: 'paragraphs', list })));

function equations(doc, list = { section: 0, steps: [] }, out = []) {
  for (const p of paragraphs(doc, list)) {
    for (const c of p.controls) {
      if (c.kind === 'equation') {
        assert.equal(typeof c.script, 'string');
        out.push({ list, para: p.index, ...c });
      } else {
        assert.equal(Object.hasOwn(c, 'script'), false);
      }
      for (let index = 0; index < c.lists; index++) {
        equations(doc, { section: list.section, steps: [...list.steps, { para: p.index, control: c.index, index }] }, out);
      }
    }
  }
  return out;
}

function check(bytes, verify) {
  const engine = new hm.Engine();
  const doc = engine.open(bytes);
  let again;
  try {
    const before = doc.state();
    const fonts = doc.fontRequests();
    const packet = doc.page(0);
    const glyphs = decodePacket(packet).glyphs;
    const outlines = engine.glyphOutlines(glyphs);
    const got = equations(doc);
    verify(got, doc);
    assert.throws(() => doc.query(J({
      kind: 'paragraphs', list: { section: 0, steps: [{ para: 0, control: 999999, index: 0 }] },
    })), err => err.code === 'BAD_ADDRESS');
    assert.equal(doc.state(), before);
    assert.equal(doc.fontRequests(), fonts);
    assert.deepEqual(doc.page(0), packet);
    assert.deepEqual(engine.glyphOutlines(glyphs), outlines);
    const saved = doc.serialize(false);
    assert.deepEqual(Buffer.from(saved), Buffer.from(bytes));
    again = engine.open(saved);
    assert.deepEqual(equations(again), got);
    assert.equal(doc.undo(), 'null');
    assert.equal(doc.redo(), 'null');
  } finally {
    again?.free();
    doc.free();
    engine.free();
  }
}

for (const format of ['hwp', 'hwpx']) {
  test(`authored equation strings and addresses survive JS/JSON: ${format}`, () => {
    assert.equal(JSON.parse(hm.capabilities()).equationScripts, true);
    check(fixture(`query-equations.${format}`), (got, doc) => {
      assert.deepEqual(got.map(c => c.script), ['"한글" `x` \\\n+ y😀', '', 'α + β']);
      assert.deepEqual(got.map(c => c.list.steps.length), [0, 1, 2]);
      assert.deepEqual(got.map(c => [c.index, c.offset, c.lists]), [[1, 14, 0], [0, 6, 0], [0, 5, 0]]);
      const body = paragraphs(doc, { section: 0, steps: [] });
      assert.equal(body[0].text, 'body😀');
      assert.deepEqual(body[0].offsets, [0, 1, 2, 3, 4, 5, 30]);
      assert.deepEqual(JSON.parse(doc.query(J({ kind: 'paragraphs', list: { section: 0 }, count: 1 }))), body);
    });
  });
}

for (const rel of ['math-001.hwp', 'hwpx/math-001.hwpx', 'task1725/text_footnote_tail_overpagination.hwp', 'task1725/text_footnote_tail_overpagination.hwpx']) {
  test(`original equation source is readable without an edit: ${rel}`, t => {
    const bytes = sample(rel);
    if (!bytes) return t.skip('HEEMANG_CORPUS is not set');
    check(bytes, got => {
      assert.ok(got.length > 0);
      if (rel.includes('math-001')) {
        assert.equal(got[0].script, ' sqrt {3} of {5}  times 25^{{1} over {3}}`');
      } else {
        assert.ok(got.some(c => c.list.steps.length > 0));
      }
    });
  });
}
