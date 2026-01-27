import { StdoutMode, WorkerOut, WorkerStart } from '../../pkg/runtime';
import RustWorker from './worker?worker&inline';

export type Lang = 'c';

// TODO: Find a way to re-use the generated types in `pkg/runtime.d.ts`
export type FsNode = string | DirNode;
export type DirNode = { [name: string]: FsNode };

export class Runtime {
  private out = new StdoutStream(1);
  private err = new StdoutStream(2);
  private in = new StdinStream();
  private currentWorker: Worker | null = null; // current worker instance
  private stopResolver: ((value: void) => void) | null = null; // promise resolver for stop()

  /**
   * The programming language of this runtime.
   */
  public readonly lang: Lang;

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
    if (this.currentWorker) {
      this.currentWorker.terminate();
      this.out.removeWorker(this.currentWorker);
      this.err.removeWorker(this.currentWorker);
      this.currentWorker = null;
      this.in.clear();
      // Resolve the stop promise if it exists, so run() doesn't hang
      if (this.stopResolver) {
        this.stopResolver();
        this.stopResolver = null;
      }
    }
  }

  // TODO: Make this function reentrant
  async run(): Promise<void> {
    const worker = new RustWorker();
    this.currentWorker = worker;

    // Store stop callback reference so error handler can clean it up
    let stopCallback: ((message: MessageEvent<WorkerOut>) => void) | null = null;
    let stopResolved = false;

    /* Set up error event listener to catch any errors from the worker */
    const errorHandler = (event: ErrorEvent) => {
      console.error('Worker error:', event.error || event.message, event);
      // Ensure cleanup happens and stop promise resolves
      if (this.stopResolver) {
        stopResolved = true;
        this.stopResolver();
        this.stopResolver = null;
      }
      // Remove stop message listener if it exists
      if (stopCallback) {
        worker.removeEventListener('message', stopCallback);
        stopCallback = null;
      }
      // Clean up worker references
      this.out.removeWorker(worker);
      this.err.removeWorker(worker);
      this.in.clear();
      if (this.currentWorker === worker) {
        this.currentWorker = null;
      }
    };
    worker.addEventListener('error', errorHandler);

    /* Set up handling for stdout/stderr */
    this.out.addWorker(worker);
    this.err.addWorker(worker);

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

    /* At this point in the code, the worker is ready to receive messages */
    worker.onmessage = (e) => console.log(e);

    /**
     * Run the worker, and wait for it to send us a Stop message.
     * Note that we must set up the listener *before* running the worker
     * to avoid a race condition.
     *
     * The worker will always send a 'stop' message:
     * - On successful completion
     * - On errors/panics (via StopGuard Drop implementation)
     *
     * The two ways stopping happens:
     * 1. The user calls stop() (manually terminates worker)
     * 2. The worker sends us a Stop message (normal completion or error)
     */
    const stop = new Promise<void>((resolve) => {
      this.stopResolver = resolve;

      const doResolve = () => {
        if (stopResolved) return;
        stopResolved = true;
        if (stopCallback) {
          worker.removeEventListener('message', stopCallback);
          stopCallback = null;
        }
        this.stopResolver = null;
        resolve();
      };

      stopCallback = (message: MessageEvent<WorkerOut>) => {
        if (message.data.type === 'stop') {
          doResolve();
        }
      };

      worker.addEventListener('message', stopCallback);
    });

    const message: WorkerStart = {
      fs: this.fs,
      stdin_buffer: this.in.buffer,
      is_debug: false,
    };
    worker.postMessage(message);

    await stop;

    /* This is just good hygiene */
    worker.removeEventListener('error', errorHandler);
    if (stopCallback) {
      worker.removeEventListener('message', stopCallback);
    }
    this.out.removeWorker(worker);
    this.err.removeWorker(worker);
    this.in.clear();
    this.currentWorker = null;
    this.stopResolver = null;
  }

  /* visit later:
        runtime.stdout.pipeTo(console.log);
        runtime.stdin.write("haha ");
   */
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
