#!/bin/sh
set -eu

if [ "$#" -lt 2 ] || [ "$#" -gt 3 ]; then
    echo "usage: $0 WINE_ENTRYPOINT PATCHED_D3D9_DLL [PREFIX]" >&2
    exit 2
fi

wine=$1
d3d9_dll=$2
prefix=${3:-/private/tmp/macroton-patched-dxvk-d3d9-probe}
repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
probe="${repo_root}/target/probes/d3d9_probe.exe"
staging=$(mktemp -d /private/tmp/macroton-patched-dxvk-probe.XXXXXX)
output=$(mktemp /private/tmp/macroton-patched-dxvk-output.XXXXXX)
timeout_seconds=${MACROTON_PROBE_TIMEOUT_SECONDS:-45}
max_attempts=${MACROTON_PROBE_ATTEMPTS:-2}
wine_debug=${MACROTON_WINEDEBUG:-warn+vulkan}
dyld_fallback=${DYLD_FALLBACK_LIBRARY_PATH:-}

cleanup() {
    WINEPREFIX="${prefix}" "${wine%/wine}/wineserver" -k >/dev/null 2>&1 || true
    rm -rf "${staging}"
    rm -f "${output}"
}
trap cleanup EXIT HUP INT TERM

if [ ! -x "${wine}" ]; then
    echo "Wine entrypoint is not executable: ${wine}" >&2
    exit 1
fi
if [ ! -f "${d3d9_dll}" ]; then
    echo "patched D3D9 DLL does not exist: ${d3d9_dll}" >&2
    exit 1
fi

mkdir -p "$(dirname -- "${probe}")" "${prefix}"
i686-w64-mingw32-gcc -O2 -Wall -Wextra -Werror \
    "${repo_root}/tools/d3d9_probe.c" -o "${probe}" \
    -ld3d9 -lgdi32 -luser32
cp "${probe}" "${staging}/d3d9_probe.exe"
cp "${d3d9_dll}" "${staging}/d3d9.dll"

if [ ! -f "${prefix}/system.reg" ]; then
    : >"${output}"
    set +e
    env \
        WINEPREFIX="${prefix}" \
        WINEDEBUG="${wine_debug}" \
        DYLD_FALLBACK_LIBRARY_PATH="${dyld_fallback}" \
        "${wine}" wineboot --init >"${output}" 2>&1 &
    init_pid=$!
    elapsed=0
    init_timed_out=0
    while kill -0 "${init_pid}" >/dev/null 2>&1; do
        if [ "${elapsed}" -ge "${timeout_seconds}" ]; then
            echo "Wine prefix initialization timed out after ${timeout_seconds}s" >&2
            kill "${init_pid}" >/dev/null 2>&1 || true
            wait "${init_pid}" >/dev/null 2>&1 || true
            init_timed_out=1
            break
        fi
        sleep 1
        elapsed=$((elapsed + 1))
    done
    if [ "${init_timed_out}" -eq 0 ]; then
        wait "${init_pid}"
        init_status=$?
    else
        init_status=124
    fi
    set -e
    if [ "${init_status}" -ne 0 ] && [ -f "${prefix}/system.reg" ]; then
        echo "initial wineboot returned ${init_status} after creating the prefix; retrying update in the warm prefix" >&2
        WINEPREFIX="${prefix}" "${wine%/wine}/wineserver" -k >/dev/null 2>&1 || true
        : >"${output}"
        set +e
        env \
            WINEPREFIX="${prefix}" \
            WINEDEBUG="${wine_debug}" \
            DYLD_FALLBACK_LIBRARY_PATH="${dyld_fallback}" \
            "${wine}" wineboot --update >"${output}" 2>&1 &
        update_pid=$!
        elapsed=0
        update_timed_out=0
        while kill -0 "${update_pid}" >/dev/null 2>&1; do
            if [ "${elapsed}" -ge "${timeout_seconds}" ]; then
                echo "Wine prefix update timed out after ${timeout_seconds}s" >&2
                kill "${update_pid}" >/dev/null 2>&1 || true
                wait "${update_pid}" >/dev/null 2>&1 || true
                update_timed_out=1
                break
            fi
            sleep 1
            elapsed=$((elapsed + 1))
        done
        if [ "${update_timed_out}" -eq 0 ]; then
            wait "${update_pid}"
            init_status=$?
        else
            init_status=124
        fi
        set -e
    fi
    if [ "${init_status}" -ne 0 ] || [ ! -f "${prefix}/system.reg" ]; then
        cat "${output}"
        echo "Wine prefix initialization failed" >&2
        exit 4
    fi
    # Measure the D3D9 process from a stopped server even when the prefix was
    # created in this invocation. Prefix initialization time is a separate gate.
    WINEPREFIX="${prefix}" "${wine%/wine}/wineserver" -k >/dev/null 2>&1 || true
fi

attempt=1
status=1
while [ "${attempt}" -le "${max_attempts}" ]; do
    : >"${output}"
    set +e
    (
        cd "${staging}"
        env \
            WINEPREFIX="${prefix}" \
            WINEDLLOVERRIDES='d3d9=n' \
            DXVK_LOG_LEVEL=info \
            WINEDEBUG="${wine_debug}" \
            DYLD_FALLBACK_LIBRARY_PATH="${dyld_fallback}" \
            "${wine}" "${staging}/d3d9_probe.exe"
    ) >"${output}" 2>&1 &
    probe_pid=$!
    elapsed=0
    timed_out=0
    while kill -0 "${probe_pid}" >/dev/null 2>&1; do
        if [ "${elapsed}" -ge "${timeout_seconds}" ]; then
            echo "patched DXVK D3D9 probe attempt ${attempt} timed out after ${timeout_seconds}s" >&2
            kill "${probe_pid}" >/dev/null 2>&1 || true
            wait "${probe_pid}" >/dev/null 2>&1 || true
            status=124
            timed_out=1
            break
        fi
        sleep 1
        elapsed=$((elapsed + 1))
    done
    if [ "${timed_out}" -eq 0 ]; then
        wait "${probe_pid}"
        status=$?
    fi
    set -e

    if grep -q 'd3d9_present=0x00000000' "${output}"; then
        break
    fi
    if [ "${attempt}" -lt "${max_attempts}" ]; then
        echo "patched DXVK D3D9 probe attempt ${attempt} did not present; retrying in the warm prefix" >&2
    fi
    attempt=$((attempt + 1))
done

cat "${output}"
for result in \
    'd3d9_create_device=0x00000000' \
    'd3d9_clear=0x00000000' \
    'd3d9_present=0x00000000'; do
    if ! grep -q "${result}" "${output}"; then
        echo "patched DXVK D3D9 probe did not pass ${result%%=*}" >&2
        exit 5
    fi
done

exit "${status}"
