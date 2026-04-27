import { $ } from 'bun';
import chalk from 'chalk';
import { existsSync } from 'node:fs';
import { cp, mkdtemp, readdir } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import process from 'node:process';

import type { Backend, BackendOptions, Json, Step } from './run';

const SOURCE_EXTENSIONS = new Set(['c', 'cc', 'cp', 'cpp', 'cxx', 'c++']);

export type LldbOptions = {
  lldbPath?: string;
};

export async function createLldbBackend(
  opts: BackendOptions,
  lldbOpts: LldbOptions = {}
): Promise<Backend> {
  const lldbPath = await detectLldbDap(lldbOpts.lldbPath);
  const { progAbs } = await compileTest(opts.testDir);

  const proc = Bun.spawn({
    cmd: [lldbPath],
    stdin: 'pipe',
    stdout: 'pipe',
    stderr: 'pipe',
  });

  const eventListeners: ((e: Json) => void)[] = [];
  const pending = new Map<number, { resolve: (v: Json) => void; reject: (e: Error) => void }>();

  const dispatch = (msg: Json) => {
    if (!msg || typeof msg !== 'object' || Array.isArray(msg)) return;
    const m = msg as { type?: string; request_seq?: number; event?: string; body?: Json };
    if (m.type === 'response' && typeof m.request_seq === 'number') {
      const p = pending.get(m.request_seq);
      if (p) {
        pending.delete(m.request_seq);
        p.resolve(msg);
      }
      return;
    }
    if (m.type === 'event') {
      handleOutputEvent(m);
      for (const cb of eventListeners) cb(msg);
    }
  };

  void readLoop(proc.stdout, dispatch).catch((err) => {
    const error = err instanceof Error ? err : new Error(String(err));
    for (const [, p] of pending) p.reject(error);
    pending.clear();
  });

  void drainStderr(proc.stderr);

  function send(req: Json): Promise<Json> {
    if (!req || typeof req !== 'object' || Array.isArray(req)) {
      return Promise.reject(new Error('lldb backend send: not an object'));
    }
    const seq = (req as { seq?: number }).seq;
    if (typeof seq !== 'number') {
      return Promise.reject(new Error('lldb backend send: missing seq'));
    }
    return new Promise<Json>((resolve, reject) => {
      pending.set(seq, { resolve, reject });
      try {
        writeFrame(proc.stdin, req);
      } catch (err) {
        pending.delete(seq);
        reject(err instanceof Error ? err : new Error(String(err)));
      }
    });
  }

  function initSteps(): Step[] {
    return [
      {
        type: 'request',
        command: 'initialize',
        arguments: {
          clientID: 'dap-harness',
          clientName: 'dap-harness',
          adapterID: 'lldb-dap',
          pathFormat: 'path',
          linesStartAt1: true,
          columnsStartAt1: true,
        },
      },
      {
        type: 'response',
        success: true,
        command: 'initialize',
      },
      {
        type: 'request',
        command: 'launch',
        arguments: {
          program: progAbs,
          cwd: path.dirname(progAbs),
          stopOnEntry: false,
        },
        $fireAndForget: true,
      },
      { type: 'event', event: 'initialized', $timeout: 10000 },
    ];
  }

  return {
    send,
    onEvent(cb) {
      eventListeners.push(cb);
    },
    initSteps,
    async shutdown() {
      try {
        await Promise.race([
          send({
            type: 'request',
            seq: 0xffff_ffff,
            command: 'disconnect',
            arguments: { terminateDebuggee: true },
          } as Json),
          new Promise((resolve) => setTimeout(resolve, 250)),
        ]);
      } catch {
        /* best effort */
      }
      try {
        proc.kill();
      } catch {
        /* ignore */
      }
    },
  };
}

async function detectLldbDap(override?: string): Promise<string> {
  const tried: string[] = [];

  const check = async (label: string, candidate: string | undefined): Promise<string | null> => {
    if (!candidate) {
      tried.push(label);
      return null;
    }
    if (existsSync(candidate)) return candidate;
    tried.push(`${label} (${candidate})`);
    return null;
  };

  const overrideHit = await check('--lldb-path', override);
  if (overrideHit) return overrideHit;

  const envHit = await check('LLDB_DAP_PATH', process.env.LLDB_DAP_PATH);
  if (envHit) return envHit;

  const xcrun = await $`xcrun -f lldb-dap`.quiet().nothrow();
  const xcrunPath = xcrun.exitCode === 0 ? xcrun.stdout.toString().trim() : '';
  const xcrunHit = await check('xcrun -f lldb-dap', xcrunPath || undefined);
  if (xcrunHit) return xcrunHit;

  const which = await $`sh -lc 'command -v lldb-dap'`.quiet().nothrow();
  const whichPath = which.exitCode === 0 ? which.stdout.toString().trim() : '';
  const whichHit = await check('command -v lldb-dap', whichPath || undefined);
  if (whichHit) return whichHit;

  throw new Error(
    [
      'lldb-dap not found.',
      `tried: ${tried.join(', ')}`,
      'install via Xcode (lldb-dap ships with the developer toolchain) or `brew install llvm`.',
      'override with --lldb-path=/abs/path/to/lldb-dap or env LLDB_DAP_PATH=/abs/path.',
    ].join('\n  ')
  );
}

