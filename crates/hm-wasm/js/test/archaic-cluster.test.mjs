import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { engine, fixture, fontCatalog, fontFile } from './env.mjs';
import { decodePacket, decodeOutlines } from '../player.mjs';

const J = JSON.stringify;
const original = '\u1112\u119e\u11abZ';
const paragraphs = doc => JSON.parse(doc.query(J({ kind: 'paragraphs', list: { section: 0 } })));
const at = offset => ({ section: 0, para: 1, offset });

test('shapes an unseen archaic composite while preserving wire edits and late outline requests', t => {
  const font = fontCatalog().fonts.find(f => f.families.some(n => n.replace(/\s/g, '').toLowerCase() === 'haansoftdotum'));
  if (!font) return t.skip('native Haansoft Dotum font is not configured');
  const e = engine();
  const bytes = readFileSync(font.path);
  // Only the engine's Latin seed is present when this sparse font is loaded.
  e.addFontFile(font.path, fontFile(bytes));
  const doc = e.create(fixture('query-equations.hwpx'));
  let reopened;
  try {
    assert.ok(!e.fontCharacters().includes('\uf53a'));
    const lead = paragraphs(doc)[0].caret[0];
    doc.transact(J({ ops: [{ op: 'insertText', at: { section: 0, para: 0, offset: lead }, text: '\n' }] }));
    assert.equal(paragraphs(doc)[1].caret[0], 0);
    doc.transact(J({ ops: [{ op: 'insertText', at: at(0), text: original }] }));
    doc.transact(J({ ops: [{ op: 'formatChars', from: at(0), to: at(4), patch: {
      fontFace: 'Haansoft Dotum', size: 1000, ratios: Array(7).fill(100),
      relSizes: Array(7).fill(100), charSpacing: 0, bold: false, italic: false,
    } }] }));
    assert.equal(paragraphs(doc)[1].text, original);
    const requests = JSON.parse(doc.glyphRequests());
    const usage = requests.find(r => r.label === font.path);
    assert.ok(usage?.glyphs.includes(15180), 'the actual composite glyph is reported to the host');
    assert.ok(!e.fontCharacters().includes('\uf53a'), 'source character collection stays in original Jamo');
    const page = JSON.parse(doc.locate(J({ kind: 'caret', position: at(0) }))).page;
    const packet = decodePacket(doc.page(page));
    const pairs = packet.glyphs;
    const found = [];
    for (let i = 0; i < pairs.length; i += 2) if (pairs[i + 1] === 15180) found.push(pairs[i], pairs[i + 1]);
    assert.equal(found.length, 2);
    assert.equal([...decodeOutlines(e.glyphOutlines(Uint32Array.from(found))).values()][0].em, 0);
    assert.ok(JSON.parse(e.outlineRequests()).includes(font.path));
    assert.ok(JSON.parse(e.glyphCoverage(J([font.path])))[0][2].includes(15180));
    e.addFontFile(font.path, fontFile(bytes));
    assert.ok([...decodeOutlines(e.glyphOutlines(Uint32Array.from(found))).values()][0].em > 0);
    assert.ok(!JSON.parse(e.outlineRequests()).includes(font.path));

    const line = packet.lines.find(l => l.para === 1 && l.list.steps.length === 0);
    assert.ok(line.stops.some(([p]) => p === 0) && line.stops.some(([p]) => p === 3));
    assert.ok(line.stops.every(([p]) => p !== 1 && p !== 2));
    const locate = offset => JSON.parse(doc.locate(J({ kind: 'caret', position: at(offset) })));
    const left = locate(0), right = locate(3);
    assert.equal(right.x - left.x, 1000);
    const nav = (offset, motion) => JSON.parse(doc.navigate(J({ position: at(offset), motion }))).position.offset;
    assert.equal(nav(0, 'right'), 3);
    assert.equal(nav(3, 'left'), 0);
    for (const ratio of [0.2, 0.8]) {
      const hit = JSON.parse(doc.locate(J({ kind: 'point', page: left.page,
        x: left.x + Math.round((right.x - left.x) * ratio), y: left.y + left.height / 2 })));
      assert.ok([0, 3].includes(hit.position.offset));
    }
    const rects = JSON.parse(doc.select(J({ from: at(0), to: at(3) })));
    assert.equal(rects.length, 1);
    assert.equal(rects[0].width, 1000);

    // Raw partial-cluster edits remain valid wire edits, and Undo restores it.
    doc.transact(J({ ops: [{ op: 'delete', from: at(1), to: at(2) }] }));
    assert.equal(paragraphs(doc)[1].text, original.slice(0, 1) + original.slice(2));
    doc.page(page);
    doc.undo();
    assert.equal(paragraphs(doc)[1].text, original);
    doc.transact(J({ ops: [{ op: 'insertText', at: at(1), text: 'X' }] }));
    assert.equal(paragraphs(doc)[1].text, original.slice(0, 1) + 'X' + original.slice(1));
    doc.page(page);
    doc.undo();
    assert.equal(paragraphs(doc)[1].text, original);
    reopened = e.open(doc.serialize(false));
    assert.equal(paragraphs(reopened)[1].text, original);
    assert.ok(!paragraphs(reopened)[1].text.includes('\uf53a'));
  } catch (error) {
    throw new Error(`${error.code ?? "JS"}: ${error.message ?? error}`, { cause: error });
  } finally {
    reopened?.free();
    doc.free();
    e.free();
  }
});
