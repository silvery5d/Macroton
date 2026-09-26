#!/bin/sh
set -eu

if [ "$#" -lt 2 ] || [ "$#" -gt 3 ]; then
    echo "usage: $0 WINE_ENTRYPOINT RENDERER [PREFIX]" >&2
    echo "renderer: wined3d or dxmt" >&2
    exit 2
fi

wine=$1
renderer=$2
prefix=${3:-/private/tmp/macroton-d3d10-microbench-${renderer}}
project_root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
benchmark="${project_root}/target/probes/d3d10_microbench.exe"

cleanup() {
    WINEPREFIX="${prefix}" WINEDEBUG=-all "$(dirname "${wine}")/wineserver" -k >/dev/null 2>&1 || true
}
trap cleanup EXIT HUP INT TERM

case "${renderer}" in
    wined3d) overrides="" ;;
    dxmt) overrides="dxgi,d3d11,d3d10core=b" ;;
    *) echo "unknown renderer: ${renderer}" >&2; exit 2 ;;
esac

mkdir -p "${project_root}/target/probes" "${prefix}"
i686-w64-mingw32-gcc -Wall -Wextra -Werror -O2 \
    "${project_root}/tools/d3d10_microbench.c" \
    -o "${benchmark}" -ld3d10 -ldxgi -luuid

env \
    WINEPREFIX="${prefix}" \
    WINEDEBUG=-all \
    WINEDLLOVERRIDES="${overrides}" \
    MVK_CONFIG_LOG_LEVEL=0 \
    DXMT_LOG_LEVEL=none \
    "${wine}" "${benchmark}"
