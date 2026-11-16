import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';

const directory = process.argv[2];
const lock = JSON.parse(await readFile(new URL('./toolchain-lock.json', import.meta.url)));

for (const toolchain of ['llvm', 'python', 'rust']) {
  const manifest = JSON.parse(
    await readFile(new URL(`../tools/toolchains/${toolchain}/provenance.json`, import.meta.url))
  );
  assert.equal(manifest.upstream.revision, lock[toolchain].revision);
  manifest.sha256 = lock[toolchain].sha256;
  for (const [name, url] of Object.entries(manifest.artifacts)) {
    const filename = path.basename(new URL(url).pathname);
    const base = process.env.ENGINE_TOOLCHAIN_BASE_URL;
    const mirror =
      base &&
      new URL(`${manifest.sha256[name]}/${filename}`, base.endsWith('/') ? base : `${base}/`).href;
    // Existing checksums can be rebuilt even after the original publisher moves
    // a versioned URL. New checksums are verified before the first mirror upload.
    let downloadUrl = mirror ?? url;
    let response = await fetch(downloadUrl);
    if (mirror && response.status === 404) {
      downloadUrl = url;
      response = await fetch(downloadUrl);
    }
    assert(response.ok, `${downloadUrl}: ${response.status}`);
    const bytes = new Uint8Array(await response.arrayBuffer());
    const sha256 = createHash('sha256').update(bytes).digest('hex');
    assert.equal(sha256, manifest.sha256[name], url);
    if (name !== 'sysroot') assert.deepEqual([...bytes.slice(0, 4)], [0, 97, 115, 109]);
    if (directory) {
      const pages = path.join(directory, 'pages', 'toolchains', 'sha256', sha256);
      const release = path.join(directory, 'release-assets');
      await mkdir(pages, { recursive: true });
      await mkdir(release, { recursive: true });
      await writeFile(path.join(pages, filename), bytes);
      await writeFile(path.join(release, `${toolchain}-${filename}`), bytes);
    }
    console.log(JSON.stringify({ toolchain, name, url, downloadUrl, sha256, size: bytes.length }));
  }
}
