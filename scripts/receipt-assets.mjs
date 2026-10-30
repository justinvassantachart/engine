import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { readFile, writeFile, mkdir, mkdtemp, rename, rm, lstat } from 'node:fs/promises';
import { resolve, join } from 'node:path';
import { gunzipSync } from 'node:zlib';

export const RECEIPT_DIRECTORY = 'cpp-cache-receipt-v1';
export const RECEIPT_URL_MARKERS = Object.freeze({
  '__WEBIDE_RECEIPT_COMPILER_ASSET__': './cpp-cache-receipt-v1/llvm.core.wasm',
  '__WEBIDE_RECEIPT_SYSROOT_ASSET__': './cpp-cache-receipt-v1/llvm-resources.tar.gz'
});
const fixed = {
  profile: 'cache-receipt-v1', capability: 'webide-object-cache-v1',
  engineAbi: 'wasm32-wasip1/c++23/legacy-eh/receipt-v1',
  transportAbi: 'webide_compiler.cache_receipt_v1(i32,i32)->i32',
  exceptionsSha256: '20986166552953de0e3ca5c1e9f341e7aec8b9ba54dcb244d0d2b2f9ca843ab4'
};
const assetNames = { compiler: 'llvm.core.wasm', sysroot: 'llvm-resources.tar.gz', notices: 'THIRD_PARTY_NOTICES.txt', provenance: 'provenance.json' };
const sha = bytes => createHash('sha256').update(bytes).digest('hex');
const digest = value => typeof value === 'string' && /^[a-f0-9]{64}$/.test(value);
const keys = (value, expected, label) => {
  assert(value && typeof value === 'object' && !Array.isArray(value), label + ' must be an object');
  assert.deepEqual(Object.keys(value).sort(), [...expected].sort(), 'Unexpected ' + label + ' fields');
};

/** This manifest is reviewed source, compiled into the package. It is never an
 * environment override, runtime registry, caller digest, or downloaded trust input. */
export function validateReceiptProfile(profile) {
  if (profile === null) return null;
  keys(profile, ['schemaVersion', ...Object.keys(fixed), ...Object.keys(assetNames), 'source', 'acceptance', 'release'], 'receipt profile');
  assert.equal(profile.schemaVersion, 1);
  for (const [key, value] of Object.entries(fixed)) assert.equal(profile[key], value, 'Receipt profile ' + key);
  keys(profile.release, ['repository', 'tag'], 'release');
  assert.equal(profile.release.repository, 'justinvassantachart/engine');
  assert.match(profile.release.tag, /^clang-cache-receipt-v\d+\.\d+\.\d+$/);
  keys(profile.source, ['llvmSha', 'yowaspSha', 'wasiLibcSha', 'sdkVersion', 'sdkSha256', 'buildType', 'assertions', 'recipeSha256', 'patchSha256'], 'source');
  assert.equal(profile.source.llvmSha, '9560ae0f2cc440e4fc891fddbc119da6f56daa59');
  assert.equal(profile.source.yowaspSha, '409b7dfdbd5ed12545eed706808fd423f1f692c6');
  assert.equal(profile.source.wasiLibcSha, '2fc32bc81b9f07f8d9525edea59bfbaf760c06d6');
  assert.equal(profile.source.sdkVersion, 29);
  assert.equal(profile.source.sdkSha256, '87d1d1a2879d139cdc624b968efad3d4a97b8078cdff95e63ac88ecafd1a0171');
  assert.equal(profile.source.buildType, 'MinSizeRel');
  assert.equal(profile.source.assertions, true);
  assert(digest(profile.source.recipeSha256) && digest(profile.source.patchSha256));
  keys(profile.acceptance, ['controlEvidenceSha256', 'observerEvidenceSha256'], 'acceptance');
  assert(Object.values(profile.acceptance).every(digest), 'Missing completed control/observer evidence identities');
  for (const [role, name] of Object.entries(assetNames)) {
    const asset = profile[role];
    keys(asset, ['name', 'sha256', 'byteLength', ...(role === 'sysroot' ? ['tarSha256', 'tarByteLength'] : [])], role);
    assert.equal(asset.name, name);
    assert(digest(asset.sha256), 'Invalid ' + role + ' digest');
    const limit = role === 'compiler' ? 96 * 1024 * 1024 : role === 'sysroot' ? 32 * 1024 * 1024 : 4 * 1024 * 1024;
    assert(Number.isSafeInteger(asset.byteLength) && asset.byteLength > 0 && asset.byteLength <= limit, 'Invalid ' + role + ' size');
    if (role === 'sysroot') {
      assert(digest(asset.tarSha256));
      assert(Number.isSafeInteger(asset.tarByteLength) && asset.tarByteLength > 0 && asset.tarByteLength <= 256 * 1024 * 1024);
    }
  }
  return profile;
}

export async function readReceiptProfile(root) {
  return validateReceiptProfile(JSON.parse(await readFile(join(root, 'src/ts/receipt-toolchain-profile.json'), 'utf8')));
}

