# Compiler observation conformance

These Node 24 WASI controls run a fresh mutable compiler instance for every
invocation. They preserve stdout, stderr, exit, complete output bytes and hashes,
and bounded filesystem, clock and entropy traces. Deliberate test-only clock,
mtime and entropy perturbations check exact semantics. They are not browser
performance measurements, engine cache-hit evidence or permission to enable a
profile. Output directories must be new; failed evidence is never overwritten.

Run `run-receipt-conformance.mjs` with `--compiler`, `--compiler-sha256`,
`--sysroot`, `--sysroot-sha256`, `--arm` and `--out`. The sysroot is a gzip tar.
Arms are `shipped`, `control`, `observer-off` and `observer-on`; add `--reference`
pointing at the preceding arm's `summary.json` for exact paired comparison.
The embedded synthetic fixtures exercise actual builtin expansion (including
aliases, token pasting, inactive/suppressed/redefined occurrences and PCH
wrappers), counter behavior, include probes, embedding and emitted assembly.
Observer-on also declines capture deliberately to check compiler independence.

`run-course-conformance.mjs` accepts the same arguments plus `--fixture`: an
archived unchanged course plan envelope or original Marbles compile fixture.
The harness only strips the host's `/workspace` and `/sysroot` namespaces,
matching the runtime adapter; it preserves source, PCH and binary bytes. It
requires all course object invocations to succeed silently in both perturbation
variants. Original assignment fixtures remain owned by the course repository.

For a successful single-TU PCH course run, invoke `run-pch-matrix.mjs` with
`--compiler`, `--compiler-sha256`, `--course` (its output directory) and `--out`.
It checks header content/removal, language/PCH compatibility, timestamp and
content-hash creation/validation combinations, and observer-on/off PCH creation
byte equality. `run-pch-receipt-safety.mjs` takes the same arguments and mutates
only an isolated copy of the PCH: metadata timestamps, a stored system-input
timestamp, override/transient/module-map flags and an out-of-range name length.
Each can preserve ordinary successful compilation; the observer must still
decline unsafe metadata without changing the ordinary result. Both support
`--reference` for paired controls.

`inspect-pch-inputs.py` is a bounded, independent test-only reader of the pinned
LLVM 22 PCH control/input blocks. It reports every input record (including
system records), metadata timestamp flags and mutation bit offsets. It is never
runtime cache authority and does not replace the compiler's ordinary PCH
validation policy. Production metadata observation must perform bounded checked
reads of the actual loaded PCH bytes and fail closed on unsupported formats.

The control must first pass exact original Marbles and PCH-on Vector semantics
and the full synthetic/PCH matrix against the shipped compiler. Observer-off/on
then repeat the controls. Real browser four-arm semantic and overhead gates,
strict engine input closure, dependency-edit tests, paired cache performance
and deployed smoke tests remain independent required acceptance gates.
