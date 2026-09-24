# Browser stdin regression

Build with the repository's existing toolchain and suppressed install scripts:

```sh
npm ci --ignore-scripts
npm run build
```

Point `PLAYWRIGHT_MODULE` at an installed `@playwright/test` ES module with its
Chromium browser installed, then run:

```sh
PLAYWRIGHT_MODULE=/absolute/path/node_modules/@playwright/test/index.mjs node tools/stdin/run.mjs
```

An optional positional argument selects another built or unpacked package's
`dist/debugger-sh.js`. The test uses only the public `Engine` API, starts an
isolated local HTTP server, asserts browser cross-origin isolation, and closes
the browser/server when finished. It does not install dependencies or change
the selected package.

Coverage includes an empty stdin read without input, two repeated Python runs
with sequential name/number input and computed stdout/stderr, stopping a
blocked input and rerunning, and C++ line/numeric input. Every completed run
must return exit code zero, and browser page errors fail the suite. Against
the original `0.3.15` package, the first empty-read case times out after printing
`BEFORE`; the corrected build must print `EMPTY b''` and complete without any
stdin write.
