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
