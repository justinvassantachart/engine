import { serve } from 'bun';
import chalk from 'chalk';
import { type ChildProcess, spawn } from 'node:child_process';
import { watch } from 'node:fs';
import { readFile, rm, writeFile } from 'node:fs/promises';
import net from 'node:net';
import path from 'node:path';
import process from 'node:process';
import { fileURLToPath } from 'node:url';
import yoctoSpinner from 'yocto-spinner';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(HERE, '../..');
const SRC = path.join(ROOT, 'src');
const SRC_TS = path.join(SRC, 'ts');
const PKG = path.join(ROOT, 'pkg');
const START_PORT = 8000;
const BUILD_MARKER = path.join(ROOT, 'node_modules/build.lock');

const spinner = yoctoSpinner();

let pendingTs = true;
let pendingRs = true;
let port = 0;
let activeProcess: ChildProcess | null = null;
let isInitializing = true;
let isDraining = false;

function writeBuildOutput(output: string, message?: string) {
  output = output.trim();
  const text = `${message ? message + '\n\n' : ''}${output}`;
  if (output.length === 0) return;
  const delim = ` | `;
  console.log(chalk.dim(`\n${text}`.replaceAll('\n', '\n' + delim)));
  console.log();
}

async function run(label: string, cmd: string, args: string[], env?: Record<string, string>) {
  spinner.start(`Building ${label}...`);
  return new Promise<boolean>((resolve) => {
    const startedAt = performance.now();
    const elapsed = () =>
      chalk.bold(`${Math.round((performance.now() - startedAt) / 1000).toFixed(1)}s`);

    let output = '';
    const append = (chunk: Buffer) => (output += chunk.toString());

    const proc = spawn(cmd, args, {
      cwd: ROOT,
      env: { ...process.env, ...env },
      stdio: ['ignore', 'pipe', 'pipe'],
      shell: true,
    });

    activeProcess = proc;
    proc.stdout.on('data', append);
    proc.stderr.on('data', append);

    proc.on('close', (code, signal) => {
      if (signal === 'SIGTERM') {
        return resolve(false);
      }

      if (code === 0) {
        if (!isInitializing) spinner.success(`Built ${label} in ${elapsed()}`);
        return resolve(true);
      }

      spinner.error(`Building ${label} failed in ${elapsed()}`);
      writeBuildOutput(output);
      resolve(false);
    });

    proc.on('error', (err) => {
      spinner.error(`Building ${label} failed in ${elapsed()}`);
      writeBuildOutput(output, err instanceof Error ? err.message : String(err));
      resolve(false);
    });
  });
}

function queueBuild(kind: 'ts' | 'rs') {
  if (kind === 'rs') pendingRs = true;
  if (kind === 'ts') pendingTs = true;
  activeProcess?.kill();
  void drainBuildQueue();
}

async function drainBuildQueue() {
  if (isDraining) return;
  isDraining = true;
  await writeFile(BUILD_MARKER, 'building\n', 'utf8');
  try {
    while (pendingRs || pendingTs) {
      if (pendingRs) {
        pendingRs = false;
        await run('rust', 'npm', ['run', 'build:rs', '--', '--dev']);
      }
      if (pendingTs) {
        pendingTs = false;
        await run('typescript', 'npm', ['run', 'build:ts'], { WASM_DEV_PORT: String(port) });
      }
    }
  } finally {
    isDraining = false;
    await rm(BUILD_MARKER, { force: true });
  }
}

function watchTree() {
  watch(SRC_TS, { recursive: true }, () => {
    queueBuild('ts');
  });
  watch(SRC, { recursive: true }, (_, filename) => {
    if (!filename) return;
    if (filename.toString().startsWith('ts/')) return;
    queueBuild('rs');
  });
}

async function isPortOpen(candidate: number) {
  return new Promise<boolean>((resolve) => {
    const server = net.createServer();
    server.once('error', () => resolve(false));
    server.once('listening', () => {
      server.close(() => resolve(true));
    });
    server.listen(candidate, '127.0.0.1');
  });
}

async function findOpenPort(start: number) {
  let candidate = start;
  while (!(await isPortOpen(candidate))) candidate += 1;
  return candidate;
}

function startWasmServer() {
  serve({
    port,
    fetch: async (req) => {
      const url = new URL(req.url);
      const match = /^\/([^/]+)\.wasm$/.exec(url.pathname);
      if (!match) return new Response('Not found', { status: 404 });
      const file = path.join(PKG, `${match[1]}.wasm`);
      try {
        const body = await readFile(file);
        return new Response(new Uint8Array(body), {
          headers: { 'content-type': 'application/wasm' },
        });
      } catch {
        return new Response('Not found', { status: 404 });
      }
    },
  });
}

async function main() {
  port = await findOpenPort(START_PORT);
  spinner.start('Starting...');
  startWasmServer();
  await drainBuildQueue();
  isInitializing = false;
  spinner.success(`Ready on ${chalk.underline(`http://localhost:${port}`)}`);
  watchTree();
}

await main();
