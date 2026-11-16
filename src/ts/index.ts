import init, { Fs, type InitOutput } from '../../pkg/engine';
import wasmBinary from '../../pkg/engine_bg.wasm';
import { Debugger } from './debugger';
import { Filesystem } from './fs';
import { Duplex, Pipe, Pool, Process, type Stdio, type WorkerMessage } from './process';
import {
  commands,
  type Context,
  type RunOptions,
  type StepOptions,
  sysroots,
  type Toolchain,
  type ToolchainOptionsMap,
  toolchains
} from './toolchains';
import { internal, Internals, join } from './util';

/**
 * A command to run: a language toolchain or built-in command, or a WASI executable
 * (raw bytes, a URL to fetch, or a path within the workspace).
 */
export type CommandLike = Uint8Array | string | Toolchain;

export type CommandOptions<C extends CommandLike> = C extends Toolchain
  ? RunOptions & ToolchainOptionsMap[C]
  : RunOptions;

/**
 * A process spawned with debugging enabled.
 */
export interface DebuggableProcess extends Process {
  /**
   * The debugging interface for this process.
   */
  readonly debugger: Debugger;
}

/** A failure to resolve a program: an exit code, and a message for stderr. */
type Failure = [code: number, message: string];

const isFailure = (value: unknown): value is Failure => Array.isArray(value);

/** Resolves a `sysroot` option (a toolchain name or a URL) to a URL. */
const sysrootUrl = (sysroot: string) =>
  Object.hasOwn(sysroots, sysroot) ? sysroots[sysroot] : sysroot;

/** The engine's WebAssembly module and the memory shared by all of its threads. */
let runtime: Promise<{ module: WebAssembly.Module; exports: InitOutput }> | undefined;

/** Compiled modules (or failures) of fetched commands, by URL. */
const modules = new Map<string, Promise<WebAssembly.Module | Failure>>();

/** Loaded sysroots (or failures), by URL. Shared by all engines, never copied. */
const sysrootCache = new Map<string, Promise<Fs | Failure>>();

async function fetchResponse(url: string): Promise<Response | Failure> {
  // Toolchain binaries are versioned by URL, so they can be cached indefinitely.
  const response = await fetch(url, { cache: 'force-cache' }).catch(() => undefined);
  if (!response?.ok) return [127, `network error (${response?.status ?? 0})`];
  return response;
}

async function fetchBytes(url: string): Promise<Uint8Array<ArrayBuffer> | Failure> {
  const response = await fetchResponse(url);
  return isFailure(response) ? response : new Uint8Array(await response.arrayBuffer());
}

function cached<T>(
  cache: Map<string, Promise<T | Failure>>,
  url: string,
  load: () => Promise<T | Failure>
): Promise<T | Failure> {
  let promise = cache.get(url);
  if (!promise) {
    cache.set(url, (promise = load()));
    // Don't remember failures, so that they may be retried.
    const forget = () => cache.delete(url);
    promise.then((value) => isFailure(value) && forget(), forget);
  }
  return promise;
}

export interface EngineOptions {
  /**
   * Create the filesystem with this default state.
   *
   * This supports the same shape as {@link Filesystem.mount}.
   */
  fs?: Parameters<Filesystem['mount']>[0];
}

/**
 * An engine is a shared workspace for running programs.
 */
export class Engine {
  /** The in-memory filesystem shared by all processes */
  public readonly fs: Filesystem;

  readonly #pool: Pool;
  readonly #processes = new Set<Process>();
  #disposed = false;

  /** Creates an engine. Remember to call {@link Engine.dispose} when you're done with it. */
  static async create(options?: EngineOptions): Promise<Engine> {
    runtime ??= (async () => {
      const binary = wasmBinary as unknown as URL | Uint8Array<ArrayBuffer>;
      const module = await WebAssembly.compile(
        binary instanceof URL ? await (await fetch(binary)).arrayBuffer() : binary
      );
      const exports = await init({ module_or_path: module });
      return { module, exports };
    })().catch((error) => {
      runtime = undefined;
      throw error;
    });

    const engine = new Engine(Internals, await runtime);
    if (options?.fs) await engine.fs.mount(options.fs);
    return engine;
  }

