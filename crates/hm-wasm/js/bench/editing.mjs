// Native WASM boundaries from the Studio typing investigation. Run with
// HEEMANG_CORPUS/HEEMANG_FONTS and optional HM_WASM_PKG to compare builds.
import { performance } from 'node:perf_hooks';
import { engine, registerFonts, sample } from '../test/env.mjs';

const runs = Number(process.env.RUNS ?? 3);
const keys = 'abcdefghijklmnopqrstuvwxyzabcdefghij';
const targets = [
  ['2025 행정업무운영 편람(최종).hwp', { section: 1, para: 7, offset: 3 }],
  ['hwpx/2024년 1분기 해외직접투자 보도자료 ff.hwpx', { section: 0, para: 6, offset: 2 }],
];
for (const [name, at] of targets) {
  const bytes = sample(name);
  if (!bytes) throw new Error(`missing corpus document: ${name}`);
  for (let run = 1; run <= runs; run++) {
    const e = engine();
    const doc = e.open(bytes);
    registerFonts(e, doc);
    const prior = JSON.parse(doc.query(JSON.stringify({ kind: 'paragraphs', list: { section: at.section }, from: at.para, count: 1 })))[0].text;
    const samples = {};
    const cpu = {};
    const time = (name, f) => {
      const start = performance.now();
      const usage = process.cpuUsage();
      const result = f();
      (samples[name] ??= []).push(performance.now() - start);
      const used = process.cpuUsage(usage);
      (cpu[name] ??= []).push((used.user + used.system) / 1000);
      return result;
    };
    doc.locate(JSON.stringify({ kind: 'caret', position: at }));
    for (const [i, key] of [...keys].entries()) {
      const checkpoint = time('checkpoint', () => doc.checkpoint());
      const receipt = JSON.parse(time('transact', () => doc.transact(JSON.stringify({ ops: [{ op: 'insertText', at: { ...at, offset: at.offset + i }, text: key }] }))));
      time('release', () => doc.releaseCheckpoint(checkpoint));
      time('locate', () => doc.locate(JSON.stringify({ kind: 'caret', position: receipt.caret })));
      time('pages', () => doc.pages());
      if (doc.pageInfo) time('pageInfo', () => doc.pageInfo(0));
    }
    const after = JSON.parse(doc.query(JSON.stringify({ kind: 'paragraphs', list: { section: at.section }, from: at.para, count: 1 })))[0].text;
    if (after !== prior.slice(0, at.offset) + keys + prior.slice(at.offset)) throw new Error('typing content differs');
    const ms = Object.fromEntries(Object.entries(samples).map(([key, values]) => {
      const ordered = values.toSorted((a, b) => a - b);
      return [key, { mean: values.reduce((a, b) => a + b) / values.length, p50: ordered[Math.ceil(values.length * .5) - 1], p95: ordered[Math.ceil(values.length * .95) - 1] }];
    }));
    console.log(JSON.stringify({ name, run, keys: keys.length, pages: JSON.parse(doc.state()).pageCount, ms, cpu: Object.fromEntries(Object.entries(cpu).map(([key, values]) => [key, values.reduce((a, b) => a + b) / values.length])) }));
    doc.free();
    e.free();
  }
}