async function compileTest(testDir: string): Promise<{ progAbs: string }> {
  // Keep lldb-dap activity out of protected folders (e.g. ~/Documents),
  // otherwise OS privacy prompts can interrupt the debug session.
  const lldbDir = await mkdtemp(path.join(tmpdir(), 'runtime-dap-lldb-'));
  await cp(testDir, lldbDir, { recursive: true });

  const sourceRels = (await collectSourceFiles(lldbDir)).sort();
  if (sourceRels.length === 0) {
    throw new Error(`no C/C++ source files found in ${testDir}`);
  }

  const progAbs = path.join(lldbDir, 'prog');
  const sourceAbs = sourceRels.map((rel) => path.join(lldbDir, rel));

  const result =
    await $`xcrun clang++ -g -O0 -fno-inline -fstandalone-debug -std=c++23 -o ${progAbs} ${sourceAbs}`
      .quiet()
      .nothrow();
  if (result.exitCode !== 0) {
    throw new Error(`lldb backend compile failed (xcrun clang++):\n${result.stderr.toString()}`);
  }

  return { progAbs };
}

function writeFrame(stdin: { write(data: string): unknown; flush(): unknown }, msg: Json) {
  const body = JSON.stringify(msg);
  const header = `Content-Length: ${Buffer.byteLength(body, 'utf8')}\r\n\r\n`;
  stdin.write(header);
  stdin.write(body);
  stdin.flush();
}

async function readLoop(stdout: ReadableStream<Uint8Array>, dispatch: (msg: Json) => void) {
  const reader = stdout.getReader();
  const decoder = new TextDecoder();
  let buffer = new Uint8Array(0);

  while (true) {
    const { value, done } = await reader.read();
    if (done) return;
    if (value && value.length > 0) {
      const copy = new Uint8Array(value.byteLength);
      copy.set(value);
      buffer = concatU8(buffer, copy);
    }

    while (true) {
      const headerEnd = findCrlfCrlf(buffer);
      if (headerEnd < 0) break;
      const headerStr = decoder.decode(buffer.slice(0, headerEnd));
      const m = headerStr.match(/Content-Length:\s*(\d+)/i);
      if (!m) throw new Error(`bad DAP header: ${JSON.stringify(headerStr)}`);
      const length = parseInt(m[1], 10);
      const start = headerEnd + 4;
      if (buffer.length < start + length) break;
      const body = decoder.decode(buffer.slice(start, start + length));
      buffer = buffer.slice(start + length);
      let msg: Json;
      try {
        msg = JSON.parse(body) as Json;
      } catch (err) {
        throw new Error(`invalid DAP body: ${String(err)}\n${body}`);
      }
      dispatch(msg);
    }
  }
}

async function drainStderr(stderr: ReadableStream<Uint8Array>) {
  const reader = stderr.getReader();
  const decoder = new TextDecoder();
  while (true) {
    const { value, done } = await reader.read();
    if (done) return;
    if (value && value.length > 0) {
      process.stderr.write(chalk.dim(decoder.decode(value)));
    }
  }
}

function handleOutputEvent(msg: { event?: string; body?: Json }) {
  if (msg.event !== 'output') return;
  const body = msg.body;
  if (!body || typeof body !== 'object' || Array.isArray(body)) return;
  const out = (body as Record<string, Json>).output;
  if (typeof out !== 'string') return;
  process.stdout.write(chalk.gray(out));
}

function concatU8(a: Uint8Array, b: Uint8Array): Uint8Array<ArrayBuffer> {
  const out = new Uint8Array(a.length + b.length);
  out.set(a, 0);
  out.set(b, a.length);
  return out;
}

function findCrlfCrlf(buf: Uint8Array): number {
  for (let i = 0; i + 3 < buf.length; i++) {
    if (buf[i] === 0x0d && buf[i + 1] === 0x0a && buf[i + 2] === 0x0d && buf[i + 3] === 0x0a) {
      return i;
    }
  }
  return -1;
}

async function collectSourceFiles(rootDir: string): Promise<string[]> {
  const out: string[] = [];

  const walk = async (dirAbs: string, relDir: string) => {
    const entries = await readdir(dirAbs, { withFileTypes: true });
    for (const entry of entries) {
      const nextRel = relDir ? path.join(relDir, entry.name) : entry.name;
      const nextAbs = path.join(dirAbs, entry.name);
      if (entry.isDirectory()) {
        await walk(nextAbs, nextRel);
        continue;
      }
      if (!entry.isFile()) continue;
      const ext = path.extname(entry.name).slice(1).toLowerCase();
      if (SOURCE_EXTENSIONS.has(ext)) out.push(nextRel);
    }
  };

  await walk(rootDir, '');
  return out;
}
