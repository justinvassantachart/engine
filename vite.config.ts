import fs from 'fs';
import path from 'path';
import { defineConfig, PluginOption } from 'vite';
import dts from 'vite-plugin-dts';

const outDir = 'dist';

export default defineConfig({
  plugins: [wasm(), dts()],
  worker: {
    format: 'es',
    plugins: () => [wasm()],
  },
  build: {
    outDir,
    lib: {
      entry: 'src/ts/index.ts',
      name: 'runtime',
    },
  },
});

function wasm(): PluginOption {
  return {
    name: 'fix-wasm-import',
    async closeBundle() {
      const file = path.join(outDir, 'runtime.js');
      if (!fs.existsSync(file)) return; // in worker builds this file doesn't exist yet
      let js = fs.readFileSync(file, { encoding: 'utf-8' });
      js = js.replaceAll(/(["'`])data:application\/wasm[^"'`]*\1/g, '""');
      fs.writeFileSync(file, js, { encoding: 'utf-8' });
    },

    /**
     * Set up `.wasm` import so that importing it returns a buffer.
     * This is taken from wasmer-js: https://github.com/wasmerio/wasmer-js/blob/main/rollup.config.mjs
     */
    async load(id) {
      if (!id.endsWith('.wasm')) return;
      const binary = await fs.readFileSync(id);
      const base64 = binary.toString('base64');
      return `
          const src = ${JSON.stringify(base64)};
          const buf = Uint8Array.from(atob(src), c => c.charCodeAt(0));
          export default buf;
        `;
    },
  };
}
