// Synthetic package-transport tests. These do not certify a compiler, create a
// product capability, or satisfy any control/observer acceptance gate.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, writeFile, readFile, rm, symlink, access, realpath } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { gzipSync } from 'node:zlib';
import { createHash } from 'node:crypto';
import { build } from 'vite';
import { validateReceiptProfile, prepareReceiptAssets, replaceReceiptUrlMarkers, receiptAssetsPlugin } from '../../scripts/receipt-assets.mjs';

const repository = fileURLToPath(new URL('../..', import.meta.url));
const sha = bytes => createHash('sha256').update(bytes).digest('hex');
async function fixture() {
  const root = await realpath(await mkdtemp(join(tmpdir(), 'engine-receipt-transport-')));
  const compiler = Buffer.from([0, 97, 115, 109, 1, 0, 0, 0]);
  const archive = Buffer.from('synthetic archive transport bytes; not a compiler sysroot');
  const files = { compiler, sysroot: gzipSync(archive), notices: Buffer.from('Synthetic license notice\n'), provenance: Buffer.from('{"synthetic":true}\n') };
  const names = { compiler: 'llvm.core.wasm', sysroot: 'llvm-resources.tar.gz', notices: 'THIRD_PARTY_NOTICES.txt', provenance: 'provenance.json' };
  const profile = {
    schemaVersion: 1, profile: 'cache-receipt-v1', capability: 'webide-object-cache-v1',
    engineAbi: 'wasm32-wasip1/c++23/legacy-eh/receipt-v1', transportAbi: 'webide_compiler.cache_receipt_v1(i32,i32)->i32',
    exceptionsSha256: '20986166552953de0e3ca5c1e9f341e7aec8b9ba54dcb244d0d2b2f9ca843ab4',
    ...Object.fromEntries(Object.entries(files).map(([role, bytes]) => [role, { name: names[role], sha256: sha(bytes), byteLength: bytes.length }])),
    source: { llvmSha: '9560ae0f2cc440e4fc891fddbc119da6f56daa59', yowaspSha: '409b7dfdbd5ed12545eed706808fd423f1f692c6',
      wasiLibcSha: '2fc32bc81b9f07f8d9525edea59bfbaf760c06d6', sdkVersion: 29, sdkSha256: '87d1d1a2879d139cdc624b968efad3d4a97b8078cdff95e63ac88ecafd1a0171',
      buildType: 'MinSizeRel', assertions: true, recipeSha256: 'a'.repeat(64), patchSha256: 'b'.repeat(64) },
    acceptance: { controlEvidenceSha256: 'c'.repeat(64), observerEvidenceSha256: 'd'.repeat(64) },
    release: { repository: 'justinvassantachart/engine', tag: 'clang-cache-receipt-v0.0.0' }
  };
  Object.assign(profile.sysroot, { tarSha256: sha(archive), tarByteLength: archive.length });
  await mkdir(join(root, 'src/ts'), { recursive: true });
  await mkdir(join(root, 'assets/cpp-exceptions'), { recursive: true });
  await writeFile(join(root, 'assets/cpp-exceptions/runtime.tar.gz'), await readFile(join(repository, 'assets/cpp-exceptions/runtime.tar.gz')));
  await writeFile(join(root, 'src/ts/receipt-toolchain-profile.json'), JSON.stringify(profile));
  const cache = join(root, '.compiler-assets/cpp-cache-receipt-v1');
  await mkdir(cache, { recursive: true });
  for (const [role, bytes] of Object.entries(files)) await writeFile(join(cache, names[role]), bytes);
  return { root, cache, profile, files };
}

test('a legacy-only source build neither fetches nor requires compiler assets', async () => {
  const root = await mkdtemp(join(tmpdir(), 'engine-legacy-no-assets-'));
  try {
    await mkdir(join(root, 'src/ts'), { recursive: true });
    await writeFile(join(root, 'src/ts/receipt-toolchain-profile.json'), 'null\n');
    assert.equal(await prepareReceiptAssets(root, { download: true }), null);
    await assert.rejects(access(join(root, '.compiler-assets')), { code: 'ENOENT' });
  } finally { await rm(root, { recursive: true, force: true }); }
});

test('source-fixed profile rejects alternate source, ABI, repository, unsafe names and incomplete evidence', async () => {
  const state = await fixture();
  try {
    assert.equal(validateReceiptProfile(state.profile), state.profile);
    const mutations = [
      profile => { profile.source.sdkVersion = 32; },
      profile => { profile.engineAbi = 'caller-authorized'; },
      profile => { profile.release.repository = 'someone/other'; },
      profile => { profile.release.tag = '../tag'; },
      profile => { profile.compiler.name = '../llvm.core.wasm'; },
      profile => { profile.compiler.byteLength = 100 * 1024 * 1024; },
      profile => { profile.sysroot.byteLength = 32 * 1024 * 1024 + 1; },
      profile => { delete profile.acceptance.observerEvidenceSha256; },
      profile => { profile.extra = true; },
      profile => { profile.exceptionsSha256 = 'e'.repeat(64); }
    ];
    for (const mutate of mutations) {
      const profile = structuredClone(state.profile); mutate(profile);
      assert.throws(() => validateReceiptProfile(profile));
    }
  } finally { await rm(state.root, { recursive: true, force: true }); }
});

