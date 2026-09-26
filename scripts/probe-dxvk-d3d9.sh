#!/bin/sh
set -eu

if [ "$#" -lt 2 ] || [ "$#" -gt 3 ]; then
    echo "usage: $0 WINE_ENTRYPOINT DXVK_ARCHIVE [PREFIX]" >&2
    exit 2
fi

wine=$1
archive=$2
prefix=${3:-/private/tmp/macroton-dxvk-d3d9-probe}
expected_sha256="9c538924110a7cdef871ca36dee218c0774124374ffdeb38af4b76be55bdf7c2"
repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
probe="${repo_root}/target/probes/d3d9_probe.exe"
staging=$(mktemp -d /private/tmp/macroton-dxvk-d3d9.XXXXXX)

cleanup() {
    WINEPREFIX="${prefix}" "${wine%/wine}/wineserver" -k >/dev/null 2>&1 || true
    rm -rf "${staging}"
}
trap cleanup EXIT HUP INT TERM

actual_sha256=$(shasum -a 256 "${archive}" | awk '{print $1}')
if [ "${actual_sha256}" != "${expected_sha256}" ]; then
    echo "DXVK archive checksum mismatch" >&2
    echo "expected ${expected_sha256}" >&2
    echo "actual   ${actual_sha256}" >&2
    exit 1
fi

mkdir -p "$(dirname -- "${probe}")"
i686-w64-mingw32-gcc -O2 -Wall -Wextra "${repo_root}/tools/d3d9_probe.c" \
    -o "${probe}" -ld3d9 -lgdi32 -luser32
tar -xzf "${archive}" -C "${staging}" dxvk-3.0.2/x32/d3d9.dll
cp "${probe}" "${staging}/d3d9_probe.exe"
cp "${staging}/dxvk-3.0.2/x32/d3d9.dll" "${staging}/d3d9.dll"

mkdir -p "${prefix}"
echo "DXVK D3D9 probe: ${wine}"
echo "Prefix: ${prefix}"
WINEPREFIX="${prefix}" \
WINEDLLOVERRIDES="d3d9=n" \
DXVK_LOG_LEVEL=info \
WINEDEBUG=-all \
"${wine}" "${staging}/d3d9_probe.exe"
