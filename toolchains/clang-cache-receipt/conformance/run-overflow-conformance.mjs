import { readFile, writeFile, mkdir, rm } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { parseArgs } from 'node:util';
import path from 'node:path';
import { invokeCompiler } from './compiler-conformance.mjs';

const { values: options } = parseArgs({ options: Object.fromEntries(
  ['compiler', 'compiler-sha256', 'arm', 'out', 'reference'].map(name => [name, { type: 'string' }])) });
for (const name of ['compiler', 'compiler-sha256', 'arm', 'out']) if (!options[name]) throw new Error(`Missing --${name}`);
if (!['shipped', 'control', 'observer-off', 'observer-on'].includes(options.arm)) throw new Error('Unknown compiler arm');
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const compiler = await readFile(options.compiler);
if (hash(compiler) !== options['compiler-sha256']) throw new Error('Compiler identity mismatch');
const reference = options.reference && JSON.parse(await readFile(options.reference, 'utf8'));
const evidence = path.resolve(options.out), root = path.join(evidence, 'root');
await mkdir(path.dirname(evidence), { recursive: true }); await mkdir(evidence); await mkdir(root);
const module = await WebAssembly.compile(compiler), records = [], failures = [];
const fixtures = [
  ['probe-count-overflow', Array.from({ length: 8193 }, (_, index) => `#if __has_include("missing-${index}.h")\n#error unexpected\n#endif\n`).join('')],
  ['probe-name-overflow', `#if __has_include("${'x'.repeat(4097)}")\n#error unexpected\n#endif\n`, true],
  ['receipt-byte-overflow', Array.from({ length: 5120 }, (_, index) => `#if __has_include("${'x'.repeat(220)}-${index}.h")\n#error unexpected\n#endif\n`).join('')],
];
for (const [label, directives, error = false] of fixtures) {
  const source = directives + 'int f(){return 7;}\n';
  await writeFile(root + '/main.cpp', source); await rm(root + '/out.o', { force: true });
  const result = await invokeCompiler({ module, root, evidence, label,
    args: ['-cc1', '-triple', 'wasm32-wasip1', '-emit-obj', '-disable-free', '-isysroot', '/', '-x', 'c++', '-std=c++23',
      ...(options.arm === 'observer-on' ? ['-fwebide-cache-receipt'] : []), '-o', '/out.o', '/main.cpp'],
    clockBase: 1704067200000000000n, entropySeed: 1, mtime: 1704067200000000000n });
  const signature = { exit: result.exit, thrown: result.thrown, stdout: result.stdout, stderr: result.stderr, output: result.output };
  records.push({ label, sourceSha256: hash(source), expectedError: error, signature, receipt: result.receipt, completionCount: result.completionCount });
  if (!result.traceComplete || (error ? result.exit === 0 : result.exit !== 0 || result.stdout || result.stderr || !result.output)) failures.push(`${label}: ordinary compilation result or complete trace differed from expectation`);
  if (options.arm !== 'observer-on' && result.completionCount) failures.push(`${label}: unexpected completion with observer disabled`);
  if (options.arm === 'observer-on' && result.receipt?.completed && !result.receipt?.unsupported?.length) failures.push(`${label}: bounded observation overflow remained eligible`);
  const paired = reference?.records.find(item => item.label === label);
  if (reference && (!paired || paired.sourceSha256 !== hash(source) || JSON.stringify(paired.signature) !== JSON.stringify(signature))) failures.push(`${label}: changed paired source or ordinary compiler result`);
  await writeFile(path.join(evidence, 'summary.json'), JSON.stringify({ compilerSha256: hash(compiler), arm: options.arm,
    runtime: { node: process.version, v8: process.versions.v8, platform: process.platform, arch: process.arch },
    purpose: 'Bounded observer overflow must decline without modifying ordinary compilation; no runtime cache or browser timing claim', failures, records }, null, 2) + '\n');
  console.log(JSON.stringify({ label, exit: result.exit, output: result.output?.sha256, traceComplete: result.traceComplete, failures: failures.length }));
}
if (failures.length) throw new Error(`${failures.length} overflow conformance failures; evidence retained`);
