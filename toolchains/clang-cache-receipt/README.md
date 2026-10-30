# Clang object-cache toolchain

This directory starts with the **unmodified reproduction control** for the shipped
`@yowasp/clang@22.0.0-git20542-10`. It is not a receipt-capable release and does not
enable a runtime profile. The source-backed candidate pins SDK 29, LLVM and
wasi-libc commits, the upstream recipe, MinSizeRel with assertions and full LTO.
The upstream release does not provide a retrieved historical build attestation;
a successful build alone does not establish historical byte reproducibility.

`inputs.json` pins compiler inputs. `Dockerfile` pins the Linux base digest,
Ubuntu package snapshot and Node archive. `npm/package-lock.json` pins packaging
tools and transitives, including the formerly ranged esbuild input. A build
records installed package versions, repository-index hashes, native tablegen
hashes, CMake caches, generated toolchains and artifact hashes. It preserves the
upstream source date epoch. `gzip -n` is only a new deterministic wrapper; parity
is checked against the **uncompressed** shipped resource tar.

Build outside the repository on a Linux x86-64 machine with sufficient memory
for LLVM's full LTO link:

```sh
docker build -t webide-clang-control toolchains/clang-cache-receipt
docker run --rm -v "$PWD:/recipe:ro" -v /absolute/build/path:/work \
  webide-clang-control bash /recipe/toolchains/clang-cache-receipt/build-control.sh /work/control
```

The dedicated GitHub workflow does this without package publication or deployment.
It uploads failure logs as well as successful output. Do not rerun a failed build
without first recording and addressing its cause. The control must pass the
four-arm semantic/browser parity and overhead gates before a patched toolchain
can be admitted or released. No scanner fallback grants receipt capability.

The workflow defaults to the unmodified `control` arm. Ordinary toolchain
commits do not trigger a rebuild; only manual dispatch or an explicit
`build-request.json` change does. The optional `observer` arm refuses to start
without the actual accepted `control-validation.json`, exact `observer.patch`
and its digest-bound candidate manifest. Those files are intentionally absent
until the corresponding gates are complete.

Observer builds retain the isolated source/build tree, verified SDK archive and
bounded ccache under immutable per-run cache keys. `build-state.py` fingerprints
every retained file, mode and symlink. Reuse first verifies that full snapshot,
the frozen environment and exact previous source patch; it reverses only that
known patch before applying the next candidate. The SDK is re-extracted from
its verified archive, compiler checks use content, and the original upstream
build recipe is rerun. The first observer build cannot reuse an earlier control
whose running workflow did not save state. Cache-save failures must be reported;
artifact/evidence upload remains independent of cache success.

The candidate retains both raw linked core and JCO-extracted core and requires
exact byte equality. The memory-free generic component adapter declines the
receipt import; real delivery belongs to the engine's private compiler host.
Build receipts remain explicitly unvalidated until semantic, real-engine and
paired browser gates pass. Validated compiler/sysroot bytes and source notices
are intended for the immutable engine package's `dist/cpp-cache-receipt-v1/`
directory, allowing the course bundler to serve them on its own origin.
