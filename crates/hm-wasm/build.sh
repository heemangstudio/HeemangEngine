#!/bin/sh
# Builds hm-wasm for wasm32 and generates its JS bindings in
# target/hm-wasm-pkg/web (an ES module with `init` for browsers and
# `initSync` for Node). WASM_BINDGEN names a wasm-bindgen 0.2.125 binary and
# WASM_OPT an optional wasm-opt run on the output.
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
cargo build --target wasm32-unknown-unknown --profile wasm-release -p hm-wasm
bindgen=${WASM_BINDGEN:-wasm-bindgen}
wasm=target/wasm32-unknown-unknown/wasm-release/hm_wasm.wasm
for t in web; do
  "$bindgen" --target "$t" --out-dir "target/hm-wasm-pkg/$t" "$wasm"
  if [ -n "${WASM_OPT:-}" ]; then
    "$WASM_OPT" -O3 --enable-bulk-memory --enable-nontrapping-float-to-int \
      --enable-sign-ext --enable-mutable-globals \
      "target/hm-wasm-pkg/$t/hm_wasm_bg.wasm" -o "target/hm-wasm-pkg/$t/hm_wasm_bg.wasm"
  fi
done
size=$(wc -c < target/hm-wasm-pkg/web/hm_wasm_bg.wasm)
gz=$(gzip -9 -c target/hm-wasm-pkg/web/hm_wasm_bg.wasm | wc -c)
echo "hm_wasm_bg.wasm: $size bytes, $gz gzipped"
