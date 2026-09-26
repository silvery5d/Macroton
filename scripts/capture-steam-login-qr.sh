#!/bin/sh
set -eu

[ "$#" -ge 2 ] && [ "$#" -le 4 ] || {
    echo "usage: $0 APPID RUNTIME_ENTRYPOINT [DATA_ROOT] [WAIT_SECONDS]" >&2
    exit 2
}

app_id=$1
runtime=$2
data_root=${3:-"$HOME/Library/Application Support/Macroton"}
wait_seconds=${4:-0}
case "$app_id" in
    '' | *[!0-9]*)
        echo "APPID must be an unsigned integer" >&2
        exit 2
        ;;
esac
case "$wait_seconds" in
    '' | *[!0-9]*)
        echo "WAIT_SECONDS must be a non-negative integer" >&2
        exit 2
        ;;
esac
[ -x "$runtime" ] || {
    echo "runtime entrypoint is not executable: $runtime" >&2
    exit 1
}

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
probe_builder=${CDP_PROBE_BUILDER:-"$repo_root/scripts/build-cdp-probe.sh"}
target_selector=${CDP_TARGET_SELECTOR:-"$repo_root/scripts/find-steam-login-cdp-target.sh"}
prefix=$data_root/games/$app_id/prefix
[ -d "$prefix" ] || {
    echo "Macroton prefix does not exist: $prefix" >&2
    exit 1
}

probe=$(mktemp /private/tmp/macroton-cdp-probe.XXXXXX)
qr_path=$(mktemp /private/tmp/macroton-steam-login-qr.XXXXXX)
frame_path=$(mktemp /private/tmp/macroton-steam-login-frame.XXXXXX)
target_error=$(mktemp /private/tmp/macroton-steam-login-target-error.XXXXXX)
success=0
cleanup() {
    rm -f "$probe" "$frame_path" "$target_error"
    if [ "$success" -ne 1 ]; then
        rm -f "$qr_path"
    fi
}
trap cleanup EXIT HUP INT TERM

"$probe_builder" "$probe" >/dev/null
started=$(date +%s)
while ! target_path=$("$target_selector" 2>"$target_error"); do
    now=$(date +%s)
    elapsed=$((now - started))
    if [ "$elapsed" -ge "$wait_seconds" ]; then
        cat "$target_error" >&2
        if grep -q 'Failed to connect\|Could not connect\|Connection refused' "$target_error"; then
            echo "Steam CEF debugging endpoint is unavailable; the client may still be updating or has not opened the login UI" >&2
        elif grep -q 'expected exactly one local Steam login CEF target, found 0' "$target_error"; then
            echo "Steam CEF is reachable, but no unique sign-in page exists yet" >&2
        fi
        exit 1
    fi
    sleep 2
done

# The target is created before its QR image has finished painting. Steam's QR
# is currently delivered as a 41x41 blob image that CSS scales to roughly
# 160x160; the loading spinner is also complete but remains physically small.
# Require both a real source image and a QR-sized rendered surface. Steam
# rotates the QR periodically, so whole-frame hash equality is not reliable.
qr_ready_expression='JSON.stringify({ready:[...document.images].some(i=>i.complete && i.naturalWidth >= 32 && i.naturalHeight >= 32 && i.getBoundingClientRect().width >= 128 && i.getBoundingClientRect().height >= 128)})'

# Wine's CEF endpoint can briefly return HTTP 500 while the login page swaps
# its QR document. Treat that as a transient transport failure instead of
# losing the whole capture attempt.
run_cdp() {
    cdp_attempt=1
    while [ "$cdp_attempt" -le 4 ]; do
        if cdp_output=$(
            WINEPREFIX="$prefix" WINEDEBUG=-all \
                "$runtime" "$probe" "$target_path" "$@"
        ); then
            printf '%s\n' "$cdp_output"
            return 0
        fi
        cdp_attempt=$((cdp_attempt + 1))
        sleep 1
    done
    return 1
}

qr_ready=0
attempt=0
while [ "$attempt" -lt 20 ]; do
    attempt=$((attempt + 1))
    if ! ready_response=$(run_cdp eval "$qr_ready_expression"); then
        sleep 1
        continue
    fi
    if printf '%s\n' "$ready_response" | jq -ser '
        [.[] | select(.id == 1 and (.result.result.value | type == "string"))
          | .result.result.value | fromjson | select(.ready == true)]
        | length == 1
    ' >/dev/null 2>&1; then
        qr_ready=1
        break
    fi
    sleep 1
done

[ "$qr_ready" -eq 1 ] || {
    echo "Steam CEF login QR image did not finish loading" >&2
    exit 1
}

response=$(run_cdp screenshot) || {
    echo "Steam CEF login QR screenshot failed after transient-error retries" >&2
    exit 1
}
image_base64=$(
    printf '%s\n' "$response" | jq -ser '
      [.[] | select(.id == 1 and (.result.data | type == "string"))]
      | if length == 1 then .[0].result.data
        else error("expected exactly one CDP screenshot response") end
    '
)
printf '%s' "$image_base64" | base64 -D -o "$frame_path"
file "$frame_path" | grep -q 'PNG image data' || {
    echo "Steam CEF screenshot response was not a PNG" >&2
    exit 1
}
[ "$(wc -c <"$frame_path")" -ge 64 ] || {
    echo "Steam CEF screenshot PNG is unexpectedly small" >&2
    exit 1
}
cp "$frame_path" "$qr_path"
chmod 600 "$qr_path"

success=1
printf '%s\n' "$qr_path"
echo "temporary owner-authentication image; delete immediately after scanning" >&2
