# Integration Guide

This guide is for teams building an IDE or editor on top of this runtime. It covers setting up code execution and the debugger.

---

## Installation

```sh
npm install debugger-sh
```

The package ships a WebAssembly binary and TypeScript bindings. Initialize it once before use:

```ts
import { Runtime } from 'debugger-sh';

const rt = await Runtime.create('c');
```

---

## Running Code

Set the virtual filesystem, then call `run()`. The program sees `/main.c` as its source file.

```ts
rt.fs = {
  'main.c': `#include <iostream>\nint main() { std::cout << "hello\\n"; }`
};

await rt.run();
```

**stdout / stderr** use a small event-style API: subscribe with `on('data', …)` and unsubscribe with `off` using the same listener function. Each chunk is a `Uint8Array` of UTF-8 bytes.

```ts
const decoder = new TextDecoder();
const onOut = (chunk: Uint8Array) => {
  console.log(decoder.decode(chunk));
};
rt.stdout.on('data', onOut);
rt.stderr.on('data', onOut);

// When tearing down (optional if the runtime is discarded):
rt.stdout.off('data', onOut);
rt.stderr.off('data', onOut);
```

**stdin** exposes `write(value: string | Uint8Array)` (UTF-8 for strings):

```ts
await rt.stdin.write('hello\n');
await rt.stdin.write(new TextEncoder().encode('hello\n'));
```

To stop a running program:

```ts
rt.stop();
```

---

## Debugger (DAP)

The debugger exposes a [Debug Adapter Protocol](https://microsoft.github.io/debug-adapter-protocol/) interface. Requests are sent synchronously and return a response. DAP messages (events, and optionally routed responses) are emitted asynchronously through the `event` listener.

```ts
const dbg = rt.debugger;

dbg.on('event', (msg) => {
  // receives both events (type: 'event') and — if you choose to route them here — responses
  console.log(msg);
});
```

### Initialization Sequence

Order matches the usual DAP lifecycle:

1. **Client →** `initialize` request
2. **Adapter →** `initialize` response (body includes **Capabilities**, e.g. `supportsConfigurationDoneRequest`)
3. **Adapter** builds the internal debugger when the worker sends its `debug` message (instrumented binary ready).
4. **Adapter →** `initialized` event — emitted only after step **2** has completed **and** step **3** has happened (so the client never configures before the adapter is ready).
5. **Client →** `setBreakpoints` (zero or more; one request per source file)
6. **Client →** `setFunctionBreakpoints` if `supportsFunctionBreakpoints` is true (this runtime advertises `false`; you can omit it)
7. **Client →** `setExceptionBreakpoints` when you have filters to set
8. **Client →** `configurationDone`
9. **Adapter →** `configurationDone` response — the debuggee then leaves its initial wait and **starts running**

Call `run()` when the worker should compile and execute; the worker blocks until step **8** completes. A typical pattern is: register `dbg.on('event', …)`, send **`initialize`**, then **`await rt.run()`** (which starts the worker). React to **`initialized`** with steps **5–8**.

```ts
let seq = 1;

dbg.on('event', (msg: { type: string; event?: string }) => {
  if (msg.type !== 'event' || msg.event !== 'initialized') return;

  dbg.send({
    type: 'request',
    seq: seq++,
    command: 'setBreakpoints',
    arguments: {
      source: { path: '/main.c' },
      breakpoints: [{ line: 5 }]
    }
  });

  dbg.send({
    type: 'request',
    seq: seq++,
    command: 'setExceptionBreakpoints',
    arguments: { filters: [] }
  });

  dbg.send({ type: 'request', seq: seq++, command: 'configurationDone', arguments: {} });
});

