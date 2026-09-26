#!/bin/sh
set -eu

if [ "$#" -lt 1 ] || [ "$#" -gt 2 ]; then
    echo "usage: $0 PATCHED_SPIRV_CROSS_SOURCE [BUILD_DIR]" >&2
    exit 2
fi

source_root=$1
build_root=${2:-$(mktemp -d /private/tmp/macroton-spirv-cross-shadow-alias.XXXXXX)}
repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cleanup_build=0
if [ "$#" -eq 1 ]; then
    cleanup_build=1
fi

cleanup() {
    if [ "$cleanup_build" -eq 1 ]; then
        rm -rf "$build_root"
    fi
}
trap cleanup EXIT HUP INT TERM

for tool in cmake clang++ spirv-as; do
    command -v "$tool" >/dev/null 2>&1 || {
        echo "missing tool: $tool" >&2
        exit 1
    }
done
for source_file in spirv_msl.cpp spirv_msl.hpp; do
    if [ ! -f "$source_root/$source_file" ]; then
        echo "invalid SPIRV-Cross source root: $source_root" >&2
        exit 1
    fi
done

cmake -S "$source_root" -B "$build_root" -G Ninja \
    -DSPIRV_CROSS_CLI=OFF \
    -DSPIRV_CROSS_ENABLE_TESTS=OFF \
    -DCMAKE_BUILD_TYPE=Release >/dev/null
cmake --build "$build_root" --target spirv-cross-msl -j 8 >/dev/null

spirv-as --target-env vulkan1.1 \
    "$repo_root/tests/fixtures/alan-wake-shadow-sampler.spvasm" \
    -o "$build_root/alan-wake-shadow-sampler.spv"

clang++ -std=c++17 -O2 -I"$source_root" \
    "$repo_root/tools/spirv_cross_shadow_alias_probe.cpp" \
    "$build_root/libspirv-cross-msl.a" \
    "$build_root/libspirv-cross-glsl.a" \
    "$build_root/libspirv-cross-core.a" \
    -o "$build_root/spirv-cross-shadow-alias-probe"

"$build_root/spirv-cross-shadow-alias-probe" \
    "$build_root/alan-wake-shadow-sampler.spv" \
    >"$build_root/alan-wake-shadow-sampler.msl"

for slot in 0 1 2; do
    expected="constant auto &s${slot}_2d_shadowSmplr = reinterpret_cast<constant sampler &>(spvDescriptorSet0.s${slot}_2dSmplr);"
    if ! grep -Fq "$expected" "$build_root/alan-wake-shadow-sampler.msl"; then
        echo "missing overlapping sampler alias for slot $slot" >&2
        exit 5
    fi
done

echo "shadow_sampler_aliases=3"
echo "fixture=FS_06b5a85f250afd382f489fce9b9114f27881193d"
