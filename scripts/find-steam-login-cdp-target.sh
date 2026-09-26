#!/bin/sh
set -eu

curl_bin=${CURL_BIN:-curl}
jq_bin=${JQ_BIN:-jq}
cef_port=${MACROTON_STEAM_CEF_PORT:-8080}
case "$cef_port" in
    ''|*[!0-9]*) echo "MACROTON_STEAM_CEF_PORT must be numeric" >&2; exit 2 ;;
esac

command -v "$jq_bin" >/dev/null 2>&1 || {
    echo "jq is unavailable: $jq_bin" >&2
    exit 1
}

if [ "${1:-}" = "--input" ]; then
    [ "$#" -eq 2 ] || {
        echo "usage: $0 [--input TARGETS_JSON | ENDPOINT]" >&2
        exit 2
    }
    targets_json=$("$jq_bin" -c . "$2")
else
    [ "$#" -le 1 ] || {
        echo "usage: $0 [--input TARGETS_JSON | ENDPOINT]" >&2
        exit 2
    }
    endpoint=${1:-http://127.0.0.1:$cef_port/json}
    case "$endpoint" in
        http://127.0.0.1:$cef_port/json | http://localhost:$cef_port/json) ;;
        *)
            echo "refusing non-local or unexpected Steam CEF endpoint: $endpoint" >&2
            exit 1
            ;;
    esac
    command -v "$curl_bin" >/dev/null 2>&1 || {
        echo "curl is unavailable: $curl_bin" >&2
        exit 1
    }
    targets_json=$(
        "$curl_bin" --fail --silent --show-error --max-time 5 "$endpoint"
    )
fi

filter='[
  .[]
  | select(.type == "page")
  | select(.title == "Sign in to Steam")
  | select(.webSocketDebuggerUrl | type == "string")
  | select(.webSocketDebuggerUrl | test("^ws://(127\\.0\\.0\\.1|localhost):" + $port + "/"))
]'
count=$(printf '%s' "$targets_json" | "$jq_bin" --arg port "$cef_port" -er "$filter | length")
[ "$count" -eq 1 ] || {
    echo "expected exactly one local Steam login CEF target, found $count" >&2
    exit 1
}

url=$(printf '%s' "$targets_json" | "$jq_bin" --arg port "$cef_port" -er "$filter | .[0].webSocketDebuggerUrl")
path=$(printf '%s' "$url" | sed -E "s#^ws://(127\\.0\\.0\\.1|localhost):$cef_port##")
case "$path" in
    /devtools/page/*) printf '%s\n' "$path" ;;
    *)
        echo "Steam login target returned an unexpected WebSocket path" >&2
        exit 1
        ;;
esac
