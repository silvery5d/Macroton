#!/bin/sh
# Compare host CGL with Win32/Win64 WGL in one explicitly selected Wine prefix.
set -eu

if [ "$#" -ne 2 ]; then
    echo "usage: sh scripts/check-wgl-bridge.sh WINE_BINARY WINEPREFIX" >&2
    exit 2
fi

wine_binary=$1
wine_prefix=$2
if [ ! -x "$wine_binary" ] || [ ! -d "$wine_prefix" ]; then
    echo "Wine binary must be executable and WINEPREFIX must exist" >&2
    exit 2
fi

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
project_dir=$(dirname "$script_dir")
probe_dir=$(mktemp -d "${TMPDIR:-/private/tmp}/macroton-wgl-probe.XXXXXX")
trap 'rm -f "$probe_dir/host-cgl" "$probe_dir/win32.exe" "$probe_dir/win64.exe"; rmdir "$probe_dir"' EXIT HUP INT TERM

clang -arch x86_64 -O2 -Wno-deprecated-declarations \
    -o "$probe_dir/host-cgl" "$project_dir/tools/macos_cgl_renderer_probe.c" \
    -framework ApplicationServices -framework OpenGL
i686-w64-mingw32-gcc -O2 -Wall -Wextra \
    -o "$probe_dir/win32.exe" "$project_dir/tools/wgl_pixel_format_probe.c" \
    -lgdi32 -luser32
x86_64-w64-mingw32-gcc -O2 -Wall -Wextra \
    -o "$probe_dir/win64.exe" "$project_dir/tools/wgl_pixel_format_probe.c" \
    -lgdi32 -luser32

echo 'Host CGL (x86_64/Rosetta):'
arch -x86_64 "$probe_dir/host-cgl"
echo 'Wine WGL (Win32):'
win32_status=0
WINEPREFIX="$wine_prefix" WINEDEBUG=-all MVK_CONFIG_LOG_LEVEL=0 \
    "$wine_binary" "$probe_dir/win32.exe" || win32_status=$?
echo 'Wine WGL (Win64):'
win64_status=0
WINEPREFIX="$wine_prefix" WINEDEBUG=-all MVK_CONFIG_LOG_LEVEL=0 \
    "$wine_binary" "$probe_dir/win64.exe" || win64_status=$?

if [ "$win32_status" -ne 0 ] || [ "$win64_status" -ne 0 ]; then
    echo "WGL bridge failed: Win32 exit=$win32_status Win64 exit=$win64_status" >&2
    exit 1
fi
echo 'WGL bridge has usable pixel formats in both architectures'
