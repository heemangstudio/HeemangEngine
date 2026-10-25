import test from 'node:test';
import assert from 'node:assert/strict';
import { hm, fixture } from './env.mjs';
import { decodePacket } from '../player.mjs';

const state = d => JSON.parse(d.state());
const text = d => JSON.parse(d.query(JSON.stringify({ kind: 'paragraphs', list: { section: 0 } }))).map(p => p.text);
const lines = d => decodePacket(d.page(0)).lines.map(l => [l.list, l.para, l.line, l.y, l.stops[0]?.[0], l.stops.at(-1)?.[0]]);

test('serializeAs writes selected membership and leaves the open document and history intact', () => {
  const e = new hm.Engine(), bytes = fixture('conversion-membership.hwpx'), doc = e.open(bytes);
  const before = state(doc), original = text(doc), initialLines = lines(doc);
  const converted = doc.serializeAs('hwp5', false), back = e.open(converted);
  assert.deepEqual(state(doc), before);
  assert.deepEqual(Buffer.from(doc.serialize(false)), bytes);
  assert.equal(state(back).format, 'hwp5');
  assert.deepEqual(lines(back), initialLines);
  assert.deepEqual(text(back), original);
  doc.transact(JSON.stringify({ ops: [{ op: 'insertText', at: { section: 0, para: 1, offset: 0 }, text: 'saved ' }] }));
  const edited = state(doc), editedText = text(doc);
  assert.equal(edited.contentEpoch, before.contentEpoch + 1);
  const editedBack = e.open(doc.serializeAs('hwp5', true));
  assert.deepEqual(state(doc), { ...edited, dirty: false });
  assert.deepEqual(text(editedBack), editedText);
  doc.undo();
  assert.deepEqual(text(doc), original);
  assert.deepEqual(Buffer.from(doc.serialize(false)), bytes);
  doc.redo();
  assert.deepEqual(text(doc), editedText);
  const sameBack = e.open(doc.serialize(true));
  assert.equal(state(doc).dirty, false);
  assert.deepEqual(text(sameBack), editedText);
  assert.throws(() => doc.markSaved(state(doc).contentEpoch - 1), e => e.code === 'STALE_EPOCH');
  assert.throws(() => doc.serializeAs('pdf', false), e => e.code === 'UNSUPPORTED_FORMAT');
  for (const d of [sameBack, editedBack, back, doc]) d.free();
  e.free();
});

test('serializeAs refuses linked text-box output without acknowledging a save', () => {
  const e = new hm.Engine(), bytes = fixture('conversion-linked.hwpx'), doc = e.open(bytes);
  const before = state(doc);
  assert.throws(() => doc.serializeAs('hwp5', true), e => e.code === 'SAVE_FAILED' && e.message.includes('linked text-box IDs'));
  assert.deepEqual(state(doc), before);
  assert.deepEqual(Buffer.from(doc.serialize(false)), bytes);
  doc.free();
  e.free();
});
