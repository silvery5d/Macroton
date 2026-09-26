#!/bin/sh
set -eu

if [ "$#" -ne 2 ]; then
    echo "usage: $0 /path/to/wine /path/to/prefix" >&2
    exit 64
fi

WINE_BIN=$1
PREFIX=$2
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
FIXTURE="$ROOT/tests/fixtures/unhandled-exception.c"
COMPILER=${MACROTON_MINGW32_CC:-i686-w64-mingw32-gcc}
WORK=$(mktemp -d "${TMPDIR:-/tmp}/macroton-crash-guard.XXXXXX")
trap 'rm -rf "$WORK"' EXIT HUP INT TERM

if ! command -v "$COMPILER" >/dev/null 2>&1; then
    echo "missing PE32 compiler: $COMPILER" >&2
    exit 69
fi
if [ ! -x "$WINE_BIN" ]; then
    echo "Wine entrypoint is not executable: $WINE_BIN" >&2
    exit 66
fi
if [ ! -d "$PREFIX" ]; then
    echo "Wine prefix does not exist: $PREFIX" >&2
    exit 66
fi
if pgrep -x winedbg >/dev/null 2>&1; then
    echo "pre-existing winedbg process makes the test ambiguous" >&2
    exit 70
fi

for VIEW in 32 64; do
    QUERY=$(WINEPREFIX="$PREFIX" "$WINE_BIN" reg.exe query \
        'HKLM\Software\Microsoft\Windows NT\CurrentVersion\AeDebug' "/reg:$VIEW")
    printf '%s\n' "$QUERY" | grep -F \
        'C:\windows\system32\macroton-disabled-debugger.exe %ld %ld' >/dev/null
done

"$COMPILER" -Os -s "$FIXTURE" -o "$WORK/unhandled-exception.exe"
set +e
WINEPREFIX="$PREFIX" "$WINE_BIN" "$WORK/unhandled-exception.exe" \
    >"$WORK/stdout.log" 2>"$WORK/stderr.log"
EXIT_CODE=$?
set -e

if [ "$EXIT_CODE" -eq 0 ]; then
    echo "crash fixture unexpectedly exited successfully" >&2
    exit 1
fi
sleep 1
if pgrep -x winedbg >/dev/null 2>&1; then
    echo "crash guard failed: winedbg was spawned" >&2
    exit 1
fi

echo "crash_guard=passed"
echo "fixture_exit_code=$EXIT_CODE"
echo "winedbg_processes=0"