async function readRegular(path) {
  const metadata = await lstat(path);
  assert(metadata.isFile() && !metadata.isSymbolicLink(), 'Compiler asset must be a regular file: ' + path);
  return readFile(path);
}

export function verifyReceiptAsset(role, bytes, profile) {
  const descriptor = profile[role];
  assert.equal(bytes.length, descriptor.byteLength, role + ' size mismatch');
  assert.equal(sha(bytes), descriptor.sha256, role + ' digest mismatch');
  if (role === 'sysroot') {
    const archive = gunzipSync(bytes, { maxOutputLength: 256 * 1024 * 1024 });
    assert.equal(archive.length, descriptor.tarByteLength, 'Raw sysroot archive size mismatch');
    assert.equal(sha(archive), descriptor.tarSha256, 'Raw sysroot archive digest mismatch');
  }
}

/** Missing assets may be downloaded only from the source-fixed release. Present
 * but corrupt assets fail rather than being silently replaced. The candidate
 * builder may seed exact verified bytes locally before immutable publication. */
export async function prepareReceiptAssets(root, { download = false } = {}) {
  const profile = await readReceiptProfile(root);
  if (!profile) return null;
  assert.equal(sha(await readRegular(join(root, 'assets/cpp-exceptions/runtime.tar.gz'))), profile.exceptionsSha256,
    'Receipt profile exception runtime differs from the engine payload');
  const directory = join(root, '.compiler-assets', RECEIPT_DIRECTORY);
  await mkdir(directory, { recursive: true });
  const pending = await mkdtemp(join(directory, '.prepare-'));
  const assets = [];
  try {
    for (const role of Object.keys(assetNames)) {
      const descriptor = profile[role], target = join(directory, descriptor.name);
      let bytes, missing = false;
      try { bytes = await readRegular(target); }
      catch (error) { if (error.code !== 'ENOENT') throw error; missing = true; }
      if (missing) {
        assert(download, 'Missing verified compiler asset; run npm run prepare:receipt-assets: ' + descriptor.name);
        execFileSync('gh', ['release', 'download', profile.release.tag, '--repo', profile.release.repository,
          '--pattern', descriptor.name, '--dir', pending], { stdio: 'inherit' });
        bytes = await readRegular(join(pending, descriptor.name));
      }
      verifyReceiptAsset(role, bytes, profile);
      assets.push({ role, descriptor, target, bytes, missing });
    }
    // No new cached member is published until every member verifies.
    for (const asset of assets) if (asset.missing) await rename(join(pending, asset.descriptor.name), asset.target);
    return { profile, directory, assets };
  } finally { await rm(pending, { recursive: true, force: true }); }
}

export function replaceReceiptUrlMarkers(code, { requireMarkers = false } = {}) {
  // Library mode preserves this comment; leaving it in the published ESM would
  // also suppress the consumer's required same-origin asset emission.
  code = code.replace(/(new\s+URL\(\s*)\/\*\s*@vite-ignore\s*\*\/\s*(?=(["'])__WEBIDE_RECEIPT_(?:COMPILER|SYSROOT)_ASSET__\2)/g, '$1');
  let replacements = 0;
  for (const [marker, relative] of Object.entries(RECEIPT_URL_MARKERS)) {
    const count = code.split(marker).length - 1;
    if (requireMarkers) assert.equal(count, 1, 'Expected one packaged asset URL: ' + marker);
    replacements += count;
    code = code.replaceAll(marker, relative);
  }
  return { code, replacements };
}

/** Run after Vite's library asset transforms. Keeping literal new URL calls in
 * the packed ESM lets the consumer bundler emit same-origin immutable assets. */
export function receiptAssetsPlugin(root) {
  let verified;
  return {
    name: 'verified-receipt-package-assets',
    async buildStart() { verified = await prepareReceiptAssets(root); },
    closeBundle: { sequential: true, async handler() {
      if (!verified) return;
      assert.deepEqual(await readReceiptProfile(root), verified.profile, 'Compiler profile changed during the build');
      const dist = resolve(root, 'dist');
      const esm = join(dist, 'debugger-sh.js'), umd = join(dist, 'debugger-sh.umd.cjs');
      // Validate the ESM transform before changing any output files.
      const replaced = replaceReceiptUrlMarkers(await readFile(esm, 'utf8'), { requireMarkers: true });
      const replacedUmd = replaceReceiptUrlMarkers(await readFile(umd, 'utf8'));
      const staged = await mkdtemp(join(dist, '.receipt-assets-'));
      try {
        for (const asset of verified.assets) await writeFile(join(staged, asset.descriptor.name), asset.bytes);
        await writeFile(join(staged, 'profile.json'), JSON.stringify(verified.profile, null, 2) + '\n');
        const target = join(dist, RECEIPT_DIRECTORY);
        await rm(target, { recursive: true, force: true });
        await rename(staged, target);
        await writeFile(esm, replaced.code);
        await writeFile(umd, replacedUmd.code);
      } finally { await rm(staged, { recursive: true, force: true }); }
    } }
  };
}
