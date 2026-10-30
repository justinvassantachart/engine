import { createHash } from 'node:crypto';
import { mkdir, readFile, rm, writeFile } from 'node:fs/promises';
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
await mkdir(evidence); // Never replace a prior gate result.
await mkdir(root);
execFileSync('tar', ['-xzf', options.sysroot, '-C', root]);
const header = `struct text {
  const char *p;
  constexpr text(const char *s) : p(s) {}
  constexpr const char *data() const { return p; }
  constexpr unsigned size() const { return __builtin_strlen(p); }
};\n`;
const fixtures = [
  { name: 'literal-function-empty', source: 'void f(){asm volatile("" ::: "memory");}\n', unsupported: false },
  { name: 'literal-global-empty', source: 'asm(""); int value=1;\n', unsupported: false },
  { name: 'expression-function-empty', source: header + 'void f(){asm((text("")));}\n', unsupported: true },
  { name: 'expression-function-nonempty', source: header + 'void f(){asm((text("nop")));}\n', unsupported: true },
  { name: 'expression-global-empty', source: header + 'asm((text(""))); int value=1;\n', unsupported: true },
  { name: 'dependent-unused-expression', source: 'template<class T> void f(){asm((T{}));}\nint value=1;\n', unsupported: true },
];
const fixtureSha256 = sha(JSON.stringify(fixtures));
const reference = options.reference && JSON.parse(await readFile(options.reference, 'utf8'));
if (reference && reference.fixtureSha256 !== fixtureSha256) throw new Error('Fixture identity mismatch');
const module = await WebAssembly.compile(compiler), records = [], failures = [];
const flags = ['-cc1', '-triple', 'wasm32-wasip1', '-Werror', '-disable-free', '-isysroot', '/',
  '-internal-isystem', '/include/c++/v1', '-internal-isystem', '/include', '-internal-isystem', '/include/wasm32-wasip1',
  '-ferror-limit', '4', '-fcolor-diagnostics', '-x', 'c++', '-std=c++23', '-fexceptions', '-fcxx-exceptions',
  '-exception-model=wasm', '-target-feature', '+exception-handling', '-mllvm', '-wasm-enable-eh', '-mllvm', '-wasm-use-legacy-eh=true'];
for (const fixture of fixtures) {
  const source = `/${fixture.name}.cpp`;
  await writeFile(root + source, fixture.source);
  await rm(root + '/out.o', { force: true });
  const result = await invokeCompiler({ module, root, evidence, label: fixture.name,
    args: [...flags, ...(options.arm === 'observer-on' ? ['-fwebide-cache-receipt'] : []), '-emit-obj', '-o', '/out.o', source],
    clockBase: 1704067200000000000n, entropySeed: 1, mtime: 1704067200000000000n });
  const signature = { exit: result.exit, thrown: result.thrown, stdout: result.stdout, stderr: result.stderr, output: result.output };
  if (!result.traceComplete || result.exit !== 0 || result.stdout || result.stderr || !result.output) failures.push(`${fixture.name}: unsuccessful/non-silent ordinary compile`);
  if (options.arm === 'observer-on') {
    if (result.completionCount !== 1 || !result.receipt?.completed || result.receipt.source !== source ||
        Boolean(result.receipt.unsupported.length) !== fixture.unsupported) failures.push(`${fixture.name}: incorrect assembly observation`);
  } else if (result.completionCount) failures.push(`${fixture.name}: unexpected receipt`);
  const expected = reference?.records.find(item => item.name === fixture.name);
  if (reference && (!expected || JSON.stringify(expected.signature) !== JSON.stringify(signature))) failures.push(`${fixture.name}: ordinary output changed`);
  records.push({ name: fixture.name, sourceSha256: sha(fixture.source), expectedUnsupported: fixture.unsupported,
    signature, receipt: result.receipt, completionCount: result.completionCount });
  await writeFile(path.join(evidence, 'summary.json'), JSON.stringify({
    arm: options.arm, compilerSha256: sha(compiler), sysrootSha256: sha(sysroot), fixtureSha256,
    purpose: 'Pinned asm API correction: inspect already-held literals; conservatively decline nonliteral/dependent expressions without evaluation or allocation',
    runtime: { node: process.version, v8: process.versions.v8 }, failures, records,
  }, null, 2) + '\n');
  console.log(JSON.stringify({ name: fixture.name, exit: result.exit, receipts: result.completionCount, failures: failures.length }));
}
if (failures.length) throw new Error(`${failures.length} assembly API conformance failures; retained ${evidence}`);
