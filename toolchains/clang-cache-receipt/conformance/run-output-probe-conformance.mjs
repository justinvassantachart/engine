import { createHash } from 'node:crypto';
import { mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { parseArgs } from 'node:util';
import path from 'node:path';
import { invokeCompiler } from './compiler-conformance.mjs';

const { values: options } = parseArgs({ options: Object.fromEntries(
  ['compiler', 'compiler-sha256', 'out', 'arm', 'reference'].map(name => [name, { type: 'string' }])) });
for (const name of ['compiler', 'compiler-sha256', 'out', 'arm']) if (!options[name]) throw new Error(`Missing --${name}`);
if (!['shipped', 'control', 'observer-off', 'observer-on'].includes(options.arm)) throw new Error('Unknown arm');
const sha = bytes => createHash('sha256').update(bytes).digest('hex');
const compiler = await readFile(options.compiler);
if (sha(compiler) !== options['compiler-sha256']) throw new Error('Compiler identity mismatch');
const evidence = path.resolve(options.out), root = path.join(evidence, 'root');
await mkdir(path.dirname(evidence), { recursive: true });
await mkdir(evidence); await mkdir(root);
const fixtures = [
  { name: 'ordinary-negative-header', path: '/ordinary-missing.h', present: false, hostBypass: false },
  { name: 'negative-own-selected-output', path: '/main.cpp.o', present: false, hostBypass: true },
  { name: 'negative-other-selected-output', path: '/other.cpp.o', present: false, hostBypass: true },
  { name: 'negative-temporary-sibling', path: '/main.cpp-deadbeef.o.tmp', present: false, hostBypass: true },
  { name: 'positive-own-selected-output', path: '/main.cpp.o', present: true, hostBypass: true },
  { name: 'positive-other-selected-output', path: '/other.cpp.o', present: true, hostBypass: true },
  { name: 'positive-temporary-sibling', path: '/main.cpp-deadbeef.o.tmp', present: true, hostBypass: true },
];
const fixtureSha256 = sha(JSON.stringify(fixtures));
const reference = options.reference && JSON.parse(await readFile(options.reference, 'utf8'));
if (reference && reference.fixtureSha256 !== fixtureSha256) throw new Error('Fixture identity mismatch');
const module = await WebAssembly.compile(compiler), records = [], failures = [];
const flags = ['-cc1', '-triple', 'wasm32-wasip1', '-Werror', '-disable-free', '-isysroot', '/',
  '-x', 'c++', '-std=c++23', '-emit-obj', ...(options.arm === 'observer-on' ? ['-fwebide-cache-receipt'] : [])];
const signature = result => ({ exit: result.exit, thrown: result.thrown, stdout: result.stdout, stderr: result.stderr, output: result.output });
const perturbation = { clockBase: 1704067200000000000n, entropySeed: 1, mtime: 1704067200000000000n };
// Record a real second selected TU output. Positive tests seed its valid object
// bytes in the isolated namespace; no production inputs or fixtures change.
await writeFile(root + '/other.cpp', 'int other_value=11;\n');
const other = await invokeCompiler({ module, root, evidence, label: 'other-selected-tu', ...perturbation,
  args: [...flags, '-o', '/other.cpp.o', '/other.cpp'] });
if (other.exit !== 0 || other.stdout || other.stderr || !other.output) throw new Error('Other selected TU failed');
const seed = await readFile(root + '/other.cpp.o');
for (const fixture of fixtures) {
  for (const name of ['/main.cpp.o', '/other.cpp.o', '/main.cpp-deadbeef.o.tmp']) await rm(root + name, { force: true });
  if (fixture.present) await writeFile(root + fixture.path, seed);
  const source = `#if __has_include("${fixture.path}") != ${Number(fixture.present)}\n#error unexpected metadata probe outcome\n#endif\nint found=${Number(fixture.present)};\n`;
  await writeFile(root + '/main.cpp', source);
  const result = await invokeCompiler({ module, root, evidence, label: fixture.name, ...perturbation,
    args: [...flags, '-o', '/main.cpp.o', '/main.cpp'] });
  if (!result.traceComplete || result.exit !== 0 || result.stdout || result.stderr || !result.output) failures.push(`${fixture.name}: ordinary metadata probe compile failed`);
  if (options.arm === 'observer-on') {
    const probe = result.receipt?.includeProbes.find(probe => probe.name === fixture.path);
    if (result.completionCount !== 1 || !result.receipt?.completed || !probe ||
        probe.found !== (fixture.present ? fixture.path : null)) failures.push(`${fixture.name}: actual metadata-only probe receipt missing`);
  } else if (result.completionCount) failures.push(`${fixture.name}: unexpected completion`);
  const expected = reference?.records.find(item => item.name === fixture.name);
  if (reference && (!expected || JSON.stringify(expected.signature) !== JSON.stringify(signature(result)))) failures.push(`${fixture.name}: ordinary output/diagnostics changed`);
  records.push({ ...fixture, sourceSha256: sha(source), signature: signature(result), receipt: result.receipt,
    completionCount: result.completionCount, metadataAccesses: result.events.filter(event => event.path === fixture.path) });
  await writeFile(path.join(evidence, 'summary.json'), JSON.stringify({
    arm: options.arm, compilerSha256: sha(compiler), fixtureSha256, selectedSources: ['/main.cpp', '/other.cpp'],
    purpose: 'Actual positive/negative metadata-only probes of selected output/temp domains; host receipt consumer must reject the six unsafe cases while normal output bookkeeping remains allowed',
    hostConsumerValidation: 'Separate actual public Engine gate required; a compiler receipt alone does not know selected batch outputs',
    runtime: { node: process.version, v8: process.versions.v8 }, failures, records,
  }, null, 2) + '\n');
  console.log(JSON.stringify({ name: fixture.name, exit: result.exit, receipts: result.completionCount, failures: failures.length }));
}
if (failures.length) throw new Error(`${failures.length} output-probe conformance failures; retained ${evidence}`);
