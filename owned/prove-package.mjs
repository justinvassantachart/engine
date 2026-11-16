import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFile, writeFile } from 'node:fs/promises';

import { proveInitialization } from './initialization-proof.mjs';

const policy = JSON.parse(await readFile('owned/source-policy.json'));
const source = process.env.ENGINE_PACKAGE_SOURCE;
assert(source && /^[a-f0-9]{40}$/.test(source), 'Exact package source is required');
const run = (command, args) => execFileSync(command, args, { stdio: 'inherit' });
const git = (...args) => execFileSync('git', args, { encoding: 'utf8' });
const sha256 = (bytes) => createHash('sha256').update(bytes).digest('hex');
const json = (file, value) => writeFile(file, JSON.stringify(value, null, 2) + '\n');
const build = () => run('npx', ['vite', 'build', '--config', 'owned/vite.config.ts']);
const index = await readFile('src/ts/index.ts');
try {
  await writeFile('src/ts/index.ts', git('show', `${policy.upstreamBase}:src/ts/index.ts`));
  build();
  await json('initialization-before.json', await proveInitialization('before'));
} finally {
  await writeFile('src/ts/index.ts', index);
}
build();
await json('initialization-after.json', await proveInitialization('after'));
const pack = () => JSON.parse(execFileSync('npm', ['pack', '--json'], { encoding: 'utf8' }))[0];
const first = pack();
const firstBytes = await readFile(first.filename);
build();
const second = pack();
const secondBytes = await readFile(second.filename);
assert(
  firstBytes.equals(secondBytes),
  'Two independently rebuilt TypeScript distributions must pack identically'
);
await json('pack.json', [second]);
await writeFile('tarball.txt', second.filename);
await json('reproducibility.json', {
  source,
  scope:
    'Two clean Vite TypeScript distribution builds and npm packs from the same compiled Rust module',
  identical: true,
  package: second.filename,
  bytes: secondBytes.length,
  firstSha256: sha256(firstBytes),
  secondSha256: sha256(secondBytes),
  node: process.version,
  npm: execFileSync('npm', ['--version'], { encoding: 'utf8' }).trim()
});
const rustChanges = git(
  'diff',
  '--name-only',
  policy.upstreamBase,
  '--',
  'src',
  'Cargo.toml',
  'Cargo.lock'
)
  .trim()
  .split('\n')
  .filter((file) => file && !file.startsWith('src/ts/'));
assert.deepEqual(rustChanges, [], 'Rust must be unchanged from main plus PR23');
await json('compiled-wasm-lineage.json', {
  source,
  upstreamBase: policy.upstreamBase,
  rustSourceTree: git(
    'ls-tree',
    '-r',
    policy.upstreamBase,
    '--',
    'src',
    'Cargo.toml',
    'Cargo.lock',
    'rust-toolchain.toml'
  )
    .trim()
    .split('\n')
    .filter((line) => !line.includes('\tsrc/ts/')),
  build: 'cargo clean --release -p engine --target wasm32-unknown-unknown; npm run build:rs',
  rustc: execFileSync('rustc', ['--version'], { encoding: 'utf8' }).trim(),
  engineWasmSha256: sha256(await readFile('dist/engine_bg.wasm')),
  bytes: (await readFile('dist/engine_bg.wasm')).length,
  requiredBehaviorGate: 'npm run test:dap includes formatting/deque (67 steps)'
});
