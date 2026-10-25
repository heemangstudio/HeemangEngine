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

for (const name of ['query-equations.hwpx', 'query-equations.hwp']) test(`definition edits are atomic, undoable and survive save: ${name}`, () => {
  withDoc(name, (doc, engine) => {
    const list = { section: 0 };
    const p = paragraphs(doc, list)[0];
    const from = { section: 0, para: 0, offset: p.caret[0] };
    const to = { ...from, offset: p.offsets[4] };
    const formats = query(doc, { kind: 'formatRuns', list });
    const shape = formats[0].runs[0].charShapeId;
    const paraShape = formats[0].paraShapeId;
    assert.equal(query(doc, { kind: 'fonts' }).length, 7);
    assert.deepEqual(query(doc, { kind: 'charShape', id: shape }), formats[0].runs[0].characters);
    assert.equal(query(doc, { kind: 'paraShape', id: 999999 }), null);
    assert.equal(query(doc, { kind: 'charShape', id: 999999 }), null);
    const before = doc.serialize(false);
    const state = JSON.parse(doc.state());
    const create = { op: 'createStyle', style: { name: 'Widget style', characters: { bold: true, ratios: [80], spacings: [3, -2] }, paragraphs: { right: 1200, lineSpacing: 1800, lineSpacingKind: 'fixed' } } };
    const numbering = { kind: 'numbering', start: 3, levels: [{ format: '^1)', numberKind: 0, start: 4 }] };
    assert.throws(() => doc.transact(J({ ops: [create, { op: 'ensureList', definition: numbering }, { op: 'formatChars', from, to, patch: { italic: true } }, { op: 'setParaShape', from, to, paraShapeId: 65535 }] })), e => e.code === 'BAD_REQUEST');
    assert.deepEqual(doc.serialize(false), before);
    assert.deepEqual(JSON.parse(doc.state()), state);
    const receipt = JSON.parse(doc.transact(J({ ops: [create, { op: 'ensureList', definition: numbering }, { op: 'formatChars', from, to, patch: { size: 1300 } }] })));
    assert.equal(receipt.undo, state.undo + 1);
    let style = query(doc, { kind: 'styles' }).at(-1);
    assert.equal(style.name, 'Widget style');
    assert.equal(style.characters.bold, true);
    assert.equal(style.characters.ratios[0], 80);
    assert.deepEqual(style.characters.spacings.slice(0, 2), [3, -2]);
    assert.equal(style.paragraphs.right, 1200);
    assert.equal(style.paragraphs.lineSpacingKind, 'fixed');
    assert.equal(style.paragraphs.lineSpacing, 1800);
    const definitions = query(doc, { kind: 'lists' });
    const n = definitions.filter(l => l.kind === 'numbering').at(-1);
    assert.equal(n.levels[0].format, '^1)');
    assert.equal(n.levels[0].start, 4);
    const ensured = JSON.parse(doc.transact(J({ ops: [{ op: 'ensureList', definition: numbering }] })));
    assert.equal(ensured.contentEpoch, receipt.contentEpoch);
    assert.equal(ensured.undo, receipt.undo);
    const afterCreate = doc.serialize(false);
    doc.undo();
    assert.deepEqual(doc.serialize(false), before);
    doc.redo();
    assert.deepEqual(doc.serialize(false), afterCreate);
    assert.equal(query(doc, { kind: 'formatContext', at: from, to }).characters.size, 1300);
    assert.deepEqual(query(doc, { kind: 'lists' }), definitions);
    doc.transact(J({ ops: [{ op: 'updateStyle', id: style.id, style: { characters: { italic: true }, paragraphs: { left: 300 } } }, { op: 'applyStyle', from, to, styleId: style.id }, { op: 'setList', from, to, list: { kind: 'numbering', id: n.id } }] }));
    style = query(doc, { kind: 'style', id: style.id });
    assert.equal(style.characters.bold, true);
    assert.equal(style.characters.italic, true);
    assert.equal(style.characters.ratios[0], 80);
    assert.equal(style.paragraphs.lineSpacingKind, 'fixed');
    assert.equal(style.paragraphs.right, 1200);
    assert.equal(query(doc, { kind: 'formatContext', at: from, to }).list.id, n.id);
    assert.throws(() => doc.transact(J({ ops: [{ op: 'deleteStyle', id: style.id }] })), e => e.code === 'BAD_REQUEST');
    const reopened = engine.open(doc.serialize(false));
    try {
      assert.deepEqual(query(reopened, { kind: 'style', id: style.id }), style);
      assert.deepEqual(query(reopened, { kind: 'lists' }), definitions);
      assert.equal(query(reopened, { kind: 'formatContext', at: from, to }).list.id, n.id);
    } finally { reopened.free(); }
    doc.transact(J({ ops: [{ op: 'setCharShape', from, to, charShapeId: shape }, { op: 'setParaShape', from, to, paraShapeId: paraShape }] }));
    assert.equal(query(doc, { kind: 'formatRuns', list })[0].runs[0].charShapeId, shape);
    assert.equal(query(doc, { kind: 'formatRuns', list })[0].paraShapeId, paraShape);
    assert.deepEqual(query(doc, { kind: 'formatContext', at: from, to }).characters, query(doc, { kind: 'charShape', id: shape }));
    doc.undo();
    assert.equal(query(doc, { kind: 'formatContext', at: from, to }).characters.italic, true);
  });
});

