#!/usr/bin/env bash
set -euo pipefail

# This path deliberately refuses to run until actual control acceptance and an
# exact candidate patch have been committed. It never publishes or enables a
# runtime capability. Cached state is an optimization, never build authority.
recipe_dir=$(cd "$(dirname "$0")" && pwd)
build_root=${1:?Pass an isolated build directory outside the engine source tree}
mkdir -p "$build_root"
build_root=$(cd "$build_root" && pwd)
mkdir -p "$build_root/evidence" "$build_root/artifacts"
exec > >(tee "$build_root/evidence/build.log") 2>&1

read_json() {
  python3 - "$1" "$2" <<'PY'
import json, sys
value = json.load(open(sys.argv[1]))
for key in sys.argv[2].split('.'):
    value = value[key]
print(value)
PY
}
read_pin() { read_json "$recipe_dir/inputs.json" "$1"; }
clone_pin() {
  local repository=$1 commit=$2 destination=$3
  git init -q "$destination"
  git -C "$destination" remote add origin "$repository"
  git -C "$destination" fetch --depth=1 origin "$commit"
  git -C "$destination" checkout --detach FETCH_HEAD
}

python3 - "$recipe_dir" <<'PY'
import hashlib, json, re, sys
from pathlib import Path
r = Path(sys.argv[1])
digest = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
gate = json.loads((r / 'control-validation.json').read_text())
patch = json.loads((r / 'observer-manifest.json').read_text())
required = {'original-marbles-six-tu', 'vector-pch-on', 'synthetic-semantics',
            'pch-metadata-safety', 'receipt-overflow', 'representative-browser',
            'paired-browser-overhead', 'artifact-difference-attribution'}
if (gate.get('schema') != 1 or gate.get('status') != 'passed' or
    gate.get('inputsSha256') != digest(r / 'inputs.json') or
    not required.issubset({item['name'] for item in gate.get('checks', []) if item.get('status') == 'passed'})):
    raise SystemExit('Actual unmodified control acceptance is missing or incomplete')
if (patch.get('schema') != 1 or patch.get('status') != 'candidate' or
    patch.get('controlValidationSha256') != digest(r / 'control-validation.json') or
    patch.get('capability') != 'webide-object-cache-v1' or
    patch.get('abi') != 'webide_compiler.cache_receipt_v1(i32,i32)->i32' or
    patch.get('patch', {}).get('path') != 'observer.patch' or
    patch['patch'].get('sha256') != digest(r / 'observer.patch')):
    raise SystemExit('Exact observation-only patch manifest is missing or invalid')
for name in ('compilerSha256', 'resourceTarSha256'):
    if not re.fullmatch('[a-f0-9]{64}', gate.get('control', {}).get(name, '')):
        raise SystemExit('Actual control artifact identities missing')
PY

environment_sha=$(cat "$recipe_dir/Dockerfile" "$recipe_dir/inputs.json" "$recipe_dir/npm/package.json" "$recipe_dir/npm/package-lock.json" | sha256sum | cut -d' ' -f1)
source_dir="$build_root/yowasp"
state_dir="$build_root/state"
for repository in "$source_dir" "$source_dir/llvm-src" "$source_dir/wasi-libc-src"; do
  git config --global --add safe.directory "$repository"
done
if [[ -e "$source_dir" ]]; then
  python3 "$recipe_dir/build-state.py" verify "$build_root"
  test "$(read_json "$state_dir/provenance.json" environmentSha256)" = "$environment_sha"
  test "$(git -C "$source_dir/llvm-src" rev-parse HEAD)" = "$(read_pin llvm.commit)"
  git -C "$source_dir/llvm-src" diff HEAD --binary > "$build_root/evidence/restored-source.patch"
  echo "$(read_json "$state_dir/provenance.json" appliedPatchSha256)  $state_dir/applied.patch" | sha256sum -c -
  echo "$(read_json "$state_dir/provenance.json" sourceDiffSha256)  $build_root/evidence/restored-source.patch" | sha256sum -c -
  cp "$state_dir/provenance.json" "$build_root/evidence/restored-provenance.json"
  rm "$state_dir/provenance.json"
  git -C "$source_dir/llvm-src" apply --reverse --index "$state_dir/applied.patch"
  test -z "$(git -C "$source_dir/llvm-src" status --porcelain --untracked-files=all)"
else
  mkdir -p "$state_dir" "$build_root/ccache"
  clone_pin "$(read_pin packaging.repository)" "$(read_pin packaging.commit)" "$source_dir"
  clone_pin "$(read_pin llvm.repository)" "$(read_pin llvm.commit)" "$source_dir/llvm-src"
  clone_pin "$(read_pin wasiLibc.repository)" "$(read_pin wasiLibc.commit)" "$source_dir/wasi-libc-src"
fi
test "$(git -C "$source_dir" rev-parse HEAD)" = "$(read_pin packaging.commit)"
test "$(git -C "$source_dir/llvm-src" rev-parse HEAD)" = "$(read_pin llvm.commit)"
test "$(git -C "$source_dir/wasi-libc-src" rev-parse HEAD)" = "$(read_pin wasiLibc.commit)"
test "$(git -C "$source_dir" log -1 --format=%ct)" = "$(read_pin packaging.sourceDateEpoch)"
echo "$(read_pin packaging.recipeSha256)  $source_dir/build.sh" | sha256sum -c -
git -C "$source_dir" diff HEAD --exit-code --ignore-submodules=dirty
git -C "$source_dir/wasi-libc-src" diff HEAD --exit-code
test -z "$(git -C "$source_dir/llvm-src" status --porcelain --untracked-files=all)"
git -C "$source_dir/llvm-src" apply --check --index "$recipe_dir/observer.patch"
git -C "$source_dir/llvm-src" apply --index "$recipe_dir/observer.patch"
cp "$recipe_dir/observer.patch" "$state_dir/applied.patch"
git -C "$source_dir/llvm-src" diff HEAD --binary > "$build_root/evidence/source.patch"

