#!/bin/sh
set -eu

# Reproducible recipe for Macroton's clean WineHQ runtime baseline.
# The produced runtime is intentionally minimal; graphics/media dependencies
# are added only after this baseline passes Macroton's Win32 probe.

version="11.14"
source_sha256="8565706de64b516c1b0a1abacbcd5835b72f88befebbc548df4e4e0ec77f73a5"
source_url="https://github.com/wine-mirror/wine/archive/refs/tags/wine-${version}.tar.gz"
task_root="${MACROTON_BUILD_ROOT:-/private/tmp/macroton-wine-${version}}"
jobs="${MACROTON_BUILD_JOBS:-10}"
archive="${task_root}/wine-${version}.tar.gz"
source_dir="${task_root}/wine-wine-${version}"
build_dir="${task_root}/build"
install_dir="${task_root}/install"

for tool in arch clang curl make shasum tar i686-w64-mingw32-gcc x86_64-w64-mingw32-gcc; do
    command -v "${tool}" >/dev/null 2>&1 || {
        echo "missing build tool: ${tool}" >&2
        exit 1
    }
done

bison_dir="$(brew --prefix bison 2>/dev/null || true)"
if [ -z "${bison_dir}" ] || [ ! -x "${bison_dir}/bin/bison" ]; then
    echo "Homebrew bison 3.8+ is required" >&2
    exit 1
fi

mkdir -p "${task_root}"
if [ ! -f "${archive}" ]; then
    curl -L --fail --retry 3 -o "${archive}" "${source_url}"
fi
actual_sha256="$(shasum -a 256 "${archive}" | awk '{print $1}')"
if [ "${actual_sha256}" != "${source_sha256}" ]; then
    echo "Wine source checksum mismatch" >&2
    echo "expected ${source_sha256}" >&2
    echo "actual   ${actual_sha256}" >&2
    exit 1
fi

if [ ! -d "${source_dir}" ]; then
    tar -xzf "${archive}" -C "${task_root}"
fi
mkdir -p "${build_dir}" "${install_dir}"

build_path="${bison_dir}/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin"
if [ ! -f "${build_dir}/Makefile" ]; then
    (
        cd "${build_dir}"
        arch -x86_64 env \
            PATH="${build_path}" \
            CC="clang -arch x86_64" \
            CXX="clang++ -arch x86_64" \
            PKG_CONFIG=false \
            "${source_dir}/configure" \
            --prefix="${install_dir}" \
            --build=x86_64-apple-darwin \
            --enable-archs=i386,x86_64 \
            --disable-tests \
            --disable-winebth_sys \
            --without-alsa --without-capi --with-coreaudio --with-cups \
            --without-dbus --without-ffmpeg --without-freetype \
            --without-gettext --without-gettextpo --without-gphoto \
            --without-gnutls --without-gssapi --without-gstreamer \
            --without-inotify --without-krb5 --without-netapi \
            --without-opencl --without-opengl --without-oss --without-pcap \
            --without-pcsclite --with-pthread --without-pulse \
            --without-sane --without-sdl --without-udev --without-usb \
            --without-v4l2 --without-vulkan --without-wayland --without-x
    )
fi

arch -x86_64 env PATH="${build_path}" make -C "${build_dir}" -j"${jobs}"
arch -x86_64 env PATH="${build_path}" make -C "${build_dir}" install

echo "Macroton Wine ${version} installed at ${install_dir}"
echo "Probe with: cargo run -- runtime probe-win32 ${install_dir}/bin/wine"
