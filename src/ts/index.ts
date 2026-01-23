import { StdoutMode, WorkerOut, WorkerStart } from '../../pkg/runtime';
import RustWorker from './worker?worker&inline';

export type Lang = 'c';

export class Runtime {
  public readonly lang: Lang;

  private out = new StdoutStream(1);
  private err = new StdoutStream(2);
  private in = new StdinStream();

  stdin: WritableStream<Uint8Array<ArrayBuffer>>;
  stdout: ReadableStream<Uint8Array<ArrayBuffer>>;
  stderr: ReadableStream<Uint8Array<ArrayBuffer>>;

  static create(lang: Lang): Runtime {
    return new Runtime(lang);
  }

  private constructor(lang: Lang) {
    this.lang = lang;

    const { writable: stdin } = new TransformStream<Uint8Array, Uint8Array>();
    this.stdin = stdin;
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
      fs: {
        'main.c': `#include <iostream> \n\n int main() { std::cout << "hello world" << std::endl; return 0; }`,
      },
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
  public readonly buffer = new SharedArrayBuffer(8 * 1024);
}
