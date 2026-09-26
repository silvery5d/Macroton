#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
output=${1:-/private/tmp/macroton-cdp-probe.exe}
compiler=${MINGW64_CC:-x86_64-w64-mingw32-gcc}

command -v "$compiler" >/dev/null 2>&1 || {
    echo "MinGW x86_64 compiler is unavailable: $compiler" >&2
    exit 1
}

SOURCE_DATE_EPOCH=1 "$compiler" \
    -O2 -Wall -Wextra -Werror \
    -Wl,--no-insert-timestamp \
    "$repo_root/tools/windows/cdp_probe.c" \
    -lws2_32 \
    -o "$output"

file "$output" | grep -q 'PE32+ executable.*x86-64' || {
    echo "CDP probe is not a PE32+ x86-64 executable: $output" >&2
    exit 1
}

echo "built reproducible Steam CEF diagnostic: $output"
shasum -a 256 "$output"
