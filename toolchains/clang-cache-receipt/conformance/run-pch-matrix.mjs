import { readFile, writeFile, mkdir, rm, cp } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { parseArgs } from 'node:util';
import path from 'node:path';
import { invokeCompiler } from './compiler-conformance.mjs';
const { values: options } = parseArgs({ options: Object.fromEntries(['compiler', 'compiler-sha256', 'course', 'out', 'reference'].map(name => [name, { type: 'string' }])) });
for (const name of ['compiler', 'compiler-sha256', 'course', 'out']) if (!options[name]) throw new Error(`Missing --${name}`);
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const bytes = await readFile(options.compiler);
if (hash(bytes) !== options['compiler-sha256']) throw new Error('Compiler identity mismatch');
const course = JSON.parse(await readFile(path.join(options.course, 'summary.json'), 'utf8'));
if (course.compilerSha256 !== hash(bytes) || course.artifacts.sources.length !== 1 || !course.artifacts.precompiledHeader || course.failures.length) throw new Error('Need successful single-TU actual PCH course conformance');
const reference = options.reference && JSON.parse(await readFile(options.reference, 'utf8'));
const evidence = path.resolve(options.out), root = path.join(evidence, 'root');
if (reference && reference.courseFixtureSha256 !== course.fixtureSha256) throw new Error('Paired PCH course fixture identity mismatch');
await mkdir(path.dirname(evidence), { recursive: true });
await mkdir(evidence);
await cp(path.join(options.course, 'root'), root, { recursive: true });
const invocation = JSON.parse(await readFile(path.join(options.course, 'variant-0-tu-0.json'), 'utf8'));
const args = invocation.args;
const output = args[args.indexOf('-o') + 1], pch = course.artifacts.precompiledHeader;
const module = await WebAssembly.compile(bytes);
const variants = [{ clockBase: 1704067200000000000n, entropySeed: 1, mtime: 1704067200000000000n },
  { clockBase: 1735734896000000000n, entropySeed: 0x91cafe, mtime: 1735734896000000000n }];
const originals = Object.fromEntries(await Promise.all(['/vector.h', '/include/c++/v1/__config', pch].map(async name => [name, await readFile(root + name)])));
const records = [], failures = [];
const signature = result => ({ exit: result.exit, thrown: result.thrown, stdout: result.stdout, stderr: result.stderr, output: result.output });
async function compile(label, actualArgs = args, variant = 0, expected = 'success', timestampFree = true) {
  await rm(root + output, { force: true });
  const result = await invokeCompiler({ module, root, evidence, label, args: actualArgs, ...variants[variant] });
  const record = { label, expected, signature: signature(result), receipt: result.receipt, completionCount: result.completionCount };
  records.push(record);
  if (!result.traceComplete) failures.push(`${label}: incomplete trace`);
  if (expected === 'success' && (result.exit !== 0 || result.stdout || result.stderr || !result.output)) failures.push(`${label}: expected silent successful compilation`);
  if (expected === 'error' && result.exit === 0) failures.push(`${label}: expected PCH validation failure`);
  if (actualArgs.includes('-fwebide-cache-receipt') && result.exit === 0 && (result.completionCount !== 1 || result.receipt?.pchMetadataIndependent !== timestampFree)) failures.push(`${label}: missing PCH metadata observation`);
  const paired = reference?.records.find(item => item.label === label);
  if (reference && (!paired || JSON.stringify(paired.signature) !== JSON.stringify(record.signature))) failures.push(`${label}: differs from paired control`);
  await writeFile(path.join(evidence, 'summary.json'), JSON.stringify({ compilerSha256: hash(bytes), courseFixtureSha256: course.fixtureSha256, arm: course.arm,
    runtime: { node: process.version, v8: process.versions.v8, platform: process.platform, arch: process.arch },
    purpose: 'Actual PCH-on Vector input/metadata matrix; source mutations are isolated test cases, not changed assignment fixtures or production artifacts', failures, records }, null, 2) + '\n');
  console.log(JSON.stringify({ label, exit: result.exit, output: result.output?.sha256, failures: failures.length }));
  return result;
}
await compile('unchanged');
await compile('unchanged-mtimes-varied', args, 1);
const changed = Buffer.from(originals['/vector.h']);
const marker = changed.indexOf('File: vector.h');
if (marker < 0) throw new Error('Exact header mutation marker missing');
changed[marker] = 'f'.charCodeAt(0);
try {
  await writeFile(root + '/vector.h', changed);
  await compile('header-same-length-content', args, 0, 'error');
  await compile('header-same-length-content-validation-off', args.filter(arg => arg !== '-fvalidate-ast-input-files-content'), 0, 'observe');
} finally { await writeFile(root + '/vector.h', originals['/vector.h']); }
try {
  await rm(root + '/vector.h');
  await compile('header-missing', args, 0, 'error');
} finally { await writeFile(root + '/vector.h', originals['/vector.h']); }
try {
  const system = Buffer.from(originals['/include/c++/v1/__config']);
  const offset = system.indexOf('//');
  if (offset < 0) throw new Error('System header comment marker missing');
  system[offset + 2] = system[offset + 2] === 32 ? 33 : 32;
  await writeFile(root + '/include/c++/v1/__config', system);
  await compile('system-header-same-length-content', args, 0, 'observe');
} finally { await writeFile(root + '/include/c++/v1/__config', originals['/include/c++/v1/__config']); }
try {
  const corrupt = Buffer.from(originals[pch]); corrupt[0] ^= 255;
  await writeFile(root + pch, corrupt);
  await compile('corrupt-pch-magic', args, 0, 'error');
} finally { await writeFile(root + pch, originals[pch]); }
await compile('language-flags-mismatch', args.map(arg => arg === '-std=c++23' ? '-std=c++20' : arg), 0, 'error');
await compile('restored');
// Recreate the unchanged prelude with and without timestamp serialization. The
// observation option is frontend-only: patched on/off creation must preserve PCH
// bytes and every consumption combination must retain the ordinary result.
const creation = args.filter((arg, index) => !['-o', output, '-include-pch', pch, '-fwebide-cache-receipt', course.artifacts.sources[0]].includes(arg) && args[index - 1] !== '-o' && args[index - 1] !== '-include-pch')
  .map(arg => arg === '-emit-obj' ? '-emit-pch' : arg === 'c++' ? 'c++-header' : arg);
