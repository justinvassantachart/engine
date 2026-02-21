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
          is_debug: true,
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

const Internals: unique symbol = Symbol();

export type Location = {
  /** The name of a function, e.g. `main`, which begins at this location. */
  readonly function?: string;
  readonly file: string;
  readonly line: number;
  readonly col: number;
};

export type BreakpointSpecifier = string | number;

export const BreakpointStatus = {
  /** Debug symbols have not been loaded yet, so the breakpoint has not been found yet */
  Pending: 'pending',
  /** This breakpoint has mapped onto a location in the most recently compiled binary */
  Resolved: 'resolved',
  /** This breakpoint has been removed and can no longer be used */
  Removed: 'removed',
} as const;

export type BreakpointStatus = (typeof BreakpointStatus)[keyof typeof BreakpointStatus];

export class Breakpoint {
  static [Internals] = {
    create: (debug: Debugger, where: BreakpointSpecifier) => new Breakpoint(debug, where),
  };

  [Internals]: {
    resolve(): void;
  };

  public readonly where: BreakpointSpecifier;

  private readonly debugger: Debugger;
  private _enabled = true;
  private _status: BreakpointStatus = BreakpointStatus.Pending;

  /**
   * The indices of locations in `debugger.locations` that this breakpoint resolves to.
   *
   * Note that a breakpoint can map onto multiple locations: consider an inline function `foo`
   * with multiple instantiations. Breaking on that function will cause us to break in multiple
   * different places, even though we only set one breakpoint e.g. `main.c:foo`.
   *  */
  private _locations: number[] = [];

  public get enabled() {
    return this._enabled;
  }

  public get status() {
    return this._status;
  }

  public enable(enabled = true) {
    if (this.enabled === enabled) return this;
    this._enabled = enabled;
    if (this.status !== BreakpointStatus.Resolved) return this;
    this._locations.forEach((idx) => {
      const buffer = this.debugger[Internals].buffer;
      if (idx < 0 || idx >= buffer.length) throw new Error(`OOB breakpoint buffer access: ${idx}`);

      if (enabled) buffer[idx]++;
      else if (buffer[idx] === 0)
        throw new Error(`Attempt to make breakpoint buffer at ${idx} negative`);
      else buffer[idx]--;
    });
    return this;
  }

  public disable() {
    return this.enable(false);
  }

  public remove() {
    this.disable();
    this._status = BreakpointStatus.Removed;
  }

  private constructor(debug: Debugger, where: BreakpointSpecifier) {
    this.debugger = debug;
    this.where = where;
    this[Internals] = { resolve: this.resolve.bind(this) };
  }

  private resolve(): void {
    if (this._status === BreakpointStatus.Removed) return;

    const locations = this.debugger.locations;
    this._locations = [];

    if (typeof this.where === 'number') {
      const idx = this.where;
      if (idx >= 0 && idx < locations.length) this._locations = [idx];
    }

    // GDB-style string: [file:][function|line]
    if (typeof this.where === 'string') {
      const colon = this.where.indexOf(':');
      const fileSpec = colon >= 0 ? this.where.slice(0, colon) : undefined;
      const rest = colon >= 0 ? this.where.slice(colon + 1) : this.where;

      const isLineNum = /^\d+$/.test(rest);
      const line = isLineNum ? parseInt(rest, 10) : undefined;
      const fn = isLineNum ? undefined : rest;

      for (let i = 0; i < locations.length; i++) {
        const loc = locations[i];
        if (fileSpec !== undefined && loc.file !== fileSpec && loc.file !== `/${fileSpec}`)
          continue;
        if (line !== undefined) {
          if (loc.line !== line) continue;
        } else if (fn !== undefined) {
          if (loc.function !== fn) continue;
        } else continue;

        this._locations.push(i);
      }
    }

    this._status = BreakpointStatus.Pending;
    if (this._locations.length > 0) this._status = BreakpointStatus.Resolved;

    // TODO: There is a race condition here where, once the worker sends over
    // the breakpoint buffer, it manages to start running code before the enabled
    // breakpoints were set in that buffer. However, considering that the linker
    // must have also ran before that point, I find it hard to believe that the
    // linker would run faster than this function, so probably not a huge issue
    // in practice.
    if (this._enabled) {
      this._enabled = false;
      this.enable(true);
    }
  }
}

export class Debugger {
  /**
   * Access to internal properties of the debugger.
   * These are put under a special symbol so that they cannot be accessed by
   * clients of the library.
   */
  [Internals]: {
    addWorker(worker: Worker): void;
    removeWorker(worker: Worker): void;

    /**
     * The breakpoint buffer.
     *
     * Each byte at index N in this buffer contains the number of breakpoints that are
     * enabled on the location with index N. For example, if I add two breakpoints, `b1` and `b2`, which
     * both map to location n, then `buffer[n] = 2`. A location will be "hit" if its number of
     * enabled breakpoints is positive.
     */
    buffer: Uint8Array;
  };

  private _locations: Array<Location> = [];
  public get locations(): ReadonlyArray<Location> {
    return this._locations;
  }

  private _breakpoints: Set<Breakpoint> = new Set();
  public get breakpoints(): ReadonlySet<Breakpoint> {
    return this._breakpoints;
  }

  constructor() {
    this[Internals] = {
      addWorker: this.addWorker.bind(this),
      removeWorker: this.removeWorker.bind(this),
      buffer: new Uint8Array(),
    };

    this.onMessage = this.onMessage.bind(this);
  }

  /**
   * Adds a breakpoint to the debugger.
   * @param where Either a GDB-style breakpoint location in the format `[file:][function|line]`
   * or the index of a specific location in {@link locations} to put a breakpoint on.
   * @returns A {@link Breakpoint} object that is initially configured to be hit.
   */
  public addBreakpoint(where: BreakpointSpecifier) {
    const bp = Breakpoint[Internals].create(this, where);
    this._breakpoints.add(bp);
    bp[Internals].resolve();
    return bp;
  }

  public removeBreakpoint(bp: Breakpoint) {
    if (!this._breakpoints.delete(bp)) return;
    bp.remove();
  }

  private addWorker(worker: Worker): void {
    worker.addEventListener('message', this.onMessage);
  }

  private onMessage(event: MessageEvent<WorkerOut>) {
    const data = event.data;
    if (data.type !== 'debug') return;

    this._locations = data.locations.map((loc) => ({
      file: data.files[loc.file],
      line: loc.line,
      col: loc.col,
      address: loc.address,
    }));

    this[Internals].buffer = new Uint8Array(data.breakpoint_buffer);
    this._breakpoints.forEach((bp) => bp[Internals].resolve());
  }

  private removeWorker(worker: Worker): void {
    worker.removeEventListener('message', this.onMessage);
  }
}
