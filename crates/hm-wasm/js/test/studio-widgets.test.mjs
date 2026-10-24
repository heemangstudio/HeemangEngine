// Queries and patches behind Studio's ruler, status bar and font menus:
// character counts, run formats, the right margin and per-slot faces.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { hm, fixture, fontCatalog, fontFile } from './env.mjs';

const J = JSON.stringify;
const query = (doc, request) => JSON.parse(doc.query(J(request)));
const paragraphs = (doc, list) => query(doc, { kind: 'paragraphs', list });
const count = (doc, request = {}) => query(doc, { kind: 'count', ...request }).characters;

function withDoc(name, run) {
  const engine = new hm.Engine();
  const doc = engine.open(fixture(name));
  try {
    run(doc, engine);
  } finally {
    doc.free();
    engine.free();
  }
}

test('counts body, nested lists and ranges like the status bar', () => {
  withDoc('query-equations.hwpx', doc => {
    const body = { section: 0 };
    const [p] = paragraphs(doc, body);
    // "body😀" counts five, the equation nothing, the table its cells.
    const table = p.controls.find(c => c.kind === 'table');
    const cells = Array.from({ length: table.lists }, (_, index) =>
      count(doc, { list: { section: 0, steps: [{ para: 0, control: table.index, index }] } }));
    assert.equal(count(doc), 5 + cells.reduce((a, b) => a + b, 0));
    const at = { section: 0, para: 0, offset: p.caret[0] };
    // Four characters, then the surrogate pair as one.
    assert.equal(count(doc, { from: at, to: { ...at, offset: p.offsets[4] } }), 4);
    assert.equal(count(doc, { from: at, to: { ...at, offset: p.offsets[5] + 1 } }), 5);
    assert.equal(count(doc, { from: at, to: at }), 0);
    assert.throws(() => doc.query(J({ kind: 'count', from: at })), err => err.code === 'BAD_REQUEST');
  });
});

for (const name of ['query-equations.hwpx', 'query-equations.hwp']) test(`reports run formats and sets the right margin: ${name}`, () => {
  withDoc(name, doc => {
    const list = { section: 0 };
    const formats = query(doc, { kind: 'formatRuns', list });
    assert.equal(formats.length, paragraphs(doc, list).length);
    assert.deepEqual(formats.map(f => f.index), formats.map((_, i) => i));
    for (const f of formats) {
      assert.ok(f.runs.length >= 1);
      assert.equal(f.runs[0].characters.fontFaces.length, 7);
    }
    const p = paragraphs(doc, list)[0];
    const at = { section: 0, para: 0, offset: p.caret[0] };
    doc.transact(J({ ops: [{ op: 'formatParas', from: at, to: at, patch: { right: 1800, left: 600 } }] }));
    const context = query(doc, { kind: 'formatContext', at });
    assert.equal(context.paragraphs.right, 1800);
    assert.equal(context.paragraphs.left, 600);
    assert.equal(query(doc, { kind: 'formatRuns', list, from: 0, count: 1 })[0].paragraphs.right, 1800);
    const again = doc.serialize(false);
    const engine = new hm.Engine();
    const reopened = engine.open(again);
    try {
      assert.equal(query(reopened, { kind: 'formatContext', at }).paragraphs.right, 1800);
    } finally {
      reopened.free();
      engine.free();
    }
    doc.undo();
    assert.equal(query(doc, { kind: 'formatContext', at }).paragraphs.right, 0);
  });
});

test('sets one script slot of the face and keeps the others', t => {
  const font = fontCatalog().fonts.find(f => /\.(ttf|otf)$/i.test(f.path) && f.families.length);
  if (!font) return t.skip('HEEMANG_FONTS not set');
  withDoc('query-equations.hwpx', (doc, engine) => {
    engine.addFontFile(font.path, fontFile(readFileSync(font.path)));
    const family = font.families[0];
    const p = paragraphs(doc, { section: 0 })[0];
    const from = { section: 0, para: 0, offset: p.caret[0] };
    const to = { ...from, offset: p.offsets[4] };
    const before = query(doc, { kind: 'formatContext', at: from, to }).characters.fontFaces;
    doc.transact(J({ ops: [{ op: 'formatChars', from, to, patch: { fontFaces: [null, family] } }] }));
    const after = query(doc, { kind: 'formatContext', at: from, to }).characters.fontFaces;
    assert.deepEqual(after, [before[0], family, ...before.slice(2)]);
    assert.throws(() => doc.transact(J({ ops: [{ op: 'formatChars', from, to, patch: { fontFaces: [null, 'No Such Family'] } }] })),
      err => err.code === 'BAD_REQUEST');
    assert.throws(() => doc.transact(J({ ops: [{ op: 'formatChars', from, to, patch: { fontFaces: Array(8).fill(null) } }] })),
      err => err.code === 'BAD_REQUEST');
  });
});
