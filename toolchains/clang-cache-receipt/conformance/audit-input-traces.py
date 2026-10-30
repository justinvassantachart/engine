#!/usr/bin/env python3
"""Audit real compiler traces against receipt closure and explicit input domains.

This test-only audit cannot grant production cache capability. Its root is the
isolated conformance filesystem, not an arbitrary host directory. Every trace
and failed comparison is retained by the invoking conformance runner.
"""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import stat

parser = argparse.ArgumentParser()
parser.add_argument("--evidence", required=True, type=Path)
parser.add_argument("--reference", type=Path)
parser.add_argument("--out", required=True, type=Path)
options = parser.parse_args()
root = (options.evidence / "root").resolve(strict=True)
if options.out.exists():
    raise SystemExit("Refusing to replace previous trace-audit evidence")

sha = lambda data: hashlib.sha256(data).hexdigest()
known = {
    "random_get", "clock_time_get", "clock_res_get", "fd_seek",
    "fd_filestat_get", "fd_prestat_get", "fd_prestat_dir_name",
    "path_filestat_get", "fd_fdstat_get", "path_open", "path_readlink",
    "fd_read", "fd_pread", "fd_close", "fd_write", "path_rename",
    "path_unlink_file",
}
fixed_absent = {"/usr/include", "/proc/self/fd", "/modules.idx"}
records, failures, counts = [], [], Counter()


def load_invocations(directory):
    values = {}
    for file in sorted(directory.glob("*.json")):
        value = json.loads(file.read_text())
        if isinstance(value, dict) and "events" in value and "args" in value:
            values[file.name] = (value, sha(file.read_bytes()))
    return values


def projected(events):
    # Memory addresses and host fd numbers naturally change when the observer
    # allocates. Keep exact ordered observable paths/results/read sizes, plus
    # clock/entropy request domains. Ordinary output bytes are compared by the
    # conformance runner itself, not redacted or normalized here.
    result = []
    for event in events:
        item = {key: event[key] for key in (
            "name", "path", "secondPath", "spelling", "secondSpelling",
            "result", "bytesRead", "target", "directoryBytesSha256"
        ) if key in event}
        if event["name"] == "path_open":
            item["openFlagsAndRights"] = event["args"][4:8]
        elif event["name"] in ("clock_time_get", "clock_res_get"):
            item["clockId"] = event["args"][0]
        elif event["name"] == "random_get":
            item["requestedBytes"] = event["args"][1]
        result.append(item)
    return result


invocations = load_invocations(options.evidence)
references = load_invocations(options.reference) if options.reference else {}
if not invocations:
    raise SystemExit("No raw compiler invocation traces found")
all_outputs = {v["args"][v["args"].index("-o") + 1] for v, _ in invocations.values() if "-o" in v["args"]}
all_temporaries = {e["path"] for v, _ in invocations.values() for e in v["events"]
                   if e["name"] == "path_open" and e["result"] == 0 and int(e["args"][4]) & 1}

