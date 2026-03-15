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

  /** Stack frames at the pause point (innermost first). Variables are resolved lazily per frame. */
  public get frames(): ReadonlyArray<PausedStackFrame> {
    return this._frames.map((f, i) => new PausedStackFrame(this.debugger, f, i));
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
  private _variables: Variable[] | null = null;

  public constructor(
    private readonly dbg: Debugger,
    private readonly raw: StackFrame,
    /** Index within the pause's frames array (0 = innermost). */
    public readonly frameIndex: number
  ) {}

  /** Index into `debugInfo.functions`. */
  public get functionIndex(): number {
    return this.raw.function;
  }

  /** Function name from DWARF (e.g. "main", "ret1"). */
  public get name(): string {
    return this.raw.name;
  }

  /**
   * Variables for this frame, resolved lazily on first access via the host.
   * Each entry has `.name`, `.ty` (type name string), and `.value` (formatted string).
   */
  public variables(): Variable[] {
    if (this._variables === null) {
      this._variables = this.dbg.getVariablesForFrame(this.frameIndex);
    }
    return this._variables;
  }
}
