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
timeout_seconds=${5:-900}
macroton_bin=${MACROTON_BIN:-target/debug/macroton}
max_rss_mib=${MACROTON_CLOUD_MAX_RSS_MIB:-3500}

case "$app_id" in ''|*[!0-9]*) echo "APPID must be unsigned decimal" >&2; exit 2 ;; esac
case "$timeout_seconds" in ''|*[!0-9]*) echo "TIMEOUT_SECONDS must be unsigned decimal" >&2; exit 2 ;; esac
case "$max_rss_mib" in ''|*[!0-9]*) echo "MACROTON_CLOUD_MAX_RSS_MIB must be unsigned decimal" >&2; exit 2 ;; esac
[ "$max_rss_mib" -gt 0 ] || {
    echo "MACROTON_CLOUD_MAX_RSS_MIB must be greater than zero" >&2
    exit 2
}
[ -x "$macroton_bin" ] || {
    echo "Macroton binary is not executable: $macroton_bin" >&2
    exit 1
}

# Cloud reconciliation is an owner-assisted foreground operation. Always
# release this exact AppID's Steam/CEF tree on success, timeout, or Ctrl-C,
# while retaining the session marker for the next retry.
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

"$macroton_bin" steam login "$app_id" \
    --profile "$profile" \
    --data-root "$data_root" \
    --runtime-root "$runtime_root" \
    --max-rss-mib "$max_rss_mib"

cat >&2 <<EOF
Macroton Cloud reconciliation is waiting for the isolated Windows Steam UI.
Resolve the Cloud confirmation there without changing any other AppID, then
leave this command running; it will verify launch_ready=true automatically.
EOF

"$macroton_bin" steam cloud-wait "$app_id" \
    --profile "$profile" \
    --data-root "$data_root" \
    --timeout-seconds "$timeout_seconds" \
    --interval-seconds 2
