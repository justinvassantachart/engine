import EventEmitter from 'eventemitter3';

// import EventEmitter from 'events';

import type {
  DebugInfo as RustDebugInfo,
  LocationInfo as RustLocation,
  StackFrame as RustStackFrame,
  WorkerOut,
} from '../../../pkg/runtime';
import { Internals } from '../internals';
import { Breakpoint, BreakpointSpecifier } from './breakpoint';
import { BreakpointHit } from './hit';

export type LocationInfo = Omit<RustLocation, 'file'> & {
  readonly file: string;
};

type DebuggerEventMap = {
  breakpoint: [BreakpointHit, Debugger];
};

export class Debugger extends EventEmitter<DebuggerEventMap> {
  /**
   * Access to internal properties of the debugger.
   * These are put under a special symbol so that they cannot be accessed by
   * clients of the library.
   */
  [Internals]: {
    addWorker(worker: Worker): void;
    removeWorker(worker: Worker): void;

    /**
     * Int32Array view over bytes 0..3 of the SharedArrayBuffer.
     * Used with `Atomics.notify()` to resume execution after a breakpoint hit.
     */
    sentinel: Int32Array;

    /**
     * Uint8Array view over bytes 4+ of the SharedArrayBuffer.
     * `flags[N]` is the enable count for location N (0-based).
     * A location is "hit" if its count is positive.
     */
    flags: Uint8Array;
  };

  private _locations: Array<LocationInfo> = [];
  public get locations(): ReadonlyArray<LocationInfo> {
    return this._locations;
  }

  private _info?: RustDebugInfo;
  /** Latest debug info sent by the worker (if any). */
  public get info(): RustDebugInfo | undefined {
    return this._info;
  }

  private _memory?: WebAssembly.Memory;
  /** Program memory sent by the worker (if any). */
  public get memory(): WebAssembly.Memory | undefined {
    return this._memory;
  }

  private _breakpoints: Set<Breakpoint> = new Set();
  public get breakpoints(): ReadonlySet<Breakpoint> {
    return this._breakpoints;
  }

  constructor() {
    super();
    this[Internals] = {
      addWorker: this.addWorker.bind(this),
      removeWorker: this.removeWorker.bind(this),
      sentinel: new Int32Array(),
      flags: new Uint8Array(),
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

  /** Resume execution after a breakpoint hit. No-op if not paused. */
  public resume(): void {
    Atomics.add(this[Internals].sentinel, 0, 1);
    Atomics.notify(this[Internals].sentinel, 0);
  }

  private addWorker(worker: Worker): void {
    worker.addEventListener('message', this.onMessage);
  }

  private onMessage(event: MessageEvent<WorkerOut>) {
    const data = event.data;

    if (data.type === 'debug') {
      console.log(data);

      const { locations, files } = data.info;
      this._locations = locations.map((loc) => ({
        file: files[loc.file],
        line: loc.line,
        col: loc.col,
        address: loc.address,
      }));

      this._info = data.info;
      this._memory = data.memory as unknown as WebAssembly.Memory;
      this[Internals].sentinel = new Int32Array(data.breakpoint_buffer, 0, 1);
      this[Internals].flags = new Uint8Array(data.breakpoint_buffer, 4);
      this._breakpoints.forEach((bp) => bp[Internals].resolve());
      this.resume();
      return;
    }

    if (data.type === 'breakpoint') {
      const loc = this._locations[data.location_index];
      if (!loc) return; // TODO: possible deadlock if no hit registered but worker waiting?
      const hit = BreakpointHit[Internals].create(this, loc, data.frames as RustStackFrame[]);
      this.emit('breakpoint', hit, this);
    }
  }

  private removeWorker(worker: Worker): void {
    worker.removeEventListener('message', this.onMessage);
  }
}

export * from './breakpoint';
export * from './hit';
