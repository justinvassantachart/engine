import { WorkerOut, WorkerStart } from '../../pkg/runtime';
import RustWorker from './worker?worker&inline';

export type Lang = 'c';

export type ExecutionResult = never;

export class Runtime {
  stdin: WritableStream<Uint8Array>;
  stdout: ReadableStream<Uint8Array>;
  stderr: ReadableStream<Uint8Array>;

  static async create(lang: Lang): Promise<void> {
    const worker = new RustWorker();
    await new Promise<void>((resolve) => {
      worker.onmessage = (e: MessageEvent<WorkerOut>) => e.data.type === 'ready' && resolve();
    });

    /** At this point in the code, the worker is ready to receive messages */
    worker.onmessage = (e) => console.log(e);
    const message: WorkerStart = {
      fs: {
        'main.c': `#include <vector> \n\n int main() { return 0; }`,
      },
    };
    worker.postMessage(message);

    throw new Error(lang);
  }

  constructor() {
    const { writable: stdin } = new TransformStream<Uint8Array, Uint8Array>();
    const { readable: stdout } = new TransformStream<Uint8Array, Uint8Array>();
    const { readable: stderr } = new TransformStream<Uint8Array, Uint8Array>();
    this.stdin = stdin;
    this.stdout = stdout;
    this.stderr = stderr;
  }

  async run(): Promise<ExecutionResult> {
    throw new Error();
  }

  /* visit later:
        runtime.stdout.pipeTo(console.log);
        runtime.stdin.write("haha ");
   */
}
