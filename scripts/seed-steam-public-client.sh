#!/bin/sh
set -eu

usage() {
  echo "usage: $0 SOURCE_STEAM_ROOT TARGET_STEAM_ROOT" >&2
  exit 2
}

[ "$#" -eq 2 ] || usage
SOURCE_ROOT=$1
TARGET_ROOT=$2

case "$SOURCE_ROOT" in /*) ;; *) echo "source Steam root must be absolute" >&2; exit 2 ;; esac
case "$TARGET_ROOT" in /*) ;; *) echo "target Steam root must be absolute" >&2; exit 2 ;; esac

[ -f "$SOURCE_ROOT/Steam.exe" ] || { echo "source Steam.exe is missing" >&2; exit 1; }
[ -f "$SOURCE_ROOT/package/steam_client_win32.installed" ] || {
  echo "source installed-client manifest is missing" >&2
  exit 1
}
[ -d "$TARGET_ROOT" ] || { echo "target Steam root is missing" >&2; exit 1; }

SOURCE_CANONICAL=$(cd "$SOURCE_ROOT" && pwd -P)
TARGET_CANONICAL=$(cd "$TARGET_ROOT" && pwd -P)
[ "$SOURCE_CANONICAL" != "$TARGET_CANONICAL" ] || {
  echo "source and target Steam roots must differ" >&2
  exit 1
}

PREFIX=$(cd "$TARGET_ROOT/../../.." && pwd -P)
[ -f "$PREFIX/.macroton-steam-bootstrap.json" ] || {
  echo "target is not inside a Macroton-bootstrapped prefix" >&2
  exit 1
}
PREFIX_DEVICE_HEX=$(printf '%x' "$(stat -f '%d' "$PREFIX")")
PREFIX_INODE_HEX=$(printf '%x' "$(stat -f '%i' "$PREFIX")")
WINE_SERVER_DIR="/tmp/.wine-$(id -u)/server-$PREFIX_DEVICE_HEX-$PREFIX_INODE_HEX"
if [ -S "$WINE_SERVER_DIR/socket" ]; then
  echo "target prefix is active; stop its exact AppID before seeding" >&2
  exit 1
fi
[ ! -f "$TARGET_ROOT/config/loginusers.vdf" ] || {
  echo "target already contains stored-login metadata; refusing to seed" >&2
  exit 1
}
if [ -d "$TARGET_ROOT/userdata" ] && find "$TARGET_ROOT/userdata" -mindepth 1 -maxdepth 1 -type d -name '[0-9]*' | grep -q .; then
  echo "target already contains account userdata; refusing to seed" >&2
  exit 1
fi

# A failed bootstrap update can leave version-addressed package fragments that
# are absent from the verified source. Keeping them beside the seeded manifest
# makes Steam resume an obsolete partial update instead of starting the copied
# client. Move only unmatched package files to a recoverable prefix-local
# quarantine; never touch config, userdata, SteamApps, Cloud, or source files.
QUARANTINE="$PREFIX/.macroton-steam-client-seed-quarantine"
[ ! -e "$QUARANTINE" ] || {
  echo "target seed quarantine already exists; inspect or recover it before reseeding" >&2
  exit 1
}
if [ -d "$TARGET_ROOT/package" ]; then
  find "$TARGET_ROOT/package" -type f | while IFS= read -r TARGET_FILE; do
    RELATIVE=${TARGET_FILE#"$TARGET_ROOT/"}
    if [ ! -f "$SOURCE_ROOT/$RELATIVE" ]; then
      QUARANTINED="$QUARANTINE/$RELATIVE"
      mkdir -p "$(dirname -- "$QUARANTINED")"
      mv "$TARGET_FILE" "$QUARANTINED"
    fi
  done
fi

# Copy only redistributable client payload. The excluded roots contain account,
# entitlement, library, Cloud, log, crash, or host-specific network state.
rsync -a \
  --exclude='/config/' \
  --exclude='/userdata/' \
  --exclude='/logs/' \
  --exclude='/steamapps/' \
  --exclude='/appcache/' \
  --exclude='/depotcache/' \
  --exclude='/dumps/' \
  --exclude='/.crash' \
  --exclude='/ssfn*' \
  --exclude='/update_hosts_cached.vdf' \
  "$SOURCE_ROOT/" "$TARGET_ROOT/"

[ ! -f "$TARGET_ROOT/config/loginusers.vdf" ] || {
  echo "stored-login metadata appeared in target after copy" >&2
  exit 1
}
if [ -d "$TARGET_ROOT/userdata" ] && find "$TARGET_ROOT/userdata" -mindepth 1 -maxdepth 1 -type d -name '[0-9]*' | grep -q .; then
  echo "account userdata appeared in target after copy" >&2
  exit 1
fi

SOURCE_MANIFEST_SHA=$(shasum -a 256 "$SOURCE_ROOT/package/steam_client_win32.installed" | awk '{print $1}')
TARGET_MANIFEST_SHA=$(shasum -a 256 "$TARGET_ROOT/package/steam_client_win32.installed" | awk '{print $1}')
[ "$SOURCE_MANIFEST_SHA" = "$TARGET_MANIFEST_SHA" ] || {
  echo "target installed-client manifest does not match source" >&2
  exit 1
}
STEAM_EXE_SHA=$(shasum -a 256 "$TARGET_ROOT/Steam.exe" | awk '{print $1}')

MARKER="$PREFIX/.macroton-steam-client-seed"
TMP_MARKER="$MARKER.tmp.$$"
{
  echo "schema=1"
  echo "source_manifest_sha256=$SOURCE_MANIFEST_SHA"
  echo "steam_exe_sha256=$STEAM_EXE_SHA"
  echo "excluded=config,userdata,logs,steamapps,appcache,depotcache,dumps,ssfn,update_hosts_cached.vdf"
} > "$TMP_MARKER"
mv "$TMP_MARKER" "$MARKER"

echo "Steam public client payload seeded"
echo "Target: $TARGET_ROOT"
echo "Manifest SHA-256: $TARGET_MANIFEST_SHA"
echo "Steam.exe SHA-256: $STEAM_EXE_SHA"
echo "Account state copied: no"
if [ -d "$QUARANTINE" ]; then
  echo "Stale package files: quarantined at $QUARANTINE"
else
  echo "Stale package files: none"
fi
echo "Evidence: $MARKER"
