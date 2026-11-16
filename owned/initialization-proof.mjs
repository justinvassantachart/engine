import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import path from 'node:path';
import { chromium, webkit } from 'playwright';

// Exercise the public API in independent browser realms. The same compiled Rust
// module is used before and after the sole TypeScript runtime patch.
export async function proveInitialization(mode) {
  assert(['before', 'after'].includes(mode));
  const server = createServer(async (request, response) => {
    response.setHeader('Cross-Origin-Opener-Policy', 'same-origin');
    response.setHeader('Cross-Origin-Embedder-Policy', 'require-corp');
    const pathname = new URL(request.url, 'http://localhost').pathname;
    if (pathname === '/') {
      response.setHeader('Content-Type', 'text/html');
      response.end(
        '<script type="module">import { Engine } from "/dist/debugger-sh.js"; window.Engine = Engine;</script>'
      );
      return;
    }
    if (!pathname.startsWith('/dist/') || pathname.includes('..')) {
      response.writeHead(404).end();
      return;
    }
    try {
      response.setHeader(
        'Content-Type',
        pathname.endsWith('.wasm') ? 'application/wasm' : 'text/javascript'
      );
      response.end(await readFile(path.resolve(`.${pathname}`)));
    } catch {
      response.writeHead(404).end();
    }
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  const url = `http://127.0.0.1:${server.address().port}/`;
  const results = [];
  try {
    for (const [name, type] of Object.entries({ chromium, webkit })) {
      const browser = await type.launch({ headless: true });
      try {
        const page = await browser.newPage();
        await page.goto(url);
        await page.waitForFunction(() => window.Engine);
        const compile = await page.evaluate(async () => {
          const original = WebAssembly.compile;
          let calls = 0;
          WebAssembly.compile = async (...args) => {
            calls++;
            if (calls === 1) throw new Error('injected cold initialization failure');
            return original(...args);
          };
          const attempt = async () => {
            try {
              const engine = await window.Engine.create();
              engine.dispose();
              return 'success';
            } catch (error) {
              return error.message;
            }
          };
          const concurrent = await Promise.all([attempt(), attempt()]);
          const retry = await attempt();
          return { calls, concurrent, retry };
        });
        assert.deepEqual(compile.concurrent, Array(2).fill('injected cold initialization failure'));
        assert.equal(compile.calls, mode === 'before' ? 1 : 2);
        assert.equal(
          compile.retry,
          mode === 'before' ? 'injected cold initialization failure' : 'success'
        );
        await page.close();
        const fetchPage = await browser.newPage();
        let requests = 0;
        await fetchPage.route('**/*.wasm', (route) => {
          requests++;
          return requests === 1 ? route.fulfill({ status: 503, body: '' }) : route.continue();
        });
        await fetchPage.goto(url);
        await fetchPage.waitForFunction(() => window.Engine);
        const attempts = await fetchPage.evaluate(async () => {
          const results = [];
          for (let i = 0; i < 2; i++) {
            try {
              const engine = await window.Engine.create();
              engine.dispose();
              results.push('success');
            } catch (error) {
              results.push(error.message);
            }
          }
          return results;
        });
        assert.notEqual(attempts[0], 'success');
        assert.equal(requests, mode === 'before' ? 1 : 2);
        if (mode === 'before') assert.equal(attempts[1], attempts[0]);
        else assert.equal(attempts[1], 'success');
        results.push({
          browser: name,
          version: browser.version(),
          compile,
          http503: { requests, attempts }
        });
      } finally {
        await browser.close();
      }
    }
  } finally {
    await new Promise((resolve) => server.close(resolve));
  }
  const sha256 = (bytes) => createHash('sha256').update(bytes).digest('hex');
  return {
    mode,
    source: process.env.ENGINE_PACKAGE_SOURCE,
    indexSha256: sha256(await readFile('src/ts/index.ts')),
    engineWasmSha256: sha256(await readFile('dist/engine_bg.wasm')),
    results
  };
}
