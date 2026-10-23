// The player decodes the version 1 fixture the Rust packet test pins.

import test from 'node:test';
import assert from 'node:assert/strict';
import { decodePacket } from '../player.mjs';
import { fixture } from './env.mjs';

test('decodes the version 1 packet fixture', () => {
  const p = decodePacket(fixture('packet-v1.bin'));
  assert.deepEqual(
    [p.version, p.page, p.contentEpoch, p.layoutEpoch, p.fontGeneration, p.width, p.height],
    [1, 2, 5, 7, 1, 59528, 84188],
  );
  assert.deepEqual(p.ops.map(o => o.op), [
    'clip', 'clip', 'glyphs', 'glyphs', 'unclip', 'rect', 'line', 'rect',
    'patch', 'path', 'patch', 'patch', 'outline', 'unclip',
  ]);
  const plain = p.ops[2];
  assert.deepEqual(
    [plain.face, plain.size, plain.ratio, plain.color, plain.flags, plain.baseline],
    [3, 1000, 95, 0x1f4e79, 0, 8500],
  );
  assert.deepEqual(plain.glyphs, [[36, 7200], [37, 7700], [1200, 8250]]);
  assert.equal(p.ops[3].flags, 3);
  assert.deepEqual(p.ops[6], { op: 'line', x1: 7200, y1: 9500, x2: 20000, y2: 9500, width: 30, color: 0xff, dash: 3 });
  assert.deepEqual(p.ops[8], { op: 'patch', item: 8, x: 1000, y: 2000, w: 3000, h: 4000 });
  const path = p.ops[9];
  assert.deepEqual([path.fill, path.width, path.stroke, path.dash, path.roundCap, path.alpha], [0xff00, 40, 0, 6, true, 128]);
  assert.deepEqual(path.els, [[0, 1000, 1000], [1, 5000, 1000], [2, 6000, 1500, 6000, 3500, 5000, 4000], [3]]);
  // The picture's frame maps the unit square to x 10000..17200, y 20000..23600.
  assert.deepEqual(p.ops[11], { op: 'patch', item: 11, x: 10000, y: 20000, w: 7200, h: 3600 });
  assert.deepEqual([p.ops[12].width, p.ops[12].dash], [20, [100, 50]]);
  assert.equal(p.lines.length, 2);
  assert.deepEqual(p.lines[1].list, { section: 0, steps: [{ para: 4, control: 1, index: 7 }] });
  assert.deepEqual(p.lines[0].stops, [[0, 7200], [1, 7700], [2, 8250], [3, 8800]]);
  assert.deepEqual([...p.glyphs], [3, 36, 3, 37, 3, 1200]);
});

test('rejects truncated and foreign packets', () => {
  const bytes = fixture('packet-v1.bin');
  assert.throws(() => decodePacket(bytes.subarray(0, 40)), /overruns|truncated/);
  const other = Uint8Array.from(bytes);
  other[4] = 9;
  assert.throws(() => decodePacket(other), /version 9/);
  assert.throws(() => decodePacket(new Uint8Array(64)), /not a page packet/);
});
