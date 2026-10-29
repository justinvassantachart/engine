import { readFile, writeFile, mkdir, rm } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { parseArgs } from 'node:util';
import { invokeCompiler } from './compiler-conformance.mjs';
import { fixtures, pchFixtures } from './receipt-fixtures.mjs';

const { values: options } = parseArgs({ options: Object.fromEntries(['compiler', 'compiler-sha256', 'sysroot', 'sysroot-sha256', 'arm', 'out', 'reference'].map(name => [name, { type: 'string' }])) });
for (const name of ['compiler', 'compiler-sha256', 'sysroot', 'sysroot-sha256', 'arm', 'out']) if (!options[name]) throw new Error(`Missing --${name}`);
if (!['shipped', 'control', 'observer-off', 'observer-on'].includes(options.arm)) throw new Error('Unknown compiler arm');
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const fixtureSource = await readFile(new URL('./receipt-fixtures.mjs', import.meta.url));
const wasm = await readFile(options.compiler), sysroot = await readFile(options.sysroot);
if (hash(wasm) !== options['compiler-sha256'] || hash(sysroot) !== options['sysroot-sha256']) throw new Error('Toolchain identity mismatch');
const evidence = path.resolve(options.out), root = path.join(evidence, 'root');
await mkdir(path.dirname(evidence), { recursive: true });
await mkdir(evidence); // Never overwrite failed evidence or reuse an old guest namespace.
await mkdir(root);
await writeFile(path.join(evidence, 'receipt-fixtures.mjs'), fixtureSource);
execFileSync('tar', ['-xzf', options.sysroot, '-C', root]);
await mkdir(path.join(root, 'system'), { recursive: true });
for (const [name, bytes] of Object.entries({ 'present.h': '#define PRESENT 7\n', 'sibling.cpp': 'static int sibling=3;\n', 'system/time-wrapper.h': '#define SYSTEM_CLOCK __TIME__\n', 'data.bin': Buffer.from([1,2,3]), 'empty.bin': Buffer.alloc(0) })) await writeFile(path.join(root, name), bytes);
const module = await WebAssembly.compile(wasm);
const observing = options.arm === 'observer-on';
const common = ['-cc1', '-triple', 'wasm32-wasip1', '-Werror', '-disable-free', '-isysroot', '/',
  '-internal-isystem', '/include/c++/v1', '-internal-isystem', '/include', '-internal-isystem', '/include/wasm32-wasip1', '-internal-isystem', '/system',
  '-ferror-limit', '4', '-fcolor-diagnostics', '-x', 'c++', '-std=c++23', '-fexceptions', '-fcxx-exceptions',
  '-exception-model=wasm', '-target-feature', '+exception-handling', '-mllvm', '-wasm-enable-eh', '-mllvm', '-wasm-use-legacy-eh=true'];
