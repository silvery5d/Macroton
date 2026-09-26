#!/bin/sh
set -eu

appid=${1:?usage: install-aoe3-win32-dependencies.sh APPID [DATA_ROOT] [RUNTIME_ROOT]}
data_root=${2:-"$HOME/Library/Application Support/Macroton"}
runtime_root=${3:-"$HOME/Library/Application Support/Macroton/runtimes"}
prefix="$data_root/games/$appid/prefix"
wine="$runtime_root/macroton-winehq-macos-11.14/Wine Devel.app/Contents/Resources/wine/bin/wine"

test -d "$prefix" || { echo "missing Macroton prefix: $prefix" >&2; exit 1; }
test -x "$wine" || { echo "missing Macroton Wine runtime: $wine" >&2; exit 1; }
command -v winetricks >/dev/null 2>&1 || {
    echo "winetricks is required; install it with Homebrew" >&2
    exit 1
}

if [ "$appid" = 105450 ]; then
    command -v curl >/dev/null 2>&1 || {
        echo "curl is required to install AoE3's x86 .NET prerequisite" >&2
        exit 1
    }
fi

WINEPREFIX="$prefix" WINE="$wine" WINESERVER="${wine}server" \
    winetricks --unattended corefonts mfc42 directplay l3codecx quartz dsound

# AoE3's PE32 executable needs the x86 .NET 2.0 registration.  On modern
# Wine WoW64, winetricks' dotnet20 verb selects the x64 installer, so fetch and
# run the x86 SP2 installer explicitly.  Keep the artifact pinned and cached
# under the user's winetricks cache; never download into the game tree.
if [ "$appid" = 105450 ]; then
    dotnet_cache="${HOME}/.cache/winetricks/dotnet20/NetFx20SP2_x86.exe"
    dotnet_url="https://web.archive.org/web/20210329003118/http://download.microsoft.com/download/c/6/e/c6e88215-0178-4c6c-b5f3-158ff77b1f38/NetFx20SP2_x86.exe"
    dotnet_sha256="6e3f363366e7d0219b7cb269625a75d410a5c80d763cc3d73cf20841084e851f"
    mkdir -p "$(dirname "$dotnet_cache")"
    if [ ! -f "$dotnet_cache" ]; then
        curl -L --fail --retry 3 -o "$dotnet_cache" "$dotnet_url"
    fi
    actual_sha256="$(shasum -a 256 "$dotnet_cache" | awk '{print $1}')"
    [ "$actual_sha256" = "$dotnet_sha256" ] || {
        echo "AoE3 x86 .NET installer checksum mismatch" >&2
        exit 1
    }
    set +e
    WINEPREFIX="$prefix" "$wine" "$dotnet_cache" /q
    installer_rc=$?
    set -e
    # The .NET bootstrapper returns 236 when installation completed but asks
    # for a Windows reboot. In a disposable Wine prefix that is a successful
    # install result, not an error. Any other code remains fatal.
    case "$installer_rc" in
        0|236) ;;
        *)
            echo "AoE3 x86 .NET installer failed with exit code $installer_rc" >&2
            exit "$installer_rc"
            ;;
    esac
    WINEPREFIX="$prefix" "$wine" wineboot --update >/dev/null 2>&1 || true
    WINEPREFIX="$prefix" "${wine}server" -k >/dev/null 2>&1 || true
    sleep 1
    WINEPREFIX="$prefix" "${wine}server" -k >/dev/null 2>&1 || true
fi

test -f "$prefix/drive_c/windows/syswow64/mfc42.dll" || {
    echo "MFC42 installation did not produce syswow64/mfc42.dll" >&2
    exit 1
}
test -f "$prefix/drive_c/windows/Fonts/cour.ttf" || {
    echo "Core Fonts installation did not produce Courier New (cour.ttf)" >&2
    exit 1
}
for legacy_file in \
    "$prefix/drive_c/windows/syswow64/dplayx.dll" \
    "$prefix/drive_c/windows/syswow64/l3codecx.ax" \
    "$prefix/drive_c/windows/syswow64/quartz.dll" \
    "$prefix/drive_c/windows/syswow64/dsound.dll"; do
    test -f "$legacy_file" || {
        echo "AoE3 legacy media/input dependency missing: $legacy_file" >&2
        exit 1
    }
done
if [ "$appid" = 105450 ]; then
    WINEPREFIX="$prefix" "$wine" reg query \
        'HKLM\Software\Wow6432Node\Microsoft\NET Framework Setup\NDP\v2.0.50727' \
        2>/dev/null | grep -q 'Install.*0x1' || {
        echo "AoE3 x86 .NET 2.0 registration is missing" >&2
        exit 1
    }
fi
echo "AoE3 Win32 dependencies ready in $prefix"
