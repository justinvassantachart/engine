# @jtrb/runtime

A browser-based C++ runtime powered by WebAssembly. Compile and run C++ programs entirely in the browser, with stdin/stdout/stderr I/O and a built-in debugger.

## Installation

```bash
npm install @jtrb/runtime
```

> **Requirements:** Your bundler or server must set these HTTP headers, as the runtime uses `SharedArrayBuffer` for stdin:
>
> ```
> Cross-Origin-Embedder-Policy: require-corp
> Cross-Origin-Opener-Policy: same-origin
> ```
>
> In Next.js, add these in `next.config.js`. In Vite, use the `vitePlugin` or configure your dev server headers.

---

## Quick start

```ts
import { Runtime } from '@jtrb/runtime';

// 1. Create the runtime (loads and compiles the WASM module)
const rt = await Runtime.create('c');

// 2. Set the virtual filesystem — the program sees these as real files
rt.fs = {
  'main.c': `
    #include <iostream>
    int main() {
      std::cout << "Hello, world!" << std::endl;
      return 0;
    }
  `
};

// 3. Subscribe to stdout / stderr (chunks are UTF-8 bytes as Uint8Array)
const decoder = new TextDecoder();
const printChunk = (chunk: Uint8Array) => {
  process.stdout.write(decoder.decode(chunk));
};
rt.stdout.on('data', printChunk);
rt.stderr.on('data', printChunk);

// 4. Perform the required DAP handshake (see below), then run
await rt.run();
```

---

## The DAP handshake

The runtime compiles programs in **debug mode**, which means a DAP (Debug Adapter Protocol) debugger is always active. The worker **blocks at startup** and will not execute your program until the handshake is complete.

This is a required step — skipping it will cause `rt.run()` to hang forever.

```ts
// Keep a monotonically increasing sequence number for DAP messages
let dapSeq = 1;

// Helper: send a DAP request and log the synchronous response
const dapSend = (command: string, args: Record<string, unknown>) => {
  return rt.debugger.send({
    type: 'request',
    seq: dapSeq++,
    command,
    arguments: args
  });
};

// Register the event listener BEFORE sending initialize.
// All async events from the runtime arrive here — initialized, stopped, etc.
rt.debugger.on('event', (msg) => {
  const m = msg as { type?: string; event?: string };

  if (m?.type === 'event' && m?.event === 'initialized') {
    // The runtime is ready to receive configuration.
    // Send an empty breakpoint list for now.
    dapSend('setBreakpoints', { source: { path: '/main.c' }, breakpoints: [] });
    dapSend('setExceptionBreakpoints', { filters: [] });
    // This unblocks the worker — the program starts executing after this
    dapSend('configurationDone', {});
  }
});

// Kick off the handshake
dapSend('initialize', {});

// Now run — resolves when the program exits
await rt.run();
```

### Why DAP?

The runtime exposes a full DAP interface so that IDEs can add debugging features (breakpoints, stepping, variable inspection) without any special runtime changes. Everything goes through standard DAP requests and events.

**What works today:**

- `initialize` / `initialized` / `configurationDone` — required startup handshake
- `setBreakpoints` — maps source lines to instrumented WASM locations (verified in the response)
- `setExceptionBreakpoints` — accepted (no filters implemented yet)
- Breakpoint hits and `stopped` events (`reason`: `breakpoint` or `step`; see [integration guide](./docs/integration.md#stepping))
- `threads`, `stackTrace`, `scopes`, `variables` — inspect the stack and locals
- `continue`, `next`, `stepIn`, `stepOut` — run, step over, step into, step out (all use the same instrumented breakpoint machinery)

For a precise description of how stepping is implemented (shared execution state between the main thread and the worker), read [**Stepping**](./docs/integration.md#stepping) in the integration guide.

---

## Wiring stdin

`rt.stdin.write()` accepts a UTF-8 string or a `Uint8Array`. The program reads via `cin`, `scanf`, `read()`, etc.

```ts
// Send a line of input (programs typically expect a trailing newline)
await rt.stdin.write('42\n');

// Or raw bytes:
await rt.stdin.write(new TextEncoder().encode('42\n'));
```

For interactive terminals (e.g. xterm.js), buffer keystrokes locally and flush on Enter:

```ts
let inputBuf = '';

terminal.onData((data) => {
  if (data === '\r') {
    terminal.write('\r\n');
    void rt.stdin.write(inputBuf + '\n');
    inputBuf = '';
  } else if (data === '\x7f') {
    if (inputBuf.length > 0) {
      inputBuf = inputBuf.slice(0, -1);
      terminal.write('\b \b');
    }
  } else {
    inputBuf += data;
    terminal.write(data);
  }
});
```

---

## Stopping a program

```ts
rt.stop(); // terminates the worker immediately; rt.run() resolves
```

---

## Full API

```ts
// Create a runtime for the given language ('c' is currently supported)
const rt = await Runtime.create('c');

rt.fs; // DirNode  — virtual filesystem, set before calling run()
rt.stdout; // RuntimeOutput — program stdout; use .on('data', fn) / .off('data', fn)
rt.stderr; // RuntimeOutput — program stderr
rt.stdin; // RuntimeStdin — program stdin; use .write(string | Uint8Array)
rt.debugger; // Debugger — DAP interface
rt.lang; // Lang     — language this runtime was created for

rt.run(); // Promise<void> — start execution, resolves on exit
rt.stop(); // void         — kill the worker

rt.debugger.send(message); // send a DAP request, returns response synchronously
rt.debugger.on('event', handler); // receive async DAP events
```

---

## Example project

See the [`demo/`](./demo) app for a Next.js + MUI example that wires up CodeMirror 6, xterm.js, and `@jtrb/runtime` into a working in-browser IDE. Start from [`demo/README.md`](./demo/README.md) and [`demo/components/CodeEditor.tsx`](./demo/components/CodeEditor.tsx).

---

## Contributing / building from source

> Requires [Cargo 1.91+](https://crates.io/), [wasm-pack](https://rustwasm.github.io/wasm-pack/), and Node v22+.

```bash
cargo install wasm-pack
npm install
npm run build   # wasm-pack build --target web && vite build
```

For local development, use:

```bash
npm run dev
```

To run the built-in demo:

```bash
npm link
cd demo
npm link @jtrb/runtime
npm run dev
```
