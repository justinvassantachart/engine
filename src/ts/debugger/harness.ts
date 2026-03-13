import type { DebugInfo, StackFrame, TypeEncoding } from '../../../pkg/runtime';
import { PausedStackFrame } from './hit';
import type { Debugger } from './index';

let ran = false;

/**
 * Dev-only harness to validate TS-side stitching logic (StackVariable -> DebugVariable -> DebugType)
 * without depending on Rust to populate real breakpoint frames yet.
 *
 * This mutates the passed debugger instance by injecting a fake `debugger.info`.
 */
export function runDebuggerStitchingHarness(debuggerInstance: Debugger): void {
  if (ran) return;
  ran = true;

  const fakeInfo: DebugInfo = {
    memory: {
      // These are not used by stitching; keep minimal plausible values.
      // The exact `MemoryType` shape is opaque here, so use a typed placeholder.
      main: {} as unknown as DebugInfo['memory']['main'],
      debug: {} as unknown as DebugInfo['memory']['debug'],
    },
    locations: [],
    files: [],
    types: [
      { name: 'int', size: 4, encoding: { type: 'signed' } },
      { name: 'float', size: 4, encoding: { type: 'float' } },
      { name: 'int*', size: 4, encoding: { type: 'address', at: 0 } },
    ],
    functions: [
      {
        name: 'foo',
        address: 0,
        variables: [
          { name: 'a', ty: 0 },
          { name: 'b', ty: 1 },
          { name: 'p', ty: 2 },
        ],
      },
    ],
  };

  const fakeFrame: StackFrame = {
    function: 0,
    variables: [
      {
        index: 0,
        pieces: [{ bit_size: null, bit_offset: null, location: { type: 'value', value: 123 } }],
      },
      {
        index: 1,
        pieces: [
          { bit_size: null, bit_offset: null, location: { type: 'value', value: 0x3f800000 } },
        ],
      },
      {
        index: 2,
        pieces: [
          { bit_size: null, bit_offset: null, location: { type: 'address', address: 0x1000 } },
        ],
      },
    ],
  };

  // Inject fake info so PausedStackFrame can resolve types/names.
  (debuggerInstance as unknown as { _info?: DebugInfo })._info = fakeInfo;

  const frame = new PausedStackFrame(debuggerInstance, fakeFrame, 0);
  const locals = frame.locals();

  const summary = locals.map((l) => ({
    name: l.name,
    type: l.type.name,
    encoding: (l.type.encoding as TypeEncoding).type,
    piece0: l.pieces[0]?.location.type,
  }));

   
  console.log('[debugger harness] stitched locals:', summary);

  if (locals.length !== 3) {
    throw new Error(`[debugger harness] expected 3 locals, got ${locals.length}`);
  }
  if (locals[0].name !== 'a' || locals[0].type.name !== 'int') {
    throw new Error('[debugger harness] local[0] did not stitch correctly');
  }
  if (locals[2].name !== 'p' || locals[2].type.encoding.type !== 'address') {
    throw new Error('[debugger harness] local[2] pointer type did not stitch correctly');
  }
}
