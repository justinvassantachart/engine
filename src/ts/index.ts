import { WorkerOut, WorkerStart } from '../../pkg/runtime';
import RustWorker from './worker?worker&inline';

export type Lang = 'c';

export type ExecutionResult = never;

export class Runtime {
  static async create(lang: Lang): Promise<void> {
    const worker = new RustWorker();
    await new Promise<void>((resolve) => {
      worker.onmessage = (e: MessageEvent<WorkerOut>) => e.data.type === 'ready' && resolve();
    });

    /** At this point in the code, the worker is ready to receive messages */
    worker.onmessage = (e) => console.log(e);
    const message: WorkerStart = {
      fs: {
        'main.c': `#include <stdio.h>`,
      },
    };
    worker.postMessage(message);

    throw new Error(lang);
  }

  async run(): Promise<ExecutionResult> {
    throw new Error();
  }

  /* visit later:
        runtime.stdout.pipeTo(console.log);
        runtime.stdin.write("haha ");
   */
}
