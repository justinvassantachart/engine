import { Debugger, LocationInfo } from '.';
import { Internals } from '../internals';

export class BreakpointHit {
  static [Internals] = {
    create: (debug: Debugger, location: LocationInfo) => new BreakpointHit(debug, location),
  };

  private readonly debugger: Debugger;

  public resume() {
    this.debugger.resume();
  }

  private constructor(
    debug: Debugger,
    public readonly location: LocationInfo
  ) {
    this.debugger = debug;
  }
}
