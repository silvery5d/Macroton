#!/bin/sh
set -eu

# Architecture proof for Macroton's native Apple Silicon Wine host.
#
# This build deliberately omits graphics and most media dependencies. It must
# first prove that Wine's Unix-side host is arm64 and that an ARM64 Windows
# payload runs natively. Running an i386 payload additionally requires an x86
# emulator implementing Wine's WoW64 emulation interface.

version="11.14"
source_sha256="8565706de64b516c1b0a1abacbcd5835b72f88befebbc548df4e4e0ec77f73a5"
source_url="https://github.com/wine-mirror/wine/archive/refs/tags/wine-${version}.tar.gz"
task_root="${MACROTON_ARM64_BUILD_ROOT:-/private/tmp/macroton-wine-arm64-${version}}"
jobs="${MACROTON_BUILD_JOBS:-10}"
archive="${task_root}/wine-${version}.tar.gz"
source_dir="${task_root}/wine-wine-${version}"
build_dir="${task_root}/build"
install_dir="${task_root}/install"

if [ "$(uname -m)" != "arm64" ]; then
    echo "this recipe must run on an Apple Silicon Mac" >&2
    exit 1
fi

for tool in brew clang clang++ curl make shasum tar i686-w64-mingw32-gcc; do
    command -v "${tool}" >/dev/null 2>&1 || {
        echo "missing build tool: ${tool}" >&2
        exit 1
    }
done

bison_dir="$(brew --prefix bison 2>/dev/null || true)"
llvm_dir="$(brew --prefix llvm 2>/dev/null || true)"
if [ -z "${bison_dir}" ] || [ ! -x "${bison_dir}/bin/bison" ]; then
    echo "Homebrew bison 3.8+ is required" >&2
    exit 1
fi
if [ -z "${llvm_dir}" ] || [ ! -x "${llvm_dir}/bin/clang" ]; then
    echo "Homebrew LLVM with the Windows ARM64 target is required" >&2
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

# Keep Apple's compiler for Mach-O host objects. Homebrew LLVM is selected
# only for PE/COFF ARM64 objects; using it as the host compiler can infer a
# non-existent CommandLineTools SDK on machines using the full Xcode SDK.
build_path="${bison_dir}/bin:${llvm_dir}/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin"
if [ ! -f "${build_dir}/Makefile" ]; then
    (
        cd "${build_dir}"
        env \
            PATH="${build_path}" \
            CC="/usr/bin/clang -arch arm64" \
            CXX="/usr/bin/clang++ -arch arm64" \
            aarch64_CC="${llvm_dir}/bin/clang" \
            aarch64_CXX="${llvm_dir}/bin/clang++" \
            PKG_CONFIG=false \
            "${source_dir}/configure" \
            --prefix="${install_dir}" \
            --build=arm64-apple-darwin \
            --enable-archs=i386,aarch64 \
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

env PATH="${build_path}" make -C "${build_dir}" -j"${jobs}"
env PATH="${build_path}" make -C "${build_dir}" install

echo "Macroton native Wine ${version} installed at ${install_dir}"
echo "Host proof: file ${install_dir}/bin/wine"
echo "ARM64 payload: ${install_dir}/lib/wine/aarch64-windows/cmd.exe"
