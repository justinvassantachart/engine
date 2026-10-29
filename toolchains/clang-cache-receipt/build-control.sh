#!/usr/bin/env bash
set -euo pipefail

# This script builds only the unmodified source-backed control. It cannot publish
# a release or grant the receipt capability to any compiler.
recipe_dir=$(cd "$(dirname "$0")" && pwd)
build_root=${1:?Pass a new build directory outside the engine source tree}
mkdir -p "$build_root"
build_root=$(cd "$build_root" && pwd)
if [[ -e "$build_root/yowasp" ]]; then
  echo 'Control build directory already contains source; choose a fresh directory.' >&2
  exit 1
fi
mkdir -p "$build_root/evidence" "$build_root/artifacts"
exec > >(tee "$build_root/evidence/build.log") 2>&1

read_pin() {
  python3 - "$recipe_dir/inputs.json" "$1" <<'PY'
import json, sys
value = json.load(open(sys.argv[1]))
for key in sys.argv[2].split('.'):
    value = value[key]
print(value)
PY
}
clone_pin() {
  local repository=$1 commit=$2 destination=$3
  git init -q "$destination"
  git -C "$destination" remote add origin "$repository"
  git -C "$destination" fetch --depth=1 origin "$commit"
  git -C "$destination" checkout --detach FETCH_HEAD
  test "$(git -C "$destination" rev-parse HEAD)" = "$commit"
}

cp "$recipe_dir/inputs.json" "$build_root/evidence/inputs.json"
cp "$recipe_dir/Dockerfile" "$build_root/evidence/Dockerfile"
cp -R /build-environment "$build_root/evidence/host-packages"
uname -a > "$build_root/evidence/uname.txt"
{ gcc --version; g++ --version; cmake --version; ccache --version; node --version; npm --version; } > "$build_root/evidence/host-tools.txt"
{ cat /proc/meminfo; cat /proc/cpuinfo; df -h; } > "$build_root/evidence/resources.txt"

clone_pin "$(read_pin packaging.repository)" "$(read_pin packaging.commit)" "$build_root/yowasp"
source_dir="$build_root/yowasp"
clone_pin "$(read_pin llvm.repository)" "$(read_pin llvm.commit)" "$source_dir/llvm-src"
clone_pin "$(read_pin wasiLibc.repository)" "$(read_pin wasiLibc.commit)" "$source_dir/wasi-libc-src"
echo "$(read_pin packaging.recipeSha256)  $source_dir/build.sh" | sha256sum -c -
test "$(git -C "$source_dir" log -1 --format=%ct)" = "$(read_pin packaging.sourceDateEpoch)"
git -C "$source_dir" diff --exit-code
git -C "$source_dir/llvm-src" diff --exit-code
git -C "$source_dir/wasi-libc-src" diff --exit-code

curl -fL --retry 2 "$(read_pin sdk.url)" -o "$build_root/wasi-sdk-29.tar.gz"
echo "$(read_pin sdk.sha256)  $build_root/wasi-sdk-29.tar.gz" | sha256sum -c -
tar -xzf "$build_root/wasi-sdk-29.tar.gz" -C "$source_dir"
cp "$source_dir/build.sh" "$build_root/evidence/upstream-build.sh"

# Generator and concurrency change scheduling only. Keep every upstream compiler
# and linker option, including full LTO, MinSizeRel and enabled assertions.
export CMAKE_GENERATOR=Ninja
export CMAKE_BUILD_PARALLEL_LEVEL=${TOOLCHAIN_JOBS:-3}
export MAKEFLAGS="-j${CMAKE_BUILD_PARALLEL_LEVEL}"
export CCACHE_DIR="$build_root/ccache"
ccache --max-size=4G
cd "$source_dir"
/usr/bin/time -v ./build.sh

# Preserve original version metadata while freezing the ranged esbuild dependency
# and all transitive package bytes. There is deliberately no npm publish step.
cp "$recipe_dir/npm/package.json" "$recipe_dir/npm/package-lock.json" "$source_dir/npmjs/"
cd "$source_dir/npmjs"
npm ci --no-audit --no-fund
npm run all
cp package.json package-lock.json "$build_root/evidence/"
cp gen/llvm.core.wasm gen/llvm-resources.tar "$build_root/artifacts/"
gzip -n -c gen/llvm-resources.tar > "$build_root/artifacts/llvm-resources.tar.gz"
cp "$source_dir/llvm-build/CMakeCache.txt" "$build_root/evidence/llvm-CMakeCache.txt"
cp "$source_dir/llvm-tblgen-build/CMakeCache.txt" "$build_root/evidence/tblgen-CMakeCache.txt"
sha256sum "$source_dir/llvm-tblgen-build/bin/"{llvm-tblgen,clang-tblgen} > "$build_root/evidence/native-tool-sha256.txt"
cp "$source_dir/Toolchain-WASI.cmake" "$source_dir/Toolchain-WASI-LLVM.cmake" "$build_root/evidence/"
sha256sum "$build_root/artifacts/"* > "$build_root/evidence/artifact-sha256.txt"
ccache --show-stats > "$build_root/evidence/ccache.txt"
python3 "$recipe_dir/record-build.py" "$build_root"
