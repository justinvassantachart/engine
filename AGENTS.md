## Developing and Building

- `npm run dev` to run the local runtime dev loop. Changes to project files will trigger automatic rebuild
- `npm run build` to build the project.
  - This command builds the Rust components to WASM and then bundles everything into the npm library.
- `npm run tools:dap` to run a suite of integration tests against the Debugger Adapter Protocol (DAP).
- `npm run tools:dap -- {test}` to run a specific integration test.
- Don't use `web_sys::console`, instead use the provided `util::log!` and `util::warn!` macros which provide native Rust formatting.
- Use `util::weak_error!` to dilute a `Result` into an `Option` (logging the error) for exceptional behaviour that doesn't need to panic, and especially in core interfaces.

You can assume that the dev server is always running. You do not need to manually rebuild the project. Instead, just use `npm run tools:dap` to test the code which will wait for in-progress builds to complete.

## Contribution Standards

- Keep all contributions as simple and elegant as possible.
- Extra code is not acceptable; prefer the smallest clear solution that solves the problem.
