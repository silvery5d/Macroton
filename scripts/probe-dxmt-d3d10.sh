#!/bin/sh
set -eu

if [ "$#" -lt 1 ] || [ "$#" -gt 2 ]; then
    echo "usage: $0 WINE_ENTRYPOINT [PREFIX]" >&2
    exit 2
fi

wine=$1
prefix=${2:-/private/tmp/macroton-dxmt-d3d10-probe}
project_root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
probe="${project_root}/target/probes/d3d10_probe.exe"
output=$(mktemp /private/tmp/macroton-dxmt-probe-output.XXXXXX)

cleanup() {
    WINEPREFIX="${prefix}" WINEDEBUG=-all "$(dirname "${wine}")/wineserver" -k >/dev/null 2>&1 || true
    rm -f "${output}"
}
trap cleanup EXIT HUP INT TERM

if [ ! -x "${wine}" ]; then
    echo "Wine entrypoint is not executable: ${wine}" >&2
    exit 1
fi

mkdir -p "${project_root}/target/probes" "${prefix}"
i686-w64-mingw32-gcc -Wall -Wextra -Werror -O2 \
    "${project_root}/tools/d3d10_probe.c" \
    -o "${probe}" -ld3d10 -ldxgi -luuid

set +e
env \
    WINEPREFIX="${prefix}" \
    WINEDEBUG=+loaddll \
    DXMT_LOG_LEVEL=trace \
    DYLD_PRINT_LIBRARIES=1 \
    WINEDLLOVERRIDES=dxgi,d3d11,d3d10core=b \
    "${wine}" "${probe}" >"${output}" 2>&1
status=$?
set -e
grep -Ei 'winemetal|DXGI.DLL|D3D11.DLL|d3d10core|Maximum supported feature|Using feature level|d3d10_create_device|adapter_vendor' "${output}" || true
exit "${status}"
