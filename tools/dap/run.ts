import { $ } from 'bun';
import chalk from 'chalk';
import { spawn } from 'child_process';
import { existsSync } from 'node:fs';
import { readdir, readFile, stat } from 'node:fs/promises';
import path from 'node:path';
import process from 'node:process';
import { fileURLToPath, pathToFileURL } from 'node:url';

type Json = null | boolean | number | string | Json[] | { [k: string]: Json };
type CaptureMap = Record<string, Json>;

type RequestStep = {
  type: 'request';
  command: string;
  arguments?: Json;
};
type ResponseStep = {
  type: 'response';
  success?: boolean;
  command?: string;
  body?: Json;
};
type EventStep = {
  type: 'event';
  event: string;
  body?: Json;
  $timeout?: number;
};
type Step = RequestStep | ResponseStep | EventStep;

type TestFile = { steps: Step[] };

const HERE = path.dirname(fileURLToPath(import.meta.url));
let ROOT = path.resolve(HERE, '../..');
let TESTS_DIR = path.join(ROOT, 'tools/dap/tests');
let DIST_ENTRY = path.join(ROOT, 'dist/runtime.js');
const DAP_TIMEOUT_MS = 1000;

const INIT_STEPS: Step[] = [
  {
    type: 'request',
    command: 'initialize',
    arguments: {
      clientID: 'dap-harness',
      clientName: 'dap-harness',
      adapterID: 'runtime',
      pathFormat: 'path',
      linesStartAt1: true,
      columnsStartAt1: true,
    },
  },
  {
    type: 'response',
    success: true,
    command: 'initialize',
    body: { supportsConfigurationDoneRequest: true },
  },
  { type: 'event', event: 'initialized', $timeout: 10000 },
];

function logInfo(msg: string) {
  console.log(`${chalk.cyan('info')} ${msg}`);
}

function logStep(msg: string) {
  console.log(`${chalk.blue('->')} ${msg}`);
}

function logOk(msg: string) {
  console.log(`${chalk.green('ok')} ${msg}`);
}

function die(msg: string): never {
  console.error(`${chalk.red('error')} ${msg}`);
  process.exit(1);
}

function parseCli(argv: string[]) {
  const tests: string[] = [];
  let build = false;
  for (const arg of argv) {
    if (arg === '--build') {
      build = true;
      continue;
    }
    tests.push(arg);
  }
  return { tests, build };
}

async function resolveRootWithNpmPrefix() {
  const out = await $`npm prefix`.cwd(HERE).quiet();
  const root = out.stdout.toString().trim();
  if (!root) die('npm prefix returned empty project root');
  ROOT = path.resolve(root);
  TESTS_DIR = path.join(ROOT, 'tools/dap/tests');
  DIST_ENTRY = path.join(ROOT, 'dist/runtime.js');
}

async function listTestNames(): Promise<string[]> {
  const entries = await readdir(TESTS_DIR, { withFileTypes: true });
  return entries
    .filter((e) => e.isDirectory())
    .map((e) => e.name)
    .sort();
}

async function buildIfNeeded(force: boolean) {
  const distMissing = !existsSync(DIST_ENTRY);
  if (!force && !distMissing) return;
  logInfo(`building runtime...`);

  await new Promise<void>((resolve) => {
    const proc = spawn('npm', ['run', 'build'], { cwd: ROOT, shell: true });

    proc.stdout.on('data', (chunk: Buffer) => {
      process.stdout.write(chalk.gray(chunk.toString()));
    });
    proc.stderr.on('data', (chunk: Buffer) => {
      process.stdout.write(chalk.gray(chunk.toString()));
    });

    proc.on('close', (code) => {
      if (code === 0) return resolve();
      die('build failed');
    });

    proc.on('error', (err) => {
      die(`build error: ${err instanceof Error ? err.message : String(err)}`);
    });
  });

  console.log();
}

async function readJsonFile<T>(filePath: string): Promise<T> {
  const raw = await readFile(filePath, 'utf8');
  try {
    return JSON.parse(raw) as T;
  } catch (err) {
    throw new Error(`invalid JSON in ${filePath}: ${String(err)}`);
  }
}

async function collectFsNode(dirPath: string): Promise<Record<string, Json>> {
  async function walk(current: string): Promise<Record<string, Json>> {
    const out: Record<string, Json> = {};
    const entries = (await readdir(current, { withFileTypes: true })).sort((a, b) =>
      a.name.localeCompare(b.name)
    );
    for (const entry of entries) {
      const abs = path.join(current, entry.name);
      if (entry.isDirectory()) {
        out[entry.name] = await walk(abs);
      } else if (entry.isFile()) {
        const content = await readFile(abs, 'utf8');
        out[entry.name] = content;
      }
    }
    return out;
  }
  return walk(dirPath);
}

function isPlaceholderString(value: unknown): value is string {
  return typeof value === 'string' && /^\{\{[a-zA-Z_]\w*\}\}$/.test(value);
}

