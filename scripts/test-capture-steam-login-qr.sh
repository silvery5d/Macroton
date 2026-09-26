#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
test_root=$(mktemp -d /private/tmp/macroton-login-qr-test.XXXXXX)
trap 'rm -rf "$test_root"' EXIT HUP INT TERM
mkdir -p "$test_root/data/games/400/prefix"

printf '%s\n' '#!/bin/sh' 'printf MZ >"$1"' >"$test_root/builder"
printf '%s\n' '#!/bin/sh' 'printf "%s\n" /devtools/page/login' >"$test_root/selector"
printf '%s\n' \
    '#!/bin/sh' \
    'state=${RUNTIME_STATE:?}' \
    'count=0' \
    '[ ! -f "$state" ] || count=$(cat "$state")' \
    'count=$((count + 1))' \
    'printf "%s\\n" "$count" >"$state"' \
    'if [ "${3:-}" = eval ]; then printf "%s\n" '\''{"id":1,"result":{"result":{"type":"string","value":"{\"ready\":true}"}}}'\''; else printf "%s\n" '\''{"id":1,"result":{"data":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII="}}'\''; fi' \
    >"$test_root/runtime"
chmod +x "$test_root/builder" "$test_root/selector" "$test_root/runtime"

qr_path=$(
    CDP_PROBE_BUILDER="$test_root/builder" \
    CDP_TARGET_SELECTOR="$test_root/selector" \
    RUNTIME_STATE="$test_root/runtime-state" \
        "$repo_root/scripts/capture-steam-login-qr.sh" \
        400 "$test_root/runtime" "$test_root/data" 2>"$test_root/stderr"
)
case "$qr_path" in
    /private/tmp/macroton-steam-login-qr.*) ;;
    *)
        echo "capture returned a non-temporary QR path: $qr_path" >&2
        exit 1
        ;;
esac
[ "$(stat -f %Lp "$qr_path")" = "600" ]
file "$qr_path" | grep -q 'PNG image data'
[ "$(cat "$test_root/runtime-state")" -ge 2 ]
grep -q 'temporary owner-authentication image' "$test_root/stderr"
rm -f "$qr_path"

# A completed 41x41 loading spinner must not be accepted as a QR image. The
# production readiness expression requires a real QR-sized surface; this
# fixture keeps that regression visible even though the probe wrapper itself
# is mocked here.
printf '%s\n' \
    '#!/bin/sh' \
    'if [ "${3:-}" = eval ]; then printf "%s\n" '\''{"id":1,"result":{"result":{"type":"string","value":"{\\"ready\\":false}"}}}'\''; else printf "%s\n" '\''{"id":1,"result":{"data":"iVBORw0KGgoAAAANSUhEUgAAACkAAAApCAQAAAB5mYQ5AAAADElEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII="}}'\''; fi' \
    >"$test_root/spinner-runtime"
chmod +x "$test_root/spinner-runtime"
before=$(find /private/tmp -maxdepth 1 -name 'macroton-steam-login-qr.*' -print | sort)
if CDP_PROBE_BUILDER="$test_root/builder" \
    CDP_TARGET_SELECTOR="$test_root/selector" \
    "$repo_root/scripts/capture-steam-login-qr.sh" \
    400 "$test_root/spinner-runtime" "$test_root/data" 0 >"$test_root/spinner-out" 2>"$test_root/spinner-err"; then
    echo "incomplete QR spinner was accepted" >&2
    exit 1
fi
after=$(find /private/tmp -maxdepth 1 -name 'macroton-steam-login-qr.*' -print | sort)
[ "$before" = "$after" ]

printf '%s\n' \
    '#!/bin/sh' \
    'state=${WAIT_SELECTOR_STATE:?}' \
    'count=0' \
    '[ ! -f "$state" ] || count=$(cat "$state")' \
    'count=$((count + 1))' \
    'printf "%s\\n" "$count" >"$state"' \
    '[ "$count" -ge 2 ] || { echo "expected exactly one local Steam login CEF target, found 0" >&2; exit 1; }' \
    'printf "%s\\n" /devtools/page/login' \
    >"$test_root/wait-selector"
chmod +x "$test_root/wait-selector"
qr_path=$(
    WAIT_SELECTOR_STATE="$test_root/wait-selector-state" \
    RUNTIME_STATE="$test_root/wait-runtime-state" \
    CDP_PROBE_BUILDER="$test_root/builder" \
    CDP_TARGET_SELECTOR="$test_root/wait-selector" \
        "$repo_root/scripts/capture-steam-login-qr.sh" \
        400 "$test_root/runtime" "$test_root/data" 4 2>"$test_root/wait-stderr"
)
[ -s "$qr_path" ]
[ "$(cat "$test_root/wait-selector-state")" -eq 2 ]
rm -f "$qr_path"

printf '%s\n' '#!/bin/sh' \
    'echo "curl: (7) Failed to connect to 127.0.0.1 port 8080: Connection refused" >&2' \
    'exit 1' >"$test_root/unavailable-selector"
chmod +x "$test_root/unavailable-selector"
if CDP_PROBE_BUILDER="$test_root/builder" \
    CDP_TARGET_SELECTOR="$test_root/unavailable-selector" \
    "$repo_root/scripts/capture-steam-login-qr.sh" \
    400 "$test_root/runtime" "$test_root/data" 0 >"$test_root/unavailable-out" 2>"$test_root/unavailable-err"; then
    echo "unavailable CEF endpoint was accepted" >&2
    exit 1
fi
grep -q 'client may still be updating' "$test_root/unavailable-err"

printf '%s\n' '#!/bin/sh' 'printf "%s\n" '\''{"id":1,"result":{"data":"not-base64"}}'\''' \
    >"$test_root/invalid-runtime"
chmod +x "$test_root/invalid-runtime"
before=$(find /private/tmp -maxdepth 1 -name 'macroton-steam-login-qr.*' -print | sort)
if CDP_PROBE_BUILDER="$test_root/builder" \
    CDP_TARGET_SELECTOR="$test_root/selector" \
    "$repo_root/scripts/capture-steam-login-qr.sh" \
    400 "$test_root/invalid-runtime" "$test_root/data" >/dev/null 2>&1; then
    echo "invalid CDP screenshot was accepted" >&2
    exit 1
fi
after=$(find /private/tmp -maxdepth 1 -name 'macroton-steam-login-qr.*' -print | sort)
[ "$before" = "$after" ]

echo "Steam login QR capture success and cleanup tests passed"
