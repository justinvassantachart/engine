#!/usr/bin/env python3
"""Retain upstream license texts with the distributed compiler/sysroot bytes."""
import hashlib
import json
from pathlib import Path
import sys

root = Path(sys.argv[1])
source = root / 'yowasp'
paths = [
    'LICENSE.txt',
    'llvm-src/llvm/LICENSE.TXT',
    'llvm-src/llvm/include/llvm/Support/LICENSE.TXT',
    'llvm-src/llvm/lib/Support/BLAKE3/LICENSE',
    'llvm-src/clang/LICENSE.TXT',
    'llvm-src/lld/LICENSE.TXT',
    'llvm-src/compiler-rt/LICENSE.TXT',
    'llvm-src/libcxx/LICENSE.TXT',
    'llvm-src/libcxxabi/LICENSE.TXT',
    'wasi-libc-src/LICENSE',
    'wasi-libc-src/LICENSE-APACHE',
    'wasi-libc-src/LICENSE-APACHE-LLVM',
    'wasi-libc-src/LICENSE-MIT',
    'wasi-libc-src/libc-top-half/musl/COPYRIGHT',
    'wasi-libc-src/libc-bottom-half/cloudlibc/LICENSE',
    'wasi-libc-src/fts/musl-fts/COPYING',
    'wasi-libc-src/tools/wasi-headers/LICENSE',
]
notices = ['Compiler and sysroot third-party notices\nSource identities are recorded in build-receipt.json.\n']
entries = []
for relative in paths:
    data = (source / relative).read_bytes()
    notices.append(f'\n=== {relative} ===\n\n' + data.decode('utf-8'))
    entries.append({'path': relative, 'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest()})
for relative in ['llvm-src/llvm/lib/Support/xxhash.cpp', 'llvm-src/llvm/lib/Support/MD5.cpp',
                 'wasi-libc-src/dlmalloc/src/malloc.c', 'wasi-libc-src/emmalloc/emmalloc.c']:
    data = (source / relative).read_bytes()
    text = data.decode('utf-8')
    end = text.find('* Quickstart') if relative.endswith('dlmalloc/src/malloc.c') else text.find('*/')
    if end < 0 or end > 16384:
        raise RuntimeError(f'Expected bounded leading source notice: {relative}')
    if not relative.endswith('dlmalloc/src/malloc.c'):
        end += 2
    notices.append(f'\n=== Leading notice: {relative} ===\n\n' + text[:end])
    entries.append({'path': relative, 'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest(),
                    'notice': 'leading source comment'})
(root / 'artifacts/THIRD_PARTY_NOTICES.txt').write_text('\n'.join(notices))
(root / 'evidence/license-identities.json').write_text(json.dumps(entries, indent=2) + '\n')
