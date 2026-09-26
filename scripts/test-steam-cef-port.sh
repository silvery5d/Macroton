#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
test_root=$(mktemp -d /private/tmp/macroton-cef-port-test.XXXXXX)
cleanup() { rm -rf "$test_root"; }
trap cleanup EXIT HUP INT TERM

cat >"$test_root/webhelper.txt" <<'EOF'
[old] --remote-debugging-address=127.0.0.1 --remote-debugging-port=8091
[remote] --remote-debugging-address=0.0.0.0 --remote-debugging-port=9000
[latest] --remote-debugging-port=8080 --remote-debugging-address=127.0.0.1
EOF
[ "$("$script_dir/steam-cef-port.sh" "$test_root/webhelper.txt")" = 8080 ]

cat >"$test_root/remote-only.txt" <<'EOF'
[remote] --remote-debugging-address=0.0.0.0 --remote-debugging-port=9000
EOF
if "$script_dir/steam-cef-port.sh" "$test_root/remote-only.txt" >/dev/null 2>&1; then
    echo "CEF port helper unexpectedly accepted a non-local debugger" >&2
    exit 1
fi

echo "Steam CEF port tests passed"