function placeholderName(value: string): string {
  return value.slice(2, -2);
}

function substitutePlaceholders(input: Json, captures: CaptureMap): Json {
  if (isPlaceholderString(input)) {
    const name = placeholderName(input);
    if (!(name in captures)) throw new Error(`unbound placeholder ${input}`);
    return captures[name];
  }
  if (Array.isArray(input)) return input.map((v) => substitutePlaceholders(v, captures));
  if (input && typeof input === 'object') {
    const out: Record<string, Json> = {};
    for (const [k, v] of Object.entries(input)) {
      out[k] = substitutePlaceholders(v as Json, captures);
    }
    return out;
  }
  return input;
}

function isObject(v: Json): v is Record<string, Json> {
  return typeof v === 'object' && v !== null && !Array.isArray(v);
}

function fmtJson(v: unknown): string {
  try {
    return JSON.stringify(v, null, 2);
  } catch {
    return String(v);
  }
}

function assertMatch(expected: Json, actual: Json, captures: CaptureMap, at = '$'): void {
  if (isPlaceholderString(expected)) {
    captures[placeholderName(expected)] = actual;
    return;
  }

  if (Array.isArray(expected)) {
    if (!Array.isArray(actual)) throw new Error(`${at}: expected array, got ${typeof actual}`);
    if (expected.length !== actual.length) {
      throw new Error(`${at}: expected array length ${expected.length}, got ${actual.length}`);
    }
    for (let i = 0; i < expected.length; i++) {
      assertMatch(expected[i], actual[i], captures, `${at}[${i}]`);
    }
    return;
  }

  if (isObject(expected)) {
    if (!isObject(actual)) throw new Error(`${at}: expected object, got ${typeof actual}`);
    for (const [key, expectedValue] of Object.entries(expected)) {
      if (key === '$array.contains') {
        if (!Array.isArray(actual)) throw new Error(`${at}: $array.contains requires actual array`);
        const templates = expectedValue;
        if (!Array.isArray(templates))
          throw new Error(`${at}: $array.contains value must be array`);
        for (let i = 0; i < templates.length; i++) {
          const template = templates[i] as Json;
          let matched = false;
          let lastErr = '';
          for (let j = 0; j < actual.length; j++) {
            const localCaps: CaptureMap = { ...captures };
            try {
              assertMatch(template, actual[j] as Json, localCaps, `${at}[$array.contains][${i}]`);
              Object.assign(captures, localCaps);
              matched = true;
              break;
            } catch (err) {
              lastErr = String(err);
            }
          }
          if (!matched) {
            throw new Error(
              `${at}: no array element matched $array.contains template index ${i}. Last mismatch: ${lastErr}`
            );
          }
        }
        continue;
      }

      if (!(key in actual)) throw new Error(`${at}.${key}: missing key in actual object`);
      assertMatch(
        expectedValue as Json,
        (actual as Record<string, Json>)[key],
        captures,
        `${at}.${key}`
      );
    }
    return;
  }

  if (!Object.is(expected, actual)) {
    throw new Error(`${at}: expected ${fmtJson(expected)}, got ${fmtJson(actual)}`);
  }
}

async function waitForEvent(
  queue: Json[],
  waitForNext: () => Promise<Json>,
  eventName: string,
  timeoutMs: number
): Promise<Json> {
  const endAt = Date.now() + timeoutMs;
  while (Date.now() < endAt) {
    for (let i = 0; i < queue.length; i++) {
      const event = queue[i];
      if (isObject(event) && event.type === 'event' && event.event === eventName) {
        queue.splice(i, 1);
        return event;
      }
    }
    const remaining = endAt - Date.now();
    if (remaining <= 0) break;
    await Promise.race([
      waitForNext(),
      new Promise((_, reject) => setTimeout(() => reject(new Error('timeout')), remaining)),
    ]).catch((err) => {
      if (String(err).includes('timeout')) return;
      throw err;
    });
  }
  throw new Error(`timed out waiting for event '${eventName}' after ${timeoutMs}ms`);
}