dbg.send({ type: 'request', seq: seq++, command: 'initialize', arguments: {} });
await rt.run();
```

### Handling a pause (`stopped`)

Whenever the debuggee stops—on a **line breakpoint** or after a **step** request—the adapter emits a `stopped` event. Use `body.reason` to tell them apart:

- **`breakpoint`** — the worker paused in normal mode because execution reached a line where you set a breakpoint.
- **`step`** — the worker paused while a step mode was active (`next`, `stepIn`, or `stepOut`). The next section describes how those modes work internally.

`threadId` is always `1` (single-threaded runtime).

```ts
if (msg.type === 'event' && msg.event === 'stopped') {
  const res = dbg.send({
    type: 'request',
    seq: n++,
    command: 'stackTrace',
    arguments: { threadId: 1 }
  }) as { body?: { stackFrames?: { id: number }[] } };
  const top = res.body?.stackFrames?.[0];
  if (!top) return;

  const scopesRes = dbg.send({
    type: 'request',
    seq: n++,
    command: 'scopes',
    arguments: { frameId: top.id }
  }) as { body?: { scopes?: { variablesReference: number }[] } };
  const localsRef = scopesRes.body?.scopes?.find((s) => s.name === 'Locals')?.variablesReference;
  if (localsRef == null) return;

  dbg.send({
    type: 'request',
    seq: n++,
    command: 'variables',
    arguments: { variablesReference: localsRef }
  });

  dbg.send({ type: 'request', seq: n++, command: 'continue', arguments: { threadId: 1 } });
}
```

### Stepping

Stepping does **not** use a separate single-stepping primitive in the CPU. The program is compiled with **instrumentation**: at each debuggable machine location there is a shared hook that can stop execution. The main thread and the worker coordinate through a small prefix on the **same `SharedArrayBuffer`** that also holds per-location breakpoint enable flags (see `DebugInfo` / `BP_PREFIX_BYTES` in the Rust sources).

That prefix (exposed to JS as the first elements of `get_bp_state()`, an `Int32Array` view) is laid out conceptually as:

| Index | Role                                                                                                                                                                                                |
| ----- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `0`   | Stack pointer handshake: non-zero while paused, cleared to resume                                                                                                                                   |
| `1`   | **Execution mode** — what to do at the next instrumented sites the worker reaches                                                                                                                   |
| `2`   | **`last_sp`** — stack pointer saved when the _previous_ pause ended; used to implement step-over and step-out                                                                                       |
| `3`   | **`last_stop_mode`** — mode that was active when the worker _decided_ to pause this time (written before mode is reset); the adapter uses this to set DAP `stopped.reason` (`breakpoint` vs `step`) |

**Modes** (`1`, written by the main-thread `Debugger` before waking the worker):

| Value | Name      | Meaning at instrumentation sites                                                                         |
| ----- | --------- | -------------------------------------------------------------------------------------------------------- |
| `0`   | Normal    | Stop only at locations where you have set a breakpoint (`setBreakpoints`).                               |
| `1`   | Step into | Stop at the next instrumented site that runs (enters callees if the next site is there).                 |
| `2`   | Step over | Stop only when the stack pointer is **≥ `last_sp`** (same or outer frame versus where you stepped from). |
| `3`   | Step out  | Stop only when the stack pointer is **> `last_sp`** (strictly outer frame).                              |

DAP wiring:

- **`continue`** — set mode to normal and wake the worker; variable handles from the previous pause are cleared.
- **`next`** — set mode to step-over, then wake.
- **`stepIn`** — set mode to step-into, then wake.
- **`stepOut`** — set mode to step-out, then wake.

After each successful stop, the worker resets mode to **normal** and updates **`last_sp`** to the current stack pointer so the next `next` / `stepOut` is relative to the line you actually landed on. The worker posts a minimal `breakpoint` message to the main thread; **pause classification for DAP** (`stopped.reason`) comes from reading **`last_stop_mode`** on that shared buffer, not from fields on the worker message.

**Caveats:**

- Stepping is **line-oriented** over instrumented WASM PCs, not a hardware single-step.
- Very dense control flow (e.g. multiple statements on one line) follows whatever the instrumentation map does—validate behavior with `npm run tools:dap` if you rely on edge cases.

### Supported Commands

| Command                   | Description                           |
| ------------------------- | ------------------------------------- |
| `initialize`              | Start session, returns capabilities   |
| `configurationDone`       | Signal setup complete, program starts |
| `setBreakpoints`          | Set breakpoints for a source file     |
| `setFunctionBreakpoints`  | Empty when advertised unsupported     |
| `setExceptionBreakpoints` | Accepted but no-op                    |
| `threads`                 | Returns a single `main` thread        |
| `stackTrace`              | Returns the current call stack        |
| `scopes`                  | Returns variable scopes for a frame   |
| `variables`               | Returns variables for a scope         |
| `continue`                | Resume execution                      |
| `next`                    | Step over                             |
| `stepIn`                  | Step into                             |
| `stepOut`                 | Step out                              |
| `disconnect`              | End session                           |

### Program End

When the program finishes, a `terminated` event is emitted:

```ts
if (msg.type === 'event' && msg.event === 'terminated') {
  // clean up debugger UI
}
```

---

## Notes

- The runtime compiles C++ to WASM in-browser using clang — the first run may take a few seconds.
- There is one thread (`id: 1`). Multi-threading is not supported.
- `send()` returns the response synchronously. DAP traffic that is pushed from the adapter arrives asynchronously via `on('event', ...)`.
- Variable handles (`variablesReference` from `scopes` / `variables`) are invalidated when you **`continue`** or issue a **step** request; always re-query after the next `stopped`.
- Scripted DAP scenarios live under `tools/dap/tests/`. From the repository root, run **`npm run tools:dap`** to execute the suite (optionally `npm run tools:dap -- <test-name>` for a single case).
