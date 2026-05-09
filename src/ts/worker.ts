import init, * as wasm from '../../pkg/engine.js';
import wasmBinary from '../../pkg/engine_bg.wasm';
import { errorResult } from './util';

try {
  await init({ module_or_path: wasmBinary });
  wasm.main();
} catch (err) {
  postMessage({
    type: 'stop',
    result: errorResult(err)
  });
}
