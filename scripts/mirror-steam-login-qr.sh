#!/bin/sh
set -eu

[ "$#" -ge 2 ] && [ "$#" -le 4 ] || {
    echo "usage: $0 APPID RUNTIME_ENTRYPOINT [DATA_ROOT] [WAIT_SECONDS]" >&2
    exit 2
}

app_id=$1
runtime=$2
data_root=${3:-"$HOME/Library/Application Support/Macroton"}
wait_seconds=${4:-30}
repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
mirror_source="$repo_root/tools/macos/image_mirror.swift"
mirror_binary=$(mktemp /private/tmp/macroton-image-mirror.XXXXXX)
stable_qr="/private/tmp/macroton-steam-login-qr-$app_id.png"
error_log=$(mktemp /private/tmp/macroton-qr-capture-error.XXXXXX)
capture_path=
mirror_pid=

cleanup() {
    [ -n "${mirror_pid:-}" ] && kill "$mirror_pid" 2>/dev/null || true
    rm -f "$mirror_binary" "$stable_qr" "$error_log" "${capture_path:-}"
}
trap cleanup EXIT HUP INT TERM

command -v swiftc >/dev/null 2>&1 || {
    echo "swiftc is required to show the native QR mirror" >&2
    exit 1
}

swiftc "$mirror_source" -o "$mirror_binary"

while :; do
    if capture_path=$("$repo_root/scripts/capture-steam-login-qr.sh" \
        "$app_id" "$runtime" "$data_root" "$wait_seconds" 2>"$error_log"); then
        cp "$capture_path" "$stable_qr"
        chmod 600 "$stable_qr"
        if [ -z "${mirror_pid:-}" ]; then
            "$mirror_binary" "$stable_qr" >/dev/null 2>&1 &
            mirror_pid=$!
        fi
        rm -f "$capture_path"
        capture_path=
        # The mirror watches the stable file. A fresh capture is useful only
        # after a QR rotation or an explicit refresh, so avoid tight polling.
        sleep 15
    else
        cat "$error_log" >&2 || true
        sleep 3
    fi
done
