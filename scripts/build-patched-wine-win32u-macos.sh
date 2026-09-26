#!/bin/sh
set -eu

if [ "$#" -lt 1 ] || [ "$#" -gt 2 ]; then
    echo "usage: $0 MOLTENVK_DYLIB [BUILD_ROOT]" >&2
    exit 2
fi

moltenvk_dylib=$1
build_root=${2:-${MACROTON_WINE_VULKAN_BUILD_ROOT:-/private/tmp/macroton-wine-vulkan-11.14}}
repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
case "$moltenvk_dylib" in
    /*) ;;
    *) moltenvk_dylib="$(pwd)/${moltenvk_dylib}" ;;
esac
case "$build_root" in
    /*) ;;
    *) build_root="$(pwd)/${build_root}" ;;
esac
version="11.14"
source_sha256="8565706de64b516c1b0a1abacbcd5835b72f88befebbc548df4e4e0ec77f73a5"
source_url="https://github.com/wine-mirror/wine/archive/refs/tags/wine-${version}.tar.gz"
archive="${build_root}/wine-${version}.tar.gz"
source_dir="${build_root}/wine-wine-${version}"
build_dir="${build_root}/build-win32u"
link_dir="${build_root}/moltenvk-link"
output="${build_dir}/dlls/win32u/win32u.so"
ntdll_output="${build_dir}/dlls/ntdll/ntdll.so"

for tool in arch brew clang curl i686-w64-mingw32-gcc make patch shasum tar x86_64-w64-mingw32-gcc; do
    command -v "$tool" >/dev/null 2>&1 || {
        echo "missing build tool: $tool" >&2
        exit 1
    }
done
if [ ! -f "$moltenvk_dylib" ]; then
    echo "MoltenVK dylib does not exist: ${moltenvk_dylib}" >&2
    exit 1
fi

bison_dir=$(brew --prefix bison)
vulkan_headers_dir=$(brew --prefix vulkan-headers)
if [ ! -x "${bison_dir}/bin/bison" ] || [ ! -d "${vulkan_headers_dir}/include" ]; then
    echo "Homebrew bison and vulkan-headers are required" >&2
    exit 1
fi

mkdir -p "$build_root" "$link_dir"
if [ ! -f "$archive" ]; then
    curl -L --fail --retry 3 -o "$archive" "$source_url"
fi
actual_source_sha256=$(shasum -a 256 "$archive" | awk '{print $1}')
if [ "$actual_source_sha256" != "$source_sha256" ]; then
    echo "Wine source checksum mismatch" >&2
    echo "expected ${source_sha256}" >&2
    echo "actual   ${actual_source_sha256}" >&2
    exit 1
fi

if [ ! -d "$source_dir" ]; then
    tar -xzf "$archive" -C "$build_root"
fi
patch_names="0002-macroton-macos-moltenvk-buffer-alignment.patch 0003-macroton-macos-vulkan-display-init.patch"
if [ "${MACROTON_SKIP_WOW64_DISPATCH_PATCHES:-0}" != "1" ]; then
    patch_names="$patch_names 0004-macroton-wow64-guest-stack-validation.patch 0005-macroton-ntdll-user-rsp-return.patch"
fi
for patch_name in $patch_names
do
    patch_file="${repo_root}/patches/wine-11.14/${patch_name}"
    if patch --dry-run --forward --batch -p1 -d "$source_dir" <"$patch_file" >/dev/null 2>&1; then
        patch --forward --batch -p1 -d "$source_dir" <"$patch_file"
    elif ! patch --dry-run --reverse --batch -p1 -d "$source_dir" <"$patch_file" >/dev/null 2>&1; then
        echo "Wine patch is neither cleanly applicable nor already applied: ${patch_name}" >&2
        exit 1
    fi
done

# RosettaX87 is deliberately opt-in. It requires a separately built and
# signed runtime_loader; applying this hook without that companion is inert,
# but keeping it out of normal builds avoids implying that the experimental
# path is release-ready.
if [ "${MACROTON_ENABLE_ROSETTAX87:-0}" = "1" ]; then
    patch_file="${repo_root}/patches/wine-11.14/0010-macroton-x87sidecar-cooperative.patch"
    if patch --dry-run --forward --batch -p1 -d "$source_dir" <"$patch_file" >/dev/null 2>&1; then
        patch --forward --batch -p1 -d "$source_dir" <"$patch_file"
    elif ! patch --dry-run --reverse --batch -p1 -d "$source_dir" <"$patch_file" >/dev/null 2>&1; then
        echo "Wine RosettaX87 patch is neither cleanly applicable nor already applied" >&2
        exit 1
    fi
fi

# Rosetta 2's signal handling has a race with Wine's far-jump WoW64 thunk.
# Keep the lretq thunk workaround opt-in until it passes the game's A/B gate.
if [ "${MACROTON_ENABLE_ROSETTA2_LRETQ:-0}" = "1" ]; then
    patch_file="${repo_root}/patches/wine-11.14/0011-macroton-rosetta2-lretq-workaround.patch"
    if patch --dry-run --forward --batch -p1 -d "$source_dir" <"$patch_file" >/dev/null 2>&1; then
        patch --forward --batch -p1 -d "$source_dir" <"$patch_file"
    elif ! patch --dry-run --reverse --batch -p1 -d "$source_dir" <"$patch_file" >/dev/null 2>&1; then
        echo "Wine Rosetta2 lretq patch is neither cleanly applicable nor already applied" >&2
        exit 1
    fi
fi

cp "$moltenvk_dylib" "${link_dir}/libMoltenVK.dylib"
mkdir -p "$build_dir"
build_path="${bison_dir}/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin"
if [ ! -f "${build_dir}/Makefile" ]; then
    (
        cd "$build_dir"
        arch -x86_64 env \
            PATH="$build_path" \
            CC="clang -arch x86_64" \
            CXX="clang++ -arch x86_64" \
            CPPFLAGS="-I${vulkan_headers_dir}/include" \
            LDFLAGS="-L${link_dir} -Wl,-rpath,@loader_path/../lib" \
            PKG_CONFIG=false \
            "$source_dir/configure" \
            --build=x86_64-apple-darwin \
            --enable-archs=i386,x86_64 \
            --disable-tests --disable-winebth_sys \
            --without-alsa --without-capi --with-coreaudio --with-cups \
            --without-dbus --without-ffmpeg --without-freetype \
            --without-gettext --without-gettextpo --without-gphoto \
            --without-gnutls --without-gssapi --without-gstreamer \
            --without-inotify --without-krb5 --without-netapi \
            --without-opencl --without-opengl --without-oss --without-pcap \
            --without-pcsclite --with-pthread --without-pulse \
            --without-sane --without-sdl --without-udev --without-usb \
            --without-v4l2 --with-vulkan --without-wayland --without-x
    )
fi

targets="dlls/win32u/win32u.so"
if [ "${MACROTON_ENABLE_ROSETTAX87:-0}" = "1" ] || [ "${MACROTON_BUILD_NTDLL:-0}" = "1" ]; then
    targets="$targets dlls/ntdll/ntdll.so"
fi
if [ "${MACROTON_ENABLE_ROSETTA2_LRETQ:-0}" = "1" ]; then
    targets="$targets dlls/wow64cpu/all"
fi
arch -x86_64 env PATH="$build_path" make -C "$build_dir" -j"${MACROTON_BUILD_JOBS:-10}" $targets
if [ ! -f "$output" ]; then
    echo "Wine build did not produce win32u.so" >&2
    exit 1
fi

echo "wine_source_sha256=${actual_source_sha256}"
echo "win32u=${output}"
echo "win32u_sha256=$(shasum -a 256 "$output" | awk '{print $1}')"
if { [ "${MACROTON_ENABLE_ROSETTAX87:-0}" = "1" ] || [ "${MACROTON_BUILD_NTDLL:-0}" = "1" ]; } && [ -f "$ntdll_output" ]; then
    echo "ntdll=${ntdll_output}"
    echo "ntdll_sha256=$(shasum -a 256 "$ntdll_output" | awk '{print $1}')"
fi
