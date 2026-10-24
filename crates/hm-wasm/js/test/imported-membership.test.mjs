import test from 'node:test';
import assert from 'node:assert/strict';
import { hm, fixture } from './env.mjs';
import { decodePacket } from '../player.mjs';

const lines = doc => decodePacket(doc.page(0)).lines.filter(l => l.para === 1).map(l => ({ line: l.line, stops: l.stops }));
const text = doc => JSON.parse(doc.query(JSON.stringify({ kind: 'paragraphs', list: { section: 0 } }))).map(p => p.text);

for (const name of ['membership-duplicate-0-8.hwpx', 'membership-duplicate-8-0.hwpx']) {
  test(`duplicate flags retain imported lines through dirty-header save and history: ${name}`, () => {
    const e = new hm.Engine(), bytes = fixture(name), doc = e.open(bytes);
    const initialLines = lines(doc), initialText = text(doc);
    assert.equal(initialLines.length, 2);
    assert.deepEqual(Buffer.from(doc.serialize(false)), bytes);
    const at = { section: 0, para: 0, offset: 0 };
    doc.transact(JSON.stringify({ ops: [{ op: 'formatParas', from: at, to: at, patch: { spaceBefore: 17 } }] }));
    assert.deepEqual(lines(doc), initialLines);
    const saved = doc.serialize(false), back = e.open(saved);
    assert.deepEqual(text(back), initialText);
    assert.deepEqual(lines(back), initialLines);
    doc.undo();
    assert.deepEqual(text(doc), initialText);
    assert.deepEqual(lines(doc), initialLines);
    doc.redo();
    assert.deepEqual(Buffer.from(doc.serialize(false)), Buffer.from(saved));
    const converted = e.open(doc.serializeAs('hwp5', false));
    assert.deepEqual(text(converted), initialText);
    assert.deepEqual(lines(converted), initialLines);
    for (const d of [converted, back, doc]) d.free();
    e.free();
  });
}
