import { playwright } from '@vitest/browser-playwright';
import path from 'node:path';
import { defineConfig } from 'vitest/config';

const headers = {
  'Cross-Origin-Opener-Policy': 'same-origin',
  'Cross-Origin-Embedder-Policy': 'require-corp'
};
export default defineConfig({
  root: path.resolve(import.meta.dirname, '../tests/browser'),
  plugins: [
    {
      name: 'isolation-on-all-responses',
      configureServer(server) {
        server.middlewares.use((_request, response, next) => {
          for (const [name, value] of Object.entries(headers)) response.setHeader(name, value);
          next();
        });
      }
    }
  ],
  resolve: {
    alias: { 'debugger-sh': path.resolve(import.meta.dirname, '../dist/debugger-sh.js') }
  },
  optimizeDeps: { exclude: ['debugger-sh'] },
  server: { headers, fs: { allow: ['../..'] } },
  test: {
    include: ['tests/**/*.ts', '../../owned/tests/*.ts'],
    testTimeout: 120_000,
    fileParallelism: false,
    browser: {
      enabled: true,
      headless: true,
      provider: playwright(),
      instances: [{ browser: process.env.ENGINE_BROWSER === 'webkit' ? 'webkit' : 'chromium' }]
    }
  }
});
