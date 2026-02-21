# util — Diagnostic Tools

This folder contails command-line utilities for debugging the runtime's WASM instrumentation pipeline. These run on your host machine (not in the browser), making it easy to inspect what the instrumentation is doing without a full browser workflow.

## bkpt-map

Visualizes where breakpoints get injected into a WASM binary. Given a `.wasm` file with DWARF debug info, it:

1. Parses DWARF debug info to extract source location mappings
2. Runs the same instrumentation used in the browser (via `wasm-instrument`)
3. Compares the original and instrumented binaries
4. Shows which source lines have breakpoints

### Usage

```bash
cargo run --manifest-path util/Cargo.toml -- <path-to-wasm>
```

### Getting a WASM file

When `is_debug` is enabled, the runtime automatically downloads a `pre-instrumentation.wasm` file before instrumenting it. Use this file as input.

### Example output

```
=== DWARF locations (22) ===
  [1] /main.c:3:0 @ 0x1d6
  [2] /main.c:5:0 @ 0x226
  ...

Binary: 1288632 -> 1229553 bytes (-59079)

16 breakpoints injected across 4 functions

/main.c:
  * line 3
    ...
  * line 5
  * line 6
  * line 7

/sys/include/c++/v1/ostream:
  * line 196 (x2)
    ...
  * line 1001
  * line 1002
  * line 1003
  * line 1004
```

- `*` marks lines where a breakpoint fires at runtime
- `(x2)` means multiple breakpoints on the same line (different WASM addresses)
- `...` collapses gaps between breakpointed lines
- "Missed DWARF locations" (if any) lists debug entries that weren't injected — typically duplicates where the same source line has multiple WASM addresses

## Project structure

This project uses a [Cargo workspace](https://doc.rust-lang.org/book/ch14-03-cargo-workspaces.html) to share code between the browser runtime and native tools.

```
runtime/
├── Cargo.toml              # workspace root — lists all members
├── src/                    # main runtime crate (cdylib, runs in browser via wasm-bindgen)
│   ├── worker.rs
│   ├── dwarf.rs            # thin wrapper around wasm-instrument with browser logging
│   └── ...
├── crates/
│   └── wasm-instrument/    # shared library crate (portable, no web dependencies)
│       ├── Cargo.toml
│       └── src/lib.rs      # DWARF parsing + WASM instrumentation logic
└── util/                   # native CLI tools (this directory)
    ├── Cargo.toml
    ├── bkpt-map.rs
    └── README.md
```

The key idea: `wasm-instrument` contains the core DWARF parsing and instrumentation logic with no `web_sys` or `wasm-bindgen` dependencies. This makes it usable from both:

- **The browser runtime** (`src/dwarf.rs` wraps it and adds `console.log` for browser-side diagnostics)
- **Native CLI tools** (`util/bkpt-map.rs` imports it directly for local analysis)

Adding a new tool is straightforward — create a new `.rs` file in `util/`, add a `[[bin]]` entry in `util/Cargo.toml`, and import from `wasm-instrument`.
