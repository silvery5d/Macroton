#!/bin/sh
set -eu

version="1.10.3"
source_sha256="9dad9868e0d0b485051a5d467c11ca796adef5537a688ae68adb358a90f465a2"
# GitHub's tag archive does not retain the commit timestamp. Use a fixed epoch
# so PE COFF/export timestamps and the final checksum are reproducible.
source_date_epoch="1704067200"

if [ "$#" -ne 2 ]; then
    echo "usage: $0 DXVK_SOURCE_ARCHIVE OUTPUT_DIRECTORY" >&2
    exit 2
fi

archive=$1
output_dir=$2
repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
patch_file="${repo_root}/patches/dxvk-${version}/0001-macroton-moltenvk-d3d9-experiment.patch"
work_root=$(mktemp -d /private/tmp/macroton-dxvk-d3d9-build.XXXXXX)
source_dir="${work_root}/dxvk-${version}"
build_dir="${work_root}/build"

cleanup() {
    rm -rf "${work_root}"
}
trap cleanup EXIT HUP INT TERM

for tool in i686-w64-mingw32-gcc i686-w64-mingw32-g++ meson ninja patch shasum tar; do
    command -v "${tool}" >/dev/null 2>&1 || {
        echo "missing build tool: ${tool}" >&2
        exit 1
    }
done

actual_sha256=$(shasum -a 256 "${archive}" | awk '{print $1}')
if [ "${actual_sha256}" != "${source_sha256}" ]; then
    echo "DXVK ${version} source checksum mismatch" >&2
    echo "expected ${source_sha256}" >&2
    echo "actual   ${actual_sha256}" >&2
    exit 1
fi

tar -xzf "${archive}" -C "${work_root}"
(
    cd "${source_dir}"
    patch -p1 < "${patch_file}"
)

SOURCE_DATE_EPOCH="${source_date_epoch}" meson setup "${build_dir}" "${source_dir}" \
    --cross-file "${source_dir}/build-win32.txt" \
    --buildtype release \
    -Denable_tests=false \
    -Denable_dxgi=false \
    -Denable_d3d10=false \
    -Denable_d3d11=false
SOURCE_DATE_EPOCH="${source_date_epoch}" ninja -C "${build_dir}" src/d3d9/d3d9.dll

mkdir -p "${output_dir}"
cp "${build_dir}/src/d3d9/d3d9.dll" "${output_dir}/d3d9.dll"
shasum -a 256 "${output_dir}/d3d9.dll" > "${output_dir}/SHA256SUMS"

echo "Built experimental DXVK ${version} D3D9 at ${output_dir}/d3d9.dll"
cat "${output_dir}/SHA256SUMS"
echo "This is a probe artifact, not a game runtime."
