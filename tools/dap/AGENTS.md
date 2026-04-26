# DAP Integration Harness (`tools/dap`)

Use this tool to run end-to-end Debugger Adapter Protocol (DAP) integration tests.

- Run all tests with `npm run tools:dap` (from repo root), or one test with `npm run tools:dap -- <test-name>`.
- Build the runtime before testing (`npm run build` or `npm run dev`); the harness does not auto-build.
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
- The harness already performs debugger session setup (`initialize` + `initialized`) before your test `steps`; your `dap.json` should only describe scenario-specific behavior.

### Placeholder Notation

`{{...}}` is treated as a JavaScript expression and evaluated against captured values.

- Use `${{var}}` in `response`/`event` expectations to capture values from actual messages.
- Use `{{var}}` for substitution/reference, and expressions like `{{var + 1}}` where needed.
- Expression variable names come from previously captured fields.
- If an expression references an unknown variable, evaluation fails.

## Adding New Tests

1. Create `tools/dap/tests/<new-test>/`.
2. Add scenario input files needed by runtime in that folder (these are mounted into `runtime.fs` for the test).
3. Add `tools/dap/tests/<new-test>/dap.json` with ordered `steps`.
4. Start from an existing test and keep expectations minimal-but-specific (assert only fields that should be stable).
5. Run `npm run tools:dap -- <new-test>`; inspect `tools/dap/output/<new-test>/` and console mismatch output when iterating.
