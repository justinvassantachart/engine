# Scoped C++ compilation reuse

All optimizations are off by default. One Engine belongs to one project and account;
create a new Engine and await disposal of the old one when either changes.

```ts
const engine = await Engine.create('c', {
  cpp: {
    scope: { projectId, accountId },
    objectCache: true,
    retainWorker: true,
    cacheCompilerModule: true,
    compilerLifetime: 'project' // defaults to bounded idle retention
  }
});
engine.debugger.enabled = false;
engine.fs = frozenFiles;
engine.binaryFiles = frozenBinaries;
engine.cppArtifacts = frozenArtifacts;
await engine.prepareCompiler(); // No student input, Clang TU action, link, or devices
await engine.compile(); // Run-profile translation units only; no linking or program execution
const result = await engine.run(); // Always links and executes a new student runtime
console.log(engine.cacheStats);
await engine.dispose();
```

`compile()` never invokes a host-device opener. It returns the same discriminated
result type as `run()`; successful preparation has exitCode 0 and runMs 0. Only one
job may be active; a second submission returns an error result. Inputs, artifacts,
binary bytes and debug mode are owned snapshots captured before worker readiness.
The host must choose when to join, cancel or drain a background request. Compile
preparation does not store a linked executable. Debug bypasses the TU cache and
retires its worker. Tests with fresh nonce headers and diagnostic pragmas must miss
or bypass. No pragma, nonce, warning flag or error is removed to obtain a hit.

## Storage and identities

`objectCache` accepts `true` or partial `CppCacheLimits`. Defaults are 128 MiB of
object data, 256 entries, 8 MiB of serialized UTF-8 metadata overall, and 1 MiB
metadata per entry. These are conservative starting budgets for small multi-file
assignments, not browser heap limits. Zero limits disable admission. The private
LRU keeps the latest result per source; actual hits update recency. Replacement,
oversize rejection and eviction release their accounted object and metadata bytes.
A cache limit cannot reject compilation. Snapshots, message clones, compiler bytes,
sysroot extraction, WASI memory and browser compiled code consume additional memory.

The v1 length-prefixed SHA-256 key includes schema and release metadata, actual
fetched compiler/sysroot bytes, embedded exception support, ordered compilation
and link arguments, profile, source path/content, the full user path set, and all
other resource/header/binary bytes. Other selected TU contents are excluded only
because direct, indirect and dynamic source includes fail closed. Actual depfiles
validate transitive dependencies before admission. Headers and source use
conservative preprocessor guards; unexpected, volatile, diagnostic, system-header
and binary-text inputs bypass. Trusted toolchain headers are bound to the actual
sysroot bytes and checked for volatile builtins. Actual trusted-header definitions
that paste tokens or invoke pragmas taint all referring aliases and wrappers;
user/support references to these macros bypass. Analysis normalizes dependency
paths before classification, admits harmless comments, and fails closed on
unsupported lexical forms or analysis bounds. Opaque PCH inputs currently bypass
TU caching because their hidden macro and diagnostic state cannot be verified;
PCH compilation and archives retain their original behavior. PCH bytes still
participate in resource identity. Unused unsafe headers do not bypass unrelated TUs.

Only successful silent object compilations are candidates. The entire optional batch
commits after every TU succeeds, before linking or guest execution, after the parent
validates owned object hashes and acknowledges publication. Compile failure or
cancellation discards an uncommitted batch. Link failure and nonzero guest exit
preserve already committed objects. Staging and committed entries share one byte,
entry and metadata budget; a batch that cannot fit is discarded atomically. Every replay verifies the input key,
object SHA-256, Wasm validation and relocatable linking metadata, then copies bytes
into the fresh job filesystem. A hit counts only when clang object compilation is
omitted. Current warnings and errors are forwarded unchanged.

The compiler constructor is an explicit internal interface; it never intercepts
global WebAssembly prototypes. At most one immutable LLVM Module is retained, with
actual-byte SHA-256 checked once at ingestion against the build-fixed legacy asset
manifest. The embedded exception runtime is verified as part of the same identity.
A worker receives one private seed per toolchain; compiler steps never read their
authority from the mutable guest filesystem, and no TU rehashes compiler bytes. A fresh Store, WASI environment,
Instance, filesystem, globals, stdin and devices are still created for every job
and compiler invocation as applicable. Student Modules and Instances are not cached.
A parent identity copy and a worker identity copy can both remain live; their byte
counts are reported separately. These counts exclude compiled-code memory and
transient copies. They are not a claim of total browser memory.

## Lifecycle and measurements

