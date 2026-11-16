import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtemp, readFile, realpath, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { chromium, webkit } from 'playwright';
import { build, preview } from 'vite';

// Test the installed tarball through a separate production consumer build. The
// consumer cannot import the Engine source tree or borrow its generated pkg/.
const archive = path.resolve(process.argv[2]);
const root = await realpath(await mkdtemp(path.join(tmpdir(), 'engine-consumer-')));
let server;
try {
  await writeFile(
    path.join(root, 'package.json'),
    JSON.stringify({ private: true, type: 'module' })
  );
  execFileSync('npm', ['install', '--ignore-scripts', '--no-audit', '--no-fund', archive], {
    cwd: root,
    stdio: 'inherit'
  });
  const pkg = JSON.parse(await readFile(path.join(root, 'node_modules/debugger-sh/package.json')));
  for (const file of [pkg.exports['.'].import, pkg.exports['.'].types, 'dist/engine_bg.wasm']) {
    assert((await readFile(path.join(root, 'node_modules/debugger-sh', file))).length > 0, file);
  }
  await writeFile(path.join(root, 'index.html'), '<script type="module" src="/main.js"></script>');
  await writeFile(
    path.join(root, 'main.js'),
    `
    import { Engine, Duplex } from 'debugger-sh';
    window.coldCreate = () => Engine.create();
    window.smoke = async () => {
      const engine = await Engine.create({ fs: { 'asset.bin': new Uint8Array([0,255,7]), 'main.cpp': '#include <cstdio>\\nint main() { puts("cpp-ok"); }' } });
      const capture = async (proc) => { let output = ''; proc.stdout.on('text', text => output += text); const result = await proc; if (result.exitCode) throw new Error(JSON.stringify(result)); return output; };
      try {
        if ((await engine.fs.readFile('/asset.bin'))[1] !== 255) throw new Error('binary mount');
        if (await capture(engine.run('cpp')) !== 'cpp-ok\\n') throw new Error('cpp output');
        const device = new Duplex();
        const proc = engine.run('python', ['-c', "import os; fd=os.open('/dev/host', os.O_RDWR); os.write(fd, b'request\\\\n'); print(os.read(fd, 99).decode())"], { devices: { '/dev/host': device }, stdin: true });
        const result = capture(proc);
        const request = await device.read();
        if (new TextDecoder().decode(request) !== 'request\\n') throw new Error('duplex request');
        await device.write('response');
        if (await result !== 'response\\n') throw new Error('duplex response');
        for (let attempt = 0; attempt < 2; attempt++) {
          const blocked = engine.run('python', ['-c', 'print("ready"); input()'], { stdin: true });
          await new Promise((resolve, reject) => {
            const timer = setTimeout(() => reject(new Error('input startup timeout')), 30000);
            blocked.stdout.on('text', text => {
              if (text.includes('ready')) { clearTimeout(timer); resolve(); }
            });
          });
          blocked.kill();
          if (!(await blocked).killed) throw new Error('cancel');
          if (await capture(engine.run('python', ['-c', 'print("rerun-ok")'])) !== 'rerun-ok\\n') throw new Error('rerun');
        }
        return { cpp: true, python: true, binary: true, duplex: true, stopRerun: true };
      } finally { engine.dispose(); }
    };
  `
  );
  await build({ root, configFile: false, base: './', logLevel: 'warn' });
  server = await preview({
    root,
    configFile: false,
    plugins: [
      {
        name: 'consumer-isolation',
        configurePreviewServer(server) {
          // Install before static serving so cached worker responses (304) retain
          // isolation headers, just like fresh responses.
          server.middlewares.use((_request, response, next) => {
            response.setHeader('Cross-Origin-Opener-Policy', 'same-origin');
            response.setHeader('Cross-Origin-Embedder-Policy', 'require-corp');
            next();
          });
        }
      }
    ],
    preview: {
      port: 0,
      headers: {
        'Cross-Origin-Opener-Policy': 'same-origin',
        'Cross-Origin-Embedder-Policy': 'require-corp'
      }
    }
  });
  for (const name of (process.env.ENGINE_BROWSERS ?? 'chromium,webkit').split(',')) {
    const browser = await { chromium, webkit }[name].launch({ headless: true });
    try {
      const page = await browser.newPage();
      page.setDefaultTimeout(180_000);
      const failures = [];
      page.on('pageerror', (error) => failures.push(error.message));
      page.on('requestfailed', (request) =>
        failures.push(`${request.url()}: ${request.failure()?.errorText}`)
      );
      await page.goto(server.resolvedUrls.local[0]);
      await page.waitForFunction(() => typeof window.smoke === 'function');
      // A real failed HTTP response must not poison later initialization.
      await page.route('**/*.wasm', (route) => route.fulfill({ status: 503, body: '' }), {
        times: 1
      });
      assert.equal(
        await page.evaluate(async () => {
          try {
            await window.coldCreate();
            return 'unexpected success';
          } catch {
            return 'cold load failed';
          }
        }),
        'cold load failed'
      );
      const result = await page
        .evaluate(() => window.smoke())
        .catch((error) => {
          throw new Error(`${name} consumer smoke failed: ${failures.join('; ')}`, {
            cause: error
          });
        });
      assert.deepEqual(failures, []);
      console.log(
        JSON.stringify({
          browser: name,
          version: browser.version(),
          package: pkg.version,
          ...result
        })
      );
    } finally {
      await browser.close();
    }
  }
} finally {
  await server?.close();
  await rm(root, { recursive: true, force: true });
}
