#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
output=$(mktemp "${TMPDIR:-/tmp}/macroton-steamwebhelper-wrapper.XXXXXX.exe")
trap 'rm -f "$output" "$output.build.log"' EXIT

"$repo_root/scripts/build-steamwebhelper-wrapper.sh" "$output" >"$output.build.log"
file_output=$(file "$output")
printf '%s\n' "$file_output"
printf '%s\n' "$file_output" | grep -F 'PE32+ executable (GUI) x86-64, for MS Windows' >/dev/null
echo "reproducible Steam CEF wrapper build test passed"
