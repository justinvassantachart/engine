import { StdoutMode, WorkerOut, WorkerStart } from '../../pkg/runtime';
import { Debugger } from './debugger';
import { Internals } from './internals';
import RustWorker from './worker?worker&inline';

export type Lang = 'c';

// TODO: Find a way to re-use the generated types in `pkg/runtime.d.ts`
export type FsNode = string | DirNode;
export type DirNode = { [name: string]: FsNode };

export class Runtime {
  private out = new StdoutStream(1);
  private err = new StdoutStream(2);
  private in = new StdinStream();
  public debugger = new Debugger();

  /** A function which, when called, rejects the ongoing execution */
  private rejector?: () => void;

  /** The ongoing `run` promise, if any */
  private promise?: Promise<void>;

  /**
   * The programming language of this runtime.
   */
  public readonly lang: Lang;

  /**
   * When `true` (default), the worker compiles with debug info and breakpoint
   * instrumentation. When `false`, runs without DWARF/instrumentation (faster).
   * Set to `false` for faster runs when you don't need breakpoints.
   */
  public debug = true;

  /**
   * The *initial* filesystem that the code sees.
   *
   * This is neither updated while the code is running, nor
   * will updating it have any effect on code that is already running.
   */
  public fs: DirNode = {};

  /**
   * A [WritableStream](https://developer.mozilla.org/en-US/docs/Web/API/WritableStream) for writing to the program's `stdin` (fd 0).
   *
   * Note that any previous input pushed to `stdin` will be cleared when the program finishes
   * running. This is to prevent subsequent runs of a program from seeing `stdin` from the previous one.
   *
   * @example
   * ```ts
   *  const rt = Runtime.create('c');
   *
   *  const encoder = new TextEncoder();
   *  const writer = rt.stdin.getWriter();
   *
   *  writer.write(encoder.encode('hello world\n'));
   * ```
   */
  public get stdin() {
    return this.in.stream;
  }

  /**
   * A [ReadableStream](https://developer.mozilla.org/en-US/docs/Web/API/ReadableStream) for reading the program's `stdout` (fd 1).
   */
  public get stdout() {
    return this.out.stream;
  }

  /**
   * A [ReadableStream](https://developer.mozilla.org/en-US/docs/Web/API/ReadableStream) for reading the program's `stderr` (fd 2).
   */
  public get stderr() {
    return this.err.stream;
  }

  static create(lang: Lang): Runtime {
    return new Runtime(lang);
  }

  private constructor(lang: Lang) {
    this.lang = lang;
  }

  /**
   * Stops the currently running execution by terminating the worker.
   * Safe to call even if no execution is running.
   * This will cause the run() promise to resolve immediately.
   */
  public stop(): void {
    this.rejector?.();
  }

  public async run() {
    if (this.promise) return this.promise;
    this.promise = this.execute();
    await this.promise;
    this.promise = undefined;
  }

  private async execute() {
    const worker = new RustWorker();

    /* Set up handling for stdout/stderr */
    this.out.addWorker(worker);
    this.err.addWorker(worker);
    this.debugger[Internals].addWorker(worker);

    try {
      await new Promise<void>(async (resolve, reject) => {
        this.rejector = () => reject('stopped worker');

        /** If the worker ever errors, we crash this promise */
        worker.addEventListener('error', (evt) => reject(evt.error));

        /* Wait for the worker to send us a Ready message */
        await new Promise<void>((resolve) => {
          const callback = (message: MessageEvent<WorkerOut>) => {
            if (message.data.type === 'ready') {
              worker.removeEventListener('message', callback);
              resolve();
            }
          };
          worker.addEventListener('message', callback);
        });

        worker.addEventListener('message', (message: MessageEvent<WorkerOut>) => {
          if (message.data.type === 'stop') resolve();
          if (message.data.type === 'download') {
            const blob = new Blob([new Uint8Array(message.data.data)]);
            const url = URL.createObjectURL(blob);
            const a = document.createElement('a');
            a.href = url;
            a.download = message.data.filename;
            document.body.appendChild(a);
            a.click();
            document.body.removeChild(a);
            URL.revokeObjectURL(url);
          }
        });

        const message: WorkerStart = {
          fs: this.fs,
          stdin_buffer: this.in.buffer,
          is_debug: this.debug,
        };
        worker.postMessage(message);
      });
    } catch (err: unknown) {
      console.log(`Unexpected error: ${err}`);
    } finally {
      this.rejector = undefined;
      this.out.removeWorker(worker);
      this.err.removeWorker(worker);
      this.in.clear();
      this.debugger[Internals].removeWorker(worker);
      worker.terminate();
    }
  }
}

class StdoutStream {
  public readonly stream: ReadableStream<Uint8Array<ArrayBuffer>>;
  private controller?: ReadableStreamDefaultController<Uint8Array<ArrayBuffer>>;
  private callback: (event: MessageEvent<WorkerOut>) => void;

  constructor(public readonly mode: StdoutMode) {
    this.stream = new ReadableStream({
      start: (controller) => (this.controller = controller),
    });

    this.callback = ((event: MessageEvent<WorkerOut>) => {
      const msg = event.data;
      if (msg.type !== 'stdout') return;
      if (msg.mode !== this.mode) return;
      this.controller?.enqueue(msg.data as Uint8Array<ArrayBuffer>);
    }).bind(this);
  }

  public addWorker(worker: Worker) {
    worker.addEventListener('message', this.callback);
  }

  public removeWorker(worker: Worker) {
    worker.removeEventListener('message', this.callback);
  }
}

class StdinStream {
  /**
   * Ring buffer to store stdin data.
   *
   * - TypeScript controls write_index, Rust controls read_index
   * - One slot is always kept empty to distinguish full from empty
   */

  private static readonly BUFFER_SIZE = 16;
  private static readonly HEADER_SIZE = 8; // 2 x i32
  private static readonly DATA_SIZE = StdinStream.BUFFER_SIZE - StdinStream.HEADER_SIZE;
  private static readonly READ_IDX = 0;
  private static readonly WRITE_IDX = 1;

  public readonly buffer = new SharedArrayBuffer(StdinStream.BUFFER_SIZE);
  public readonly stream: WritableStream<Uint8Array>;

  // Made this to manage indexes easier
  private readonly indices: Int32Array;
  private readonly data: Int8Array;

  constructor() {
    this.stream = new WritableStream({
      write: (chunk) => this.write(chunk),
    });
    this.indices = new Int32Array(this.buffer, 0, 2);
    this.data = new Int8Array(this.buffer, StdinStream.HEADER_SIZE);
  }

  public clear() {
    this.indices.fill(0);
  }

  private async write(chunk: Uint8Array): Promise<void> {
    const { DATA_SIZE, READ_IDX, WRITE_IDX } = StdinStream;
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
export * from './debugger/harness';