for (const name of ['query-equations.hwpx', 'query-equations.hwp']) test(`style deletion compacts references and bullet definitions deduplicate through history: ${name}`, () => {
  withDoc(name, (doc, engine) => {
    const from = { section: 0, para: 0, offset: paragraphs(doc, { section: 0 })[0].caret[0] };
    const create = name => ({ op: 'createStyle', style: { name } });
    if (!query(doc, { kind: 'styles' }).length) doc.transact(J({ ops: [create('Baseline')] }));
    doc.transact(J({ ops: [create('Unused'), create('Used')] }));
    const [unused, used] = query(doc, { kind: 'styles' }).slice(-2);
    doc.transact(J({ ops: [{ op: 'applyStyle', from, to: from, styleId: used.id }] }));
    doc.transact(J({ ops: [{ op: 'deleteStyle', id: unused.id }] }));
    const shifted = query(doc, { kind: 'style', id: unused.id });
    assert.equal(shifted.name, 'Used');
    assert.equal(shifted.next, unused.id);
    assert.equal(query(doc, { kind: 'formatContext', at: from }).styleId, unused.id);
    doc.undo();
    assert.equal(query(doc, { kind: 'formatContext', at: from }).styleId, used.id);
    doc.redo();
    const bullet = { op: 'ensureList', definition: { kind: 'bullet', bullet: '◆' } };
    const countBefore = query(doc, { kind: 'lists' }).length;
    doc.transact(J({ ops: [bullet] }));
    const b = query(doc, { kind: 'lists' }).at(-1);
    assert.equal(b.bullet, '◆');
    const state = JSON.parse(doc.state());
    doc.transact(J({ ops: [bullet] }));
    assert.deepEqual(JSON.parse(doc.state()), state);
    doc.undo();
    assert.equal(query(doc, { kind: 'lists' }).length, countBefore);
    doc.redo();
    doc.transact(J({ ops: [{ op: 'setList', from, to: from, list: { kind: 'bullet', id: b.id } }] }));
    const reopened = engine.open(doc.serialize(false));
    try {
      assert.equal(query(reopened, { kind: 'formatContext', at: from }).styleId, unused.id);
      assert.equal(query(reopened, { kind: 'formatContext', at: from }).list.kind, 'bullet');
      assert.equal(query(reopened, { kind: 'list', listKind: 'bullet', id: b.id }).bullet, '◆');
    } finally { reopened.free(); }
    assert.throws(() => doc.transact(J({ ops: [{ op: 'deleteStyle', id: 0 }] })), e => e.code === 'BAD_REQUEST');
    assert.throws(() => doc.transact(J({ ops: [{ op: 'ensureList', definition: { kind: 'bullet', bullet: '😀' } }] })), e => e.code === 'BAD_REQUEST');
    assert.throws(() => doc.transact(J({ ops: [{ op: 'setList', from, to: from, list: { kind: 'outline', id: 1 } }] })), e => e.code === 'BAD_REQUEST');
  });
});

