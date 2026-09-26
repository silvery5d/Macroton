#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
capture_script=$script_dir/capture-runtime-evidence-session.sh
test_root=$(mktemp -d /private/tmp/macroton-live-evidence-script-test.XXXXXX)
cleanup() {
    rm -rf "$test_root"
}
trap cleanup EXIT HUP INT TERM

profile=$test_root/profile.conf
data_root=$test_root/data
runtime_root=$test_root/runtimes
fake_macroton=$test_root/macroton
fake_swiftc=$test_root/swiftc
mkdir -p "$data_root" "$runtime_root"
printf 'schema=1\n' > "$profile"

cat > "$fake_macroton" <<'FAKE'
#!/bin/sh
set -eu
command=$1
shift
case "$command" in
    steam)
        case "$1" in
            session-status)
                printf '{"console_state": "%s"}\n' "${MACROTON_FAKE_SESSION:-unlocked}"
                ;;
            launch-status)
                printf '{"state": "%s", "process_id": %s}\n' \
                    "${MACROTON_FAKE_LAUNCH_STATE:-running-observed}" \
                    "${MACROTON_FAKE_PID:-4242}"
                ;;
            *) exit 91 ;;
        esac
        ;;
    evidence)
        subcommand=$1
        shift
        if [ "$subcommand" = bundle-status ]; then
            printf '{"runtime_state": "%s", "automated_bundle_ready": false}\n' \
                "${MACROTON_FAKE_RUNTIME_STATE:-ready}"
            exit 0
        fi
        printf '%s\n' "$subcommand" >> "$MACROTON_FAKE_LOG"
        [ "${MACROTON_FAKE_FAIL:-}" != "$subcommand" ] || exit 73
        output=
        while [ "$#" -gt 0 ]; do
            if [ "$1" = --output ]; then
                shift
                output=$1
                break
            fi
            shift
        done
        [ -n "$output" ] || exit 92
        printf '{"subcommand":"%s"}\n' "$subcommand" > "$output"
        ;;
    stop)
        printf 'stop:%s\n' "$1" >> "$MACROTON_FAKE_LOG"
        [ "${MACROTON_FAKE_STOP_FAIL:-0}" = 0 ] || exit 74
        ;;
    resource)
        printf '{"process_count": %s, "total_rss_kib": %s}\n' \
            "${MACROTON_FAKE_RESOURCE_PROCESS_COUNT:-0}" \
            "${MACROTON_FAKE_RESOURCE_RSS_KIB:-0}"
        ;;
    *) exit 90 ;;
esac
FAKE
chmod +x "$fake_macroton"

cat > "$fake_swiftc" <<'FAKE'
#!/bin/sh
set -eu
[ "${MACROTON_FAKE_SWIFTC_FAIL:-0}" = 0 ] || exit 79
source_file=
output=
while [ "$#" -gt 0 ]; do
    case "$1" in
        *.swift) source_file=$1 ;;
        -o) shift; output=$1 ;;
    esac
    shift
done
[ -n "$source_file" ] && [ -n "$output" ] || exit 80
case "$source_file" in
    *macos_window_locator.swift)
        body='[ "$1" = 4242 ]; [ "$2" = --activate ]; printf "77\\n"'
        ;;
    *macos_focus_stream.swift)
        body='printf "window_id,pid,frame,elapsed_ms,focused,luma,sha256\\n77,4242,1,0,true,10,aa\\n"'
        ;;
    *macos_audio_probe.swift)
        body='printf "{\\"pid\\":4242}\\n"'
        ;;
    *macos_interaction_probe.swift)
        body='mkdir -p "$3"; printf before > "$3/before.png"; printf after > "$3/after.png"; [ "$5" = --write-json ]; printf "{\\"pid\\":4242}\\n" > "$3/interaction.json"'
        ;;
    *) exit 81 ;;
esac
printf '#!/bin/sh\nset -eu\n%s\n' "$body" > "$output"
chmod +x "$output"
FAKE
chmod +x "$fake_swiftc"