test('cached asset corruption, raw sysroot mismatch and symlinks fail without replacing evidence', async () => {
  const state = await fixture();
  try {
    assert.equal((await prepareReceiptAssets(state.root)).assets.length, 4);
    const compilerPath = join(state.cache, 'llvm.core.wasm');
    await writeFile(compilerPath, 'corrupted');
    await assert.rejects(prepareReceiptAssets(state.root), /size mismatch/);
    assert.equal(await readFile(compilerPath, 'utf8'), 'corrupted');
    await writeFile(compilerPath, state.files.compiler);
    const profile = structuredClone(state.profile); profile.sysroot.tarSha256 = 'f'.repeat(64);
    await writeFile(join(state.root, 'src/ts/receipt-toolchain-profile.json'), JSON.stringify(profile));
    await assert.rejects(prepareReceiptAssets(state.root), /Raw sysroot archive digest mismatch/);
    await writeFile(join(state.root, 'src/ts/receipt-toolchain-profile.json'), JSON.stringify(state.profile));
    const outside = join(state.root, 'compiler-copy.wasm'); await writeFile(outside, state.files.compiler);
    await rm(compilerPath); await symlink(outside, compilerPath);
    await assert.rejects(prepareReceiptAssets(state.root), /regular file/);
  } finally { await rm(state.root, { recursive: true, force: true }); }
});

test('package URL replacement requires each exact static marker', () => {
  assert.throws(() => replaceReceiptUrlMarkers('no asset references', { requireMarkers: true }));
  assert.throws(() => replaceReceiptUrlMarkers('__WEBIDE_RECEIPT_COMPILER_ASSET__'.repeat(2), { requireMarkers: true }));
  const result = replaceReceiptUrlMarkers('new URL(/* @vite-ignore */ "__WEBIDE_RECEIPT_COMPILER_ASSET__", import.meta.url); new URL(/* @vite-ignore */ "__WEBIDE_RECEIPT_SYSROOT_ASSET__", import.meta.url); new URL(/* @vite-ignore */ "unrelated", import.meta.url)', { requireMarkers: true });
  assert.equal(result.replacements, 2);
  assert.match(result.code, /new URL\("\.\/cpp-cache-receipt-v1\/llvm\.core\.wasm", import\.meta\.url\)/);
  assert(!result.code.includes('__WEBIDE_'));
  assert.equal(result.code.match(/@vite-ignore/g)?.length, 1, 'Unrelated dynamic URL semantics must stay unchanged');
});

test('library packs external bytes and consumer Vite emits exact same-origin assets', async () => {
  const state = await fixture();
  try {
    await writeFile(join(state.root, 'entry.js'), `export const urls = [
      new URL(/* @vite-ignore */ '__WEBIDE_RECEIPT_COMPILER_ASSET__', import.meta.url).href,
      new URL(/* @vite-ignore */ '__WEBIDE_RECEIPT_SYSROOT_ASSET__', import.meta.url).href];`);
    await build({ configFile: false, root: state.root, logLevel: 'silent', plugins: [receiptAssetsPlugin(state.root)],
      build: { target: 'esnext', lib: { entry: join(state.root, 'entry.js'), name: 'SyntheticTransportFixture',
        formats: ['es', 'umd'], fileName: format => format === 'es' ? 'debugger-sh.js' : 'debugger-sh.umd.cjs' } } });
    const esm = await readFile(join(state.root, 'dist/debugger-sh.js'), 'utf8');
    assert(esm.includes('./cpp-cache-receipt-v1/llvm.core.wasm'));
    assert(!esm.includes('__WEBIDE_') && !esm.includes('data:application/wasm'));
    assert(!esm.includes('@vite-ignore'), 'Consumer must be permitted to emit the packaged assets');
    for (const role of Object.keys(state.files)) {
      assert.deepEqual(await readFile(join(state.root, 'dist/cpp-cache-receipt-v1', state.profile[role].name)), state.files[role]);
    }
    const consumer = join(state.root, 'consumer'); await mkdir(consumer);
    await writeFile(join(consumer, 'index.html'), '<script type="module" src="/main.js"></script>');
    await writeFile(join(consumer, 'main.js'), 'import { urls } from "../dist/debugger-sh.js"; globalThis.fixtureUrls = urls;');
    const result = await build({ configFile: false, root: consumer, logLevel: 'silent',
      // Synthetic bytes are deliberately tiny; force their normal external-asset
      // path. The actual compiler/sysroot far exceed the production inline limit.
      build: { target: 'esnext', assetsInlineLimit: 0, write: false } });
    const output = result.output ?? result[0].output;
    const assets = output.filter(entry => entry.type === 'asset');
    for (const bytes of [state.files.compiler, state.files.sysroot]) {
      assert(assets.some(asset => sha(asset.source) === sha(bytes)), 'Consumer did not emit the exact packaged asset');
    }
    assert(output.filter(entry => entry.type === 'chunk').every(entry => !entry.code.includes('__WEBIDE_')));
  } finally { await rm(state.root, { recursive: true, force: true }); }
});
