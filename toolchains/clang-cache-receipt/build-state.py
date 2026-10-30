#!/usr/bin/env python3
"""Record/verify isolated build-state bytes; never grant compiler capability."""
import hashlib
import json
import os
from pathlib import Path
import stat
import sys

ROOTS = ("yowasp", "ccache", "wasi-sdk-29.tar.gz")
MAX_ENTRIES = 500_000
MAX_BYTES = 512 * 1024**3


def digest(path):
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()


def inventory(root):
    result, total = {}, 0

    def visit(path):
        nonlocal total
        if len(result) >= MAX_ENTRIES:
            raise RuntimeError("Build state exceeds entry bound")
        relative = path.relative_to(root).as_posix()
        info = path.lstat()
        entry = {"mode": stat.S_IMODE(info.st_mode)}
        if stat.S_ISLNK(info.st_mode):
            entry.update(type="symlink", target=os.readlink(path))
        elif stat.S_ISREG(info.st_mode):
            total += info.st_size
            if total > MAX_BYTES:
                raise RuntimeError("Build state exceeds byte bound")
            entry.update(type="file", bytes=info.st_size, sha256=digest(path))
        elif stat.S_ISDIR(info.st_mode):
            entry.update(type="directory")
        else:
            raise RuntimeError(f"Unsupported build-state file type: {relative}")
        result[relative] = entry
        if entry["type"] == "directory":
            for child in sorted(path.iterdir()):
                visit(child)

    for name in ROOTS:
        path = root / name
        if not path.exists():
            raise RuntimeError(f"Missing required build state: {name}")
        visit(path)
    return result, total


def run(mode, root):
    root = root.resolve()
    state = root / "state"
    provenance = state / "provenance.json"
    manifest = state / "tree-manifest.json"
    if not provenance.is_file():
        raise RuntimeError("No verified preparation provenance; state is not reusable")
    facts = json.loads(provenance.read_text())
    if facts.get("schema") != 1 or facts.get("purpose") != "isolated-observer-build-state":
        raise RuntimeError("Unknown state provenance schema")
    if mode == "verify":
        expected = json.loads(manifest.read_text())
        if expected.get("schema") != 1 or expected.get("provenanceSha256") != digest(provenance):
            raise RuntimeError("Build-state provenance changed")
    entries, total = inventory(root)
    actual = {"schema": 1, "provenanceSha256": digest(provenance),
              "entries": entries, "bytes": total}
    if mode == "record":
        temporary = state / "tree-manifest.json.tmp"
        temporary.write_text(json.dumps(actual, sort_keys=True, separators=(",", ":")) + "\n")
        temporary.replace(manifest)
    elif mode == "verify":
        if actual != expected:
            changed = [name for name in sorted(set(entries) | set(expected["entries"]))
                       if entries.get(name) != expected["entries"].get(name)]
            raise RuntimeError(f"Refusing changed build state: {changed[:12]} ({len(changed)} paths)")
    else:
        raise RuntimeError("Expected record or verify")
    print(json.dumps({"mode": mode, "files": len(entries), "bytes": total,
                      "manifestSha256": digest(manifest), "compilerValidated": False}))


if __name__ == "__main__":
    if len(sys.argv) != 3:
        raise SystemExit("Usage: build-state.py record|verify <isolated-build-root>")
    run(sys.argv[1], Path(sys.argv[2]))
