#!/bin/sh
set -eu

usage() {
    echo "usage: $0 APPID PROFILE <WINDOW_ID|auto> <PID|auto> EVIDENCE_DIR DATA_ROOT RUNTIME_ROOT <key NAME|click X_FRACTION Y_FRACTION>" >&2
    exit 2
}

[ "$#" -ge 9 ] && [ "$#" -le 10 ] || usage

app_id=$1
profile=$2
window_id=$3
game_pid=$4
evidence_dir=$5
data_root=$6
runtime_root=$7
shift 7

case "$app_id" in
    ''|*[!0-9]*) echo "APPID must contain decimal digits" >&2; exit 2 ;;
esac
case "$window_id" in
    auto) ;;
    ''|*[!0-9]*) echo "WINDOW_ID must be 'auto' or contain decimal digits" >&2; exit 2 ;;
    *) [ "$window_id" -gt 0 ] || { echo "WINDOW_ID must be greater than zero" >&2; exit 2; } ;;
esac
case "$game_pid" in
    auto) ;;
    ''|*[!0-9]*) echo "PID must be 'auto' or contain decimal digits" >&2; exit 2 ;;
    *) [ "$game_pid" -gt 0 ] || { echo "PID must be greater than zero" >&2; exit 2; } ;;
esac
case "$evidence_dir" in
    /*) ;;
    *) echo "EVIDENCE_DIR must be an absolute path" >&2; exit 2 ;;
esac
case "$1:$#" in
    key:2) ;;
    click:3) ;;
    *) usage ;;
esac

[ -f "$profile" ] || { echo "profile is absent: $profile" >&2; exit 1; }
[ -d "$data_root" ] || { echo "data root is absent: $data_root" >&2; exit 1; }
[ -d "$runtime_root" ] || { echo "runtime root is absent: $runtime_root" >&2; exit 1; }
[ ! -e "$evidence_dir" ] || {
    echo "refusing to replace an existing evidence directory: $evidence_dir" >&2
    exit 1
}

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(dirname -- "$script_dir")
macroton_bin=${MACROTON_BIN:-$repo_root/target/debug/macroton}
swiftc_bin=${SWIFTC_BIN:-swiftc}
[ -x "$macroton_bin" ] || {
    echo "Macroton binary is absent or not executable: $macroton_bin" >&2
    echo "run cargo build first or set MACROTON_BIN" >&2
    exit 1
}
command -v "$swiftc_bin" >/dev/null 2>&1 || {
    echo "Swift compiler is unavailable: $swiftc_bin" >&2
    exit 1
}

focus_frames=${MACROTON_FOCUS_FRAMES:-600}
focus_fps=${MACROTON_FOCUS_FPS:-60}
audio_seconds=${MACROTON_AUDIO_SECONDS:-10}
silence_dbfs=${MACROTON_SILENCE_DBFS:--50}
interaction_delay_ms=${MACROTON_INTERACTION_DELAY_MS:-500}
min_runtime_seconds=${MACROTON_MIN_RUNTIME_SECONDS:-1800}
timeout_seconds=${MACROTON_TIMEOUT_SECONDS:-14400}

resume_status=$(
    "$macroton_bin" evidence bundle-status "$app_id" \
        --profile "$profile" --evidence-dir "$evidence_dir" \
        --data-root "$data_root" --runtime-root "$runtime_root"
)
echo "$resume_status" | grep -q '"runtime_state": "ready"' || {
    echo "runtime evidence capture requires ready profile/prefix/runtime provenance" >&2
    echo "$resume_status" >&2
    exit 1
}

session_status=$("$macroton_bin" steam session-status --json)
echo "$session_status" | grep -q '"console_state": "unlocked"' || {
    echo "runtime evidence capture requires an unlocked macOS console session" >&2
    exit 1
}

launch_status=$(
    "$macroton_bin" steam launch-status "$app_id" --data-root "$data_root" --json
)
echo "$launch_status" | grep -q '"state": "running-observed"' || {
    echo "AppID $app_id is not in the running-observed Steam lifecycle" >&2
    exit 1
}
if [ "$game_pid" = auto ]; then
    game_pid=$(echo "$launch_status" | sed -n 's/.*"process_id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')
    case "$game_pid" in
        ''|*[!0-9]*)
            echo "Steam running-observed lifecycle did not contain one parseable game PID" >&2
            exit 1
            ;;
    esac
    echo "Resolved Steam-tracked game PID: $game_pid"
fi
echo "$launch_status" | grep -Eq "\"process_id\":[[:space:]]*$game_pid([,}[:space:]]|$)" || {
    echo "requested PID $game_pid does not match Steam's tracked AppID $app_id process" >&2
    exit 1
}

temporary_root=$(mktemp -d /private/tmp/macroton-live-evidence.XXXXXX)
watcher_pid=
cleanup_required=0
stop_and_verify() {
    stop_ok=1
    if ! "$macroton_bin" stop "$app_id" \
        --runtime-root "$runtime_root" --data-root "$data_root"; then
        echo "failed to stop exact AppID $app_id prefix" >&2
        stop_ok=0
    fi
    if ! resource_status=$(
        "$macroton_bin" resource status "$app_id" --data-root "$data_root" --json
    ); then
        echo "failed to inspect exact AppID $app_id prefix after stop" >&2
        return 1
    fi
    echo "$resource_status" | grep -q '"process_count": 0' || {
        echo "exact AppID prefix still has residual processes after stop" >&2
        return 1
    }
    echo "$resource_status" | grep -q '"total_rss_kib": 0' || {
        echo "exact AppID prefix still has residual RSS after stop" >&2
        return 1
    }
    [ "$stop_ok" -eq 1 ]
}
cleanup() {
    original_status=$?
    trap - EXIT
    if [ -n "$watcher_pid" ] && kill -0 "$watcher_pid" >/dev/null 2>&1; then
        kill "$watcher_pid" >/dev/null 2>&1 || true
        wait "$watcher_pid" >/dev/null 2>&1 || true
    fi
    if [ "$cleanup_required" -eq 1 ]; then
        if ! stop_and_verify; then
            echo "runtime evidence cleanup could not prove zero exact-prefix resources" >&2
            original_status=96
        fi
    fi
    rm -rf "$temporary_root"
    exit "$original_status"
}
trap cleanup EXIT
trap 'exit 130' HUP INT TERM

# From this point onward, any setup or probe failure owns cleanup of the exact
# already-running AppID prefix. Preflight refusals above remain read-only.
cleanup_required=1

focus_tool=$temporary_root/macos-focus-stream
audio_tool=$temporary_root/macos-audio-probe
interaction_tool=$temporary_root/macos-interaction-probe
window_tool=$temporary_root/macos-window-locator
"$swiftc_bin" -warnings-as-errors "$repo_root/tools/macos_window_locator.swift" \
    -o "$window_tool"
"$swiftc_bin" -warnings-as-errors "$repo_root/tools/macos_focus_stream.swift" \
    -o "$focus_tool"
"$swiftc_bin" -warnings-as-errors "$repo_root/tools/macos_audio_probe.swift" \
    -o "$audio_tool"
"$swiftc_bin" -warnings-as-errors "$repo_root/tools/macos_interaction_probe.swift" \
    -o "$interaction_tool"

if [ "$window_id" = auto ]; then
    window_id=$("$window_tool" "$game_pid" --activate)
    echo "Resolved unique game window: WindowServer ID $window_id for PID $game_pid"
fi

mkdir -p "$evidence_dir/raw"

"$macroton_bin" evidence verify-independence "$app_id" \
    --profile "$profile" --data-root "$data_root" --runtime-root "$runtime_root" \
    --output "$evidence_dir/independence.json"

"$macroton_bin" evidence watch-exit "$app_id" \
    --profile "$profile" --data-root "$data_root" --runtime-root "$runtime_root" \
    --min-runtime-seconds "$min_runtime_seconds" --timeout-seconds "$timeout_seconds" \
    --output "$evidence_dir/exit.json" &
watcher_pid=$!

confirm() {
    message=$1
    if [ "${MACROTON_AUTO_CONFIRM:-0}" = 1 ]; then
        echo "$message"
        return 0
    fi
    printf '%s Press Return when ready: ' "$message"
    IFS= read -r _answer
}

confirm "Place the exact game window in a continuously moving gameplay scene."
"$focus_tool" "$window_id" "$game_pid" "$focus_frames" "$focus_fps" \
    > "$evidence_dir/raw/focus.csv"
"$macroton_bin" evidence verify-focus-stream "$app_id" \
    --profile "$profile" --data-root "$data_root" --runtime-root "$runtime_root" \
    --probe "$evidence_dir/raw/focus.csv" --min-frames "$focus_frames" \
    --min-duration-seconds 9 --max-gap-ms 100 --black-luma 1 \
    --max-black-frames 0 --output "$evidence_dir/focus.json"
"$macroton_bin" evidence verify-motion-rate "$app_id" \
    --profile "$profile" --data-root "$data_root" --runtime-root "$runtime_root" \
    --probe "$evidence_dir/raw/focus.csv" --min-duration-seconds 9 \
    --min-changed-rate 20 --max-unchanged-ms 200 \
    --output "$evidence_dir/motion.json"

confirm "Move to a scene with known continuous game audio and trigger that audio."
"$audio_tool" "$window_id" "$game_pid" "$audio_seconds" "$silence_dbfs" \
    > "$evidence_dir/raw/audio.json"
"$macroton_bin" evidence verify-audio "$app_id" \
    --profile "$profile" --data-root "$data_root" --runtime-root "$runtime_root" \
    --probe "$evidence_dir/raw/audio.json" --output "$evidence_dir/audio.json"

confirm "Move to the reviewed static menu where the configured action causes a visible change."
"$interaction_tool" "$window_id" "$game_pid" \
    "$evidence_dir/raw/interaction-capture" "$interaction_delay_ms" \
    --write-json "$@" > /dev/null
"$macroton_bin" evidence verify-interaction "$app_id" \
    --profile "$profile" --data-root "$data_root" --runtime-root "$runtime_root" \
    --probe "$evidence_dir/raw/interaction-capture/interaction.json" \
    --capture-dir "$evidence_dir/raw/interaction-capture" \
    --min-changed-ratio 0.01 --output "$evidence_dir/interaction.json"

confirm "Continue playing until the stability interval has elapsed, then quit using the game's own UI."
if ! wait "$watcher_pid"; then
    watcher_pid=
    echo "normal-exit watcher failed; the exact AppID prefix will be stopped" >&2
    exit 1
fi
watcher_pid=

cleanup_required=0
stop_and_verify || {
    echo "normal evidence completion could not prove zero exact-prefix resources" >&2
    exit 1
}

"$macroton_bin" evidence bundle-status "$app_id" \
    --profile "$profile" --evidence-dir "$evidence_dir" \
    --data-root "$data_root" --runtime-root "$runtime_root" \
    > "$evidence_dir/bundle-status.json"

echo "Captured one-session automated runtime evidence: $evidence_dir"
echo "Inspect bundle-status.json for the profile-specific Cloud action. Profiles with mapped Cloud files require a fresh roundtrip; source-bound unsupported profiles record it as not applicable. Human visual, listening, semantic-input, save-reload and stability gates always remain."
