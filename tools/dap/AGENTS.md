# DAP Integration Harness (`tools/dap`)

Use this tool to run end-to-end Debugger Adapter Protocol (DAP) integration tests.

- Run all tests with `npm run tools:dap` (from repo root), or one test with `npm run tools:dap <test-name>`.
- It is safe (and encouraged) to run this alongside `npm run dev`. This tool will wait for any in-progress builds initiated by `npm run dev` to complete before starting the tests.
- The harness links the local package into `tools/dap` before running tests.
- It executes scripted DAP request/response/event flows from `tools/dap/tests/*/dap.json` against a real runtime session.
- For triage, per-test artifacts/log outputs are written to `tools/dap/output/<test-name>/`, including emitted WASM files (`pre.wasm`, `post.wasm`) and derived dumps like `pre.wat`/`post.wat` (and `pre.dwarf` when `llvm-dwarfdump` is available).
- The command prints step-by-step progress and mismatch details to stdout, and exits non-zero on failures.

## Test Cases

Each test is a directory under `tools/dap/tests/<test-name>/` with a required `dap.json` file:

```json
{
  "steps": [
    { "type": "request", "command": "initialize", "arguments": {} },
    { "type": "response", "success": true, "command": "initialize", "body": {} },
    { "type": "event", "event": "initialized", "$timeout": 10000 }
  ]
}
```

- `request`: sends a DAP request (`command` required, optional `arguments`).
- `response`: matches the previous request response (partial structural match is allowed; include only fields you care about).
- `event`: waits for a DAP event name (`event` required, optional `body`, optional `$timeout` in ms; default is 1000ms).
- `expect`: evaluates JavaScript (`run`, required) with prior captures in scope and built-in functions (see below). If `expect` is present, the return value is matched like a `response` body (partial structure, `${{…}}` captures). If `expect` is omitted, only failure is a thrown error or an undefined result.
- The harness already performs debugger session setup (`initialize` + `initialized`) before your test `steps`; your `dap.json` should only describe scenario-specific behavior.

### Placeholder Notation

`{{...}}` is treated as a JavaScript expression and evaluated against captured values (same evaluation environment as `expect` steps).

- Use `${{var}}` in `response`/`event` expectations to capture values from actual messages.
- Use `{{var}}` for substitution/reference, and expressions like `{{var + 1}}` where needed.
- Expression variable names come from previously captured fields.
- If an expression references an unknown variable, evaluation fails.

#### Helper Functions

- `hex` is a small helper for tests: numbers become strings like `0xff`; strings that look like hex (`0x…`) become integers; strings of decimal digits become a hex string via that integer. Errors from `hex` surface like other script errors.

## Adding New Tests

1. Create `tools/dap/tests/<new-test>/`.
2. Add scenario input files needed by runtime in that folder (these are mounted into `runtime.fs` for the test).
3. Add `tools/dap/tests/<new-test>/dap.json` with ordered `steps`.
4. Start from an existing test and keep expectations minimal-but-specific (assert only fields that should be stable).
5. Run `npm run tools:dap -- <new-test>`; inspect `tools/dap/output/<new-test>/` and console mismatch output when iterating.

## Running Against `lldb-dap` (Golden Reference)

For any test, you can run the same `dap.json` scenario against a real `lldb-dap` subprocess instead of the runtime:

```
npm run tools:dap -- --lldb
npm run tools:dap -- --lldb <test-name>
npm run tools:dap -- --lldb-path=/abs/path/to/lldb-dap
```

This lets you observe how a reference DAP implementation (the one shipped with Xcode / Homebrew LLVM) responds to the same scenario, so you can use it as a "golden standard" while iterating on the runtime's DAP behavior.

How it works:

- The harness compiles each test's `main.{cpp,c,cc}` to a native binary at `tools/dap/output/<test>/lldb/prog` using `xcrun clang++ -g -O0 -fno-inline -fstandalone-debug` (or `clang` for `.c`).
- It spawns `lldb-dap` over stdio with standard `Content-Length`-framed DAP, auto-injects a `launch` request after `initialize`, then runs your `dap.json` steps unchanged.
- Outgoing `source.path: "/main.cpp"` (the runtime's virtualized path) is rewritten to the absolute path of the on-disk source so `setBreakpoints` resolves correctly.
- `--lldb` mode is **exploratory**: mismatches are printed in the same diff format as runtime mode, but the process always exits 0. The full request/response/event stream lands in `tools/dap/output/<test>/log.json` regardless of pass/fail — that's the artifact you read to see what lldb-dap actually returned.

Adapter discovery order:

1. `--lldb-path=<abs-path>` flag
2. `LLDB_DAP_PATH` env var
3. `xcrun -f lldb-dap` (Xcode toolchain)
4. `command -v lldb-dap` on `PATH`

Expected divergences (treat them as data, not bugs):

- `threadId`: lldb-dap reports a real TID (e.g. `11053006`); runtime uses `1`. Capture it with `${{tid}}` if you want a portable test.
- `value` / `type` formatting: lldb-dap and the runtime stringify variables differently, especially for compound types.
- `frameId`, `variablesReference`: opaque integers — already captured via `${{...}}` in the existing tests, so they don't need to match literally.
