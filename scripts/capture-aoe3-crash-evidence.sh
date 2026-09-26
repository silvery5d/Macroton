#!/bin/sh
set -eu

usage() {
    echo "usage: $0 PROFILE EVIDENCE_DIR [MAX_RSS_MIB]" >&2
    exit 2
}

[ "$#" -ge 2 ] && [ "$#" -le 3 ] || usage
profile=$1
evidence_dir=$2
max_rss=${3:-1800}
wait_seconds=${MACROTON_AOE3_WAIT_SECONDS:-30}
case "$evidence_dir" in
    /*) ;;
    *) echo "EVIDENCE_DIR must be absolute" >&2; exit 2 ;;
esac
case "$max_rss" in
    ''|*[!0-9]*) echo "MAX_RSS_MIB must be an integer" >&2; exit 2 ;;
esac
[ -f "$profile" ] || { echo "profile is absent: $profile" >&2; exit 1; }
[ ! -e "$evidence_dir" ] || { echo "refusing to replace evidence: $evidence_dir" >&2; exit 1; }

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
macroton_bin=${MACROTON_BIN:-$root/target/debug/macroton}
data_root=${MACROTON_DATA_ROOT:-$HOME/Library/Application Support/Macroton}
[ -x "$macroton_bin" ] || { echo "Macroton binary is absent: $macroton_bin" >&2; exit 1; }

game_root="$data_root/games/105450"
steam_root="$game_root/prefix/drive_c/Program Files (x86)/Steam"
gameprocess_log="$steam_root/logs/gameprocess_log.txt"
baseline_gameprocess_lines=0
[ -f "$gameprocess_log" ] && baseline_gameprocess_lines=$(wc -l <"$gameprocess_log")
mkdir -p "$evidence_dir"
launched=0
cleanup() {
    if [ "$launched" -eq 1 ]; then
        "$macroton_bin" stop 105450 --data-root "$data_root" >/dev/null 2>&1 || true
    fi
}
trap cleanup EXIT HUP INT TERM

start_ms=$(date +%s)
set +e
"$macroton_bin" launch 105450 --profile "$profile" \
    --data-root "$data_root" --max-rss-mib "$max_rss" \
    >"$evidence_dir/launch.stdout" 2>"$evidence_dir/launch.stderr"
launch_status=$?
set -e
if [ "$launch_status" -eq 0 ] || ! rg -q "already has an incompatible|active Steam session provenance" "$evidence_dir/launch.stderr"; then
    launched=1
fi

# `launch` submits to Steam and does not wait for the game. Give the tracked
# lifecycle a short, explicit observation window before the cleanup trap runs.
if [ "$launched" -eq 1 ]; then
    observed=0
    elapsed=0
    while [ "$elapsed" -lt "$wait_seconds" ]; do
        status_json=$("$macroton_bin" steam launch-status 105450 \
            --data-root "$data_root" --json 2>/dev/null || true)
        if printf '%s' "$status_json" | rg -q '"state": "running-observed"'; then
            observed=1
            break
        fi
        if printf '%s' "$status_json" | rg -q '"state": "exited"|"state": "failed"'; then
            break
        fi
        sleep 1
        elapsed=$((elapsed + 1))
    done
    if [ "$observed" -eq 1 ]; then
        sleep 15
    fi
fi

final_status_json=$("$macroton_bin" steam launch-status 105450 \
    --data-root "$data_root" --json 2>/dev/null || true)

printf '%s\n' "schema=1" "appid=105450" "profile=$profile" \
    "max_rss_mib=$max_rss" "launch_status=$launch_status" \
    "started_unix_seconds=$start_ms" "baseline_gameprocess_lines=$baseline_gameprocess_lines" \
    "final_status_json=$final_status_json" >"$evidence_dir/collection.txt"

for item in steam-client.log resource-guard.log; do
    [ -f "$game_root/$item" ] && cp "$game_root/$item" "$evidence_dir/$item"
done
for item in logs/gameprocess_log.txt \
    steamapps/common/Age\ Of\ Empires\ 3/bin/age3_d3d9.log \
    steamapps/common/Age\ Of\ Empires\ 3/bin/age3x_d3d9.log; do
    [ -f "$steam_root/$item" ] && cp "$steam_root/$item" "$evidence_dir/$(basename "$item")"
done

if [ -f "$evidence_dir/gameprocess_log.txt" ]; then
    tail -n +$((baseline_gameprocess_lines + 1)) "$evidence_dir/gameprocess_log.txt" \
        >"$evidence_dir/gameprocess-new.txt"
    rg -n "AppID 105450|exit code|age3.exe" "$evidence_dir/gameprocess-new.txt" \
        >"$evidence_dir/gameprocess-summary.txt" || true
fi
if [ -f "$evidence_dir/steam-client.log" ]; then
    rg -n "Unhandled|page fault|division by zero|Assertion Failed" \
        "$evidence_dir/steam-client.log" >"$evidence_dir/wine-exception-summary.txt" || true
fi

echo "AoE3 crash evidence captured at $evidence_dir"
echo "launch_status=$launch_status"
