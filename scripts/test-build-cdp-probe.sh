#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
test_root=$(mktemp -d /private/tmp/macroton-cdp-probe-test.XXXXXX)
trap 'rm -rf "$test_root"' EXIT HUP INT TERM

"$repo_root/scripts/build-cdp-probe.sh" "$test_root/first.exe" >/dev/null
"$repo_root/scripts/build-cdp-probe.sh" "$test_root/second.exe" >/dev/null
cmp "$test_root/first.exe" "$test_root/second.exe"

echo "reproducible CDP probe build test passed"
