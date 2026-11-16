import fs from 'node:fs';
import path from 'node:path';
import { defineConfig, mergeConfig, type Plugin } from 'vite';

import upstream from '../vite.config';

const lock = JSON.parse(fs.readFileSync('owned/toolchain-lock.json', 'utf8'));
const mirror = 'https://justinvassantachart.github.io/engine/toolchains/sha256/';

function manifest(name: string) {
  const original = JSON.parse(fs.readFileSync(`tools/toolchains/${name}/provenance.json`, 'utf8'));
  if (original.upstream.revision !== lock[name].revision)
    throw new Error(`Unreviewed ${name} revision`);
  return {
    ...original,
    originArtifacts: original.artifacts,
    artifacts: Object.fromEntries(
      Object.entries(original.artifacts).map(([key, url]) => {
        const digest = lock[name].sha256[key];
        if (!digest) throw new Error(`Unpinned ${name}/${key}`);
        return [key, `${mirror}${digest}/${path.basename(new URL(url as string).pathname)}`];
      })
    ),
    sha256: lock[name].sha256
  };
}

// Distribution policy belongs to this owned recipe. Upstream's build and source
// manifests are unchanged; its WASM/worker and public APIs are built from source.
function distribution(): Plugin {
  return {
    name: 'owned-distribution',
    enforce: 'pre',
    resolveFileUrl({ fileName }) {
      if (fileName.endsWith('.wasm')) return JSON.stringify(`./${fileName}`);
    },
    load(id) {
      const toolchain = /\/tools\/toolchains\/([^/]+)\/provenance\.json$/.exec(id)?.[1];
      if (toolchain) return JSON.stringify(manifest(toolchain));
      if (!id.endsWith('.wasm')) return;
      const reference = this.emitFile({
        type: 'asset',
        fileName: path.basename(id),
        source: fs.readFileSync(id)
      });
      return `export default new URL(import.meta.ROLLUP_FILE_URL_${reference}, import.meta.url);`;
    },
    closeBundle() {
      fs.mkdirSync('dist', { recursive: true });
      fs.writeFileSync(
        'dist/toolchain-provenance.json',
        JSON.stringify(
          {
            cpp: manifest('llvm'),
            python: manifest('python'),
            rust: manifest('rust')
          },
          null,
          2
        ) + '\n'
      );
      if (process.env.ENGINE_PACKAGE_SOURCE)
        fs.writeFileSync(
          'dist/source.json',
          JSON.stringify({
            source: process.env.ENGINE_PACKAGE_SOURCE,
            repository: 'justinvassantachart/engine',
            upstreamMain: 'd14753f4ee78f3655db8f555f685102f64b61dc2',
            upstreamBase: '28b6738e5746e2c2950c5b60e0b5a7cc746429b0',
            runtimePatches: ['retry-failed-initialization']
          }) + '\n'
        );
    }
  };
}

export default defineConfig(
  mergeConfig(upstream, {
    plugins: [distribution()],
    worker: { plugins: () => [distribution(), ...(upstream.worker?.plugins?.() ?? [])] }
  })
);
