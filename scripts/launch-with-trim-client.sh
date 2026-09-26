#!/bin/sh
set -eu

if [ "$#" -lt 4 ] || [ "$#" -gt 5 ]; then
    echo "usage: $0 APPID PROFILE DATA_ROOT RUNTIME_ROOT [TIMEOUT_SECONDS]" >&2
    exit 2
fi
app_id=$1
profile=$2
data_root=$3
runtime_root=$4
timeout_seconds=${5:-120}
macroton_bin=${MACROTON_BIN:-target/debug/macroton}
max_rss_mib=${MACROTON_TRIM_MAX_RSS_MIB:-2200}

case "$app_id" in ''|*[!0-9]*) echo "APPID must be unsigned decimal" >&2; exit 2 ;; esac
case "$timeout_seconds" in ''|*[!0-9]*) echo "TIMEOUT_SECONDS must be unsigned decimal" >&2; exit 2 ;; esac
case "$max_rss_mib" in ''|*[!0-9]*) echo "MACROTON_TRIM_MAX_RSS_MIB must be unsigned decimal" >&2; exit 2 ;; esac
[ "$timeout_seconds" -gt 0 ] || { echo "TIMEOUT_SECONDS must be greater than zero" >&2; exit 2; }
[ "$max_rss_mib" -gt 0 ] || { echo "MACROTON_TRIM_MAX_RSS_MIB must be greater than zero" >&2; exit 2; }
[ -x "$macroton_bin" ] || { echo "Macroton binary is not executable: $macroton_bin" >&2; exit 1; }
expected_executable=$(awk -F= '$1 == "executable" { value=$2 } END { print value }' "$profile" 2>/dev/null || true)
# Profiles with a launcher must opt in to an earlier trim point.  Otherwise
# wait for the real game executable, so shutting down Steam cannot terminate
# a launcher that still owns the game handoff.
trim_after_executable=$(awk -F= '$1 == "trim_after_executable" { value=$2 } END { print value }' "$profile" 2>/dev/null || true)
[ -n "$trim_after_executable" ] && expected_executable=$trim_after_executable
expected_executable=${expected_executable##*/}
expected_executable=${expected_executable##*\\}
[ -n "$expected_executable" ] || { echo "profile has no executable= field: $profile" >&2; exit 2; }

cleanup() {
    "$macroton_bin" stop "$app_id" \
        --data-root "$data_root" \
        --runtime-root "$runtime_root" \
        --preserve-session >/dev/null 2>&1 || true
}
on_signal() {
    trap - HUP INT TERM
    exit 130
}
trap cleanup EXIT
trap on_signal HUP INT TERM

"$macroton_bin" launch "$app_id" \
    --profile "$profile" \
    --data-root "$data_root" \
    --runtime-root "$runtime_root" \
    --max-rss-mib "$max_rss_mib"

started=$(date +%s)
while :; do
    status=$(
        "$macroton_bin" resource status "$app_id" \
            --data-root "$data_root" --json 2>/dev/null || true
    )
    # Resource JSON is produced by Macroton itself. Match the configured game
    # executable only. Launcher profiles therefore remain untouched until the
    # profile's explicit trim_after_executable (normally the real game image)
    # appears.
    executable=$(
        printf '%s' "$status" | EXPECTED_EXECUTABLE="$expected_executable" python3 -c '
import json, sys
import os
try:
    data=json.load(sys.stdin)
except Exception:
    raise SystemExit(0)
expected=os.environ.get("EXPECTED_EXECUTABLE", "").lower()
for process in data.get("processes", []):
    if process.get("category") == "windows-game":
        command=str(process.get("command", ""))
        if command.lower() == expected:
            print(command)
            break
'
    )
    if [ -n "$executable" ]; then
        echo "Detected game process: $executable" >&2
        "$macroton_bin" steam trim-client "$app_id" \
            --profile "$profile" \
            --data-root "$data_root" \
            --runtime-root "$runtime_root"
        trap - EXIT HUP INT TERM
        exit 0
    fi
    now=$(date +%s)
    if [ $((now - started)) -ge "$timeout_seconds" ]; then
        echo "Timed out waiting for the configured game process; stopping AppID $app_id" >&2
        exit 1
    fi
    sleep 1
done
