import { Debugger, LocationInfo } from '.';
import type {
  DebugFunction,
  DebugInfo,
  DebugType,
  DebugVariable,
  Piece,
  StackFrame,
  StackVariable,
} from '../../../pkg/runtime';
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

  /** Stack frames at the pause point (last = most recent). */
  public get frames(): ReadonlyArray<PausedStackFrame> {
    return this._frames.map((f, i) => new PausedStackFrame(this.debugger, f, i));
  }

  /** Most recent stack frame (if any). */
  public get frame(): PausedStackFrame | undefined {
    const frames = this.frames;
    return frames.length > 0 ? frames[frames.length - 1] : undefined;
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

export type PausedLocal = Readonly<{
  /** Variable name from DWARF. */
  name: string;
  /** Variable definition (contains `ty` index into `debugInfo.types`). */
  variable: DebugVariable;
  /** Resolved debug type for this variable. */
  type: DebugType;
  /** Runtime-reconstructed pieces for this variable's value. */
  pieces: readonly Piece[];
  /** Index into `DebugFunction.variables`. */
  variableIndex: number;
}>;

export class PausedStackFrame {
  public constructor(
    private readonly debug: Debugger,
    private readonly raw: StackFrame,
    /** Index within the pause's frames array. */
    public readonly frameIndex: number
  ) {}

  /** Index into `debugInfo.functions`. */
  public get functionIndex(): number {
    return this.raw.function;
  }

  public get function(): DebugFunction | undefined {
    return this.debug.info?.functions[this.raw.function];
  }

  /**
   * Resolve locals by stitching:
   * `StackVariable.index` -> `DebugFunction.variables[index]` -> `DebugVariable.ty` -> `DebugInfo.types[ty]`.
   */
  public locals(): PausedLocal[] {
    const info: DebugInfo | undefined = this.debug.info;
    if (!info) return [];

    const fn = info.functions[this.raw.function];
    if (!fn) return [];

    const out: PausedLocal[] = [];
    for (const sv of this.raw.variables) {
      const local = resolveLocal(info, fn, sv);
      if (local) out.push(local);
    }
    return out;
  }
}

function resolveLocal(info: DebugInfo, fn: DebugFunction, sv: StackVariable): PausedLocal | null {
  const variableIndex = sv.index;
  const variable = fn.variables[variableIndex];
  if (!variable) return null;

  const type = info.types[variable.ty];
  if (!type) return null;

  return {
    name: variable.name,
    variable,
    type,
    pieces: sv.pieces,
    variableIndex,
  };
}
