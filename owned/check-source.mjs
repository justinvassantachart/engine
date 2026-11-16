import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { readFile } from 'node:fs/promises';

const policy = JSON.parse(await readFile(new URL('./source-policy.json', import.meta.url)));
const git = (...args) => execFileSync('git', args, { encoding: 'utf8' });
const changed = git('diff', '--name-only', policy.upstreamBase, 'HEAD')
  .trim()
  .split('\n')
  .filter(Boolean);
assert(
  changed.every(
    (file) =>
      file.startsWith('owned/') ||
      file === '.github/workflows/owned-migration-package.yml' ||
      file === 'src/ts/index.ts'
  ),
  `Unexpected upstream changes: ${changed}`
);
assert.equal(
  git('diff', policy.upstreamBase, 'HEAD', '--', 'src'),
  await readFile(new URL('./patches/retry-failed-initialization.patch', import.meta.url), 'utf8'),
  'Runtime source must differ from main plus PR23 by only the reviewed initialization patch'
);
console.log(
  JSON.stringify({
    upstreamBase: policy.upstreamBase,
    runtimePatches: policy.runtimePatches,
    changed
  })
);
