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

The debugger exposes a [Debug Adapter Protocol](https://microsoft.github.io/debug-adapter-protocol/) interface. Requests are sent synchronously and return a response. Events are emitted asynchronously through the `dap` listener.

```ts
const dbg = rt.debugger;

dbg.on('dap', (msg) => {
  // receives both events (type: 'event') and — if you choose to route them here — responses
  console.log(msg);
});
```

### Initialization Sequence

The DAP initialization sequence must happen after `run()` is called but before the program starts executing. The runtime pauses automatically before execution and waits for `configurationDone`.

```ts
// 1. Send initialize immediately after calling run()
dbg.send({ type: 'request', seq: 1, command: 'initialize', arguments: {} });
// → response: capabilities

// 2. Wait for the `initialized` event — fires when compilation is done and
//    the runtime is ready to accept configuration.
dbg.on('dap', (msg) => {
  if (msg.type === 'event' && msg.event === 'initialized') {
    // 3. Set breakpoints (one call per source file)
    dbg.send({
      type: 'request',
      seq: 2,
      command: 'setBreakpoints',
      arguments: {
        source: { path: '/main.c' },
        breakpoints: [{ line: 5 }],
      },
    });

    // 4. Signal that configuration is done — the program starts running after this
    dbg.send({ type: 'request', seq: 3, command: 'configurationDone', arguments: {} });
  }
});
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
- `send()` returns the response synchronously. Events arrive asynchronously via `on('dap', ...)`.
