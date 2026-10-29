#!/usr/bin/env python3
"""Record build identities without asserting compatibility or historical reproducibility."""
import hashlib
import json
from pathlib import Path
import sys

root = Path(sys.argv[1])
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
    "kind": "unmodified-control",
    "compatibilityValidated": False,
    "receiptCapability": None,
    "inputs": inputs,
    "artifacts": artifacts,
    "matchesShippedCompiler": artifacts["llvm.core.wasm"]["sha256"] == inputs["shippedControl"]["compilerSha256"],
    "matchesShippedResourceTar": artifacts["llvm-resources.tar"]["sha256"] == inputs["shippedControl"]["resourceTarSha256"],
}
(root / "evidence/build-receipt.json").write_text(json.dumps(result, indent=2) + "\n")
print(json.dumps(result, indent=2))
