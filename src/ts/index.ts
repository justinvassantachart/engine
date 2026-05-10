import EventEmitter from 'events';

import { StdoutMode, WorkerOut, WorkerStart } from '../../pkg/engine';
import init from '../../pkg/engine';
import wasmBinary from '../../pkg/engine_bg.wasm';
import { Debugger } from './debugger';
import { errorResult, Internals } from './util';
import RustWorker from './worker?worker&inline';

export type Lang = 'c';

/** The engine ran to completion with the provided `exitCode`. */
export type CompletedResult = { type: 'completed'; exitCode: number };
/** The engine was stopped by calling `stop`. */
export type StoppedResult = { type: 'stopped' };
/** The engine had an error. This is an error with the engine itself, and not the user's code. */
export type ErrorResult = { type: 'error'; error: { type: string; message: string } };
/** The result of calling {@link Engine.run} */
export type RunResult = CompletedResult | StoppedResult | ErrorResult;

export type FsNode = string | DirNode;
export type DirNode = { [name: string]: FsNode };

export class Engine {
  public readonly stdout = new Stdout(1);
  public readonly stderr = new Stdout(2);
  public readonly stdin = new Stdin();
  public readonly debugger: Debugger;

  /** A function which, when called, rejects the ongoing execution */
  private rejector?: () => void;

  /** The ongoing `run` promise, if any */
  private promise?: Promise<RunResult>;

  /**
   * The programming language of this engine.
   */
  public readonly lang: Lang;

  /**
   * The *initial* filesystem that the code sees.
   *
   * This is neither updated while the code is running, nor
   * will updating it have any effect on code that is already running.
   */
  public fs: DirNode = {};

  static async create(lang: Lang): Promise<Engine> {
    await init({ module_or_path: wasmBinary });
    return new Engine(lang);
  }

  private constructor(lang: Lang) {
    this.debugger = new Debugger();
    this.lang = lang;
  }

  /**
   * Stops the currently running execution.
   */
  public stop(): void {
    this.rejector?.();
  }

  public async run(): Promise<RunResult> {
    if (this.promise) return this.promise;
    this.promise = this.execute();
    const result = await this.promise;
    this.promise = undefined;
    return result;
  }

  private async execute(): Promise<RunResult> {
    const worker = new RustWorker();

    /* Set up handling for stdout/stderr */
    this.stdout[Internals].attach(worker);
    this.stderr[Internals].attach(worker);
    this.debugger[Internals].attach(worker);

    try {
      return await new Promise<RunResult>(async (resolve, reject) => {
        this.rejector = () => reject('stopped');

        /** If the worker ever errors, we crash this promise */
        worker.addEventListener('error', (evt) => reject(evt.error));

        /* Wait for the worker to send us a Ready message */
        await new Promise<void>((resolveReady) => {
          const callback = (message: MessageEvent<WorkerOut>) => {
            if (message.data.type === 'ready') {
              worker.removeEventListener('message', callback);
              resolveReady();
            }
          };
          worker.addEventListener('message', callback);
        });

        worker.addEventListener('message', (message: MessageEvent<WorkerOut>) => {
          if (message.data.type === 'stop')
            resolve({ type: 'completed', exitCode: message.data.exit_code });
        });

        const message: WorkerStart = {
          fs: this.fs,
          stdin_buffer: this.stdin[Internals].buffer,
          is_debug: this.debugger.enabled
        };
        worker.postMessage(message);
      });
    } catch (err: unknown) {
      if (err === 'stopped') return { type: 'stopped' };
      return errorResult(err);
    } finally {
      this.rejector = undefined;
      this.stdout[Internals].detach(worker);
      this.stderr[Internals].detach(worker);
      this.stdin[Internals].clear();
      worker.terminate();
    }
  }
}

export class Stdout extends EventEmitter<{ data: [Uint8Array] }> {
  private readonly callback: (event: MessageEvent<WorkerOut>) => void;

  [Internals]: {
    attach(worker: Worker): void;
    detach(worker: Worker): void;
  };

  constructor(private readonly mode: StdoutMode) {
    super();
    this.callback = ((event: MessageEvent<WorkerOut>) => {
      const msg = event.data;
      if (msg.type !== 'stdout') return;
      if (msg.mode !== this.mode) return;
      const chunk = msg.data as Uint8Array<ArrayBuffer>;
      this.emit('data', chunk);
    }).bind(this);

    this[Internals] = {
      attach: this.attach.bind(this),
      detach: this.detach.bind(this)
    };
  }

  private attach(worker: Worker) {
    worker.addEventListener('message', this.callback);
  }

  private detach(worker: Worker) {
    worker.removeEventListener('message', this.callback);
  }
}

export class Stdin {
  /**
   * Ring buffer to store stdin data.
   *
   * - TypeScript controls write_index, Rust controls read_index
   * - One slot is always kept empty to distinguish full from empty
   */

  private static readonly BUFFER_SIZE = 2048;
  private static readonly HEADER_SIZE = 8; // 2 x i32
  private static readonly DATA_SIZE = Stdin.BUFFER_SIZE - Stdin.HEADER_SIZE;
  private static readonly READ_IDX = 0;
  private static readonly WRITE_IDX = 1;
  private static encoder = new TextEncoder();

  private readonly buffer = new SharedArrayBuffer(Stdin.BUFFER_SIZE);
  private readonly indices: Int32Array;
  private readonly data: Int8Array;

  [Internals]: {
    clear(): void;
    buffer: SharedArrayBuffer;
  };

  constructor() {
    this.indices = new Int32Array(this.buffer, 0, 2);
    this.data = new Int8Array(this.buffer, Stdin.HEADER_SIZE);
    this[Internals] = {
      clear: this.clear.bind(this),
      buffer: this.buffer
    };
  }

  private clear() {
    this.indices.fill(0);
  }

  public async write(value: Uint8Array | string): Promise<void> {
    const chunk = typeof value === 'string' ? Stdin.encoder.encode(value) : value;
    return this.writeBytes(chunk);
  }

  private async writeBytes(chunk: Uint8Array): Promise<void> {
    const { DATA_SIZE, READ_IDX, WRITE_IDX } = Stdin;
    let offset = 0;

    while (offset < chunk.length) {
      const readIdx = Atomics.load(this.indices, READ_IDX);
      let writeIdx = Atomics.load(this.indices, WRITE_IDX);

      if (writeIdx === DATA_SIZE - 1 && readIdx > 0) writeIdx = 0;
      const available = readIdx <= writeIdx ? DATA_SIZE - writeIdx - 1 : readIdx - writeIdx - 1;

      if (available === 0) {
        await Atomics.waitAsync(this.indices, READ_IDX, readIdx).value;
        continue;
      }

      const toWrite = Math.min(chunk.length - offset, available);
      this.data.set(chunk.subarray(offset, offset + toWrite), writeIdx);

      // Write index & notify reader
      Atomics.store(this.indices, WRITE_IDX, (writeIdx + toWrite) % DATA_SIZE);
      Atomics.notify(this.indices, WRITE_IDX);
      offset += toWrite;
    }
  }
}

export * from './debugger';