success_dir=$test_root/success
success_log=$test_root/success.log
MACROTON_BIN=$fake_macroton SWIFTC_BIN=$fake_swiftc \
MACROTON_FAKE_LOG=$success_log MACROTON_AUTO_CONFIRM=1 \
MACROTON_FOCUS_FRAMES=1 MACROTON_MIN_RUNTIME_SECONDS=1 \
MACROTON_TIMEOUT_SECONDS=2 \
    "$capture_script" 606680 "$profile" auto auto "$success_dir" \
        "$data_root" "$runtime_root" key escape >/dev/null
[ -f "$success_dir/focus.json" ]
[ -f "$success_dir/motion.json" ]
[ -f "$success_dir/interaction.json" ]
[ -f "$success_dir/raw/interaction-capture/interaction.json" ]
[ -f "$success_dir/audio.json" ]
[ -f "$success_dir/independence.json" ]
[ -f "$success_dir/exit.json" ]
[ "$(cat "$success_dir/bundle-status.json")" = '{"runtime_state": "ready", "automated_bundle_ready": false}' ]
[ "$(cat "$success_log" | tail -n 1)" = 'stop:606680' ]
[ "$(grep -c '^stop:' "$success_log")" -eq 1 ]

residual_dir=$test_root/residual
residual_log=$test_root/residual.log
if MACROTON_BIN=$fake_macroton SWIFTC_BIN=$fake_swiftc \
    MACROTON_FAKE_LOG=$residual_log MACROTON_AUTO_CONFIRM=1 \
    MACROTON_FAKE_RESOURCE_PROCESS_COUNT=1 MACROTON_FAKE_RESOURCE_RSS_KIB=1024 \
    MACROTON_FOCUS_FRAMES=1 MACROTON_MIN_RUNTIME_SECONDS=1 \
    MACROTON_TIMEOUT_SECONDS=2 \
    "$capture_script" 606680 "$profile" auto auto "$residual_dir" \
        "$data_root" "$runtime_root" key escape >/dev/null 2>&1; then
    echo "residual-resource case unexpectedly succeeded" >&2
    exit 1
fi
[ "$(grep -c '^stop:' "$residual_log")" -eq 1 ]

stop_failure_dir=$test_root/stop-failure
stop_failure_log=$test_root/stop-failure.log
if MACROTON_BIN=$fake_macroton SWIFTC_BIN=$fake_swiftc \
    MACROTON_FAKE_LOG=$stop_failure_log MACROTON_AUTO_CONFIRM=1 \
    MACROTON_FAKE_STOP_FAIL=1 MACROTON_FOCUS_FRAMES=1 \
    MACROTON_MIN_RUNTIME_SECONDS=1 MACROTON_TIMEOUT_SECONDS=2 \
    "$capture_script" 606680 "$profile" auto auto "$stop_failure_dir" \
        "$data_root" "$runtime_root" key escape >/dev/null 2>&1; then
    echo "failed-stop case unexpectedly succeeded" >&2
    exit 1
fi
[ "$(grep -c '^stop:' "$stop_failure_log")" -eq 1 ]

locked_dir=$test_root/locked
locked_log=$test_root/locked.log
if MACROTON_BIN=$fake_macroton SWIFTC_BIN=$fake_swiftc \
    MACROTON_FAKE_LOG=$locked_log MACROTON_FAKE_SESSION=locked \
    MACROTON_AUTO_CONFIRM=1 "$capture_script" 606680 "$profile" 77 4242 \
        "$locked_dir" "$data_root" "$runtime_root" key escape \
        >/dev/null 2>&1; then
    echo "locked-session case unexpectedly succeeded" >&2
    exit 1
fi
[ ! -e "$locked_dir" ]
[ ! -e "$locked_log" ]

runtime_mismatch_dir=$test_root/runtime-mismatch
runtime_mismatch_log=$test_root/runtime-mismatch.log
if MACROTON_BIN=$fake_macroton SWIFTC_BIN=$fake_swiftc \
    MACROTON_FAKE_LOG=$runtime_mismatch_log \
    MACROTON_FAKE_RUNTIME_STATE=prefix-runtime-provenance-mismatch \
    MACROTON_AUTO_CONFIRM=1 "$capture_script" 606680 "$profile" auto auto \
        "$runtime_mismatch_dir" "$data_root" "$runtime_root" key escape \
        >/dev/null 2>&1; then
    echo "runtime-provenance mismatch case unexpectedly succeeded" >&2
    exit 1
