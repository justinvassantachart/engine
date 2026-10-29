import { readFile, writeFile, mkdir, rm } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { parseArgs } from 'node:util';
import path from 'node:path';
import { invokeCompiler } from './compiler-conformance.mjs';
const { values: options } = parseArgs({ options: Object.fromEntries(['compiler', 'compiler-sha256', 'sysroot', 'sysroot-sha256', 'fixture', 'arm', 'out', 'reference'].map(name => [name, { type: 'string' }])) });
for (const name of ['compiler', 'compiler-sha256', 'sysroot', 'sysroot-sha256', 'fixture', 'arm', 'out']) if (!options[name]) throw new Error(`Missing --${name}`);
if (!['shipped', 'control', 'observer-off', 'observer-on'].includes(options.arm)) throw new Error('Unknown compiler arm');
const digest = bytes => createHash('sha256').update(bytes).digest('hex');
const wasm = await readFile(options.compiler), sysroot = await readFile(options.sysroot), fixtureBytes = await readFile(options.fixture);
if (digest(wasm) !== options['compiler-sha256'] || digest(sysroot) !== options['sysroot-sha256']) throw new Error('Toolchain identity mismatch');
const fixture = JSON.parse(fixtureBytes), evidence = path.resolve(options.out), root = path.join(evidence, 'root');
const reference = options.reference ? JSON.parse(await readFile(options.reference, 'utf8')) : null;
if (reference && reference.fixtureSha256 !== digest(fixtureBytes)) throw new Error('Paired course fixture identity mismatch');
await mkdir(path.dirname(evidence), { recursive: true });
await mkdir(evidence); // Keep every prior outcome and guest namespace isolated.
await mkdir(root);
execFileSync('tar', ['-xzf', options.sysroot, '-C', root]);
const normalized = name => {
  const value = name.replace(/^\/(workspace|sysroot)\//, '/');
  if (!value.startsWith('/') || value.split('/').includes('..')) throw new Error(`Unowned path: ${name}`);
  return value;
};
const entries = [], paths = new Set();
function add(name, bytes) {
  if (paths.has(name)) throw new Error(`Owned path collision ${name}`);
  paths.add(name); entries.push([name, bytes]);
}
let artifacts;
if (fixture.plan) {
  const plan = fixture.plan;
  for (const [name, source] of Object.entries(plan.files)) add(normalized(name), source);
  for (const [name, bytes] of Object.entries(plan.binaryFiles)) add(normalized(name), Buffer.from(bytes, 'base64'));
  artifacts = { sources: plan.cppArtifacts.sources.map(normalized), archives: plan.cppArtifacts.archives.map(normalized), precompiledHeader: plan.cppArtifacts.precompiledHeader && normalized(plan.cppArtifacts.precompiledHeader) };
} else {
  function visit(tree, prefix = '') {
    for (const [name, value] of Object.entries(tree)) {
      const full = prefix + '/' + name;
      if (typeof value === 'string') add(normalized(full), value); else visit(value, full);
    }
  }
  visit(fixture.variants[0].files);
  for (const [name, bytes] of Object.entries(fixture.binaryFiles)) add(normalized(name), Buffer.from(bytes, 'base64'));
  artifacts = fixture.cppArtifacts;
}
for (const [name, bytes] of entries) { await mkdir(path.dirname(root + name), { recursive: true }); await writeFile(root + name, bytes); }
const module = await WebAssembly.compile(wasm);
const flags = ['-cc1', '-triple', 'wasm32-wasip1', '-Werror', '-emit-obj', '-disable-free', '-isysroot', '/',
  '-internal-isystem', '/include/c++/v1', '-internal-isystem', '/include', '-internal-isystem', '/include/wasm32-wasip1',
  '-ferror-limit', '4', '-fcolor-diagnostics', '-x', 'c++', '-std=c++23', '-fexceptions', '-fcxx-exceptions', '-exception-model=wasm',
  '-target-feature', '+exception-handling', '-mllvm', '-wasm-enable-eh', '-mllvm', '-wasm-use-legacy-eh=true'];
const variants = [{ clockBase: 1704067200000000000n, entropySeed: 1, mtime: 1704067200000000000n },
  { clockBase: 1735734896000000000n, entropySeed: 0x91cafe, mtime: 1735734896000000000n }];
const signature = result => ({ exit: result.exit, thrown: result.thrown, stdout: result.stdout, stderr: result.stderr, output: result.output });
const records = [], failures = [];
for (const [variant, perturbation] of variants.entries()) {
  for (const source of artifacts.sources) await rm(root + source + '.o', { force: true });
  for (const [index, source] of artifacts.sources.entries()) {
    const label = `variant-${variant}-tu-${index}`;
    const result = await invokeCompiler({ module, root, evidence, label, ...perturbation,
      args: [...flags, '-o', source + '.o', ...(artifacts.precompiledHeader ? ['-include-pch', artifacts.precompiledHeader, '-fvalidate-ast-input-files-content'] : []), ...(options.arm === 'observer-on' ? ['-fwebide-cache-receipt'] : []), source] });
    const record = { label, variant, source, signature: signature(result), receipt: result.receipt, completionCount: result.completionCount };
    records.push(record);
    if (!result.traceComplete || result.exit !== 0 || result.stdout || result.stderr || !result.output) failures.push(`${label}: non-silent, unsuccessful or incompletely traced compile`);
    if (options.arm === 'observer-on') {
      if (result.completionCount !== 1 || result.receipt?.completed !== true || result.receipt.schema !== 1 || result.receipt.capability !== 'webide-object-cache-v1' || result.receipt.source !== source || result.receipt.volatileBuiltins.length || result.receipt.unsupported.length) failures.push(`${label}: incomplete/ineligible observer receipt`);
      if (artifacts.precompiledHeader && (!result.receipt?.pchMetadataIndependent || result.receipt.importedPchPaths.length !== 1 || result.receipt.importedPchPaths[0] !== artifacts.precompiledHeader)) failures.push(`${label}: missing parsed timestamp-free PCH proof`);
    } else if (result.completionCount !== 0) failures.push(`${label}: unexpected receipt`);
    if (reference) {
      const expected = reference.records.find(entry => entry.label === label);
      if (!expected || JSON.stringify(expected.signature) !== JSON.stringify(record.signature)) failures.push(`${label}: differs from paired reference`);
    }
    if (variant === 1 && JSON.stringify(records.find(entry => entry.variant === 0 && entry.source === source)?.signature) !== JSON.stringify(record.signature)) failures.push(`${label}: differs under perturbation`);
    await writeFile(path.join(evidence, 'summary.json'), JSON.stringify({ arm: options.arm, fixtureSha256: digest(fixtureBytes), compilerSha256: digest(wasm), sysrootSha256: digest(sysroot),
      runtime: { node: process.version, v8: process.versions.v8, platform: process.platform, arch: process.arch },
      normalization: 'Unchanged course source/binary bytes; /workspace and /sysroot scope stripping as BrowserRuntimeSession. Fresh mutable compiler instance per invocation.',
      environment: 'Node WASI conformance; no guest execution/cache hits/browser performance acceptance', artifacts, failures, records }, null, 2) + '\n');
    console.log(JSON.stringify({ label, source, output: result.output, receipts: result.completionCount, failures: failures.length }));
  }
}
if (failures.length) throw new Error(`${failures.length} course conformance failures; evidence retained`);
console.log('Course object/diagnostic/exit and configured receipt assertions passed; engine/browser gates remain separate.');
