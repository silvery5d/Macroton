#!/bin/sh
set -eu

usage() {
    echo "usage: $0 SDL2_DLL [--expect-source MARKER]" >&2
    exit 2
}

[ "$#" -ge 1 ] || usage
dll=$1
shift
expected_source=
if [ "$#" -eq 2 ] && [ "$1" = "--expect-source" ]; then
    expected_source=$2
elif [ "$#" -ne 0 ]; then
    usage
fi

[ -f "$dll" ] || {
    echo "SDL2 DLL is not a regular file: $dll" >&2
    exit 1
}

strings_output=$(strings -a "$dll")
require() {
    needle=$1
    printf '%s\n' "$strings_output" | rg -F -q "$needle" || {
        echo "missing SDL2 renderer signature: $needle" >&2
        exit 1
    }
}

require SDL_CreateRenderer
require SDL_CreateTexture
require SDL_SetRenderTarget
require SDL_RenderCopy
require SDL_RenderPresent

if [ -n "$expected_source" ]; then
    printf '%s\n' "$strings_output" | rg -F -q "$expected_source" || {
        echo "expected SDL2 source/version marker not found: $expected_source" >&2
        exit 1
    }
fi

version_markers=$(printf '%s\n' "$strings_output" | rg -o 'SDL2-[0-9]+\.[0-9]+\.[0-9]+[^[:space:]]*|[0-9]+\.[0-9]+ \(ANGLE [^)]+' | sort -u | tr '\n' ';' | sed 's/;$//')
[ -n "$version_markers" ] || version_markers=unknown
dll_json=$(printf '%s' "$dll" | sed 's/\\/\\\\/g; s/"/\\"/g')
markers_json=$(printf '%s' "$version_markers" | sed 's/\\/\\\\/g; s/"/\\"/g')

printf '{\n  "schema": 1,\n  "dll": "%s",\n  "renderer_signatures": "pass",\n  "version_markers": "%s"\n}\n' \
    "$dll_json" "$markers_json"
