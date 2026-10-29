// hm-wasm adapter for perf/wasm-bench.py --adapter: open, then lay out
// every page, with no fonts registered.
export async function initialize(modulePath, binaryPath) {
  const { pathToFileURL } = await import('node:url');
  const { readFileSync } = await import('node:fs');
  const api = await import(pathToFileURL(modulePath));
  const exports = api.initSync({ module: readFileSync(binaryPath) });
  const engine = new api.Engine();
  return {
    memory: exports.memory,
    open: bytes => engine.open(bytes),
    // The layout runs on the first request that needs it.
    paginate: doc => JSON.parse(doc.state()).pageCount,
    dispose: doc => doc.free(),
  };
}