const variants = [
  { clockBase: 1704067200000000000n, entropySeed: 1, mtime: 1704067200000000000n },
  { clockBase: 1735734896000000000n, entropySeed: 0x91cafe, mtime: 1735734896000000000n },
];
const records = [], failures = [];
const signature = result => ({ exit: result.exit, thrown: result.thrown, stdout: result.stdout, stderr: result.stderr, output: result.output });
const reference = options.reference ? JSON.parse(await readFile(options.reference, 'utf8')) : null;
if (reference && reference.fixtureSourceSha256 !== hash(fixtureSource)) throw new Error('Paired fixture source identity mismatch');
async function record(fixture, suffix, args, variant, capture = {}) {
  await rm(path.join(root, 'out.o'), { force: true });
  const label = `${fixture.name}-${suffix}`;
  const result = await invokeCompiler({ module, root, evidence, label, ...variant, ...capture, args });
  if (!result.traceComplete) failures.push(`${label}: incomplete trace`);
  if (!observing && result.completionCount !== 0) failures.push(`${label}: unexpected receipt with observer disabled`);
  if (observing && result.exit === 0 && capture.receiptLimit !== 0) {
    const receipt = result.receipt;
    if (result.completionCount !== 1 || receipt?.completed !== true || receipt.capability !== 'webide-object-cache-v1' || receipt.schema !== 1) failures.push(`${label}: missing or invalid completion`);
    if (receipt) {
      if (JSON.stringify([...receipt.volatileBuiltins].sort()) !== JSON.stringify([...(fixture.volatile ?? [])].sort())) failures.push(`${label}: wrong builtin observations`);
      if (receipt.usesCounter !== Boolean(fixture.counter)) failures.push(`${label}: wrong counter observation`);
      if (fixture.unsupported && receipt.unsupported.length === 0) failures.push(`${label}: unsupported construct was not reported`);
      if (!fixture.unsupported && receipt.unsupported.length !== 0) failures.push(`${label}: unexplained unsupported construct: ${receipt.unsupported.join(', ')}`);
      for (const name of fixture.probes ?? []) if (!receipt.includeProbes.some(probe => probe.name === name)) failures.push(`${label}: missing ${name} include probe`);
      if (fixture.pchTimestampFree !== undefined && receipt.pchMetadataIndependent !== fixture.pchTimestampFree) failures.push(`${label}: wrong parsed PCH metadata independence`);
    }
  }
  const entry = { label, signature: signature(result), receipt: result.receipt, completionCount: result.completionCount, fixture };
  records.push(entry);
  const referenceLabel = suffix === 'declined' ? `${fixture.name}-0` : label;
  const expected = reference?.records.find(record => record.label === referenceLabel);
  if (reference && (!expected || JSON.stringify(expected.signature) !== JSON.stringify(entry.signature))) failures.push(`${label}: differs from paired reference`);
  await writeFile(path.join(evidence, 'summary.json'), JSON.stringify({ arm: options.arm, compilerSha256: hash(wasm), sysrootSha256: hash(sysroot),
    runtime: { node: process.version, v8: process.versions.v8, platform: process.platform, arch: process.arch },
    fixtureSourceSha256: hash(fixtureSource),
    environment: 'Node WASI conformance; controlled test-only clock/entropy/mtime perturbations; not browser performance acceptance', failures, records }, null, 2) + '\n');
  console.log(JSON.stringify({ label, exit: result.exit, output: result.output?.sha256, receipts: result.completionCount, failures: failures.length }));
  return result;
}
for (const fixture of fixtures) {
  await writeFile(path.join(root, `${fixture.name}.cpp`), fixture.source);
  const pair = [];
  for (const [index, variant] of variants.entries()) pair.push(await record(fixture, index,
    [...common, ...(observing ? ['-fwebide-cache-receipt'] : []), '-emit-obj', '-o', '/out.o', `/${fixture.name}.cpp`], variant));
  if (!fixture.volatile && JSON.stringify(signature(pair[0])) !== JSON.stringify(signature(pair[1]))) failures.push(`${fixture.name}: unexpected perturbation difference`);
  if (fixture.error ? pair.some(result => result.exit === 0) : pair.some(result => result.exit !== 0)) failures.push(`${fixture.name}: unexpected compiler result`);
  if (observing && !fixture.error) {
    const declined = await record(fixture, 'declined', [...common, '-fwebide-cache-receipt', '-emit-obj', '-o', '/out.o', `/${fixture.name}.cpp`], variants[0], { receiptLimit: 0 });
    if (JSON.stringify(signature(declined)) !== JSON.stringify(signature(pair[0]))) failures.push(`${fixture.name}: declining receipt changed result`);
  }
}
for (const fixture of pchFixtures) {
  await writeFile(path.join(root, `${fixture.name}.h`), fixture.header);
  await writeFile(path.join(root, `${fixture.name}.cpp`), fixture.source);
  for (const timestampFree of [true, false]) {
    const suffix = timestampFree ? 'timestamp-free' : 'timestamp-bearing';
    const pch = `/${fixture.name}-${suffix}.pch`;
    const created = await invokeCompiler({ module, root, evidence, label: `${fixture.name}-${suffix}-create`, ...variants[0],
      args: [...common, '-emit-pch', ...(timestampFree ? ['-fno-pch-timestamp'] : []), '-o', pch, `/${fixture.name}.h`] });
    if (created.exit !== 0) { failures.push(`${fixture.name}: PCH creation failed`); continue; }
    for (const [index, variant] of variants.entries()) await record({ ...fixture, pchTimestampFree: timestampFree }, `${suffix}-${index}`,
      [...common, ...(observing ? ['-fwebide-cache-receipt'] : []), '-include-pch', pch, '-fvalidate-ast-input-files-content', '-emit-obj', '-o', '/out.o', `/${fixture.name}.cpp`], variant);
  }
}
await writeFile(path.join(evidence, 'failures.json'), JSON.stringify(failures, null, 2) + '\n');
if (failures.length) throw new Error(`${failures.length} conformance failures; evidence retained in ${evidence}`);
console.log('All configured conformance assertions passed. Browser and integration gates remain separate.');
