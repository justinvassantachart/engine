#!/usr/bin/env python3
"""Record build identities without asserting compatibility or historical reproducibility."""
import hashlib
import json
from pathlib import Path
import sys

root = Path(sys.argv[1])
kind = sys.argv[2] if len(sys.argv) > 2 else 'control'
if kind not in ('control', 'observer'):
    raise RuntimeError('Unknown build kind')
inputs = json.loads((root / "evidence/inputs.json").read_text())
cache = (root / "evidence/llvm-CMakeCache.txt").read_text()
for expected in ("CMAKE_BUILD_TYPE:STRING=MinSizeRel", "LLVM_ENABLE_ASSERTIONS:BOOL=ON"):
    if expected not in cache.splitlines():
        raise RuntimeError(f"Control configuration mismatch: missing {expected}")
toolchain = (root / "evidence/Toolchain-WASI-LLVM.cmake").read_text()
if "wasi-sdk-29.0-x86_64-linux" not in toolchain or "-flto" not in toolchain:
    raise RuntimeError("Control must retain SDK29 and upstream full LTO")

def identity(path):
    return {"bytes": path.stat().st_size,
            "sha256": hashlib.file_digest(path.open("rb"), "sha256").hexdigest()}

artifacts = {path.name: identity(path) for path in sorted((root / "artifacts").iterdir())}
result = {
    "schema": 1,
    "kind": "unmodified-control" if kind == 'control' else 'observer-candidate',
    "compatibilityValidated": False,
    "receiptCapability": None,
    "inputs": inputs,
    "artifacts": artifacts,
    "matchesShippedCompiler": artifacts["llvm.core.wasm"]["sha256"] == inputs["shippedControl"]["compilerSha256"],
    "matchesShippedResourceTar": artifacts["llvm-resources.tar"]["sha256"] == inputs["shippedControl"]["resourceTarSha256"],
}
if kind == 'observer':
    result['controlValidationSha256'] = identity(root / 'evidence/control-validation.json')['sha256']
    result['observerManifestSha256'] = identity(root / 'evidence/observer-manifest.json')['sha256']
    result['sourcePatchSha256'] = identity(root / 'evidence/source.patch')['sha256']
    result['packagedCoreMatchesRaw'] = artifacts['raw-llvm.core.wasm'] == artifacts['llvm.core.wasm']
    if not result['packagedCoreMatchesRaw']:
        raise RuntimeError('Packaging changed the raw primary compiler core')
(root / "evidence/build-receipt.json").write_text(json.dumps(result, indent=2) + "\n")
print(json.dumps(result, indent=2))
