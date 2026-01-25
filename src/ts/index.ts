import { StdoutMode, WorkerOut, WorkerStart } from '../../pkg/runtime';
import RustWorker from './worker?worker&inline';

export type Lang = 'c';

export class Runtime {
  public readonly lang: Lang;

  private out = new StdoutStream(1);
  private err = new StdoutStream(2);
  private in = new StdinStream();

  public fs: WorkerStart['fs'] = {};
  public stdin: WritableStream<Uint8Array<ArrayBuffer>>;
  public stdout: ReadableStream<Uint8Array<ArrayBuffer>>;
  public stderr: ReadableStream<Uint8Array<ArrayBuffer>>;

  static create(lang: Lang): Runtime {
    return new Runtime(lang);
  }

  private constructor(lang: Lang) {
    this.lang = lang;
    this.stdin = this.in.stream;
    this.stdout = this.out.stream;
    this.stderr = this.err.stream;
  }

  // TODO: Make this function reentrant
  async run(): Promise<void> {
    const worker = new RustWorker();

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
    const message: WorkerStart = {
      fs: this.fs,
      stdin_buffer: this.in.buffer,
    };
    worker.postMessage(message);

    /* Wait for the worker to send us a Stop message */
    await new Promise<void>((resolve) => {
      const callback = (message: MessageEvent<WorkerOut>) => {
        if (message.data.type === 'stop') {
          worker.removeEventListener('message', callback);
          resolve();
        }
      };
      worker.addEventListener('message', callback);
    });

    /* This is just good hygiene */
    this.out.removeWorker(worker);
    this.err.removeWorker(worker);
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

    console.log(this.buffer);
  }
}
