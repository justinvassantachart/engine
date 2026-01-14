import fs from 'fs';
import path from 'path';
import { defineConfig } from 'vite';
import dts from 'vite-plugin-dts';

const outDir = 'dist';

export default defineConfig({
  plugins: [
    dts(),
    {
      name: 'fix-wasm-import',
      async closeBundle() {
        const file = path.join(outDir, 'runtime.js');
        let js = fs.readFileSync(file, { encoding: 'utf-8' });
        js = js.replaceAll(/(["'`])data:application\/wasm[^"'`]*\1/g, '""');
        fs.writeFileSync(file, js, { encoding: 'utf-8' });
      },
      async load(id) {
        if (!id.endsWith('.wasm')) return;
        const binary = await fs.readFileSync(id);
        const base64 = binary.toString('base64');
        return `
var isNode = typeof process !== 'undefined' && process.versions != null && process.versions.node != null;
const src = ${JSON.stringify(base64)};

let buf = undefined;
if (isNode) {
  buf = Buffer.from(src, 'base64');
}
else {
  buf = Uint8Array.from(atob(src), c => c.charCodeAt(0));
}
export default buf;
`;
      },
    },
  ],
  build: {
    outDir,
    lib: {
      entry: 'src/ts/index.ts',
      name: 'runtime',
    },
  },
});
