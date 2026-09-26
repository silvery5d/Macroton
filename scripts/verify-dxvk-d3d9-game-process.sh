#!/bin/sh
set -eu

if [ "$#" -ne 3 ]; then
    echo "usage: $0 GAME_PID EXPECTED_D3D9_DLL EXPECTED_MOLTENVK_DYLIB" >&2
    exit 2
fi

game_pid=$1
expected_d3d9=$2
expected_moltenvk=$3

for tool in lsof shasum strings; do
    command -v "${tool}" >/dev/null 2>&1 || {
        echo "missing tool: ${tool}" >&2
        exit 1
    }
done

if ! strings "${expected_d3d9}" | grep -q 'DXVK'; then
    echo "expected d3d9.dll does not contain a DXVK identity marker: ${expected_d3d9}" >&2
    exit 1
fi
case "${game_pid}" in
    ''|*[!0-9]*) echo "GAME_PID must be numeric" >&2; exit 2 ;;
esac
for expected in "${expected_d3d9}" "${expected_moltenvk}"; do
    if [ ! -f "${expected}" ]; then
        echo "expected renderer artifact does not exist: ${expected}" >&2
        exit 1
    fi
done

mapped=$(mktemp /private/tmp/macroton-renderer-maps.XXXXXX)
cleanup() {
    rm -f "${mapped}"
}
trap cleanup EXIT HUP INT TERM

# Field output avoids locale-dependent table columns. `n` records contain the
# exact mapped-file paths for the selected process.
lsof -nP -a -p "${game_pid}" -Fn >"${mapped}"

find_mapped_basename() {
    wanted=$1
    sed -n 's/^n//p' "${mapped}" | while IFS= read -r path; do
        case "${path}" in
            */"${wanted}") printf '%s\n' "${path}" ;;
        esac
    done
}

mapped_d3d9=$(find_mapped_basename d3d9.dll | head -n 1 || true)
mapped_moltenvk=$(find_mapped_basename libMoltenVK.dylib | head -n 1 || true)
if [ -z "${mapped_d3d9}" ]; then
    echo "renderer proof failed: process ${game_pid} has no mapped d3d9.dll" >&2
    exit 1
fi
if [ -z "${mapped_moltenvk}" ]; then
    echo "renderer proof failed: process ${game_pid} has no mapped libMoltenVK.dylib" >&2
    exit 1
fi

expected_d3d9_sha=$(shasum -a 256 "${expected_d3d9}" | awk '{print $1}')
mapped_d3d9_sha=$(shasum -a 256 "${mapped_d3d9}" | awk '{print $1}')
expected_moltenvk_sha=$(shasum -a 256 "${expected_moltenvk}" | awk '{print $1}')
mapped_moltenvk_sha=$(shasum -a 256 "${mapped_moltenvk}" | awk '{print $1}')

if [ "${mapped_d3d9_sha}" != "${expected_d3d9_sha}" ]; then
    echo "renderer proof failed: mapped d3d9.dll is not the verified DXVK artifact" >&2
    echo "mapped:   ${mapped_d3d9} (${mapped_d3d9_sha})" >&2
    echo "expected: ${expected_d3d9} (${expected_d3d9_sha})" >&2
    exit 1
fi
if [ "${mapped_moltenvk_sha}" != "${expected_moltenvk_sha}" ]; then
    echo "renderer proof failed: mapped MoltenVK is not the verified runtime artifact" >&2
    echo "mapped:   ${mapped_moltenvk} (${mapped_moltenvk_sha})" >&2
    echo "expected: ${expected_moltenvk} (${expected_moltenvk_sha})" >&2
    exit 1
fi

echo "renderer-proof=dxvk-d3d9-moltenvk"
echo "pid=${game_pid}"
echo "d3d9=${mapped_d3d9}"
echo "d3d9_sha256=${mapped_d3d9_sha}"
echo "moltenvk=${mapped_moltenvk}"
echo "moltenvk_sha256=${mapped_moltenvk_sha}"
