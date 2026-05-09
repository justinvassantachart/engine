import EventEmitter from 'events';

import { DapAdapter, WorkerOut } from '../../pkg/engine';
import { Internals } from './util';

type DebuggerEventMap = {
  event: [unknown];
  artifact: [Artifact];
};

export class Artifact {
  public readonly name: string;
  public readonly data: Uint8Array;

  constructor(name: string, data: Uint8Array) {
    this.name = name;
    this.data = data;
  }

  public download() {
    const bytes = new Uint8Array(this.data);
    const blob = new Blob([bytes]);
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = this.name;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
  }
}

export class Debugger extends EventEmitter<DebuggerEventMap> {
  /**
   * Access to internal properties of the debugger.
   * These are put under a special symbol so that they cannot be accessed by
   * clients of the library.
   */
  [Internals]: {
    attach(worker: Worker): void;
  };

  private readonly dap: DapAdapter;

  constructor() {
    super();
    this[Internals] = {
      attach: this.attach.bind(this)
    };

    this.onMessage = this.onMessage.bind(this);
    this.dap = new DapAdapter();
    this.dap.on(this.onMessage);
  }

  public send(message: unknown): unknown {
    // return the response from the DAP adapter. This is sync. DAP events are async and emitted through the on('event') listener.
    return this.dap.sendMessage(message);
  }

  private attach(worker: Worker) {
    worker.addEventListener('message', (event: MessageEvent<WorkerOut>) => {
      if (event.data.type !== 'artifact') return;
      const artifact = new Artifact(event.data.name, new Uint8Array(event.data.data));
      this.emit('artifact', artifact);
    });
    this.dap.attach(worker);
  }

  private onMessage(message: unknown) {
    this.emit('event', message);
  }
}
