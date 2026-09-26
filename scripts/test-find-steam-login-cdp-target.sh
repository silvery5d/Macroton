#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
test_root=$(mktemp -d /private/tmp/macroton-cdp-target-test.XXXXXX)
trap 'rm -rf "$test_root"' EXIT HUP INT TERM

printf '%s\n' '[
  {"type":"page","title":"Store","webSocketDebuggerUrl":"ws://127.0.0.1:8080/devtools/page/store"},
  {"type":"page","title":"Sign in to Steam","webSocketDebuggerUrl":"ws://127.0.0.1:8080/devtools/page/login"}
]' >"$test_root/unique.json"
[ "$("$repo_root/scripts/find-steam-login-cdp-target.sh" --input "$test_root/unique.json")" = "/devtools/page/login" ]

printf '%s\n' '[
  {"type":"page","title":"Sign in to Steam","webSocketDebuggerUrl":"ws://127.0.0.1:8080/devtools/page/one"},
  {"type":"page","title":"Sign in to Steam","webSocketDebuggerUrl":"ws://localhost:8080/devtools/page/two"}
]' >"$test_root/ambiguous.json"
if "$repo_root/scripts/find-steam-login-cdp-target.sh" --input "$test_root/ambiguous.json" >/dev/null 2>&1; then
    echo "ambiguous Steam login targets were accepted" >&2
    exit 1
fi

printf '%s\n' '[
  {"type":"page","title":"Sign in to Steam","webSocketDebuggerUrl":"ws://example.com:8080/devtools/page/login"}
]' >"$test_root/external.json"
if "$repo_root/scripts/find-steam-login-cdp-target.sh" --input "$test_root/external.json" >/dev/null 2>&1; then
    echo "external CEF target was accepted" >&2
    exit 1
fi

echo "Steam login CEF target selection tests passed"