async function runTest(testName: string): Promise<void> {
  const testDir = path.join(TESTS_DIR, testName);
  const testStat = await stat(testDir).catch(() => null);
  if (!testStat || !testStat.isDirectory()) throw new Error(`unknown test '${testName}'`);

  const dapPath = path.join(testDir, 'dap.json');
  const file = await readJsonFile<TestFile>(dapPath);
  if (!Array.isArray(file.steps)) throw new Error(`${dapPath}: expected top-level steps[]`);

  const fsNode = await collectFsNode(testDir);
  const { Runtime } = (await import(pathToFileURL(DIST_ENTRY).href)) as {
    Runtime: {
      create(lang: 'c'): Promise<{
        fs: Json;
        stdout: ReadableStream<Uint8Array>;
        stderr: ReadableStream<Uint8Array>;
        run(): Promise<void>;
        debugger: { send(msg: Json): Json; on(name: 'event', cb: (m: Json) => void): void };
      }>;
    };
  };

  const runtime = await Runtime.create('c');
  runtime.fs = fsNode;

  const decoder = new TextDecoder();
  runtime.stdout.pipeTo(
    new WritableStream<Uint8Array>({
      write(chunk) {
        process.stdout.write(chalk.gray(decoder.decode(chunk)));
      },
    })
  );
  runtime.stderr.pipeTo(
    new WritableStream<Uint8Array>({
      write(chunk) {
        process.stdout.write(chalk.gray(decoder.decode(chunk)));
      },
    })
  );
  const eventQueue: Json[] = [];
  let resolveEventWaiter: ((v: Json) => void) | null = null;
  runtime.debugger.on('event', (msg: Json) => {
    eventQueue.push(msg);
    if (resolveEventWaiter) {
      const fn = resolveEventWaiter;
      resolveEventWaiter = null;
      fn(msg);
    }
  });
  const waitForNextEvent = () =>
    new Promise<Json>((resolve) => {
      resolveEventWaiter = resolve;
    });

  const runPromise = runtime.run();
  const captures: CaptureMap = {};
  let seq = 1;
  let lastResponse: Json | null = null;
  const executeStep = async (step: Step, label: string, visible: boolean) => {
    if (step.type === 'request') {
      if (visible) logStep(`${label} ${step.command}`);
      const reqObj = substitutePlaceholders(
        {
          type: 'request',
          seq: seq++,
          command: step.command,
          arguments: step.arguments ?? {},
        },
        captures
      ) as Json;
      lastResponse = runtime.debugger.send(reqObj) as Json;
      if (isObject(lastResponse) && lastResponse.success === false) {
        throw new Error(
          `${label} command '${step.command}' returned success=false\nresponse:\n${fmtJson(lastResponse)}`
        );
      }
      return;
    }

    if (step.type === 'response') {
      if (!lastResponse) throw new Error(`${label}: no prior request response available`);
      if (visible) logStep(label);
      const expected = substitutePlaceholders(step as unknown as Json, captures);
      try {
        assertMatch(expected, lastResponse, captures);
      } catch (err) {
        throw new Error(
          `${label} mismatch\nexpected:\n${fmtJson(expected)}\nreceived:\n${fmtJson(lastResponse)}\nreason: ${String(err)}`
        );
      }
      return;
    }

    if (visible) logStep(`${label} ${step.event}`);
    const timeout = step.$timeout ?? DAP_TIMEOUT_MS;
    const actualEvent = await waitForEvent(eventQueue, waitForNextEvent, step.event, timeout);
    const { $timeout: _ignored, ...expectedStep } = step;
    const expected = substitutePlaceholders(expectedStep as unknown as Json, captures);
    try {
      assertMatch(expected, actualEvent, captures);
    } catch (err) {
      throw new Error(
        `${label} mismatch\nexpected:\n${fmtJson(expected)}\nreceived:\n${fmtJson(actualEvent)}\nreason: ${String(err)}`
      );
    }
  };

  logInfo(`${chalk.bold(testName)} (${file.steps.length} steps)`);
  logStep(`[0/${file.steps.length}] setup debugger session`);
  for (const step of INIT_STEPS) {
    await executeStep(step, '[init]', false);
  }

  for (let i = 0; i < file.steps.length; i++) {
    const step = file.steps[i];
    const label = `[${i + 1}/${file.steps.length}] ${step.type}`;
    await executeStep(step, label, true);
  }

  // Best-effort wait for program completion after continue; avoid hanging forever.
  await Promise.race([runPromise, new Promise((resolve) => setTimeout(resolve, 1500))]);
  logOk(`${testName} passed`);
}

async function main() {
  await resolveRootWithNpmPrefix();
  const { tests: requestedTests, build } = parseCli(process.argv.slice(2));
  await buildIfNeeded(build);

  const available = await listTestNames();
  const tests = requestedTests.length ? requestedTests : available;
  if (tests.length === 0) die(`no tests found in ${TESTS_DIR}`);

  for (const test of tests) {
    if (!available.includes(test)) {
      die(`unknown test '${test}'. Available: ${available.join(', ')}`);
    }
  }

  const failed: { name: string; error: string }[] = [];

  for (const test of tests) {
    try {
      await runTest(test);
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      failed.push({ name: test, error: message });
      console.error(`${chalk.red('FAIL')} ${chalk.bold(test)}\n${chalk.dim(message)}`);
    }
  }

  if (failed.length > 0) {
    console.error(`\n${chalk.red(`${failed.length}/${tests.length} test(s) failed`)}`);
    for (const f of failed) {
      console.error(`${chalk.red('-')} ${f.name}`);
    }
    process.exit(1);
  }

  console.log(`\n${chalk.green(chalk.bold(`all ${tests.length} test(s) passed`))}`);
}

await main();
