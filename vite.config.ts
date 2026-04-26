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
  const packageJson = JSON.parse(fs.readFileSync('package.json', 'utf-8')) as {
    name: string;
    version: string;
  };
  const packageName = packageJson.name;
  const packageVersion = packageJson.version;
  const npmDistUrl = `https://cdn.jsdelivr.net/npm/${packageName}@${packageVersion}/dist`;

  return {
    name: 'fix-wasm-import',
    async closeBundle() {
      const pkgDir = 'pkg';
      if (fs.existsSync(pkgDir)) {
        const wasmFiles = fs.readdirSync(pkgDir).filter((file) => file.endsWith('.wasm'));
        fs.mkdirSync(outDir, { recursive: true });
        for (const wasmFile of wasmFiles) {
          fs.copyFileSync(path.join(pkgDir, wasmFile), path.join(outDir, wasmFile));
        }
      }

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
      const devPort = process.env.WASM_DEV_PORT;
      const wasmFile = path.basename(id);
      if (devPort) {
        return `export default new URL("http://localhost:${devPort}/${wasmFile}");`;
      }
      return `export default new URL(${JSON.stringify(`${npmDistUrl}/${wasmFile}`)});`;
    },
  };
}
