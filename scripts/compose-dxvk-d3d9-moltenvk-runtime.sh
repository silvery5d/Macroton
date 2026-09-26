#!/bin/sh
set -eu

if [ "$#" -ne 6 ]; then
    echo "usage: $0 BASE_WINE_ROOT PATCHED_WIN32U PATCHED_D3D9_DLL MOLTENVK_DYLIB COMPONENT_VERSION DESTINATION" >&2
    exit 2
fi

base_runtime=$1
patched_win32u=$2
d3d9_dll=$3
moltenvk_dylib=$4
component_version=$5
destination=$6

case "${component_version}" in
    dxvk-[0-9]*-moltenvk-[0-9]*) ;;
    *)
        echo "component version must look like dxvk-X-moltenvk-Y: ${component_version}" >&2
        exit 2
        ;;
esac

for tool in codesign cp mkdir mktemp mv shasum; do
    command -v "${tool}" >/dev/null 2>&1 || {
        echo "missing tool: ${tool}" >&2
        exit 1
    }
done
if [ ! -x "${base_runtime}/bin/wine" ]; then
    echo "base runtime has no executable bin/wine: ${base_runtime}" >&2
    exit 1
fi
for input in "${patched_win32u}" "${d3d9_dll}" "${moltenvk_dylib}"; do
    if [ ! -f "${input}" ]; then
        echo "component file does not exist: ${input}" >&2
        exit 1
    fi
done
if [ -e "${destination}" ]; then
    echo "destination already exists; refusing to overwrite: ${destination}" >&2
    exit 1
fi

staging=$(mktemp -d "${destination}.staging.XXXXXX")
cleanup() {
    rm -rf "${staging}"
}
trap cleanup EXIT HUP INT TERM

if ! cp -cR "${base_runtime}/." "${staging}/" 2>/dev/null; then
    cp -R "${base_runtime}/." "${staging}/"
fi

d3d9_relative="i386-windows/d3d9.dll"
moltenvk_relative="x86_64-unix/libMoltenVK.dylib"
win32u_relative="x86_64-unix/win32u.so"
cp "${patched_win32u}" "${staging}/lib/wine/${win32u_relative}"
cp "${d3d9_dll}" "${staging}/lib/wine/${d3d9_relative}"
cp "${moltenvk_dylib}" "${staging}/lib/wine/${moltenvk_relative}"
# The copied dylib is a new bundle member. Ad-hoc signing makes the exact
# runtime-local artifact loadable without borrowing another app's signature.
codesign --force --sign - "${staging}/lib/wine/${moltenvk_relative}"

mkdir -p "${staging}/share/macroton/components"
manifest="${staging}/share/macroton/components/dxvk-d3d9-moltenvk-${component_version}.conf"
d3d9_sha256=$(shasum -a 256 "${staging}/lib/wine/${d3d9_relative}" | awk '{print $1}')
moltenvk_sha256=$(shasum -a 256 "${staging}/lib/wine/${moltenvk_relative}" | awk '{print $1}')
win32u_sha256=$(shasum -a 256 "${staging}/lib/wine/${win32u_relative}" | awk '{print $1}')
{
    echo "schema=1"
    echo "id=dxvk-d3d9-moltenvk-${component_version}"
    echo "dxvk_source=https://github.com/doitsujin/dxvk/releases/tag/v1.10.3"
    echo "dxvk_source_sha256=9dad9868e0d0b485051a5d467c11ca796adef5537a688ae68adb358a90f465a2"
    echo "moltenvk_source=https://github.com/KhronosGroup/MoltenVK/tree/v1.4.2"
    echo "moltenvk_commit=db66022459ffb663aa2b50f6b018bc2e124f5edf"
    echo "spirv_cross_commit=6c09849fe88c48eaed08413aa022aaa136a3a057"
    echo "wine_source_sha256=8565706de64b516c1b0a1abacbcd5835b72f88befebbc548df4e4e0ec77f73a5"
    echo "wine_patches=0002-macroton-macos-moltenvk-buffer-alignment,0003-macroton-macos-vulkan-display-init"
    echo "file.${win32u_relative}=${win32u_sha256}"
    echo "file.${d3d9_relative}=${d3d9_sha256}"
    echo "file.${moltenvk_relative}=${moltenvk_sha256}"
} >"${manifest}"

mv "${staging}" "${destination}"
trap - EXIT HUP INT TERM
echo "Composed Macroton D3D9-to-Metal runtime: ${destination}"
echo "Component manifest: ${destination}/share/macroton/components/$(basename "${manifest}")"
