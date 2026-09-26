#!/bin/sh
set -eu

version="0.74"
archive_sha256="2598981a8b725653773e277470a95dda4253b8a14d36e0dc96dce0e3800f0ceb"

if [ "$#" -ne 3 ]; then
    echo "usage: $0 BASE_WINE_ROOT DXMT_ARCHIVE DESTINATION" >&2
    exit 2
fi

base_runtime=$1
archive=$2
destination=$3

for tool in cp find mkdir mktemp shasum tar; do
    command -v "${tool}" >/dev/null 2>&1 || {
        echo "missing tool: ${tool}" >&2
        exit 1
    }
done

if [ ! -x "${base_runtime}/bin/wine" ]; then
    echo "base runtime has no executable bin/wine: ${base_runtime}" >&2
    exit 1
fi
if [ ! -f "${archive}" ]; then
    echo "DXMT archive does not exist: ${archive}" >&2
    exit 1
fi
if [ -e "${destination}" ]; then
    echo "destination already exists; refusing to overwrite: ${destination}" >&2
    exit 1
fi

actual_sha256=$(shasum -a 256 "${archive}" | awk '{print $1}')
if [ "${actual_sha256}" != "${archive_sha256}" ]; then
    echo "DXMT archive checksum mismatch" >&2
    echo "expected ${archive_sha256}" >&2
    echo "actual   ${actual_sha256}" >&2
    exit 1
fi

staging=$(mktemp -d "${destination}.staging.XXXXXX")
extracted=$(mktemp -d "${destination}.dxmt.XXXXXX")
cleanup() {
    rm -rf "${staging}" "${extracted}"
}
trap cleanup EXIT HUP INT TERM

# APFS cloning avoids another full physical copy when supported. Fall back to
# a regular recursive copy for other macOS volumes.
if ! cp -cR "${base_runtime}/." "${staging}/" 2>/dev/null; then
    cp -R "${base_runtime}/." "${staging}/"
fi
tar -xzf "${archive}" -C "${extracted}"

component_files="
i386-windows/d3d10core.dll
i386-windows/d3d11.dll
i386-windows/dxgi.dll
i386-windows/winemetal.dll
x86_64-windows/d3d10core.dll
x86_64-windows/d3d11.dll
x86_64-windows/dxgi.dll
x86_64-windows/winemetal.dll
x86_64-unix/winemetal.so
"

for relative_path in ${component_files}; do
    source_file="${extracted}/v${version}/${relative_path}"
    target_file="${staging}/lib/wine/${relative_path}"
    if [ ! -f "${source_file}" ]; then
        echo "DXMT archive is missing ${relative_path}" >&2
        exit 1
    fi
    cp "${source_file}" "${target_file}"
done

mkdir -p "${staging}/share/macroton/components"
manifest="${staging}/share/macroton/components/dxmt-${version}.conf"
{
    echo "schema=1"
    echo "id=dxmt-${version}"
    echo "source=https://github.com/3Shain/dxmt/releases/tag/v${version}"
    echo "license=LGPL-2.1"
    echo "archive_sha256=${archive_sha256}"
    for relative_path in ${component_files}; do
        file_sha256=$(shasum -a 256 "${staging}/lib/wine/${relative_path}" | awk '{print $1}')
        echo "file.${relative_path}=${file_sha256}"
    done
} >"${manifest}"

mv "${staging}" "${destination}"
echo "Composed Wine + DXMT ${version} runtime: ${destination}"
echo "Component manifest: ${destination}/share/macroton/components/dxmt-${version}.conf"