fi
[ ! -e "$runtime_mismatch_dir" ]
[ ! -e "$runtime_mismatch_log" ]

mismatch_dir=$test_root/mismatch
mismatch_log=$test_root/mismatch.log
if MACROTON_BIN=$fake_macroton SWIFTC_BIN=$fake_swiftc \
    MACROTON_FAKE_LOG=$mismatch_log MACROTON_FAKE_PID=9999 \
    MACROTON_AUTO_CONFIRM=1 "$capture_script" 606680 "$profile" 77 4242 \
        "$mismatch_dir" "$data_root" "$runtime_root" key escape \
        >/dev/null 2>&1; then
    echo "PID-mismatch case unexpectedly succeeded" >&2
    exit 1
fi
[ ! -e "$mismatch_dir" ]
[ ! -e "$mismatch_log" ]

missing_pid_dir=$test_root/missing-pid
missing_pid_log=$test_root/missing-pid.log
if MACROTON_BIN=$fake_macroton SWIFTC_BIN=$fake_swiftc \
    MACROTON_FAKE_LOG=$missing_pid_log MACROTON_FAKE_PID=null \
    MACROTON_AUTO_CONFIRM=1 "$capture_script" 606680 "$profile" auto auto \
        "$missing_pid_dir" "$data_root" "$runtime_root" key escape \
        >/dev/null 2>&1; then
    echo "missing-auto-PID case unexpectedly succeeded" >&2
    exit 1
fi
[ ! -e "$missing_pid_dir" ]
[ ! -e "$missing_pid_log" ]

compile_dir=$test_root/compile-failure
compile_log=$test_root/compile-failure.log
if MACROTON_BIN=$fake_macroton SWIFTC_BIN=$fake_swiftc \
    MACROTON_FAKE_LOG=$compile_log MACROTON_AUTO_CONFIRM=1 \
    MACROTON_FAKE_SWIFTC_FAIL=1 "$capture_script" 606680 "$profile" 77 4242 \
        "$compile_dir" "$data_root" "$runtime_root" key escape \
        >/dev/null 2>&1; then
    echo "probe-compile-failure case unexpectedly succeeded" >&2
    exit 1
fi
[ ! -e "$compile_dir" ]
[ "$(tail -n 1 "$compile_log")" = 'stop:606680' ]

failed_dir=$test_root/failed
failed_log=$test_root/failed.log
if MACROTON_BIN=$fake_macroton SWIFTC_BIN=$fake_swiftc \
    MACROTON_FAKE_LOG=$failed_log MACROTON_FAKE_FAIL=verify-audio \
    MACROTON_AUTO_CONFIRM=1 MACROTON_FOCUS_FRAMES=1 \
    MACROTON_MIN_RUNTIME_SECONDS=1 MACROTON_TIMEOUT_SECONDS=2 \
    "$capture_script" 606680 "$profile" 77 4242 "$failed_dir" \
        "$data_root" "$runtime_root" click 0.5 0.5 >/dev/null 2>&1; then
    echo "probe-failure case unexpectedly succeeded" >&2
    exit 1
fi
[ -d "$failed_dir" ]
[ "$(cat "$failed_log" | tail -n 1)" = 'stop:606680' ]

existing_dir=$test_root/existing
mkdir "$existing_dir"
if MACROTON_BIN=$fake_macroton SWIFTC_BIN=$fake_swiftc \
    MACROTON_FAKE_LOG=$test_root/existing.log MACROTON_AUTO_CONFIRM=1 \
    "$capture_script" 606680 "$profile" 77 4242 "$existing_dir" \
        "$data_root" "$runtime_root" key escape >/dev/null 2>&1; then
    echo "existing-directory case unexpectedly succeeded" >&2
    exit 1
fi
[ ! -e "$test_root/existing.log" ]

echo "runtime evidence session wrapper tests passed"
