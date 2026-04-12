import EventEmitter from 'events';

import { DapAdapter, LocationInfo as RustLocation } from '../../pkg/runtime';
import { Internals } from './internals';

export type LocationInfo = Omit<RustLocation, 'file'> & {
  readonly file: string;
};

type DebuggerEventMap = {
  event: [unknown];
};

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
      attach: this.attach.bind(this),
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
    this.dap.attach(worker);
  }

  private onMessage(message: unknown) {
    this.emit('event', message);
  }
}
