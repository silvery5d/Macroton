#!/bin/sh
set -eu

usage() {
    echo "usage: $0 APPID PROFILE STEAM_ROOT RUNTIME_ROOT DATA_ROOT OUTPUT.gputrace EVIDENCE.json [TIMEOUT_SECONDS]" >&2
    exit 2
}

[ "$#" -ge 7 ] && [ "$#" -le 8 ] || usage

app_id=$1
profile=$2
steam_root=$3
runtime_root=$4
data_root=$5
output=$6
evidence_output=$7
timeout_seconds=${8:-60}

case "$app_id" in
    ''|*[!0-9]*) echo "APPID must contain decimal digits" >&2; exit 2 ;;
esac
case "$timeout_seconds" in
    ''|*[!0-9]*) echo "TIMEOUT_SECONDS must contain decimal digits" >&2; exit 2 ;;
esac
[ "$timeout_seconds" -ge 1 ] && [ "$timeout_seconds" -le 300 ] || {
    echo "TIMEOUT_SECONDS must be between 1 and 300" >&2
    exit 2
}
case "$output" in
    /*.gputrace) ;;
    *) echo "OUTPUT must be an absolute .gputrace path" >&2; exit 2 ;;
esac
[ ! -e "$output" ] || {
    echo "refusing to replace existing GPU capture: $output" >&2
    exit 1
}
case "$evidence_output" in
    /*.json) ;;
    *) echo "EVIDENCE must be an absolute .json path" >&2; exit 2 ;;
esac
[ ! -e "$evidence_output" ] || {
    echo "refusing to replace existing GPU evidence: $evidence_output" >&2
    exit 1
}
[ "$output" != "$evidence_output" ] || {
    echo "GPU capture and evidence paths must differ" >&2
    exit 2
}
[ -f "$profile" ] || { echo "profile is absent: $profile" >&2; exit 1; }
[ -d "$steam_root" ] || { echo "Steam root is absent: $steam_root" >&2; exit 1; }
[ -d "$runtime_root" ] || { echo "runtime root is absent: $runtime_root" >&2; exit 1; }

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(dirname -- "$script_dir")
macroton_bin=${MACROTON_BIN:-$repo_root/target/debug/macroton}
[ -x "$macroton_bin" ] || {
    echo "Macroton binary is absent or not executable: $macroton_bin" >&2
    echo "run cargo build first or set MACROTON_BIN" >&2
    exit 1
}

mkdir -p "$(dirname -- "$output")"
mkdir -p "$(dirname -- "$evidence_output")"
launched=0
cleanup() {
    [ "$launched" -eq 1 ] || return 0
    [ "${MACROTON_KEEP_RUNNING:-0}" = 1 ] && return 0
    "$macroton_bin" stop "$app_id" \
        --runtime-root "$runtime_root" --data-root "$data_root" >/dev/null 2>&1 || true
}
trap cleanup EXIT
trap 'exit 130' HUP INT TERM

resource_status=$(
    "$macroton_bin" resource status "$app_id" --data-root "$data_root" --json
)
echo "$resource_status" | grep -q '"process_count": 0' || {
    echo "refusing GPU capture because the AppID prefix is not idle" >&2
    exit 1
}

launched=1
METAL_CAPTURE_ENABLED=1 \
MVK_CONFIG_AUTO_GPU_CAPTURE_SCOPE=2 \
MVK_CONFIG_AUTO_GPU_CAPTURE_OUTPUT_FILE="$output" \
    "$macroton_bin" launch "$app_id" --profile "$profile" \
        --steam-root "$steam_root" --runtime-root "$runtime_root" \
        --data-root "$data_root"

deadline=$(( $(date +%s) + timeout_seconds ))
while [ "$(date +%s)" -lt "$deadline" ]; do
    capture_ready=0
    if [ -f "$output" ] && [ -s "$output" ]; then
        capture_ready=1
    elif [ -d "$output" ] && find "$output" -type f -size +0c -print -quit | grep -q .; then
        capture_ready=1
    fi
    if [ "$capture_ready" -eq 1 ]; then
        if "$macroton_bin" evidence verify-gpu-capture "$app_id" \
                --profile "$profile" --capture "$output" \
                --output "$evidence_output" --data-root "$data_root" \
                --runtime-root "$runtime_root" --stability-ms 500; then
            echo "MoltenVK first-frame capture: $output"
            echo "Bound evidence: $evidence_output"
            echo "The capture proves GPU command recording only; it is not a presentation or gameplay pass."
            exit 0
        fi
    fi
    sleep 1
done

echo "MoltenVK did not create and verify a first-frame capture within ${timeout_seconds}s: $output" >&2
exit 1