const observerStates = course.arm.startsWith('observer-') ? [false, true] : [false];
const pchHashes = {};
for (const timestampFree of [true, false]) for (const contentHashes of [true, false]) for (const observer of observerStates) {
  const label = `create-${timestampFree ? 'timestamp-free' : 'timestamp-bearing'}-content-hashes-${contentHashes ? 'on' : 'off'}-observer-${observer ? 'on' : 'off'}`;
  const madePch = `/${label}.pch`;
  const result = await invokeCompiler({ module, root, evidence, label, ...variants[0],
    args: [...creation.filter(arg => contentHashes || arg !== '-fvalidate-ast-input-files-content'), ...(timestampFree ? ['-fno-pch-timestamp'] : []), ...(observer ? ['-fwebide-cache-receipt'] : []), '-o', madePch, '/cs106b-prelude.h'] });
  if (result.exit !== 0 || result.stdout || result.stderr || !result.output) { failures.push(`${label}: PCH creation failure`); continue; }
  const hashKey = `${timestampFree}:${contentHashes}`;
  const prior = pchHashes[hashKey];
  if (prior && result.output.sha256 !== prior) failures.push(`${label}: observation option changed serialized PCH bytes`);
  pchHashes[hashKey] = result.output.sha256;
  for (const consumedObserver of observerStates) for (const contentValidation of [true, false]) for (const variant of [0, 1]) {
    const consumedArgs = args.filter(arg => arg !== '-fwebide-cache-receipt' && (contentValidation || arg !== '-fvalidate-ast-input-files-content')).map(arg => arg === pch ? madePch : arg);
    if (consumedObserver) consumedArgs.splice(-1, 0, '-fwebide-cache-receipt');
    await compile(`${label}-consume-observer-${consumedObserver ? 'on' : 'off'}-validation-${contentValidation ? 'on' : 'off'}-variant-${variant}`, consumedArgs, variant, timestampFree || variant === 0 || (contentHashes && contentValidation) ? 'success' : 'error', timestampFree);
  }
}
await writeFile(path.join(evidence, 'failures.json'), JSON.stringify(failures, null, 2) + '\n');
if (failures.length) throw new Error(`${failures.length} PCH matrix failures; evidence retained`);
console.log('Actual Vector PCH mutation and creation/consumption matrix passed; real-engine cache admission remains separate.');
