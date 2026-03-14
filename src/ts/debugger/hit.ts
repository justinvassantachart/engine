import { Debugger, LocationInfo } from '.';
import type { StackFrame, Variable } from '../../../pkg/runtime';
import { Internals } from '../internals';

export class BreakpointHit {
  static [Internals]: {
    create: (debug: Debugger, location: LocationInfo, frames: StackFrame[]) => BreakpointHit;
  } = {
    create: (debug, location, frames) => new BreakpointHit(debug, location, frames),
  };

  private readonly debugger: Debugger;
  private readonly _frames: StackFrame[];

  public resume() {
    this.debugger.resume();
  }

  /** Stack frames at the pause point (innermost first). */
  public get frames(): ReadonlyArray<PausedStackFrame> {
    return this._frames.map((f, i) => new PausedStackFrame(f, i));
  }

  /** Most recent (innermost) stack frame, if any. */
  public get frame(): PausedStackFrame | undefined {
    return this.frames[0];
  }

  private constructor(
    debug: Debugger,
    public readonly location: LocationInfo,
    frames: StackFrame[]
  ) {
    this.debugger = debug;
    this._frames = frames;
  }
}

export class PausedStackFrame {
  public constructor(
    private readonly raw: StackFrame,
    /** Index within the pause's frames array (0 = innermost). */
    public readonly frameIndex: number
  ) {}

  /** Index into `debugInfo.functions`. */
  public get functionIndex(): number {
    return this.raw.function;
  }

  /**
   * Variables resolved by Rust for this frame.
   * Each entry has `.name`, `.ty` (type name string), and `.value` (formatted string).
   */
  public variables(): Variable[] {
    return Array.from(this.raw.variables) as Variable[];
  }
}