for filename, (value, digest) in invocations.items():
    problems, domains, reads, namespace = [], Counter(), set(), {}
    receipt = value.get("receipt")
    eligible = bool(value.get("exit") == 0 and not value.get("thrown") and
                    not value.get("stdout") and not value.get("stderr") and
                    receipt and receipt.get("completed") is True and
                    not receipt.get("volatileBuiltins") and not receipt.get("unsupported") and
                    receipt.get("pchMetadataIndependent") is True)
    deps = set(receipt.get("dependencies", [])) if receipt else set()
    action = "pch" if "-emit-pch" in value["args"] else "object"
    direct_pch = value["args"][value["args"].index("-include-pch") + 1] if "-include-pch" in value["args"] else None
    if not value.get("traceComplete"):
        problems.append("incomplete raw WASI trace")
    for event in value["events"]:
        name, result = event["name"], event["result"]
        counts[name] += 1
        if name not in known:
            problems.append("unclassified syscall " + name)
        path = event.get("path")
        if path is None:
            # These query the fixed preopen descriptor table before its root
            # name is returned; they do not select a pathname input.
            if name not in {"fd_prestat_get", "fd_prestat_dir_name", "random_get", "clock_time_get", "clock_res_get"}:
                problems.append("unresolved descriptor path for " + name)
            domains["fixed-runtime-descriptor-or-clock-entropy"] += 1
            continue
        if path in {"<stdin>", "<stdout>", "<stderr>"}:
            if name in {"fd_read", "fd_pread"} and event.get("bytesRead", 0):
                problems.append("compiler content read from stdio")
            domains["fixed-stdio"] += 1
            continue
        if not path.startswith("/") or ".." in path.split("/"):
            problems.append("noncanonical trace path " + str(path))
            continue
        output = path in all_outputs or path in all_temporaries
        # A PCH produced by a previous separate action becomes an explicit
        # imported compiler input and is required in the receipt's closure.
        if path == direct_pch or (receipt and path in receipt.get("importedPchPaths", [])):
            output = False
        if output:
            domains["generated-output-forbidden-as-input"] += 1
            if name in {"fd_read", "fd_pread"} and event.get("bytesRead", 0) and eligible:
                problems.append("eligible receipt read generated output " + path)
        elif path in fixed_absent:
            domains["fixed-absent-environment"] += 1
            if result == 0:
                problems.append("fixed-absent input was present " + path)
        else:
            # This finite check explains the observed fixture namespace. The
            # Engine independently binds its complete owned namespace before
            # lookup, including positive and negative search candidates.
            candidate = root / path.lstrip("/")
            try:
                info = candidate.lstat()
                if stat.S_ISLNK(info.st_mode):
                    kind = "symlink"
                elif stat.S_ISDIR(info.st_mode):
                    kind = "directory"
                elif stat.S_ISREG(info.st_mode):
                    kind = "regular"
                else:
                    kind = "unsupported"
            except FileNotFoundError:
                kind = "absent"
            namespace[path] = kind
            domains["owned-namespace-" + kind] += 1
            if kind in {"symlink", "unsupported"} and eligible:
                problems.append("eligible receipt encountered unsupported namespace " + path)
            if name in {"path_rename", "path_unlink_file"} and result == 0:
                problems.append("mutation outside generated output domain " + path)
        if name in {"fd_read", "fd_pread"} and event.get("bytesRead", 0):
            reads.add(path)
            if eligible and path not in deps:
                problems.append("content read absent from eligible dependency closure " + path)
        if name == "path_open" and result == 0 and not output and not (int(event["args"][4]) & 1):
            if eligible and path not in deps:
                problems.append("opened input absent from eligible dependency closure " + path)
        if name == "path_readlink" and result == 0 and eligible:
            problems.append("eligible receipt encountered path alias " + path)

    reference_name = filename
    if filename.endswith("-declined.json"):
        reference_name = filename.removesuffix("-declined.json") + "-0.json"
    paired = references.get(reference_name)
    projection = projected(value["events"])
    exact_trace = None
    if options.reference:
        if paired is None:
            problems.append("missing paired invocation " + reference_name)
        else:
            exact_trace = projection == projected(paired[0]["events"])
            if not exact_trace:
                problems.append("observer changed projected syscall sequence")
    record = {"invocation": filename, "traceSha256": digest, "action": action,
              "eligible": eligible, "dependencies": len(deps), "contentReadPaths": sorted(reads),
              "domains": dict(domains), "observedNamespace": namespace,
              "projectedTraceSha256": sha(json.dumps(projection, separators=(",", ":")).encode()),
              "reference": reference_name if paired else None, "exactProjectedTraceMatch": exact_trace,
              "failures": sorted(set(problems))}
    records.append(record)
    failures.extend(filename + ": " + message for message in record["failures"])

options.out.write_text(json.dumps({
    "schema": 1, "purpose": "Test-only real WASI trace closure and on/off observation audit",
    "productionAuthority": False, "status": "passed" if not failures else "failed",
    "evidence": str(options.evidence.resolve()), "reference": str(options.reference.resolve()) if options.reference else None,
    "policies": {"content": "Every eligible content read and input open belongs to receipt dependencies",
                 "namespace": "Engine binds complete immutable toolchain/user namespace; fixed-absent domains separately enforced",
                 "output": "Generated object/temporary domain is never an eligible input",
                 "metadata": "Controlled clock/entropy/mtime variants compared by semantic conformance; PCH metadata proof required",
                 "stdio": "No content input; ordinary diagnostic/output bytes compared exactly"},
    "syscalls": dict(counts), "failures": failures, "records": records
}, indent=2) + "\n")
print(json.dumps({"status": "passed" if not failures else "failed", "invocations": len(records), "failures": len(failures)}))
raise SystemExit(1 if failures else 0)
