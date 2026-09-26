#!/bin/sh
set -eu

if [ "$#" -lt 1 ] || [ "$#" -gt 2 ]; then
    echo "usage: $0 MOLTENVK_SOURCE_ROOT [OUTPUT_DYLIB]" >&2
    exit 2
fi

source_root=$1
output_dylib=${2:-${source_root}/Package/Release/MoltenVK/dynamic/dylib/macOS/libMoltenVK.dylib}
repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
expected_moltenvk_commit="db66022459ffb663aa2b50f6b018bc2e124f5edf"
expected_spirv_cross_commit="6c09849fe88c48eaed08413aa022aaa136a3a057"
patch_file="${repo_root}/patches/spirv-cross/0001-msl-alias-overlapping-combined-samplers.patch"
spirv_cross_root="${source_root}/External/SPIRV-Cross"
intermediates="${source_root}/External/build/Intermediates/macOS"
rebuilt_spirv_cross="${intermediates}/Build/Products/Release/libSPIRVCross.a"
packaged_spirv_cross="${source_root}/External/build/Release/SPIRVCross.xcframework/macos-arm64_x86_64/libSPIRVCross.a"

for tool in git make shasum xcodebuild; do
    command -v "$tool" >/dev/null 2>&1 || {
        echo "missing build tool: $tool" >&2
        exit 1
    }
done
if [ ! -x "${source_root}/fetchDependencies" ] || [ ! -f "$patch_file" ]; then
    echo "invalid MoltenVK source tree or missing Macroton patch: ${source_root}" >&2
    exit 1
fi

actual_moltenvk_commit=$(git -C "$source_root" rev-parse HEAD)
if [ "$actual_moltenvk_commit" != "$expected_moltenvk_commit" ]; then
    echo "MoltenVK commit mismatch" >&2
    echo "expected ${expected_moltenvk_commit}" >&2
    echo "actual   ${actual_moltenvk_commit}" >&2
    exit 1
fi
if ! git -C "$source_root" diff --quiet || ! git -C "$source_root" diff --cached --quiet; then
    echo "MoltenVK source tree must be clean before the reproducible build" >&2
    exit 1
fi

# MoltenVK's dependency fetcher hard-resets dependency checkouts. It must run
# before the Macroton SPIRV-Cross patch, never after it.
(cd "$source_root" && ./fetchDependencies --macos --keep-cache)

actual_spirv_cross_commit=$(git -C "$spirv_cross_root" rev-parse HEAD)
if [ "$actual_spirv_cross_commit" != "$expected_spirv_cross_commit" ]; then
    echo "SPIRV-Cross commit mismatch" >&2
    echo "expected ${expected_spirv_cross_commit}" >&2
    echo "actual   ${actual_spirv_cross_commit}" >&2
    exit 1
fi
if ! git -C "$spirv_cross_root" diff --quiet || ! git -C "$spirv_cross_root" diff --cached --quiet; then
    echo "SPIRV-Cross checkout is not clean after dependency fetch" >&2
    exit 1
fi

git -C "$spirv_cross_root" apply --check "$patch_file"
git -C "$spirv_cross_root" apply "$patch_file"
"${repo_root}/scripts/verify-spirv-cross-shadow-alias.sh" "$spirv_cross_root"

xcodebuild -quiet \
    -project "${source_root}/ExternalDependencies.xcodeproj" \
    -scheme SPIRV-Cross-macOS \
    -destination generic/platform=macOS \
    -configuration Release \
    -derivedDataPath "$intermediates" build

if [ ! -f "$rebuilt_spirv_cross" ] || [ ! -f "$packaged_spirv_cross" ]; then
    echo "SPIRV-Cross build did not produce the expected library" >&2
    exit 1
fi
cp "$rebuilt_spirv_cross" "$packaged_spirv_cross"

(cd "$source_root" && make macos MVK_CONFIG_LOG_LEVEL=1)

produced_dylib="${source_root}/Package/Release/MoltenVK/dynamic/dylib/macOS/libMoltenVK.dylib"
if [ ! -f "$produced_dylib" ]; then
    echo "MoltenVK build did not produce the expected dylib" >&2
    exit 1
fi
if [ "$output_dylib" != "$produced_dylib" ]; then
    output_parent=$(dirname "$output_dylib")
    mkdir -p "$output_parent"
    cp "$produced_dylib" "$output_dylib"
fi

echo "moltenvk_commit=${actual_moltenvk_commit}"
echo "spirv_cross_commit=${actual_spirv_cross_commit}"
echo "moltenvk_dylib=${output_dylib}"
echo "moltenvk_sha256=$(shasum -a 256 "$output_dylib" | awk '{print $1}')"
