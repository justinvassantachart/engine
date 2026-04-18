# Integration Guide

This guide is for teams building an IDE or editor on top of this runtime. It covers setting up code execution and the debugger.

---

## Installation

```sh
npm install runtime
```

The package ships a WebAssembly binary and TypeScript bindings. Initialize it once before use:

```ts
import { Runtime } from 'runtime';

const rt = await Runtime.create('c');
```

---

## Running Code

Set the virtual filesystem, then call `run()`. The program sees `/main.c` as its source file.

```ts
rt.fs = {
  'main.c': `#include <iostream>\nint main() { std::cout << "hello\\n"; }`,
};

await rt.run();
```

**stdout / stderr** are exposed as `ReadableStream<Uint8Array>`:

```ts
rt.stdout.pipeTo(
  new WritableStream({
    write(chunk) {
      console.log(new TextDecoder().decode(chunk));
    },
  })
);
```

**stdin** is a `WritableStream<Uint8Array>`:

```ts
const writer = rt.stdin.getWriter();
writer.write(new TextEncoder().encode('hello\n'));
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
      breakpoints: [{ line: 5 }],
    },
  });

  dbg.send({
    type: 'request',
    seq: seq++,
    command: 'setExceptionBreakpoints',
    arguments: { filters: [] },
  });

  dbg.send({ type: 'request', seq: seq++, command: 'configurationDone', arguments: {} });
});

dbg.send({ type: 'request', seq: seq++, command: 'initialize', arguments: {} });
await rt.run();
```

### Handling a Breakpoint Hit

When the program hits a breakpoint, a `stopped` event is emitted:

```ts
if (msg.type === 'event' && msg.event === 'stopped') {
  // inspect the stack
  const res = dbg.send({
    type: 'request',
    seq: n++,
    command: 'stackTrace',
    arguments: { threadId: 1 },
  });

  // get scopes for a frame
  dbg.send({ type: 'request', seq: n++, command: 'scopes', arguments: { frameId: 0 } });

  // get variables for a scope (variablesReference comes from scopes response)
  dbg.send({
    type: 'request',
    seq: n++,
    command: 'variables',
    arguments: { variablesReference: 1 },
  });

  // resume
  dbg.send({ type: 'request', seq: n++, command: 'continue', arguments: { threadId: 1 } });
}
```

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
