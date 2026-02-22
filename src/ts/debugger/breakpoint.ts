import { Debugger } from '.';
import { Internals } from '../internals';

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
      const flags = this.debugger[Internals].flags;
      if (idx < 0 || idx >= flags.length) throw new Error(`OOB breakpoint flag access: ${idx}`);

      if (enabled) flags[idx]++;
      else if (flags[idx] === 0)
        throw new Error(`Attempt to make breakpoint flag at ${idx} negative`);
      else flags[idx]--;
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
          /** TODO: Breaking on function names not supported yet */
          if (fn !== undefined) continue;
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
