import EventEmitter from 'events';

import type { LocationInfo as RustLocation, WorkerOut } from '../../../pkg/runtime';
import init, { DebugHost } from '../../../pkg/runtime';
import wasmBinary from '../../../pkg/runtime_bg.wasm';
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

    /** Rust Host instance, created when DebugInfo arrives (after the wasm module has loaded). */
    host: DebugHost | null;

    /**
     * Int32Array view over bytes 0..11 of the SharedArrayBuffer.
     * Used with `Atomics.notify()` to resume execution after a breakpoint hit.
     */
    sentinel: Int32Array;

    /**
     * Uint8Array view over bytes 16+ of the SharedArrayBuffer.
     * `flags[N]` is the enable count for location N (0-based).
     * A location is "hit" if its count is positive.
     */
    flags: Uint8Array;
  };

  private _locations: Array<LocationInfo> = [];
  public get locations(): ReadonlyArray<LocationInfo> {
    return this._locations;
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
      host: null,
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

  private async onMessage(event: MessageEvent<WorkerOut>) {
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

      const info = data.info;
      await init(wasmBinary);
      this[Internals].host = new DebugHost(info);
      this[Internals].sentinel = new Int32Array(info.breakpoints, 0, 4);
      this[Internals].flags = new Uint8Array(info.breakpoints, 16);
      this._breakpoints.forEach((bp) => bp[Internals].resolve());
      this.resume();
      return;
    }

    if (data.type === 'breakpoint') {
      const index = this[Internals].sentinel[2];
      const loc = this._locations[index];
      if (!loc) return; // TODO: possible deadlock if no hit registered but worker waiting?
      const hit = BreakpointHit[Internals].create(this, loc, []);
      this.emit('breakpoint', hit, this);
    }
  }

  private removeWorker(worker: Worker): void {
    worker.removeEventListener('message', this.onMessage);
  }
}

export * from './breakpoint';
export * from './hit';
