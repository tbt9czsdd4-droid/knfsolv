// Starts the analysis worker (src/bin/worker.rs). Trunk builds worker.js and worker_bg.wasm
// without content hashes, so the page can always find them by name.
importScripts("./worker.js");
wasm_bindgen({ module_or_path: "./worker_bg.wasm" });