if [[ ! -f "$build_root/wasi-sdk-29.tar.gz" ]]; then
  curl -fL --retry 2 "$(read_pin sdk.url)" -o "$build_root/wasi-sdk-29.tar.gz"
fi
echo "$(read_pin sdk.sha256)  $build_root/wasi-sdk-29.tar.gz" | sha256sum -c -
# Only the isolated, recipe-owned SDK directory is replaced; no shared service,
# checkout or host tool installation is changed.
rm -rf "$source_dir/wasi-sdk-29.0-x86_64-linux"
tar -xzf "$build_root/wasi-sdk-29.tar.gz" -C "$source_dir"
cp "$recipe_dir/inputs.json" "$recipe_dir/observer-manifest.json" "$recipe_dir/control-validation.json" "$recipe_dir/Dockerfile" "$build_root/evidence/"
cp "$source_dir/build.sh" "$build_root/evidence/upstream-build.sh"
cp -R /build-environment "$build_root/evidence/host-packages"
{ gcc --version; g++ --version; cmake --version; ccache --version; node --version; npm --version; } > "$build_root/evidence/host-tools.txt"
{ uname -a; cat /proc/meminfo; cat /proc/cpuinfo; df -h; } > "$build_root/evidence/resources.txt"
python3 - "$build_root" "$environment_sha" <<'PY'
import hashlib, json, sys
from pathlib import Path
r = Path(sys.argv[1]); digest = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
facts = {'schema': 1, 'purpose': 'isolated-observer-build-state',
         'environmentSha256': sys.argv[2],
         'appliedPatchSha256': digest(r / 'state/applied.patch'),
         'sourceDiffSha256': digest(r / 'evidence/source.patch'),
         'controlValidationSha256': digest(r / 'evidence/control-validation.json'),
         'observerManifestSha256': digest(r / 'evidence/observer-manifest.json'),
         'compilerValidated': False}
(r / 'state/provenance.json').write_text(json.dumps(facts, indent=2) + '\n')
PY

export CMAKE_GENERATOR=Ninja
export CMAKE_BUILD_PARALLEL_LEVEL=${TOOLCHAIN_JOBS:-3}
export MAKEFLAGS="-j${CMAKE_BUILD_PARALLEL_LEVEL}"
export CCACHE_DIR="$build_root/ccache"
export CCACHE_COMPILERCHECK=content
ccache --max-size=4G
cd "$source_dir"
/usr/bin/time -v ./build.sh
cp llvm-build/bin/llvm "$build_root/artifacts/raw-llvm.core.wasm"

cp "$recipe_dir/npm/package.json" "$recipe_dir/npm/package-lock.json" "$source_dir/npmjs/"
cd "$source_dir/npmjs"
npm ci --no-audit --no-fund
npx --no-install jco parse "$recipe_dir/receipt-noop-adapter.wat" -o "$state_dir/receipt-noop-adapter.wasm"
python3 - "$state_dir/receipt-noop-adapter.wasm" <<'PY'
import json, shlex, sys
from pathlib import Path
file = Path('package.json'); package = json.loads(file.read_text())
original = package['scripts']['transpile']
if original.count('--wasi-command') != 1:
    raise SystemExit('Pinned package transport command changed')
package['scripts']['transpile'] = original.replace('--wasi-command', '--wasi-command --adapt ' + shlex.quote('webide_compiler=' + sys.argv[1]))
file.write_text(json.dumps(package, indent=2) + '\n')
PY
npm run all
cmp "$build_root/artifacts/raw-llvm.core.wasm" gen/llvm.core.wasm
cp package.json package-lock.json "$build_root/evidence/"
cp gen/llvm.core.wasm gen/llvm-resources.tar llvm.wasm "$build_root/artifacts/"
gzip -n -c gen/llvm-resources.tar > "$build_root/artifacts/llvm-resources.tar.gz"
cp "$source_dir/llvm-build/CMakeCache.txt" "$build_root/evidence/llvm-CMakeCache.txt"
cp "$source_dir/llvm-tblgen-build/CMakeCache.txt" "$build_root/evidence/tblgen-CMakeCache.txt"
sha256sum "$source_dir/llvm-tblgen-build/bin/"{llvm-tblgen,clang-tblgen} > "$build_root/evidence/native-tool-sha256.txt"
cp "$source_dir/Toolchain-WASI.cmake" "$source_dir/Toolchain-WASI-LLVM.cmake" "$build_root/evidence/"
ccache --show-stats > "$build_root/evidence/ccache.txt"
node "$recipe_dir/verify-core.mjs" "$build_root/artifacts/raw-llvm.core.wasm" observer > "$build_root/evidence/core-imports.json"
python3 "$recipe_dir/record-notices.py" "$build_root"
sha256sum "$build_root/artifacts/"* > "$build_root/evidence/artifact-sha256.txt"
python3 "$recipe_dir/record-build.py" "$build_root" observer
