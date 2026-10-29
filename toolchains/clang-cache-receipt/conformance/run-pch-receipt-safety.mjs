import { readFile, writeFile, mkdir, cp, rm } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { parseArgs } from 'node:util';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { invokeCompiler } from './compiler-conformance.mjs';

const { values: options } = parseArgs({ options: Object.fromEntries(
  ['compiler', 'compiler-sha256', 'course', 'out', 'reference'].map(name => [name, { type: 'string' }])) });
for (const name of ['compiler', 'compiler-sha256', 'course', 'out']) if (!options[name]) throw new Error(`Missing --${name}`);
const sha = bytes => createHash('sha256').update(bytes).digest('hex');
const compiler = await readFile(options.compiler);
if (sha(compiler) !== options['compiler-sha256']) throw new Error('Compiler identity mismatch');
const course = JSON.parse(await readFile(path.join(options.course, 'summary.json'), 'utf8'));
if (course.compilerSha256 !== sha(compiler) || course.artifacts.sources.length !== 1 || !course.artifacts.precompiledHeader || course.failures.length) throw new Error('Need successful actual PCH course conformance');
const reference = options.reference && JSON.parse(await readFile(options.reference, 'utf8'));
const evidence = path.resolve(options.out), root = path.join(evidence, 'root');
await mkdir(path.dirname(evidence), { recursive: true });
await mkdir(evidence);
await cp(path.join(options.course, 'root'), root, { recursive: true });
const invocation = JSON.parse(await readFile(path.join(options.course, 'variant-0-tu-0.json'), 'utf8'));
const originalArgs = invocation.args.filter(arg => arg !== '-fwebide-cache-receipt');
const output = originalArgs[originalArgs.indexOf('-o') + 1], pch = root + course.artifacts.precompiledHeader;
const originalPch = await readFile(pch);
if (reference && reference.originalPchSha256 !== sha(originalPch)) throw new Error('Paired PCH input identity mismatch');
const metadata = JSON.parse(execFileSync('python3', [path.join(path.dirname(fileURLToPath(import.meta.url)), 'inspect-pch-inputs.py'), pch], { encoding: 'utf8' }));
if (!metadata.metadataIndependent) throw new Error('Baseline PCH does not have independently parsed timestamp-free metadata');
const input = metadata.inputs.find(input => input.name === '/include/c++/v1/__functional/pointer_to_binary_function.h');
if (!input || input.id <= metadata.numUserInputs) throw new Error('Pinned synthetic system-record mutation target missing');
const module = await WebAssembly.compile(compiler), records = [], failures = [];
const signature = result => ({ exit: result.exit, thrown: result.thrown, stdout: result.stdout, stderr: result.stderr, output: result.output });
function bits(bytes, offset, width, value) {
  for (let bit = 0; bit < width; bit++) {
    const index = offset + bit, mask = 1 << (index % 8);
    bytes[Math.floor(index / 8)] = (bytes[Math.floor(index / 8)] & ~mask) | ((value >>> bit) & 1 ? mask : 0);
  }
}
const cases = [
  { label: 'unchanged', valid: true, mutate() {} },
  { label: 'metadata-timestamps-present', valid: false, mutate(bytes) { bits(bytes, metadata.metadataValueBitOffsets[6], 1, 1); } },
  { label: 'system-record-nonzero-time', valid: false, mutate(bytes) { bits(bytes, input.valueBitOffsets[2], 32, 1); } },
  { label: 'system-record-overridden', valid: false, mutate(bytes) { bits(bytes, input.valueBitOffsets[3], 1, 1); } },
  { label: 'system-record-transient', valid: false, mutate(bytes) { bits(bytes, input.valueBitOffsets[4], 1, 1); } },
  { label: 'system-record-module-map', valid: false, mutate(bytes) { bits(bytes, input.valueBitOffsets[6], 1, 1); } },
  { label: 'system-record-name-length-outside-blob', valid: false, mutate(bytes) { bits(bytes, input.valueBitOffsets[7], 16, 16383); } },
];
try {
  for (const test of cases) {
    const modified = Buffer.from(originalPch); test.mutate(modified); await writeFile(pch, modified);
    const observerStates = course.arm.startsWith('observer-') ? [false, true] : [false];
    let ordinary;
    for (const observer of observerStates) {
      const label = `${test.label}-observer-${observer ? 'on' : 'off'}`;
      const args = observer ? [...originalArgs.slice(0, -1), '-fwebide-cache-receipt', originalArgs.at(-1)] : originalArgs;
      await rm(root + output, { force: true });
      const result = await invokeCompiler({ module, root, evidence, label, args, clockBase: 1704067200000000000n, entropySeed: 1, mtime: 1704067200000000000n });
      const record = { label, pchSha256: sha(modified), expectedMetadataIndependent: test.valid, signature: signature(result), receipt: result.receipt, completionCount: result.completionCount };
      records.push(record);
      if (!result.traceComplete) failures.push(`${label}: incomplete trace`);
      if (!observer) ordinary = record.signature;
      else {
        if (JSON.stringify(record.signature) !== JSON.stringify(ordinary)) failures.push(`${label}: observer changed ordinary compiler behavior`);
        if (test.valid && result.exit === 0 && (!result.receipt?.pchMetadataIndependent || result.completionCount !== 1)) failures.push(`${label}: valid timestamp-free PCH proof missing`);
        if (!test.valid && result.receipt?.pchMetadataIndependent && !result.receipt?.unsupported?.length) failures.push(`${label}: unsafe PCH record admitted`);
      }
      const paired = reference?.records.find(item => item.label === `${test.label}-observer-off`);
      if (reference && (!paired || JSON.stringify(paired.signature) !== JSON.stringify(record.signature))) failures.push(`${label}: differs from paired control`);
      await writeFile(path.join(evidence, 'summary.json'), JSON.stringify({ compilerSha256: sha(compiler), originalPchSha256: sha(originalPch), arm: course.arm,
        runtime: { node: process.version, v8: process.versions.v8, platform: process.platform, arch: process.arch },
        purpose: 'Isolated binary PCH mutation controls for fail-closed receipt parsing; production artifacts and course source are unchanged', mutationTarget: input, failures, records }, null, 2) + '\n');
      console.log(JSON.stringify({ label, exit: result.exit, output: result.output?.sha256, failures: failures.length }));
    }
  }
} finally { await writeFile(pch, originalPch); }
if (failures.length) throw new Error(`${failures.length} PCH receipt-safety failures; evidence retained`);
