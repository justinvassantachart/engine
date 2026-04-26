# @jtrb/runtime

A browser-based C++ runtime powered by WebAssembly. Compile and run C++ programs entirely in the browser, with full stdin/stdout/stderr streaming and a built-in debugger.

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
  `,
};

// 3. Pipe stdout and stderr to wherever you want output
const decoder = new TextDecoder();
rt.stdout.pipeTo(
  new WritableStream({
    write: (chunk) => process.stdout.write(decoder.decode(chunk)),
  })
);

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
    arguments: args,
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
- `setBreakpoints` — accepted (does nothing, support coming soon)
- `setExceptionBreakpoints` — accepted (does nothing, might support later)

**Coming soon:**

- Breakpoint hits (`stopped` event)
- Variable inspection (`scopes`, `variables` requests)
- Step over / step in / step out

---

## Wiring stdin

`rt.stdin` is a `WritableStream`. Write UTF-8 encoded bytes to it and the program reads them via `cin`, `scanf`, `read()`, etc.

```ts
const encoder = new TextEncoder();
const writer = rt.stdin.getWriter();

// Send a line of input (programs typically expect a trailing newline)
await writer.write(encoder.encode('42\n'));
writer.releaseLock();
```

For interactive terminals (e.g. xterm.js), buffer keystrokes locally and flush on Enter:

```ts
let inputBuf = '';

terminal.onData((data) => {
  if (data === '\r') {
    terminal.write('\r\n');
    const w = rt.stdin.getWriter();
    w.write(encoder.encode(inputBuf + '\n'));
    w.releaseLock();
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
rt.stdout; // ReadableStream<Uint8Array> — program stdout
rt.stderr; // ReadableStream<Uint8Array> — program stderr
rt.stdin; // WritableStream<Uint8Array> — program stdin
rt.debugger; // Debugger — DAP interface
rt.lang; // Lang     — language this runtime was created for

rt.run(); // Promise<void> — start execution, resolves on exit
rt.stop(); // void         — kill the worker

rt.debugger.send(message); // send a DAP request, returns response synchronously
rt.debugger.on('event', handler); // receive async DAP events
```

---

## Example project

See the [`ide/`](./ide) folder for a complete Next.js example that wires up CodeMirror 6, xterm.js, and `@jtrb/runtime` into a working in-browser IDE. The [`CodeEditor.tsx`](./ide/app/components/CodeEditor.tsx) component is heavily commented and walks through every step of the integration.

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