for (const name of ['query-equations.hwpx', 'query-equations.hwp']) test(`character effects round-trip through history and save: ${name}`, () => {
  withDoc(name, (doc, engine) => {
    const p = paragraphs(doc, { section: 0 })[0];
    const from = { section: 0, para: 0, offset: p.caret[0] };
    const to = { ...from, offset: p.offsets[4] };
    const effects = {
      superscript: true, emboss: true, kerning: true, underlineKind: 'above', underlineShape: 2, underlineColor: 0x0000FF,
      strike: true, strikeShape: 3, strikeColor: 0x00FF00, outline: 1, shadow: 2, shadowColor: 0x808080, shadowOffset: [12, -7],
      emphasis: 2, shadeColor: 0x00FFFF, relSizes: [90, 110], offsets: [-5, 10],
    };
    const original = query(doc, { kind: 'formatContext', at: from, to }).characters;
    const state = JSON.parse(doc.state());
    for (const bad of [{ outline: 7 }, { shadow: 3 }, { underlineKind: 'middle' }, { shadowOffset: [101, 0] }, { relSizes: [9] }, { offsets: [-101] }, { emphasis: 13 }]) {
      assert.throws(() => doc.transact(J({ ops: [{ op: 'formatChars', from, to, patch: bad }] })), e => e.code === 'BAD_REQUEST', J(bad));
    }
    assert.deepEqual(JSON.parse(doc.state()), state);
    doc.transact(J({ ops: [{ op: 'formatChars', from, to, patch: effects }] }));
    // HWP 5 files older than 5.0.3 store no strikeout color.
    const check = (d, saved = false) => {
      const c = query(d, { kind: 'formatContext', at: from, to }).characters;
      for (const [k, v] of Object.entries(effects)) {
        if (saved && k === 'strikeColor' && name.endsWith('.hwp')) continue;
        if (k === 'relSizes' || k === 'offsets') assert.deepEqual(c[k].slice(0, 2), v, k);
        else assert.deepEqual(c[k], v, k);
      }
      assert.equal(c.subscript, false);
      assert.equal(c.engrave, false);
    };
    check(doc);
    // One flag of each exclusive pair clears the other.
    doc.transact(J({ ops: [{ op: 'formatChars', from, to, patch: { subscript: true, engrave: true } }] }));
    const flipped = query(doc, { kind: 'formatContext', at: from, to }).characters;
    assert.deepEqual([flipped.superscript, flipped.subscript, flipped.emboss, flipped.engrave], [false, true, false, true]);
    doc.undo();
    check(doc);
    const reopened = engine.open(doc.serialize(false));
    try { check(reopened, true); } finally { reopened.free(); }
    doc.undo();
    assert.deepEqual(query(doc, { kind: 'formatContext', at: from, to }).characters, original);
  });
});

test('font candidates name what a host registers and whether a substitute already draws the family', t => {
  withDoc('query-equations.hwpx', (doc, engine) => {
    const [batang, unknown] = query(doc, { kind: 'fontCandidates', names: ['바탕', 'No Such Family'] });
    assert.equal(batang.name, '바탕');
    assert.deepEqual(batang.candidates.slice(0, 3), ['바탕', '한컴바탕', '함초롬바탕']);
    assert.deepEqual(unknown.candidates, ['No Such Family']);
    const font = fontCatalog().fonts.find(f => /\.(ttf|otf)$/i.test(f.path) && f.families.some(n => /^(함초롬바탕|HCR Batang|한컴바탕|HanCom Batang)$/i.test(n)));
    if (!font) return t.skip('no Hancom Batang face in HEEMANG_FONTS');
    assert.equal(batang.resolves, false);
    engine.addFontFile(font.path, fontFile(readFileSync(font.path)));
    assert.equal(query(doc, { kind: 'fontCandidates', names: ['바탕'] })[0].resolves, true);
    const p = paragraphs(doc, { section: 0 })[0];
    const from = { section: 0, para: 0, offset: p.caret[0] };
    doc.transact(J({ ops: [{ op: 'formatChars', from, to: { ...from, offset: p.offsets[2] }, patch: { fontFace: '바탕' } }] }));
    assert.equal(query(doc, { kind: 'formatContext', at: from, to: { ...from, offset: p.offsets[2] } }).characters.fontFace, '바탕');
  });
});
