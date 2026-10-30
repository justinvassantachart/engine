import { readFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';

const [file, arm] = process.argv.slice(2);
if (!file || !['control', 'observer'].includes(arm)) throw new Error('Usage: verify-core.mjs <raw-core> control|observer');
const bytes = await readFile(file), module = await WebAssembly.compile(bytes);
const imports = WebAssembly.Module.imports(module), exports = WebAssembly.Module.exports(module);
const observed = imports.filter(item => item.module === 'webide_compiler');
if (arm === 'control' ? observed.length !== 0 :
    observed.length !== 1 || observed[0].name !== 'cache_receipt_v1' || observed[0].kind !== 'function') {
  throw new Error('Unexpected compiler observation import');
}
if (imports.some(item => !['wasi_snapshot_preview1', 'webide_compiler'].includes(item.module)) ||
    !exports.some(item => item.name === 'memory' && item.kind === 'memory') ||
    !exports.some(item => item.name === '_start' && item.kind === 'function')) {
  throw new Error('Unexpected core WASI boundary');
}
console.log(JSON.stringify({ schema: 1, arm, bytes: bytes.length,
  sha256: createHash('sha256').update(bytes).digest('hex'), imports, exports,
  receiptCapabilityValidated: false,
  note: 'Module validation/import inventory only; exact host ABI, receipt semantics and browser overhead require separate execution gates.' }, null, 2));