`clearCaches()` advances the cache epoch synchronously. An active job can finish
and deliver output, but staged/late results cannot repopulate storage; its worker
retires afterward. `stop()` fences output and completion, terminates the worker,
clears both caches and returns a drain promise. `cancelCurrentOperation()` also
terminates a noncooperative guest but preserves committed objects and verified
compiler resources. Callers must explicitly choose this additive policy.

`releaseCompilerResources({ signal })` releases the idle worker, Module and restart
seed while keeping bounded objects. It defers behind an active operation and can
be cancelled with its signal; it never kills a guest for memory release. The next
preparation verifies the toolchain again before any object lookup. `dispose()`
synchronously closes the scope, drains pending work, clears callbacks/inputs and
permanently prevents reuse. Scope, operation and cache identities fence effects
independently, and retired worker slots cannot impersonate replacements.

Errors, nonzero outcomes and Debug retire the worker but preserve committed valid
objects and immutable code. Idle retention defaults to 30 seconds, configurable
from 1 ms to 5 minutes. Explicit `compilerLifetime: 'project'` omits that timer;
the host must implement visibility release and permanent disposal. No heartbeat,
global seed, export/import, persistence or cross-project reuse exists. The only
available toolchain profile is `legacy`; `cache-receipt-v1` selection fails explicitly
until an actual verified compiler/artifact profile is shipped. Legacy scanner/PCH
admission rules remain conservative.

`cacheStats` is an owned snapshot: bytes, entries, evictions, oversize admissions,
scope/operation/cache identities, worker retention/retirement and active or latest
job measurements. `stagingObjectBytes` and `stagingMetadataBytes` are included in
the total byte counters, not an additional allowance. Toolchain ingestion/hash
counts and Module clone support are reported separately. `latest.phase` and
`observeCompiler(listener)` report source-free compiler phases and source indices;
observers cannot authorize cache publication. Worker cache
statistics distinguish hashing, lookup/validation, hits/misses/bypass reasons,
object compiler-step time and linking. Compiler Module statistics distinguish
module compilation, byte hashing/comparison, hits and fresh compiler instantiation.
Compiler-step time includes module/instance setup; it is not an isolated native
clang execution clock. `timing.buildMs` includes toolchain fetching/extraction and
all pre-execution work; `totalMs` includes worker readiness and host bookkeeping.
Do not add overlapping clocks. Marker, graphics-byte and paint endpoints require
host instrumentation and are unavailable from Engine alone. Bounded diagnostics
contain paths/reasons, never source text.

## Initial assignment-open contract

WebIDE and the course host must schedule one idle/low-priority compile after a
complete assignment snapshot has been admitted. Build a pure owned plan preserving
transforms, aliases, generated graphics main, entrypoint, PCH, archives and resources.
Do not invoke foreground prepare/Run from a timer: those paths may clear terminal
state and emit foreground events. Warmup must not execute main, open devices,
write student files, update dirty/history state or emit foreground analytics.

Foreground Run can flush a pending warmup or join an in-flight warmup only for
complete frozen snapshot and scope equality. Otherwise cancel and await draining
before mutating the engine. Edits, navigation, account replacement and teardown
cancel stale work. A failure is visible and nonblocking with no retry loop. Run
always performs a fresh link and execution. The Engine does not infer assignment
lifecycle. Browser-specific enablement requires real browser validation; a WebKit
run does not establish Safari compatibility.

## Validation

Use the documented build, lint and DAP commands in AGENTS.md. Additional focused checks:

```sh
node --experimental-strip-types --test tools/runtime-cache/*.test.*
cargo test -p engine-cpp-cache-tests
npx tsc --noEmit
PLAYWRIGHT_MODULE=/absolute/path/to/playwright/index.mjs node tools/runtime-cache/browser.mjs --measure-memory
```

The Chrome harness preserves raw receipts and screenshots outside production
behavior, counterbalances three arms and separates cold, unchanged, equal-length
semantic edits and header edits. Its synthetic six-TU matrix is distinct from
real Marble/Vector staging validation. JS heap and available browser-attributed
memory measurements have explicit limits; unavailable endpoints remain unavailable.
In the exact Marble snapshot, the generated driver is eligible but its other
translation units currently bypass due to volatile or unsupported system macros.
Compiler worker/Module reuse still applies to eligible ordinary Run-profile jobs.
Actual Safari automation was unavailable because remote automation is disabled;
new course optimizations remain off there and the prior compiler trap is unresolved.
The historical exploratory JSON/DOCX files named in the implementation request
were not present locally, so those figures are reference claims, not reproduced
benchmarks or acceptance guarantees. No one-to-three-second edited Run is promised.
