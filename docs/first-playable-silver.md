# First playable target: Silver (AppID 606680)

Silver is Macroton's first real compatibility target because it exercises the
exact product boundary:

- its installed macOS main executable is i386;
- Steam marks the title incompatible with macOS 10.15 or newer;
- the same AppID owns a separate Windows depot (`606681`);
- the Windows requirements explicitly support 32-bit Windows;
- it is a single-player OpenGL 2.0 game without an anti-cheat dependency.

## Verified result

On the current Apple M4 Pro test machine:

1. Macroton classifies the installed Mac build as `legacy-mac-32`.
2. Steam downloaded Windows depot `606681`, manifest
   `1882163069931705787` (18,994 files).
3. `silver.exe` is a PE32 Intel 80386 GUI executable.
4. The pinned public Wine 11.14 runtime passed Macroton's isolated PE32
   `cmd.exe` probe.
5. Windows Steam completed legitimate account authentication inside the
   isolated per-AppID prefix and launched AppID `606680`.
6. Steam recorded the application running and loaded the game's real
   `steam_api.dll`; authentication was not copied or bypassed.
7. Disabling injection of the optional `gameoverlayrenderer.dll` and
   `gameoverlayrenderer64.dll` removed a hang in Overlay audio initialization.
8. The game completed its opening cinematic and rendered the main menu with
   Start, Load, Options, and Quit Game controls.
9. A synthetic left-button event with a real press interval opened the Options
   screen, proving the menu is interactive rather than a static frame.

This is Macroton's first open-runtime, interactive-main-menu proof on Apple
Silicon. It establishes rendering and mouse input. Gameplay, save/load, and
long-session stability remain separate gates.

## Compatibility profile

The verified profile is `compatibility/606680.conf` and selects:

```text
Windows depot: 606681
Runtime ID:    macroton-winehq-vulkan-shadowalias-11.14
Executable:    silver.exe
Renderer:      wined3d-opengl
```

Macroton recognizes both Steam depot layouts used on macOS:

```text
~/Library/Application Support/Steam/steamapps/content/app_606680/depot_606681/
~/Library/Application Support/Steam/Steam.AppBundle/Steam/Contents/MacOS/steamapps/content/app_606680/depot_606681/
```

Macroton's Steam launch policy automatically disables the optional overlay DLL
injection for managed Wine processes. It does not disable `steam_api.dll`,
ownership validation, or Windows Steam authentication.

## Reproduction sequence

After installing the pinned public runtime and obtaining Valve's installer with
a separately verified checksum:

```sh
macroton runtime install \
  runtimes/macroton-winehq-macos-11.14.conf \
  /path/to/wine-devel-11.14-osx64.tar.xz

macroton steam bootstrap 606680 \
  --profile compatibility/606680.conf \
  --installer /path/to/SteamSetup.exe \
  --installer-sha256 EXPECTED_SHA256

macroton plan 606680 \
  --profile compatibility/606680.conf \
  --steam-root /path/to/native/Steam

macroton steam login 606680 \
  --profile compatibility/606680.conf \
  --steam-root /path/to/native/Steam \
  --data-root /path/to/clean/macroton/data

macroton launch 606680 \
  --profile compatibility/606680.conf \
  --steam-root /path/to/native/Steam \
  --data-root /path/to/clean/macroton/data
```

Macroton verifies Valve's installer, installs Windows Steam into an audited
isolated prefix, maps the already downloaded owned depot into a standard Steam
library, and launches through `Steam.exe -applaunch 606680`. Macroton does not
copy account state or bypass Steamworks.

The roots have different authority. `--steam-root` discovers the owned Windows
depot downloaded by native Steam; `--data-root` owns the clean Windows Steam
prefix, login, saves and Cloud state. Its no-copy appmanifest intentionally has
`SizeOnDisk=0`, so `steam install-status` reports `manifest-only`; a ready
`macroton plan` against the native Steam root is the executable/depot/runtime
proof. This prevents a synthetic manifest from being mistaken for a locally
downloaded Windows-prefix installation.

Before a playable rating, validation must still record:

- graphics correctness;
- audio output;
- mouse/keyboard input;
- ability to begin gameplay and save/load;
- whether Steam ownership or Steamworks integration is required at runtime.

## Sources

- Steam store: <https://store.steampowered.com/app/606680/Silver/>
- Depot inventory: <https://steamdb.info/app/606680/depots/>
