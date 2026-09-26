#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
capture_script=$script_dir/capture-moltenvk-first-frame.sh
test_root=$(mktemp -d /private/tmp/macroton-capture-script-test.XXXXXX)
cleanup() {
    rm -rf "$test_root"
}
trap cleanup EXIT HUP INT TERM

profile=$test_root/profile.conf
steam_root=$test_root/steam
runtime_root=$test_root/runtimes
data_root=$test_root/data
fake_macroton=$test_root/macroton
mkdir -p "$steam_root" "$runtime_root" "$data_root"
printf 'schema=1\n' > "$profile"

cat > "$fake_macroton" <<'FAKE'
#!/bin/sh
set -eu
case "$1" in
    resource)
        printf '{"process_count": %s}\n' "${MACROTON_FAKE_PROCESS_COUNT:-0}"
        ;;
    launch)
        [ "${METAL_CAPTURE_ENABLED:-}" = 1 ]
        [ "${MVK_CONFIG_AUTO_GPU_CAPTURE_SCOPE:-}" = 2 ]
        printf 'launch\n' >> "$MACROTON_FAKE_LOG"
        [ "${MACROTON_FAKE_LAUNCH_FAIL:-0}" = 0 ] || exit 7
        if [ "${MACROTON_FAKE_CAPTURE:-1}" = 1 ]; then
            mkdir -p "$MVK_CONFIG_AUTO_GPU_CAPTURE_OUTPUT_FILE"
            printf 'frame' > "$MVK_CONFIG_AUTO_GPU_CAPTURE_OUTPUT_FILE/frame.bin"
        fi
        ;;
    evidence)
        printf 'evidence\n' >> "$MACROTON_FAKE_LOG"
        [ "${MACROTON_FAKE_EVIDENCE_FAIL:-0}" = 0 ] || exit 6
        while [ "$#" -gt 0 ]; do
            if [ "$1" = --output ]; then
                shift
                printf '{"status":"gpu-command-capture-pass"}\n' > "$1"
                exit 0
            fi
            shift
        done
        exit 8
        ;;
    stop)
        printf 'stop:%s\n' "$2" >> "$MACROTON_FAKE_LOG"
        ;;
    *) exit 9 ;;
esac
FAKE
chmod +x "$fake_macroton"

success_trace=$test_root/success.gputrace
success_evidence=$test_root/success.json
success_log=$test_root/success.log
MACROTON_BIN=$fake_macroton MACROTON_FAKE_LOG=$success_log \
    "$capture_script" 221380 "$profile" "$steam_root" "$runtime_root" \
        "$data_root" "$success_trace" "$success_evidence" 2 >/dev/null
[ "$(cat "$success_trace/frame.bin")" = frame ]
[ "$(cat "$success_evidence")" = '{"status":"gpu-command-capture-pass"}' ]
[ "$(cat "$success_log")" = "launch
evidence
stop:221380" ]

timeout_trace=$test_root/timeout.gputrace
timeout_evidence=$test_root/timeout.json
timeout_log=$test_root/timeout.log
if MACROTON_BIN=$fake_macroton MACROTON_FAKE_LOG=$timeout_log \
    MACROTON_FAKE_CAPTURE=0 "$capture_script" 221380 "$profile" \
        "$steam_root" "$runtime_root" "$data_root" "$timeout_trace" "$timeout_evidence" 1 \
        >/dev/null 2>&1; then
    echo "timeout case unexpectedly succeeded" >&2
    exit 1
fi
[ ! -e "$timeout_trace" ]
[ ! -e "$timeout_evidence" ]
[ "$(cat "$timeout_log")" = "launch
stop:221380" ]

existing_trace=$test_root/existing.gputrace
existing_evidence=$test_root/existing.json
existing_log=$test_root/existing.log
mkdir "$existing_trace"
printf 'old' > "$existing_trace/frame.bin"
if MACROTON_BIN=$fake_macroton MACROTON_FAKE_LOG=$existing_log \
    "$capture_script" 221380 "$profile" "$steam_root" "$runtime_root" \
        "$data_root" "$existing_trace" "$existing_evidence" 2 >/dev/null 2>&1; then
    echo "existing-output case unexpectedly succeeded" >&2
    exit 1
fi
[ "$(cat "$existing_trace/frame.bin")" = old ]
[ ! -e "$existing_log" ]

busy_trace=$test_root/busy.gputrace
busy_evidence=$test_root/busy.json
busy_log=$test_root/busy.log
if MACROTON_BIN=$fake_macroton MACROTON_FAKE_LOG=$busy_log \
    MACROTON_FAKE_PROCESS_COUNT=2 "$capture_script" 221380 "$profile" \
        "$steam_root" "$runtime_root" "$data_root" "$busy_trace" \
        "$busy_evidence" 2 >/dev/null 2>&1; then
    echo "busy-prefix case unexpectedly succeeded" >&2
    exit 1
fi
[ ! -e "$busy_trace" ]
[ ! -e "$busy_evidence" ]
[ ! -e "$busy_log" ]

failed_trace=$test_root/failed.gputrace
failed_evidence=$test_root/failed.json
failed_log=$test_root/failed.log
if MACROTON_BIN=$fake_macroton MACROTON_FAKE_LOG=$failed_log \
    MACROTON_FAKE_LAUNCH_FAIL=1 "$capture_script" 221380 "$profile" \
        "$steam_root" "$runtime_root" "$data_root" "$failed_trace" \
        "$failed_evidence" 2 >/dev/null 2>&1; then
    echo "failed-launch case unexpectedly succeeded" >&2
    exit 1
fi
[ ! -e "$failed_trace" ]
[ ! -e "$failed_evidence" ]
[ "$(cat "$failed_log")" = "launch
stop:221380" ]

unbound_trace=$test_root/unbound.gputrace
unbound_evidence=$test_root/unbound.json
unbound_log=$test_root/unbound.log
if MACROTON_BIN=$fake_macroton MACROTON_FAKE_LOG=$unbound_log \
    MACROTON_FAKE_EVIDENCE_FAIL=1 "$capture_script" 221380 "$profile" \
        "$steam_root" "$runtime_root" "$data_root" "$unbound_trace" \
        "$unbound_evidence" 1 >/dev/null 2>&1; then
    echo "unbound-capture case unexpectedly succeeded" >&2
    exit 1
fi
[ "$(cat "$unbound_trace/frame.bin")" = frame ]
[ ! -e "$unbound_evidence" ]
grep -q '^evidence$' "$unbound_log"
[ "$(tail -n 1 "$unbound_log")" = "stop:221380" ]

echo "MoltenVK capture wrapper tests passed"