  /** @internal */
  constructor(key: typeof Internals, runtime: { module: WebAssembly.Module; exports: InitOutput }) {
    internal(key);
    const { module, exports } = runtime;
    this.fs = new Filesystem(Internals, new Fs());
    this.#pool = new Pool((gate) => {
      const worker = new Worker(new URL('./worker.ts', import.meta.url), { type: 'module' });
      worker.postMessage({ type: 'init', module, memory: exports.memory, gate });
      return worker;
    });
  }

  /** Kills any running processes and releases the memory held by the engine. */
  public dispose(): void {
    if (this.#disposed) return;
    this.#disposed = true;
    for (const process of this.#processes) process.kill();
    this.#pool.dispose();
    this.fs[Internals].dispose();
  }

  /**
   * Runs a command.
   *
   * ```ts
   * // Run a WASI program
   * await engine.run("https://example.com/test.wasm");
   *
   * // Run a WASI executable
   * const bytes: Uint8Array = // ... //;
   * await engine.run(bytes);
   *
   * // Run a built-in language toolchain or command.
   * await engine.run("cpp");
   * ```
   *
   * Setting {@link RunOptions.debug} to `true` allows debugging the process using {@link Process.debugger}.
   */
  public run<C extends CommandLike>(
    command: C,
    options: CommandOptions<C> & { debug: true }
  ): DebuggableProcess;
  public run<C extends CommandLike>(command: C, options?: CommandOptions<C>): Process;
  public run<C extends CommandLike>(
    command: C,
    args: string[],
    options: CommandOptions<C> & { debug: true }
  ): DebuggableProcess;
  public run<C extends CommandLike>(
    command: C,
    args: string[],
    options?: CommandOptions<C>
  ): Process;
  public run(command: CommandLike, args?: string[] | RunOptions, options?: RunOptions): Process {
    if (this.#disposed) throw new Error('The engine has been disposed');
    if (!Array.isArray(args)) [args, options] = [[], args];
    options ??= {};

    const flowing = () => {
      const pipe = new Pipe();
      pipe.flow();
      return pipe;
    };
    const closed = () => new Pipe({ capacity: 0 });
    const stdin = (): Pipe => {
      const value = options.stdin;
      if (value instanceof Pipe) return value;
      if (typeof value === 'string' || value instanceof Uint8Array) {
        const bytes = typeof value === 'string' ? new TextEncoder().encode(value) : value;
        const pipe = new Pipe({ capacity: bytes.length });
        if (bytes.length) {
          pipe.write(bytes); // runs immediately
          pipe.close();
        }
        return pipe;
      }
      if (value === true) return new Pipe();
      return closed();
    };
    const stdio = {
      stdin: stdin(),
      stdout: options.stdout ?? flowing(),
      stderr: options.stderr ?? flowing()
    };
    // Pipes given to the process (including its own) are closed once it exits.
    const pipes = [...Object.values(stdio), ...Object.values(options.devices ?? {})]
      .flatMap((device) => (device instanceof Duplex ? [device.to, device.from] : [device]))
      .filter((device): device is Pipe => device instanceof Pipe && device[Internals].close);
    const debug = options.debug ? new Debugger(Internals) : undefined;
    const process = this.#spawn(stdio, debug, command, args, options, pipes);

    this.#processes.add(process);
    const finish = () => {
      this.#processes.delete(process);
      debug?.[Internals].receive({ type: 'stop' });
    };
    process.then(finish, finish);
    return process;
  }

  #spawn(
    stdio: Stdio,
    debug: Debugger | undefined,
    command: CommandLike,
    args: string[],
    options: StepOptions,
    pipes?: Pipe[]
  ): Process {
    return new Process(
      Internals,
      stdio,
      debug,
      (process) => this.#exec(process, command, args, options),
      pipes
    );
  }

  async #exec(
    process: Process,
    command: CommandLike,
    args: string[],
    options: StepOptions
  ): Promise<number> {
    if (command instanceof Uint8Array) return this.#job(process, { bytes: command }, args, options);
    if (/^https?:\/\//.test(command)) return this.#job(process, { url: command }, args, options);
    if (command.startsWith('/') || command.startsWith('.'))
      return this.#job(process, { path: command }, args, options);
    if (Object.hasOwn(toolchains, command))
      return this.#toolchain(process, command as Toolchain, args, options);
    if (Object.hasOwn(commands, command)) {
      const [url, name] = commands[command];
      return this.#job(process, { url }, args, { name, ...options });
    }
    await process[Internals].error(`${command}: command not found`);
    return 127;
  }

  #toolchain(
    process: Process,
    name: Toolchain,
    args: string[],
    options: StepOptions
  ): Promise<number> {
    const env = options.env ?? {};
    const ctx: Context<Toolchain> = {
      args,
      options: options as Context<Toolchain>['options'],
      env,
      fs: this.fs,
      process,
      run: async (command, args, step = {}) => {
        const { stdio } = process[Internals];
        const child = this.#spawn(stdio, process.debugger, command, args, { env, ...step });
        const release = process[Internals].onKill(() => child.kill());
        const { exitCode } = await child;
        release();
        return exitCode;
      },
      sysroot: async (name) => {
        const sysroot = await this.#sysroot(sysrootUrl(name));
        return isFailure(sysroot) ? undefined : sysroot;
      }
    };
    return (toolchains[name] as (ctx: Context<Toolchain>) => Promise<number>)(ctx);
  }

  /** Loads a sysroot, fetching and extracting it into shared memory. */
  #sysroot(url: string): Promise<Fs | Failure> {
    return cached(sysrootCache, url, async () => {
      const bytes = await fetchBytes(url);
      if (isFailure(bytes)) return bytes;
      const fs = new Fs();
      await fs.mount('/', bytes);
      return fs;
    });
  }

  /** Runs a single WASI program on a worker. */
  async #job(
    process: Process,
    program: { bytes?: Uint8Array; url?: string; path?: string },
    args: string[],
    options: StepOptions
  ): Promise<number> {
    const name = options.name ?? program.path ?? program.url ?? 'main';
    const fail = async ([code, message]: Failure) => (
      await process[Internals].error(`${name}: ${message}`),
      code
    );
    const debug = options.debug === 'python' ? 'python' : options.debug ? 'wasm' : undefined;

    let sysroot: number | undefined;
    if (options.sysroot) {
      const loaded = await this.#sysroot(sysrootUrl(options.sysroot));
      if (isFailure(loaded)) return fail(loaded);
      sysroot = loaded.ptr();
    }

    let { bytes } = program;
    let module: WebAssembly.Module | undefined;
    if (program.url && debug === 'wasm') {
      // Instrumentation for debugging requires the raw binary.
      const fetched = await fetchBytes(program.url);
      if (isFailure(fetched)) return fail(fetched);
      bytes = fetched;
    } else if (program.url) {
      const compiled = await cached(modules, program.url, async () => {
        const response = await fetchResponse(program.url!);
        if (isFailure(response)) return response;
        return WebAssembly.compileStreaming(response).catch((): Failure => [126, 'cannot execute']);
      });
      if (isFailure(compiled)) return fail(compiled);
      module = compiled;
    }

    const { stdin, stdout, stderr } = process[Internals].stdio;
    const python = debug === 'python' ? process.debugger?.[Internals].python() : undefined;
    const devices = Object.entries({ ...options.devices, ...python }).map(([path, device]) => {
      const [from, to] = device instanceof Duplex ? [device.from, device.to] : [device];
      const pipe = from instanceof Pipe;
      const buffer = pipe ? from[Internals].buffer : from;
      return { path: join(path), buffer, pipe, write: to?.[Internals].buffer };
    });
    const job = {
      name,
      module,
      bytes,
      path: program.path,
      args,
      env: Object.entries(options.env ?? {}),
      workspace: this.fs[Internals].ptr,
      sysroot,
      stdin: stdin[Internals].buffer,
      stdout: stdout[Internals].buffer,
      stderr: stderr[Internals].buffer,
      devices,
      debug
    };

    const worker = await this.#pool.acquire();
    const exited = new Promise<number>((resolve, reject) => {
      let release = () => {};
      const done = (finish: () => void) => {
        worker.worker.removeEventListener('message', onMessage);
        worker.worker.removeEventListener('error', onError);
        release();
        finish();
      };
      const onMessage = ({ data }: MessageEvent<WorkerMessage>) => {
        if (data.type === 'exit')
          done(() => (this.#pool.release(worker), resolve(data.code as number)));
        else if (data.type === 'error')
          done(() => (void this.#pool.kill(worker), reject(new Error(String(data.message)))));
        else if (debug) process.debugger?.[Internals].receive(data);
      };
      const onError = (e: ErrorEvent) =>
        done(() => (void this.#pool.kill(worker), reject(e.error ?? new Error(e.message))));
      // A killed job's worker is terminated, and replaced (lazily) by a fresh one.
      release = process[Internals].onKill(() =>
        done(() => (void this.#pool.kill(worker), resolve(137)))
      );

      worker.worker.addEventListener('message', onMessage);
      worker.worker.addEventListener('error', onError);
      worker.worker.postMessage({ type: 'run', job });
    });
    // The Python debugger's duplex belongs to this job.
    return exited.finally(() => Object.values(python ?? {}).forEach((pipe) => pipe.close()));
  }
}

export type {
  Debugger,
  DebuggerProtocol,
  Frame,
  Scope,
  Stop,
  Variable,
  VariableOptions
} from './debugger';
export type {
  Directory,
  ExportOptions,
  File,
  FileContents,
  FileStat,
  FileTree,
  Timestamp,
  Timestamps
} from './fs';
export type { Filesystem } from './fs';
export type { Device, PipeOptions, ProcessResult, Readable, Writable } from './process';
export type { Process } from './process';
export { Duplex, Pipe } from './process';
export type {
  RunOptions,
  CppOptions,
  PythonOptions,
  RustOptions,
  Toolchain,
  ToolchainOptionsMap
} from './toolchains';
