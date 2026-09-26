#!/bin/sh
set -eu

version="2.87.3"
archive_sha256="6fb954bed55bf70e948c5045a663a9df31ea206faf105e327bafe46c318f867f"

if [ "$#" -lt 2 ] || [ "$#" -gt 3 ]; then
    echo "usage: $0 DXMT_WINE_ENTRYPOINT DGVOODOO_ZIP [PREFIX]" >&2
    exit 2
fi

wine=$1
archive=$2
prefix=${3:-/private/tmp/macroton-dgvoodoo-dxmt-probe}
project_root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
probe_root=$(mktemp -d /private/tmp/macroton-dgvoodoo-probe.XXXXXX)
output=$(mktemp /private/tmp/macroton-dgvoodoo-output.XXXXXX)

cleanup() {
    WINEPREFIX="${prefix}" WINEDEBUG=-all "$(dirname "${wine}")/wineserver" -k >/dev/null 2>&1 || true
    rm -rf "${probe_root}"
    rm -f "${output}"
}
trap cleanup EXIT HUP INT TERM

if [ ! -x "${wine}" ]; then
    echo "DXMT Wine entrypoint is not executable: ${wine}" >&2
    exit 1
fi
if [ ! -f "${archive}" ]; then
    echo "dgVoodoo archive does not exist: ${archive}" >&2
    exit 1
fi

actual_sha256=$(shasum -a 256 "${archive}" | awk '{print $1}')
if [ "${actual_sha256}" != "${archive_sha256}" ]; then
    echo "dgVoodoo ${version} archive checksum mismatch" >&2
    echo "expected ${archive_sha256}" >&2
    echo "actual   ${actual_sha256}" >&2
    exit 1
fi

unzip -jq "${archive}" MS/x86/D3D9.dll dgVoodoo.conf -d "${probe_root}"
i686-w64-mingw32-gcc -Wall -Wextra -Werror -O2 \
    "${project_root}/tools/d3d9_probe.c" \
    -o "${probe_root}/d3d9_probe.exe" -ld3d9
mkdir -p "${prefix}"

set +e
(
    cd "${probe_root}"
    env \
        WINEPREFIX="${prefix}" \
        WINEDEBUG=+loaddll \
        DXMT_LOG_LEVEL=trace \
        WINEDLLOVERRIDES='d3d9=n;dxgi,d3d11,d3d10core=b' \
        "${wine}" "${probe_root}/d3d9_probe.exe"
) >"${output}" 2>&1
status=$?
set -e

grep -Ei 'd3d9\.dll.*native|winemetal|DXGI\.DLL|D3D11\.DLL|Maximum supported feature|Using feature level|d3d9_create_device|Unhandled page fault' "${output}" || true
if grep -q 'Unhandled page fault' "${output}"; then
    echo "dgVoodoo D3D9 device creation crashed; rejecting this backend chain" >&2
    exit 5
fi
if ! grep -q 'd3d9_create_device=0x00000000' "${output}"; then
    echo "D3D9 device creation did not complete successfully" >&2
    exit 6
fi
exit "${status}"
