import { createHash } from 'node:crypto';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';
import { parseArgs } from 'node:util';
import path from 'node:path';
import { invokeCompiler } from './compiler-conformance.mjs';

const { values: options } = parseArgs({ options: Object.fromEntries(
  ['compiler', 'compiler-sha256', 'sysroot', 'sysroot-sha256', 'out', 'arm', 'reference'].map(name => [name, { type: 'string' }])) });
for (const name of ['compiler', 'compiler-sha256', 'sysroot', 'sysroot-sha256', 'out', 'arm']) if (!options[name]) throw new Error(`Missing --${name}`);
if (!['shipped', 'control', 'observer-off', 'observer-on'].includes(options.arm)) throw new Error('Unknown arm');
const sha = bytes => createHash('sha256').update(bytes).digest('hex');
const compiler = await readFile(options.compiler), sysroot = await readFile(options.sysroot);
if (sha(compiler) !== options['compiler-sha256'] || sha(sysroot) !== options['sysroot-sha256']) throw new Error('Compiler/sysroot identity mismatch');
const evidence = path.resolve(options.out), root = path.join(evidence, 'root');
await mkdir(path.dirname(evidence), { recursive: true });
await mkdir(evidence); await mkdir(root); await mkdir(root + '/umbrella'); await mkdir(root + '/cache');
execFileSync('tar', ['-xzf', options.sysroot, '-C', root]);
await writeFile(root + '/module.modulemap', 'module M { umbrella "umbrella" export * module * { export * } }\n');
await writeFile(root + '/umbrella/a.h', 'inline int a(){return 7;}\n');
const module = await WebAssembly.compile(compiler);
const result = await invokeCompiler({ module, root, evidence, label: 'module',
  clockBase: 1704067200000000000n, entropySeed: 1, mtime: 1704067200000000000n,
  args: ['-cc1', '-triple', 'wasm32-wasip1', '-isysroot', '/', '-internal-isystem', '/include/c++/v1',
    '-internal-isystem', '/include', '-internal-isystem', '/include/wasm32-wasip1', '-x', 'c++', '-std=c++23',
    '-fmodules', '-fimplicit-module-maps', '-fmodules-cache-path=/cache', '-emit-module', '-fmodule-name=M',
    ...(options.arm === 'observer-on' ? ['-fwebide-cache-receipt'] : []), '-o', '/module.pcm', '/module.modulemap'] });
const signature = { exit: result.exit, thrown: result.thrown, stdout: result.stdout, stderr: result.stderr, output: result.output };
const enumeration = result.events.filter(event => event.name === 'fd_readdir');
const reference = options.reference && JSON.parse(await readFile(options.reference, 'utf8'));
const failures = [];
if (!result.traceComplete || result.exit !== 0 || result.stdout || result.stderr || !result.output) failures.push('ordinary module action failed');
if (!enumeration.length || !enumeration.some(event => event.result === 0 && event.bytesRead > 0 && event.path === '/umbrella')) failures.push('required real directory enumeration not observed');
if (result.completionCount !== 0) failures.push('unsupported module action produced an object receipt');
if (reference && JSON.stringify(reference.signature) !== JSON.stringify(signature)) failures.push('observer changed ordinary module behavior');
await writeFile(path.join(evidence, 'summary.json'), JSON.stringify({
  arm: options.arm, compilerSha256: sha(compiler), sysrootSha256: sha(sysroot), signature, enumeration,
  purpose: 'Retain real directory enumeration as an explicitly rejected cache input domain; modules/actions are outside strict cc1 object policy',
  expectedTraceAuditRejection: 'directory enumeration is outside receipt input policy',
  runtime: { node: process.version, v8: process.versions.v8 }, failures,
}, null, 2) + '\n');
console.log(JSON.stringify({ exit: result.exit, directoryEnumerations: enumeration.length, failures }));
if (failures.length) throw new Error('Directory enumeration conformance failed; evidence retained');
